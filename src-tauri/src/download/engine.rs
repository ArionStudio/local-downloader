use super::{
    AuthSource, BrowserAuthSource, FormatAnalysis, FormatOption, FormatSelection, JobStatus,
    Pipeline, Preset, StartDownloadRequest,
};
use crate::{commands, process_control, redaction, tools};
use regex::Regex;
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    thread,
    time::Duration,
};
use tauri::AppHandle;

enum ProcessLine {
    Stdout(String),
    Stderr(String),
}

#[derive(Debug, Clone)]
struct AuthAttempt {
    auth: AuthSource,
    label: String,
}

pub fn analyze_formats(
    app: &AppHandle,
    url: &str,
    auth: &AuthSource,
) -> Result<FormatAnalysis, String> {
    let yt_dlp = tools::find_tool(app, "yt-dlp").ok_or_else(|| {
        "yt-dlp was not found. Bundle it in src-tauri/binaries or install it locally.".to_string()
    })?;
    let attempts = auth_attempts(auth, None);
    let mut last_error = None;

    for attempt in attempts {
        let mut args = vec![
            "-J".to_string(),
            "--no-warnings".to_string(),
            "--no-playlist".to_string(),
        ];
        append_auth_args(&mut args, &attempt.auth);
        args.push(url.to_string());

        let output = Command::new(&yt_dlp)
            .args(args)
            .output()
            .map_err(|error| format!("Could not start yt-dlp: {error}"))?;

        if output.status.success() {
            let value: Value =
                serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
            return Ok(parse_format_analysis(value));
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        last_error = Some(format!(
            "{} failed: {}",
            attempt.label,
            redaction::sanitize_log_line(stderr.trim())
        ));
    }

    Err(last_error.unwrap_or_else(|| "Could not inspect available formats.".to_string()))
}

pub fn run_download(
    app: AppHandle,
    state: commands::AppState,
    job_id: String,
    input: StartDownloadRequest,
    preset: Preset,
    fallback_auth: Option<AuthSource>,
    cancel_flag: Arc<AtomicBool>,
) {
    if let Err(error) = run_download_inner(
        app.clone(),
        state.clone(),
        job_id.clone(),
        input,
        preset,
        fallback_auth,
        cancel_flag,
    ) {
        fail_job(&app, &state, &job_id, &error);
    }
    state.remove_cancel_flag(&job_id);
}

fn run_download_inner(
    app: AppHandle,
    state: commands::AppState,
    job_id: String,
    mut input: StartDownloadRequest,
    preset: Preset,
    fallback_auth: Option<AuthSource>,
    cancel_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    update_phase(
        &app,
        &state,
        &job_id,
        JobStatus::Resolving,
        2.0,
        "Resolving",
    )?;
    if cancel_flag.load(Ordering::SeqCst) {
        mark_canceled(&app, &state, &job_id)?;
        return Ok(());
    }

    let yt_dlp = tools::find_tool(&app, "yt-dlp").ok_or_else(|| {
        "yt-dlp was not found. Bundle it in src-tauri/binaries or install it locally.".to_string()
    })?;
    let ffmpeg = tools::find_tool(&app, "ffmpeg");

    if matches!(&preset.pipeline, Pipeline::HttpResolveThenDownload) {
        log(
            &app,
            &state,
            &job_id,
            "info",
            "Using yt-dlp generic extraction before HTTP stream fallback.",
        )?;
    }

    let ffmpeg_location = ffmpeg.as_ref().map(|path| path.display().to_string());
    let attempts = auth_attempts(&input.auth, fallback_auth);
    let mut last_error = None;

    for (index, attempt) in attempts.iter().enumerate() {
        input.auth = attempt.auth.clone();
        let phase = if index == 0 {
            format!("Starting yt-dlp ({})", attempt.label)
        } else {
            format!("Retrying yt-dlp ({})", attempt.label)
        };

        match run_yt_dlp_attempt(
            &app,
            &state,
            &job_id,
            &input,
            &preset,
            ffmpeg_location.clone(),
            yt_dlp.as_path(),
            &cancel_flag,
            &phase,
        )? {
            AttemptOutcome::Succeeded | AttemptOutcome::Canceled => return Ok(()),
            AttemptOutcome::Failed(error) => {
                last_error = Some(error.clone());
                if let Some(next) = attempts.get(index + 1) {
                    log(
                        &app,
                        &state,
                        &job_id,
                        "warn",
                        &format!("{} failed; retrying with {}.", attempt.label, next.label),
                    )?;
                }
            }
        }
    }

    mark_failed(
        &app,
        &state,
        &job_id,
        &last_error.unwrap_or_else(|| "yt-dlp failed.".to_string()),
    )?;
    Ok(())
}

enum AttemptOutcome {
    Succeeded,
    Failed(String),
    Canceled,
}

#[allow(clippy::too_many_arguments)]
fn run_yt_dlp_attempt(
    app: &AppHandle,
    state: &commands::AppState,
    job_id: &str,
    input: &StartDownloadRequest,
    preset: &Preset,
    ffmpeg_location: Option<String>,
    yt_dlp: &std::path::Path,
    cancel_flag: &Arc<AtomicBool>,
    phase: &str,
) -> Result<AttemptOutcome, String> {
    let args = build_yt_dlp_args(input, preset, ffmpeg_location);
    update_phase(app, state, job_id, JobStatus::Downloading, 5.0, phase)?;

    let mut command = Command::new(yt_dlp);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process_control::isolate_process_group(&mut command);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start yt-dlp: {error}"))?;
    state.set_process(job_id, child.id())?;
    let _process_registration = ProcessRegistration { state, job_id };

    let (sender, receiver) = mpsc::channel::<ProcessLine>();

    if let Some(stdout) = child.stdout.take() {
        let sender = sender.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = sender.send(ProcessLine::Stdout(line));
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let sender = sender.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = sender.send(ProcessLine::Stderr(line));
            }
        });
    }

    loop {
        if cancel_flag.load(Ordering::SeqCst) {
            process_control::force_kill_process_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
            mark_canceled(app, state, job_id)?;
            return Ok(AttemptOutcome::Canceled);
        }

        drain_process_lines(app, state, job_id, &receiver, cancel_flag, 64)?;

        if cancel_flag.load(Ordering::SeqCst) {
            process_control::force_kill_process_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
            mark_canceled(app, state, job_id)?;
            return Ok(AttemptOutcome::Canceled);
        }

        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Could not read yt-dlp status: {error}"))?
        {
            while let Ok(line) = receiver.try_recv() {
                handle_process_line(app, state, job_id, line)?;
            }

            if cancel_flag.load(Ordering::SeqCst) {
                mark_canceled(app, state, job_id)?;
                return Ok(AttemptOutcome::Canceled);
            }

            if status.success() {
                let job = state.update_job(job_id, |job| {
                    job.status = JobStatus::Completed;
                    job.progress = 100.0;
                    job.phase = "Completed".to_string();
                    job.error_message = None;
                })?;
                commands::emit_job(app, &job, None);
                return Ok(AttemptOutcome::Succeeded);
            }

            return Ok(AttemptOutcome::Failed(format!(
                "yt-dlp exited with status {status}"
            )));
        }

        thread::sleep(Duration::from_millis(180));
    }
}

