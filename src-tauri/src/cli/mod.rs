mod args;
mod cookies;
use crate::{
    backend::{self, AppState},
    download::{self, AuthSource, BrowserAuthSource, Job, JobStatus, StartDownloadRequest},
    redaction,
    runtime::Runtime,
    tools, youtube_api_keys,
};
use args::*;
use clap::Parser;
use fs2::FileExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const BROWSERS: &[&str] = &[
    "firefox", "zen", "chrome", "chromium", "helium", "brave", "edge", "safari", "vivaldi",
    "opera", "whale",
];

fn emit(value: &Value) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = serde_json::to_writer(&mut out, value);
    let _ = writeln!(out);
}
fn read_input(path: &str) -> Result<String, String> {
    let mut text = String::new();
    if path == "-" {
        std::io::stdin()
            .take(4 * 1024 * 1024)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
    } else {
        text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    }
    Ok(text)
}
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path))
    }
}
fn private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn desktop_dir() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|p| p.join("dev.local.downloader"))
        .ok_or("Cannot determine the desktop data directory.".into())
}
fn runtime(path: Option<PathBuf>, events: bool) -> Result<Runtime, String> {
    let data_dir = absolute(&match path {
        Some(p) => p,
        None => dirs::data_dir()
            .ok_or("Cannot determine a data directory; pass --data-dir.")?
            .join("dev.local.downloader.cli"),
    })?;
    // Never let CLI recovery or process ownership interfere with desktop jobs.
    if data_dir == desktop_dir()?
        || (data_dir.exists()
            && desktop_dir()?.exists()
            && data_dir.canonicalize().ok() == desktop_dir()?.canonicalize().ok())
    {
        return Err(
            "Use a separate CLI data directory. Copy configuration with settings import-desktop."
                .into(),
        );
    }
    private_dir(&data_dir)?;
    let download_dir = dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|p| p.join("Downloads")))
        .ok_or("Cannot determine a download directory.")?;
    Ok(Runtime {
        data_dir,
        resource_dir: std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_owned)),
        download_dir,
        on_event: Arc::new(move |event| {
            if events {
                emit(&json!({"schemaVersion":1,"type":"job","job":event.job,"log":event.log}));
            }
        }),
    })
}
fn lock_runner(app: &Runtime) -> Result<File, String> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(app.data_dir.join("runner.lock"))
        .map_err(|e| e.to_string())?;
    file.try_lock_exclusive().map_err(|_|"Another CLI download is running in this data directory. Use a separate --data-dir for parallel batches.".to_string())?;
    Ok(file)
}
fn browser_source(raw: &str) -> Result<BrowserAuthSource, String> {
    let (browser, profile) = raw
        .split_once(':')
        .map_or((raw, None), |(b, p)| (b, Some(p.to_owned())));
    let browser = browser.to_ascii_lowercase();
    if !BROWSERS.contains(&browser.as_str()) {
        return Err(format!(
            "Unsupported browser '{browser}'. Run cookies browsers."
        ));
    }
    if profile.as_ref().is_some_and(|p| p.is_empty()) {
        return Err("Browser profile cannot be empty.".into());
    }
    Ok(BrowserAuthSource { browser, profile })
}
fn selected_auth(args: &AuthArgs, saved: &AuthSource) -> Result<AuthSource, String> {
    if args.no_cookies {
        return Ok(AuthSource::None);
    }
    if let Some(path) = &args.cookie_file {
        return Ok(AuthSource::CookieFile {
            path: absolute(path)?.display().to_string(),
        });
    }
    if args.browser.is_empty() {
        return Ok(saved.clone());
    }
    let browsers = args
        .browser
        .iter()
        .map(|s| browser_source(s))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AuthSource::Browser {
        browser: browsers[0].browser.clone(),
        profile: browsers[0].profile.clone(),
        browsers,
    })
}
fn validate_auth(auth: &AuthSource) -> Result<(), String> {
    match auth {
        AuthSource::None => Ok(()),
        AuthSource::CookieFile { path } => {
            if Path::new(path).is_file() {
                Ok(())
            } else {
                Err(format!("Cookie file does not exist: {path}"))
            }
        }
        AuthSource::Browser {
            browser,
            profile,
            browsers,
        } => {
            let sources = if browsers.is_empty() {
                vec![BrowserAuthSource {
                    browser: browser.clone(),
                    profile: profile.clone(),
                }]
            } else {
                browsers.clone()
            };
            for source in sources {
                browser_source(&source.browser)?;
            }
            Ok(())
        }
    }
}
fn requests(args: &DownloadArgs, saved: &AuthSource) -> Result<Vec<StartDownloadRequest>, String> {
    let mut requests = if let Some(file) = &args.request {
        vec![
            serde_json::from_str::<StartDownloadRequest>(&read_input(file)?)
                .map_err(|e| format!("Invalid download request: {e}"))?,
        ]
    } else {
        let auth = selected_auth(&args.auth, saved)?;
        let format = if args.audio_only {
            download::FormatSelection::AudioOnly
        } else if args.video_only {
            download::FormatSelection::VideoOnly {
                format_id: args.format_id.clone(),
            }
        } else if let Some(id) = &args.format_id {
            download::FormatSelection::Format {
                format_id: id.clone(),
            }
        } else {
            download::FormatSelection::Best
        };
        let segment = if args.start.is_some() || args.end.is_some() {
            Some(download::SegmentSelection {
                enabled: true,
                start_seconds: args.start.unwrap_or(0.0),
                end_seconds: args.end,
            })
        } else {
            None
        };
        let mut result = Vec::new();
        for url in &args.urls {
            let analysis = backend::analyze_url(url)?;
            let preset_id = args
                .preset
                .clone()
                .or_else(|| analysis.presets.first().map(|p| p.id.clone()))
                .ok_or("No matching preset.")?;
            let catalogue = preset_id == "youtube-channel-catalogue";
            result.push(StartDownloadRequest {
                allow_saved_auth: !args.auth.no_cookies,
                url: analysis.normalized_url,
                preset_id,
                channel_urls: if catalogue { args.urls.clone() } else { vec![] },
                youtube_catalogue_content: match args.catalogue_content {
                    Some(Content::Videos) => download::YoutubeCatalogueContent::Videos,
                    Some(Content::Shorts) => download::YoutubeCatalogueContent::Shorts,
                    _ => download::YoutubeCatalogueContent::All,
                },
                output_profile: if matches!(args.profile, Some(Profile::Xrbazaar)) {
                    download::OutputProfile::Xrbazaar
                } else {
                    download::OutputProfile::Original
                },
                output_dir: args
                    .output
                    .as_ref()
                    .map(|p| absolute(p).map(|p| p.display().to_string()))
                    .transpose()?,
                export_name: args.export_name.clone(),
                filename_template: args.filename_template.clone(),
                auth: auth.clone(),
                advanced: Some(download::AdvancedDownloadOptions {
                    format: format.clone(),
                    segment: segment.clone(),
                }),
            });
            if catalogue {
                break;
            }
        }
        result
    };
    for request in &mut requests {
        validate_auth(&request.auth)?;
        if let Some(path) = &request.output_dir {
            request.output_dir = Some(absolute(Path::new(path))?.display().to_string());
        }
        if let Some(segment) = request
            .advanced
            .as_ref()
            .and_then(|a| a.segment.as_ref())
            .filter(|s| s.enabled)
        {
            if !segment.start_seconds.is_finite()
                || segment.start_seconds < 0.0
                || segment
                    .end_seconds
                    .is_some_and(|end| !end.is_finite() || end <= segment.start_seconds)
            {
                return Err(
                    "Trim requires a nonnegative start and an end greater than start.".into(),
                );
            }
        }
    }
    Ok(requests)
}
fn file_info(path: &Path, hashes: bool) -> Result<Value, String> {
    let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
    let sha = if hashes {
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        Some(format!("{:x}", hash.finalize()))
    } else {
        None
    };
    let content_type = match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "json" => "application/json",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    };
    Ok(
        json!({"path":absolute(path)?.display().to_string(),"byteLength":metadata.len(),"sha256":sha,"contentType":content_type}),
    )
}
fn files(jobs: &[Job], hashes: bool) -> Vec<Value> {
    let mut seen = HashSet::new();
    jobs.iter()
        .flat_map(|j| &j.ready_paths)
        .filter(|p| seen.insert((*p).clone()))
        .map(|p| {
            file_info(Path::new(p), hashes)
                .unwrap_or_else(|e| json!({"path":p,"missing":true,"error":e}))
        })
        .collect()
}
fn cancellation() -> Result<Arc<AtomicBool>, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || {
        flag.store(true, Ordering::SeqCst);
    })
    .map_err(|e| e.to_string())?;
    Ok(stop)
}
fn download(app: &Runtime, args: DownloadArgs) -> Result<(Value, i32), String> {
    let _guard = lock_runner(app)?;
    let state = AppState::new(&app.data_dir, true)?;
    let settings = backend::settings_with_defaults(app, &state)?;
    let inputs = requests(&args, &settings.auth)?;
    let stop = cancellation()?;
    let started = Instant::now();
    let mut jobs = Vec::new();
    let mut errors = Vec::new();
    let mut timed_out = false;
    struct StopProcesses(AppState);
    impl Drop for StopProcesses {
        fn drop(&mut self) {
            self.0.stop_all_processes();
        }
    }
    let _process_guard = StopProcesses(state.clone());
    let control = app.data_dir.join("control");
    private_dir(&control)?;
    for input in inputs {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let url = input.url.clone();
        let task = match backend::prepare_download(app, &state, input, !args.auth.no_cookies) {
            Ok(task) => task,
            Err(error) => {
                errors.push(json!({"url":url,"message":redaction::sanitize_log_line(&error)}));
                continue;
            }
        };
        let id = task.job.id.clone();
        let cancel_path = control.join(format!("{id}.cancel"));
        let runtime = app.clone();
        let worker_state = state.clone();
        let worker = thread::spawn(move || task.run(runtime, worker_state));
        while !worker.is_finished() {
            timed_out = args
                .timeout
                .is_some_and(|seconds| started.elapsed() >= Duration::from_secs(seconds));
            if stop.load(Ordering::SeqCst) || timed_out || cancel_path.exists() {
                stop.store(true, Ordering::SeqCst);
                let _ = state.cancel(&id);
            }
            thread::sleep(Duration::from_millis(100));
        }
        worker
            .join()
            .map_err(|_| "Download worker terminated unexpectedly.".to_string())?;
        let _ = fs::remove_file(cancel_path);
        jobs.push(state.get_job(&id)?.ok_or("Download job disappeared.")?);
    }
    let code = if timed_out {
        124
    } else if stop.load(Ordering::SeqCst)
        || jobs.iter().any(|j| matches!(j.status, JobStatus::Canceled))
    {
        130
    } else if !errors.is_empty() || jobs.iter().any(|j| matches!(j.status, JobStatus::Failed)) {
        1
    } else {
        0
    };
    Ok((
        json!({"files":files(&jobs,true),"jobs":jobs,"errors":errors}),
        code,
    ))
}
fn list_keys(state: &AppState) -> Result<Value, String> {
    Ok(json!(state
        .youtube_api_key_ids()?
        .into_iter()
        .enumerate()
        .map(|(n, id)| json!({"id":id,"label":format!("YouTube API key {}",n+1)}))
        .collect::<Vec<_>>()))
}