fn drain_process_lines(
    app: &AppHandle,
    state: &commands::AppState,
    job_id: &str,
    receiver: &Receiver<ProcessLine>,
    cancel_flag: &Arc<AtomicBool>,
    limit: usize,
) -> Result<(), String> {
    for _ in 0..limit {
        if cancel_flag.load(Ordering::SeqCst) {
            break;
        }

        match receiver.try_recv() {
            Ok(line) => handle_process_line(app, state, job_id, line)?,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
    Ok(())
}

struct ProcessRegistration<'a> {
    state: &'a commands::AppState,
    job_id: &'a str,
}

impl Drop for ProcessRegistration<'_> {
    fn drop(&mut self) {
        self.state.clear_process(self.job_id);
    }
}

fn build_yt_dlp_args(
    input: &StartDownloadRequest,
    preset: &Preset,
    ffmpeg_location: Option<String>,
) -> Vec<String> {
    let mut args = vec![
        "--newline".to_string(),
        "--no-color".to_string(),
        "--progress".to_string(),
    ];

    if let Some(output_dir) = input.output_dir.as_ref().filter(|value| !value.is_empty()) {
        args.push("-P".to_string());
        args.push(output_dir.clone());
    }

    args.push("-o".to_string());
    args.push(
        input
            .filename_template
            .clone()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "%(title).180B [%(id)s].%(ext)s".to_string()),
    );

    if let Some(ffmpeg_location) = ffmpeg_location {
        args.push("--ffmpeg-location".to_string());
        args.push(ffmpeg_location);
    }

    let format_selector = format_selector(input);
    args.extend(["-f".to_string(), format_selector]);

    if matches!(
        input.advanced.as_ref().map(|advanced| &advanced.format),
        Some(FormatSelection::AudioOnly)
    ) {
        args.extend([
            "--extract-audio".to_string(),
            "--audio-format".to_string(),
            "mp3".to_string(),
        ]);
    } else {
        args.extend(["--merge-output-format".to_string(), "mp4".to_string()]);
    }

    if preset.id.contains("multiple") {
        args.push("--yes-playlist".to_string());
    } else {
        args.push("--no-playlist".to_string());
    }

    if let Some(segment) = input
        .advanced
        .as_ref()
        .and_then(|advanced| advanced.segment.as_ref().filter(|segment| segment.enabled))
    {
        if let Some(section) = download_section(segment.start_seconds, segment.end_seconds) {
            args.push("--download-sections".to_string());
            args.push(section);
            args.push("--force-keyframes-at-cuts".to_string());
        }
    }

    append_auth_args(&mut args, &input.auth);

    args.push(input.url.clone());
    args
}

fn format_selector(input: &StartDownloadRequest) -> String {
    match input.advanced.as_ref().map(|advanced| &advanced.format) {
        Some(FormatSelection::AudioOnly) => "ba/bestaudio/best".to_string(),
        Some(FormatSelection::VideoOnly { format_id }) => format_id
            .as_ref()
            .filter(|id| !id.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| "bv*/bestvideo*/best".to_string()),
        Some(FormatSelection::Format { format_id }) if !format_id.trim().is_empty() => {
            format!("{format_id}+ba/{format_id}/best")
        }
        _ => "bv*+ba/bestvideo*+bestaudio/best".to_string(),
    }
}

fn download_section(start_seconds: f64, end_seconds: Option<f64>) -> Option<String> {
    let start_seconds = start_seconds.max(0.0);
    let end_seconds = end_seconds.filter(|end| *end > start_seconds);
    let start = section_time(start_seconds);
    let end = end_seconds.map(section_time).unwrap_or_default();
    Some(format!("*{start}-{end}"))
}

fn section_time(seconds: f64) -> String {
    format!("{seconds:.3}")
}

fn append_auth_args(args: &mut Vec<String>, auth: &AuthSource) {
    match auth {
        AuthSource::None => {}
        AuthSource::Browser { .. } => {
            if let Some(source) = browser_sources(auth).into_iter().next() {
                args.push("--cookies-from-browser".to_string());
                args.push(browser_cookie_arg(&source));
            }
        }
        AuthSource::CookieFile { path } => {
            args.push("--cookies".to_string());
            args.push(path.clone());
        }
    }
}

fn auth_attempts(primary: &AuthSource, fallback: Option<AuthSource>) -> Vec<AuthAttempt> {
    let mut attempts = expand_auth_source(primary);

    if matches!(primary, AuthSource::None) {
        if let Some(fallback) = fallback {
            attempts.extend(expand_auth_source(&fallback));
        }
    }

    if attempts.is_empty() {
        attempts.push(AuthAttempt {
            auth: AuthSource::None,
            label: "no auth".to_string(),
        });
    }

    dedupe_auth_attempts(attempts)
}