fn execute(cli: Cli) -> Result<(Value, i32), String> {
    let events = matches!(&cli.command,Command::Download(a) if a.events)
        || matches!(&cli.command, Command::Prepare { events: true, .. });
    let app = runtime(cli.data_dir, events)?;
    if let Command::Download(args) = cli.command {
        return download(&app, args);
    }
    let state = AppState::new(&app.data_dir, false)?;
    let result = match cli.command {
        Command::Presets => json!(download::presets::all_presets()),
        Command::Analyze { url } => json!(backend::analyze_url(&url)?),
        Command::Formats { url, auth } => {
            let url = download::sites::normalize_url(&url)?;
            let auth = selected_auth(&auth, &state.get_settings()?.auth)?;
            validate_auth(&auth)?;
            json!(download::engine::analyze_formats(&app, &url, &auth)?)
        }
        Command::Schema => {
            json!({"schemaVersion":1,"downloadRequest":schemars::schema_for!(StartDownloadRequest),"settings":schemars::schema_for!(backend::Settings),"job":schemars::schema_for!(Job),"browsers":BROWSERS,"exitCodes":{"0":"success","1":"operation failed","2":"invalid command syntax","124":"timeout","130":"canceled"}})
        }
        Command::Tools(Tools::Platform) => json!(tools::tool_platform()),
        Command::Tools(Tools::Status) => json!(tools::check_tool_updates(&app)),
        Command::Tools(Tools::Install { tool }) => {
            for name in if tool == "all" {
                vec!["yt-dlp", "ffmpeg"]
            } else {
                vec![tool.as_str()]
            } {
                tools::install_tool_update(&app, name)?;
            }
            json!(tools::check_tool_updates(&app))
        }
        Command::Settings(Settings::Show) => json!(backend::settings_with_defaults(&app, &state)?),
        Command::Settings(Settings::Set { request }) => {
            let settings: backend::Settings =
                serde_json::from_str(&read_input(&request)?).map_err(|e| e.to_string())?;
            validate_auth(&settings.auth)?;
            state.update_settings(&settings)?;
            json!(backend::settings_with_defaults(&app, &state)?)
        }
        Command::Settings(Settings::ImportDesktop { from }) => {
            let from = from.unwrap_or(desktop_dir()?);
            let db = rusqlite::Connection::open_with_flags(
                from.join("downloader.sqlite3"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(|e| e.to_string())?;
            use rusqlite::OptionalExtension;
            let raw: Option<String> = db
                .query_row(
                    "SELECT value_json FROM settings WHERE key='settings'",
                    [],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if let Some(raw) = raw {
                state.update_settings(&serde_json::from_str(&raw).map_err(|e| e.to_string())?)?;
            }
            let ids: Option<String> = db
                .query_row(
                    "SELECT value_json FROM settings WHERE key='youtube_api_key_ids'",
                    [],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if let Some(raw) = ids {
                state.set_youtube_api_key_ids(
                    &serde_json::from_str::<Vec<String>>(&raw).map_err(|e| e.to_string())?,
                )?;
            }
            json!({"settings":backend::settings_with_defaults(&app,&state)?,"apiKeys":list_keys(&state)?})
        }
        Command::Cookies(Cookies::Browsers) => json!(BROWSERS),
        Command::Cookies(Cookies::Export {
            url,
            browser,
            output,
        }) => cookies::export(&app, &url, &browser_source(&browser)?, &output)?,
        Command::Keys(Keys::List) => list_keys(&state)?,
        Command::Keys(Keys::Add { .. }) => {
            let key = read_input("-")?;
            let key = key.trim();
            if key.is_empty() {
                return Err("API key cannot be empty.".into());
            }
            let mut ids = state.youtube_api_key_ids()?;
            if youtube_api_keys::load_all(&ids)?.iter().any(|k| k == key) {
                return Err("That API key is already saved.".into());
            }
            let id = uuid::Uuid::new_v4().to_string();
            youtube_api_keys::store(&id, key)?;
            ids.push(id.clone());
            if let Err(e) = state.set_youtube_api_key_ids(&ids) {
                let _ = youtube_api_keys::remove(&id);
                return Err(e);
            }
            list_keys(&state)?
        }
        Command::Keys(Keys::Remove { id }) => {
            let mut ids = state.youtube_api_key_ids()?;
            if !ids.contains(&id) {
                return Err("API key ID not found.".into());
            }
            let previous = youtube_api_keys::load_optional(&id)?;
            youtube_api_keys::remove(&id)?;
            ids.retain(|i| i != &id);
            if let Err(e) = state.set_youtube_api_key_ids(&ids) {
                if let Some(value) = previous {
                    let _ = youtube_api_keys::store(&id, &value);
                }
                return Err(e);
            }
            list_keys(&state)?
        }
        Command::History(History::List) => json!(state.list_jobs()?),
        Command::History(History::Show { job_id, logs }) => {
            let job = state.get_job(&job_id)?.ok_or("Job not found.")?;
            if logs {
                json!(download::JobDetail {
                    job,
                    logs: state.logs_for_job(&job_id)?
                })
            } else {
                json!(job)
            }
        }
        Command::Files { job, hashes } => {
            let jobs = if let Some(id) = job {
                vec![state.get_job(&id)?.ok_or("Job not found.")?]
            } else {
                state.list_jobs()?
            };
            json!(files(&jobs, hashes))
        }
        Command::Cancel { job_id } => {
            uuid::Uuid::parse_str(&job_id).map_err(|_| "Invalid job ID.")?;
            let job = state.get_job(&job_id)?.ok_or("Job not found.")?;
            if job.status.is_terminal() {
                json!({"jobId":job_id,"status":job.status,"alreadyFinished":true})
            } else {
                if lock_runner(&app).is_ok() {
                    return Err(
                        "No CLI runner owns this job; it will be recovered on the next download."
                            .into(),
                    );
                }
                private_dir(&app.data_dir.join("control"))?;
                fs::write(
                    app.data_dir
                        .join("control")
                        .join(format!("{job_id}.cancel")),
                    b"",
                )
                .map_err(|e| e.to_string())?;
                json!({"jobId":job_id,"cancelRequested":true})
            }
        }
        Command::Prepare { file, timeout, .. } => {
            let source = absolute(&file)?;
            let stop = cancellation()?;
            let done = Arc::new(AtomicBool::new(false));
            let timed_out = Arc::new(AtomicBool::new(false));
            let watcher = if let Some(seconds) = timeout {
                let done = done.clone();
                let stop = stop.clone();
                let expired = timed_out.clone();
                Some(thread::spawn(move || {
                    let started = Instant::now();
                    while !done.load(Ordering::SeqCst) {
                        if started.elapsed() >= Duration::from_secs(seconds) {
                            expired.store(true, Ordering::SeqCst);
                            stop.store(true, Ordering::SeqCst);
                            break;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                }))
            } else {
                None
            };
            let result = (|| {
                let ffmpeg = tools::find_tool(&app, "ffmpeg")
                    .ok_or("FFmpeg is required. Run tools install ffmpeg.")?;
                let ffprobe = tools::find_tool(&app, "ffprobe")
                    .ok_or("ffprobe is required. Run tools install ffmpeg.")?;
                download::xrbazaar::prepare_video(
                    &ffmpeg,
                    &ffprobe,
                    &source,
                    &stop,
                    &mut |_| Ok(()),
                    &mut |progress| {
                        if events {
                            emit(
                                &json!({"schemaVersion":1,"type":"progress","phase":"preparing","progress":progress}),
                            );
                        }
                        Ok(())
                    },
                )
            })();
            done.store(true, Ordering::SeqCst);
            if let Some(w) = watcher {
                let _ = w.join();
            }
            if stop.load(Ordering::SeqCst) {
                return Ok((
                    json!({"source":source,"canceled":true}),
                    if timed_out.load(Ordering::SeqCst) {
                        124
                    } else {
                        130
                    },
                ));
            }
            let output = result?;
            json!({"source":source,"profile":"xrbazaar","file":file_info(&output,true)?})
        }
        Command::Download(_) => unreachable!(),
    };
    Ok((result, 0))
}

pub fn main() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                print!("{e}");
                return 0;
            }
            emit(
                &json!({"schemaVersion":1,"type":"result","ok":false,"error":{"code":"invalid_arguments","message":e.to_string()}}),
            );
            return 2;
        }
    };
    match execute(cli) {
        Ok((result, code)) => {
            emit(
                &json!({"schemaVersion":1,"type":"result","ok":code==0,"exitCode":code,"result":result}),
            );
            code
        }
        Err(error) => {
            emit(
                &json!({"schemaVersion":1,"type":"result","ok":false,"exitCode":1,"error":{"code":"operation_failed","message":redaction::sanitize_log_line(&error)}}),
            );
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_cookies_overrides_saved_browser_auth() {
        let auth = selected_auth(
            &AuthArgs {
                no_cookies: true,
                ..Default::default()
            },
            &backend::Settings::default().auth,
        )
        .unwrap();
        assert!(matches!(auth, AuthSource::None));
    }
    #[test]
    fn rejects_invalid_trim_before_starting_a_job() {
        let cli = Cli::try_parse_from([
            "downloader-cli",
            "download",
            "https://example.com/v.mp4",
            "--start",
            "10",
            "--end",
            "5",
            "--no-cookies",
        ])
        .unwrap();
        let Command::Download(args) = cli.command else {
            panic!()
        };
        assert!(requests(&args, &AuthSource::None)
            .unwrap_err()
            .contains("Trim requires"));
    }
    #[test]
    fn catalogue_keeps_all_channels_in_one_request() {
        let cli = Cli::try_parse_from([
            "downloader-cli",
            "download",
            "https://youtube.com/@first",
            "https://youtube.com/@second",
            "--export-name",
            "collection",
            "--no-cookies",
        ])
        .unwrap();
        let Command::Download(args) = cli.command else {
            panic!()
        };
        let req = requests(&args, &AuthSource::None).unwrap();
        assert_eq!(req.len(), 1);
        assert_eq!(req[0].channel_urls.len(), 2);
    }
}