fn expand_auth_source(auth: &AuthSource) -> Vec<AuthAttempt> {
    match auth {
        AuthSource::None => vec![AuthAttempt {
            auth: AuthSource::None,
            label: "no auth".to_string(),
        }],
        AuthSource::CookieFile { path } if !path.trim().is_empty() => vec![AuthAttempt {
            auth: AuthSource::CookieFile { path: path.clone() },
            label: "cookies.txt".to_string(),
        }],
        AuthSource::CookieFile { .. } => vec![],
        AuthSource::Browser { .. } => browser_sources(auth)
            .into_iter()
            .filter(|source| !source.browser.trim().is_empty())
            .map(|source| AuthAttempt {
                label: browser_label(&source),
                auth: AuthSource::Browser {
                    browser: source.browser,
                    profile: source.profile,
                    browsers: vec![],
                },
            })
            .collect(),
    }
}

fn browser_sources(auth: &AuthSource) -> Vec<BrowserAuthSource> {
    match auth {
        AuthSource::Browser {
            browser,
            profile,
            browsers,
        } => {
            if browsers
                .iter()
                .any(|source| !source.browser.trim().is_empty())
            {
                browsers.clone()
            } else if !browser.trim().is_empty() {
                vec![BrowserAuthSource {
                    browser: browser.clone(),
                    profile: profile.clone(),
                }]
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

fn dedupe_auth_attempts(attempts: Vec<AuthAttempt>) -> Vec<AuthAttempt> {
    let mut seen = Vec::<String>::new();
    attempts
        .into_iter()
        .filter(|attempt| {
            let key = match &attempt.auth {
                AuthSource::None => "none".to_string(),
                AuthSource::CookieFile { path } => format!("file:{path}"),
                AuthSource::Browser { .. } => browser_sources(&attempt.auth)
                    .first()
                    .map(browser_cookie_arg)
                    .unwrap_or_else(|| "browser:".to_string()),
            };
            if seen.contains(&key) {
                false
            } else {
                seen.push(key);
                true
            }
        })
        .collect()
}

fn browser_label(source: &BrowserAuthSource) -> String {
    let browser = source.browser.trim();
    if let Some(profile) = source
        .profile
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        format!("{browser} cookies ({profile})")
    } else {
        format!("{browser} cookies")
    }
}

fn browser_cookie_arg(source: &BrowserAuthSource) -> String {
    let browser = source.browser.trim().to_ascii_lowercase();
    let profile = source.profile.as_ref().and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    });

    match browser.as_str() {
        "zen" => format_browser_arg(
            "firefox",
            profile.or_else(|| discover_firefox_style_profile(".zen")),
        ),
        "helium" => format_browser_arg("chromium", profile.or_else(discover_helium_profile)),
        _ => format_browser_arg(&browser, profile),
    }
}

fn format_browser_arg(browser: &str, profile: Option<String>) -> String {
    match profile {
        Some(profile) if !profile.trim().is_empty() => format!("{browser}:{profile}"),
        _ => browser.to_string(),
    }
}

fn discover_firefox_style_profile(relative_dir: &str) -> Option<String> {
    let base = home_dir()?.join(relative_dir);
    let entries = fs::read_dir(base).ok()?;
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.is_dir() && path.join("cookies.sqlite").exists())
        .map(|path| path.display().to_string())
}

fn discover_helium_profile() -> Option<String> {
    let home = home_dir()?;
    let candidates = [
        home.join(".config/net.imput.helium/Default"),
        home.join(".var/app/net.imput.helium/config/net.imput.helium/Default"),
    ];

    candidates
        .into_iter()
        .find(|path| path.join("Cookies").exists())
        .map(|path| path.display().to_string())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn parse_format_analysis(value: Value) -> FormatAnalysis {
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let duration = value.get("duration").and_then(Value::as_f64);
    let mut formats = value
        .get("formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_format_option)
        .collect::<Vec<_>>();

    formats.sort_by(|left, right| {
        right
            .has_video
            .cmp(&left.has_video)
            .then_with(|| right.height.cmp(&left.height))
            .then_with(|| {
                right
                    .fps
                    .partial_cmp(&left.fps)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                right
                    .tbr
                    .partial_cmp(&left.tbr)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    FormatAnalysis {
        title,
        duration,
        formats,
    }
}

fn parse_format_option(format: &Value) -> Option<FormatOption> {
    let format_id = format.get("format_id")?.as_str()?.to_string();
    let vcodec = format
        .get("vcodec")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let acodec = format
        .get("acodec")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let has_video = vcodec.as_deref().is_some_and(|value| value != "none");
    let has_audio = acodec.as_deref().is_some_and(|value| value != "none");

    if !has_video && !has_audio {
        return None;
    }

    let ext = format
        .get("ext")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let width = format.get("width").and_then(Value::as_i64);
    let height = format.get("height").and_then(Value::as_i64);
    let fps = format.get("fps").and_then(Value::as_f64);
    let tbr = format.get("tbr").and_then(Value::as_f64);
    let filesize = format
        .get("filesize")
        .or_else(|| format.get("filesize_approx"))
        .and_then(Value::as_i64);
    let label = format_label(
        &format_id,
        ext.as_deref(),
        width,
        height,
        fps,
        tbr,
        filesize,
        has_video,
        has_audio,
    );

    Some(FormatOption {
        format_id,
        label,
        ext,
        width,
        height,
        fps,
        tbr,
        filesize,
        vcodec,
        acodec,
        has_video,
        has_audio,
    })
}

#[allow(clippy::too_many_arguments)]
fn format_label(
    format_id: &str,
    ext: Option<&str>,
    _width: Option<i64>,
    height: Option<i64>,
    fps: Option<f64>,
    tbr: Option<f64>,
    filesize: Option<i64>,
    has_video: bool,
    has_audio: bool,
) -> String {
    let mut parts = vec![format_id.to_string()];
    if has_video {
        if let Some(height) = height {
            parts.push(format!("{height}p"));
        }
        if let Some(fps) = fps.filter(|value| *value > 0.0) {
            parts.push(format!("{fps:.0}fps"));
        }
    } else if has_audio {
        parts.push("audio".to_string());
    }
    if let Some(ext) = ext {
        parts.push(ext.to_string());
    }
    if let Some(tbr) = tbr.filter(|value| *value > 0.0) {
        parts.push(format!("{tbr:.0}k"));
    }
    if let Some(filesize) = filesize {
        parts.push(human_size(filesize));
    }
    parts.join(" / ")
}

fn human_size(bytes: i64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.2}GiB", bytes / GIB)
    } else {
        format!("{:.1}MiB", bytes / MIB)
    }
}

fn handle_process_line(
    app: &AppHandle,
    state: &commands::AppState,
    job_id: &str,
    line: ProcessLine,
) -> Result<(), String> {
    let (level, raw) = match line {
        ProcessLine::Stdout(line) => ("info", line),
        ProcessLine::Stderr(line) => {
            let level = if line.to_ascii_lowercase().contains("error") {
                "error"
            } else {
                "warn"
            };
            (level, line)
        }
    };

    let clean = redaction::sanitize_log_line(&raw);
    let progress = parse_progress(&clean);
    let output_path = parse_output_path(&clean);
    let log = state.append_log(job_id, level, &clean)?;

    let job = state.update_job(job_id, |job| {
        if let Some(output_path) = output_path {
            job.output_path = Some(output_path);
        }
        if let Some(progress) = progress {
            job.progress = progress.percent;
            job.speed = progress.speed;
            job.eta = progress.eta;
            job.phase = "Downloading".to_string();
        } else if clean.contains("[Merger]") || clean.contains("[VideoConvertor]") {
            job.status = JobStatus::Postprocessing;
            job.phase = "Postprocessing".to_string();
            job.progress = job.progress.max(95.0);
        }
    })?;

    commands::emit_job(app, &job, Some(log));
    Ok(())
}

fn parse_output_path(line: &str) -> Option<String> {
    let patterns = [
        r"\[download\] Destination: (.+)$",
        r#"\[Merger\] Merging formats into "(.+)"$"#,
        r"\[download\] (.+) has already been downloaded$",
    ];

    patterns.iter().find_map(|pattern| {
        Regex::new(pattern)
            .ok()
            .and_then(|regex| regex.captures(line))
            .and_then(|captures| captures.get(1).map(|match_| match_.as_str().to_string()))
    })
}

struct ParsedProgress {
    percent: f64,
    speed: Option<String>,
    eta: Option<String>,
}

fn parse_progress(line: &str) -> Option<ParsedProgress> {
    let percent_regex = Regex::new(r"\[download\]\s+([0-9]+(?:\.[0-9]+)?)%").ok()?;
    let percent = percent_regex
        .captures(line)?
        .get(1)?
        .as_str()
        .parse::<f64>()
        .ok()?;

    let speed = Regex::new(r"\sat\s+([^\s]+)")
        .ok()
        .and_then(|regex| regex.captures(line))
        .and_then(|captures| captures.get(1).map(|match_| match_.as_str().to_string()));

    let eta = Regex::new(r"\sETA\s+([^\s]+)")
        .ok()
        .and_then(|regex| regex.captures(line))
        .and_then(|captures| captures.get(1).map(|match_| match_.as_str().to_string()));

    Some(ParsedProgress {
        percent,
        speed,
        eta,
    })
}

fn update_phase(
    app: &AppHandle,
    state: &commands::AppState,
    job_id: &str,
    status: JobStatus,
    progress: f64,
    phase: &str,
) -> Result<(), String> {
    let job = state.update_job(job_id, |job| {
        job.status = status;
        job.progress = progress;
        job.phase = phase.to_string();
    })?;
    commands::emit_job(app, &job, None);
    Ok(())
}

fn log(
    app: &AppHandle,
    state: &commands::AppState,
    job_id: &str,
    level: &str,
    message: &str,
) -> Result<(), String> {
    let clean = redaction::sanitize_log_line(message);
    let log = state.append_log(job_id, level, &clean)?;
    if let Some(job) = state.get_job(job_id)? {
        commands::emit_job(app, &job, Some(log));
    }
    Ok(())
}

fn mark_failed(
    app: &AppHandle,
    state: &commands::AppState,
    job_id: &str,
    error: &str,
) -> Result<(), String> {
    let clean = redaction::sanitize_log_line(error);
    let log = state.append_log(job_id, "error", &clean)?;
    let job = state.update_job(job_id, |job| {
        job.status = JobStatus::Failed;
        job.phase = "Failed".to_string();
        job.error_message = Some(clean);
    })?;
    commands::emit_job(app, &job, Some(log));
    Ok(())
}

fn mark_canceled(app: &AppHandle, state: &commands::AppState, job_id: &str) -> Result<(), String> {
    let log = state.append_log(job_id, "warn", "Canceled by user.")?;
    let job = state.update_job(job_id, |job| {
        job.status = JobStatus::Canceled;
        job.phase = "Canceled".to_string();
        job.speed = None;
        job.eta = None;
    })?;
    commands::emit_job(app, &job, Some(log));
    Ok(())
}

fn fail_job(app: &AppHandle, state: &commands::AppState, job_id: &str, error: &str) {
    let clean = redaction::sanitize_log_line(error);
    let _ = state.append_log(job_id, "error", &clean);
    if let Ok(job) = state.update_job(job_id, |job| {
        job.status = JobStatus::Failed;
        job.phase = "Failed".to_string();
        job.error_message = Some(clean.clone());
    }) {
        commands::emit_job(app, &job, None);
    }
}
