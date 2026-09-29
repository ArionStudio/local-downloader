use super::{
    chrome_cookies, AuthSource, BrowserAuthSource, FormatAnalysis, FormatOption, FormatSelection,
    JobLog, JobStatus, Pipeline, Preset, SiteKind, StartDownloadRequest,
};
use crate::runtime::Runtime;
use crate::{backend, process_control, redaction, tools};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use regex::Regex;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use url::Url;

const X_TWEET_RESULT_QUERY_ID: &str = "-4_LMahNlI4MuLJ-EAFEog";
const X_TWEET_RESULT_OPERATION: &str = "TweetResultByRestId";
const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const X_GRAPHQL_FEATURES: &[&str] = &[
    "creator_subscriptions_tweet_preview_api_enabled",
    "premium_content_api_read_enabled",
    "communities_web_enable_tweet_community_results_fetch",
    "c9s_tweet_anatomy_moderator_badge_enabled",
    "responsive_web_grok_analyze_button_fetch_trends_enabled",
    "responsive_web_grok_analyze_post_followups_enabled",
    "rweb_cashtags_composer_attachment_enabled",
    "responsive_web_jetfuel_frame",
    "responsive_web_grok_share_attachment_enabled",
    "responsive_web_grok_annotations_enabled",
    "articles_preview_enabled",
    "responsive_web_edit_tweet_api_enabled",
    "rweb_conversational_replies_downvote_enabled",
    "graphql_is_translatable_rweb_tweet_is_translatable_enabled",
    "view_counts_everywhere_api_enabled",
    "longform_notetweets_consumption_enabled",
    "responsive_web_twitter_article_tweet_consumption_enabled",
    "content_disclosure_indicator_enabled",
    "content_disclosure_ai_generated_indicator_enabled",
    "responsive_web_grok_show_grok_translated_post",
    "responsive_web_grok_analysis_button_from_backend",
    "post_ctas_fetch_enabled",
    "rweb_cashtags_enabled",
    "freedom_of_speech_not_reach_fetch_enabled",
    "standardized_nudges_misinfo",
    "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled",
    "longform_notetweets_rich_text_read_enabled",
    "longform_notetweets_inline_media_enabled",
    "profile_label_improvements_pcf_label_in_post_enabled",
    "responsive_web_profile_redirect_enabled",
    "rweb_tipjar_consumption_enabled",
    "verified_phone_label_enabled",
    "responsive_web_grok_image_annotation_enabled",
    "responsive_web_grok_imagine_annotation_enabled",
    "responsive_web_grok_community_note_auto_translation_is_enabled",
    "responsive_web_graphql_skip_user_profile_image_extensions_enabled",
    "responsive_web_graphql_timeline_navigation_enabled",
];

#[derive(Debug, Clone)]
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
    app: &Runtime,
    url: &str,
    auth: &AuthSource,
) -> Result<FormatAnalysis, String> {
    let yt_dlp = tools::find_tool(app, "yt-dlp").ok_or_else(|| {
        "yt-dlp was not found. Bundle it in src-tauri/binaries or install it locally.".to_string()
    })?;
    let attempts = auth_attempts(auth, None);
    let mut last_error = None;
    let embedded_target = if should_resolve_embedded_page(url) {
        resolve_page_video_target(yt_dlp.as_path(), url, auth)
    } else {
        None
    };

    for attempt in attempts {
        let target_url = if let Some(target_url) = &embedded_target {
            target_url.clone()
        } else if let Some(feed_url) = linkedin_feed_update_url(url) {
            resolve_linkedin_feed_stream_url(yt_dlp.as_path(), &feed_url, &attempt.auth)
                .unwrap_or_else(|_| url.to_string())
        } else if is_x_article_url(url) {
            resolve_x_article_video_urls(yt_dlp.as_path(), url, &attempt.auth)
                .ok()
                .and_then(|videos| videos.into_iter().next().map(|video| video.url))
                .unwrap_or_else(|| url.to_string())
        } else {
            url.to_string()
        };
        let mut args = vec![
            "-J".to_string(),
            "--no-warnings".to_string(),
            "--no-playlist".to_string(),
        ];
        append_auth_args(&mut args, &attempt.auth);
        args.push(target_url);

        let output = Command::new(&yt_dlp)
            .arg("--ignore-config")
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
    app: Runtime,
    state: backend::AppState,
    job_id: String,
    input: StartDownloadRequest,
    preset: Preset,
    fallback_auth: Option<AuthSource>,
    cancel_flag: Arc<AtomicBool>,
) {
    let xrbazaar = input.output_profile == super::OutputProfile::Xrbazaar;
    if let Err(error) = run_download_inner(
        app.clone(),
        state.clone(),
        job_id.clone(),
        input,
        preset,
        fallback_auth,
        cancel_flag.clone(),
    ) {
        if cancel_flag.load(Ordering::SeqCst) {
            let _ = mark_canceled(&app, &state, &job_id);
        } else {
            fail_job(&app, &state, &job_id, &error);
        }
    } else if let Ok(Some(job)) = state.get_job(&job_id) {
        if !job.status.is_terminal() {
            if cancel_flag.load(Ordering::SeqCst) {
                let _ = mark_canceled(&app, &state, &job_id);
            } else if let Ok(job) = state.update_job(&job_id, |job| {
                job.status = JobStatus::Completed;
                job.progress = 100.0;
                job.phase = if xrbazaar {
                    "Ready for XRBAZAAR"
                } else {
                    "Completed"
                }
                .into();
                job.speed = None;
                job.eta = None;
                job.error_message = None;
            }) {
                backend::emit_job(&app, &job, None);
            }
        }
    }
    state.remove_cancel_flag(&job_id);
}

fn run_download_inner(
    app: Runtime,
    state: backend::AppState,
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
    if matches!(&preset.pipeline, Pipeline::YoutubeChannelExport) {
        return super::youtube_export::run(&app, &state, &job_id, &input, &cancel_flag);
    }
    let ffmpeg = tools::find_tool(&app, "ffmpeg");

    if matches!(&preset.pipeline, Pipeline::HttpResolveThenDownload) {
        log(
            &app,
            &state,
            &job_id,
            "info",
            "Using HTTP stream resolution before yt-dlp download.",
        )?;
    }

    let ffmpeg_location = ffmpeg.as_ref().map(|path| path.display().to_string());
    let attempts = auth_attempts(&input.auth, fallback_auth);
    let mut last_error = None;
    let mut attempt_errors = Vec::<(String, String)>::new();
    if is_generic_page_video_preset(&preset) && should_resolve_embedded_page(&input.url) {
        match run_page_video_attempts(
            &app,
            &state,
            &job_id,
            &input,
            &preset,
            ffmpeg_location.clone(),
            yt_dlp.as_path(),
            &cancel_flag,
            &attempts,
        )? {
            Some(AttemptOutcome::Succeeded | AttemptOutcome::Canceled) => return Ok(()),
            Some(AttemptOutcome::Failed(error)) => {
                log(
                    &app,
                    &state,
                    &job_id,
                    "error",
                    &format!("Page video resolution failed: {error}"),
                )?;
                mark_failed(&app, &state, &job_id, &error)?;
                return Ok(());
            }
            None => {
                log(
                    &app,
                    &state,
                    &job_id,
                    "warn",
                    "Could not resolve embedded page videos; trying the standard extractor.",
                )?;
            }
        }
    }

    if is_x_article_preset(&preset) && is_x_article_url(&input.url) {
        match run_x_article_video_attempts(
            &app,
            &state,
            &job_id,
            &input,
            &preset,
            ffmpeg_location.clone(),
            yt_dlp.as_path(),
            &cancel_flag,
            &attempts,
        )? {
            Some(AttemptOutcome::Succeeded | AttemptOutcome::Canceled) => return Ok(()),
            Some(AttemptOutcome::Failed(error)) => {
                log(
                    &app,
                    &state,
                    &job_id,
                    "error",
                    &format!("X article video resolution failed: {error}"),
                )?;
                mark_failed(&app, &state, &job_id, &error)?;
                return Ok(());
            }
            None => {
                log(
                    &app,
                    &state,
                    &job_id,
                    "warn",
                    "Could not resolve X article videos; trying the standard extractor.",
                )?;
            }
        }
    }

    if is_linkedin_video_post_preset(&preset) {
        if let Some(feed_url) = linkedin_feed_update_url(&input.url) {
            match run_linkedin_feed_stream_attempts(
                &app,
                &state,
                &job_id,
                &input,
                &feed_url,
                &preset,
                ffmpeg_location.clone(),
                yt_dlp.as_path(),
                &cancel_flag,
                &attempts,
            )? {
                Some(AttemptOutcome::Succeeded | AttemptOutcome::Canceled) => return Ok(()),
                Some(AttemptOutcome::Failed(error)) => {
                    log(
                        &app,
                        &state,
                        &job_id,
                        "error",
                        &format!("LinkedIn resolved stream failed: {error}"),
                    )?;
                    mark_failed(&app, &state, &job_id, &error)?;
                    return Ok(());
                }
                None => {
                    log(
                        &app,
                        &state,
                        &job_id,
                        "warn",
                        "Could not resolve a LinkedIn stream from feed HTML; trying the standard extractor.",
                    )?;
                }
            }
        }
    }

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
                let error = match run_yt_dlp_with_chrome_cookie_export(
                    &app,
                    &state,
                    &job_id,
                    &input,
                    &preset,
                    ffmpeg_location.clone(),
                    yt_dlp.as_path(),
                    &cancel_flag,
                    &attempt.auth,
                    &error,
                )? {
                    Some(AttemptOutcome::Succeeded) => return Ok(()),
                    Some(AttemptOutcome::Canceled) => return Ok(()),
                    Some(AttemptOutcome::Failed(retry_error)) => retry_error,
                    None => error,
                };
                last_error = Some(error.clone());
                attempt_errors.push((attempt.label.clone(), error.clone()));
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

    let log_hint = state
        .logs_for_job(&job_id)
        .ok()
        .and_then(|logs| failure_hint_from_logs(&logs));
    let auth_failure = browser_cookie_failure_summary(&attempt_errors);
    let error = auth_failure.unwrap_or_else(|| match last_error {
        Some(error) if !is_generic_yt_dlp_exit_error(&error) => error,
        Some(error) => log_hint.unwrap_or(error),
        None => log_hint.unwrap_or_else(|| "yt-dlp failed.".to_string()),
    });
    mark_failed(&app, &state, &job_id, &error)?;
    Ok(())
}

enum AttemptOutcome {
    Succeeded,
    Failed(String),
    Canceled,
}

#[allow(clippy::too_many_arguments)]
fn run_page_video_attempts(
    app: &Runtime,
    state: &backend::AppState,
    job_id: &str,
    input: &StartDownloadRequest,
    preset: &Preset,
    ffmpeg_location: Option<String>,
    yt_dlp: &Path,
    cancel_flag: &Arc<AtomicBool>,
    attempts: &[AuthAttempt],
) -> Result<Option<AttemptOutcome>, String> {
    let (mut targets, mut preferred_attempt) = match resolve_embedded_page_video_urls(&input.url) {
        Ok(targets) => (targets, None),
        Err(error) => {
            log(
                app,
                state,
                job_id,
                "warn",
                &format!(
                    "Could not inspect the public HTML ({error}); checking extractor page data."
                ),
            )?;
            (Vec::new(), None)
        }
    };

    if targets.is_empty() {
        log(
            app,
            state,
            job_id,
            "info",
            "No embedded media descriptor was found in the public HTML; checking extractor page data.",
        )?;

        for (attempt_index, attempt) in attempts.iter().enumerate() {
            if cancel_flag.load(Ordering::SeqCst) {
                mark_canceled(app, state, job_id)?;
                return Ok(Some(AttemptOutcome::Canceled));
            }

            update_phase(
                app,
                state,
                job_id,
                JobStatus::Resolving,
                3.0,
                &format!("Inspecting page data ({})", attempt.label),
            )?;
            match resolve_embedded_page_video_urls_from_dump(yt_dlp, &input.url, &attempt.auth) {
                Ok(resolved) if !resolved.is_empty() => {
                    targets = resolved;
                    preferred_attempt = Some(attempt_index);
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    log(
                        app,
                        state,
                        job_id,
                        "warn",
                        &format!(
                            "Could not inspect extractor page data with {}: {error}",
                            attempt.label
                        ),
                    )?;
                }
            }
        }
    }

    if targets.is_empty() {
        return Ok(None);
    }

    let target_count = targets.len();
    log(
        app,
        state,
        job_id,
        "info",
        &format!("Resolved {target_count} embedded page video(s)."),
    )?;

    let mut attempt_order = (0..attempts.len()).collect::<Vec<_>>();
    if let Some(preferred_attempt) = preferred_attempt {
        attempt_order.rotate_left(preferred_attempt);
    }

    for (target_index, target_url) in targets.iter().enumerate() {
        if cancel_flag.load(Ordering::SeqCst) {
            mark_canceled(app, state, job_id)?;
            return Ok(Some(AttemptOutcome::Canceled));
        }

        let mut last_error = None;
        for (order_index, attempt_index) in attempt_order.iter().enumerate() {
            let attempt = &attempts[*attempt_index];
            let mut stream_input = input.clone();
            stream_input.url = target_url.clone();
            stream_input.auth = attempt.auth.clone();
            stream_input.filename_template = page_video_filename_template(
                input.filename_template.as_deref(),
                &input.url,
                target_index,
                target_count,
            );

            let phase = format!(
                "Downloading page video {}/{} ({})",
                target_index + 1,
                target_count,
                attempt.label
            );
            match run_yt_dlp_attempt(
                app,
                state,
                job_id,
                &stream_input,
                preset,
                ffmpeg_location.clone(),
                yt_dlp,
                cancel_flag,
                &phase,
            )? {
                AttemptOutcome::Succeeded => {
                    last_error = None;
                    break;
                }
                AttemptOutcome::Canceled => return Ok(Some(AttemptOutcome::Canceled)),
                AttemptOutcome::Failed(error) => {
                    let error = match run_yt_dlp_with_chrome_cookie_export(
                        app,
                        state,
                        job_id,
                        &stream_input,
                        preset,
                        ffmpeg_location.clone(),
                        yt_dlp,
                        cancel_flag,
                        &attempt.auth,
                        &error,
                    )? {
                        Some(AttemptOutcome::Succeeded) => {
                            last_error = None;
                            break;
                        }
                        Some(AttemptOutcome::Canceled) => {
                            return Ok(Some(AttemptOutcome::Canceled));
                        }
                        Some(AttemptOutcome::Failed(retry_error)) => retry_error,
                        None => error,
                    };
                    last_error = Some(error);

                    if let Some(next_attempt_index) = attempt_order.get(order_index + 1) {
                        log(
                            app,
                            state,
                            job_id,
                            "warn",
                            &format!(
                                "Page video {}/{} failed with {}; retrying with {}.",
                                target_index + 1,
                                target_count,
                                attempt.label,
                                attempts[*next_attempt_index].label
                            ),
                        )?;
                    }
                }
            }
        }

        if let Some(error) = last_error {
            return Ok(Some(AttemptOutcome::Failed(format!(
                "Page video {}/{} failed: {error}",
                target_index + 1,
                target_count
            ))));
        }
    }

    Ok(Some(AttemptOutcome::Succeeded))
}

fn page_video_filename_template(
    requested: Option<&str>,
    page_url: &str,
    target_index: usize,
    target_count: usize,
) -> Option<String> {
    let requested = requested.filter(|value| !value.trim().is_empty());
    if target_count == 1 {
        return requested.map(str::to_string).or_else(|| {
            page_media_filename(page_url).map(|filename| format!("{filename}.%(ext)s"))
        });
    }

    let suffix = format!("-video-{:02}", target_index + 1);
    if let Some(template) = requested {
        if let Some(extension_index) = template.rfind(".%(ext)s") {
            let mut indexed = template.to_string();
            indexed.insert_str(extension_index, &suffix);
            return Some(indexed);
        }
        return Some(format!("{template}{suffix}"));
    }

    page_media_filename(page_url)
        .map(|filename| format!("{filename}{suffix}.%(ext)s"))
        .or_else(|| Some(format!("page{suffix}.%(ext)s")))
}

#[allow(clippy::too_many_arguments)]
fn run_x_article_video_attempts(
    app: &Runtime,
    state: &backend::AppState,
    job_id: &str,
    input: &StartDownloadRequest,
    preset: &Preset,
    ffmpeg_location: Option<String>,
    yt_dlp: &Path,
    cancel_flag: &Arc<AtomicBool>,
    attempts: &[AuthAttempt],
) -> Result<Option<AttemptOutcome>, String> {
    let mut last_error = None;

    for attempt in attempts {
        if cancel_flag.load(Ordering::SeqCst) {
            mark_canceled(app, state, job_id)?;
            return Ok(Some(AttemptOutcome::Canceled));
        }

        update_phase(
            app,
            state,
            job_id,
            JobStatus::Resolving,
            3.0,
            "Resolving X article videos",
        )?;
        log(
            app,
            state,
            job_id,
            "info",
            &format!("Resolving X article videos ({}).", attempt.label),
        )?;

        let videos = match resolve_x_article_video_urls(yt_dlp, &input.url, &attempt.auth) {
            Ok(videos) => videos,
            Err(error) => {
                last_error = Some(error.clone());
                log(
                    app,
                    state,
                    job_id,
                    "warn",
                    &format!(
                        "Could not resolve X article videos with {}: {error}",
                        attempt.label
                    ),
                )?;
                continue;
            }
        };

        if videos.is_empty() {
            last_error = Some("X article did not contain downloadable videos.".to_string());
            log(
                app,
                state,
                job_id,
                "warn",
                "X article metadata did not contain downloadable videos.",
            )?;
            continue;
        }

        let video_count = videos.len();
        log(
            app,
            state,
            job_id,
            "info",
            &format!("Resolved {video_count} X article video(s)."),
        )?;

        let tweet_id = x_article_tweet_id(&input.url).unwrap_or_else(|| "article".to_string());
        for (index, video) in videos.iter().enumerate() {
            if cancel_flag.load(Ordering::SeqCst) {
                mark_canceled(app, state, job_id)?;
                return Ok(Some(AttemptOutcome::Canceled));
            }

            let mut stream_input = input.clone();
            stream_input.url = video.url.clone();
            stream_input.auth = AuthSource::None;
            if stream_input
                .filename_template
                .as_ref()
                .map_or(true, |value| value.trim().is_empty())
            {
                stream_input.filename_template = Some(format!(
                    "x-article-{tweet_id}-video-{} [%(id)s].%(ext)s",
                    index + 1
                ));
            }

            let phase = format!("Downloading X article video {}/{}", index + 1, video_count);
            match run_yt_dlp_attempt(
                app,
                state,
                job_id,
                &stream_input,
                preset,
                ffmpeg_location.clone(),
                yt_dlp,
                cancel_flag,
                &phase,
            )? {
                AttemptOutcome::Succeeded => {}
                AttemptOutcome::Canceled => return Ok(Some(AttemptOutcome::Canceled)),
                AttemptOutcome::Failed(error) => {
                    last_error = Some(error);
                    log(
                        app,
                        state,
                        job_id,
                        "warn",
                        &format!(
                            "Resolved X article video {}/{} did not download successfully.",
                            index + 1,
                            video_count
                        ),
                    )?;
                    return Ok(Some(AttemptOutcome::Failed(last_error.unwrap_or_else(
                        || "X article video download failed.".to_string(),
                    ))));
                }
            }
        }

        return Ok(Some(AttemptOutcome::Succeeded));
    }

    Ok(Some(AttemptOutcome::Failed(last_error.unwrap_or_else(
        || "X article did not expose downloadable videos. Check that the selected browser is logged in to X and use an article URL in the form /<screen>/article/<tweet-id>.".to_string(),
    ))))
}

#[allow(clippy::too_many_arguments)]
fn run_linkedin_feed_stream_attempts(
    app: &Runtime,
    state: &backend::AppState,
    job_id: &str,
    input: &StartDownloadRequest,
    page_url: &str,
    preset: &Preset,
    ffmpeg_location: Option<String>,
    yt_dlp: &Path,
    cancel_flag: &Arc<AtomicBool>,
    attempts: &[AuthAttempt],
) -> Result<Option<AttemptOutcome>, String> {
    let mut saw_stream = false;
    let mut last_error = None;

    for attempt in attempts {
        if cancel_flag.load(Ordering::SeqCst) {
            mark_canceled(app, state, job_id)?;
            return Ok(Some(AttemptOutcome::Canceled));
        }

        update_phase(
            app,
            state,
            job_id,
            JobStatus::Resolving,
            3.0,
            "Resolving LinkedIn stream",
        )?;
        log(
            app,
            state,
            job_id,
            "info",
            &format!("Resolving LinkedIn feed stream ({}).", attempt.label),
        )?;

        let stream_url = match resolve_linkedin_feed_stream_url(yt_dlp, page_url, &attempt.auth) {
            Ok(stream_url) => stream_url,
            Err(error) => {
                if let Some(source) = targeted_chrome_cookie_source(&attempt.auth, &error) {
                    log(
                        app,
                        state,
                        job_id,
                        "info",
                        "Retrying LinkedIn feed stream with direct Chrome cookie export.",
                    )?;
                    match resolve_linkedin_feed_stream_url_with_chrome_export(
                        yt_dlp, page_url, &source,
                    ) {
                        Ok(stream_url) => stream_url,
                        Err(retry_error) => {
                            last_error = Some(retry_error.clone());
                            log(
                                app,
                                state,
                                job_id,
                                "warn",
                                &format!(
                                    "Could not resolve LinkedIn feed stream with direct Chrome cookie export: {retry_error}"
                                ),
                            )?;
                            continue;
                        }
                    }
                } else {
                    last_error = Some(error.clone());
                    log(
                        app,
                        state,
                        job_id,
                        "warn",
                        &format!(
                            "Could not resolve LinkedIn feed stream with {}: {error}",
                            attempt.label
                        ),
                    )?;
                    continue;
                }
            }
        };

        saw_stream = true;
        log(
            app,
            state,
            job_id,
            "info",
            "Resolved LinkedIn HLS/DASH playlist from feed metadata.",
        )?;

        let mut stream_input = input.clone();
        stream_input.url = stream_url;
        stream_input.auth = AuthSource::None;

        match run_yt_dlp_attempt(
            app,
            state,
            job_id,
            &stream_input,
            preset,
            ffmpeg_location.clone(),
            yt_dlp,
            cancel_flag,
            "Downloading LinkedIn stream",
        )? {
            AttemptOutcome::Succeeded => return Ok(Some(AttemptOutcome::Succeeded)),
            AttemptOutcome::Canceled => return Ok(Some(AttemptOutcome::Canceled)),
            AttemptOutcome::Failed(error) => {
                last_error = Some(error);
                log(
                    app,
                    state,
                    job_id,
                    "warn",
                    "Resolved LinkedIn stream did not download successfully.",
                )?;
            }
        }
    }

    if saw_stream {
        Ok(Some(AttemptOutcome::Failed(last_error.unwrap_or_else(
            || "LinkedIn stream fallback failed.".to_string(),
        ))))
    } else {
        Ok(None)
    }
}

fn resolve_linkedin_feed_stream_url_with_chrome_export(
    yt_dlp: &Path,
    page_url: &str,
    source: &BrowserAuthSource,
) -> Result<String, String> {
    let cookie_path = chrome_cookies::export(source, page_url)
        .map_err(|error| format!("Could not export Chrome cookies directly: {error}"))?;
    let auth = AuthSource::CookieFile {
        path: cookie_path.display().to_string(),
    };
    let result = resolve_linkedin_feed_stream_url(yt_dlp, page_url, &auth).or_else(|error| {
        resolve_linkedin_feed_stream_url_with_cookie_file_http(page_url, &cookie_path)
            .map_err(|http_error| format!("{error}; direct HTTP fallback failed: {http_error}"))
    });
    let _ = fs::remove_file(&cookie_path);
    result
}

#[allow(clippy::too_many_arguments)]
fn run_yt_dlp_with_chrome_cookie_export(
    app: &Runtime,
    state: &backend::AppState,
    job_id: &str,
    input: &StartDownloadRequest,
    preset: &Preset,
    ffmpeg_location: Option<String>,
    yt_dlp: &Path,
    cancel_flag: &Arc<AtomicBool>,
    auth: &AuthSource,
    error: &str,
) -> Result<Option<AttemptOutcome>, String> {
    let Some(source) = targeted_chrome_cookie_source(auth, error) else {
        return Ok(None);
    };

    log(
        app,
        state,
        job_id,
        "info",
        "Retrying yt-dlp with direct Chrome cookie export.",
    )?;

    let cookie_path = match chrome_cookies::export(&source, &input.url) {
        Ok(path) => path,
        Err(error) => {
            return Ok(Some(AttemptOutcome::Failed(format!(
                "Could not export Chrome cookies directly: {error}"
            ))));
        }
    };

    let mut retry_input = input.clone();
    retry_input.auth = AuthSource::CookieFile {
        path: cookie_path.display().to_string(),
    };
    let outcome = run_yt_dlp_attempt(
        app,
        state,
        job_id,
        &retry_input,
        preset,
        ffmpeg_location,
        yt_dlp,
        cancel_flag,
        "Retrying yt-dlp (direct Chrome cookie export)",
    );
    let _ = fs::remove_file(&cookie_path);
    outcome.map(Some)
}

fn targeted_chrome_cookie_source(auth: &AuthSource, error: &str) -> Option<BrowserAuthSource> {
    if !should_try_targeted_chrome_cookie_export(error) {
        return None;
    }

    browser_sources(auth)
        .into_iter()
        .find(chrome_cookies::can_export)
}

fn should_try_targeted_chrome_cookie_export(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("item does not exist")
        || lower.contains("desktop keyring")
        || lower.contains("secretstorage")
        || lower.contains("org.freedesktop.secrets")
}

#[allow(clippy::too_many_arguments)]
fn run_yt_dlp_attempt(
    app: &Runtime,
    state: &backend::AppState,
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
    let mut attempt_logs = Vec::new();

    let mut command = Command::new(yt_dlp);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process_control::isolate_process_group(&mut command);

    let mut child = process_control::ChildGuard(
        command
            .spawn()
            .map_err(|error| format!("Could not start yt-dlp: {error}"))?,
    );
    state.set_process(job_id, child.id())?;
    let _process_registration = ProcessRegistration { state, job_id };

    let (sender, receiver) = mpsc::channel::<ProcessLine>();

    let mut reader_threads = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let sender = sender.clone();
        let app = app.clone();
        let state = state.clone();
        let job_id = job_id.to_owned();
        reader_threads.push(thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let line = ProcessLine::Stdout(line);
                // Publish originals even while the worker prepares an earlier
                // video for XRBAZAAR. The worker retries any publication error.
                if let Some(path) = ready_path_from_line(&line) {
                    let _ = publish_ready_file(&app, &state, &job_id, &path);
                }
                let _ = sender.send(line);
            }
        }));
    }

    if let Some(stderr) = child.stderr.take() {
        let sender = sender.clone();
        reader_threads.push(thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = sender.send(ProcessLine::Stderr(line));
            }
        }));
    }

    let download_pid = child.id();
    let mut prepared = HashSet::new();
    let mut file_ready = |path: &str| -> Result<(), String> {
        if !prepared.insert(path.to_string()) {
            return Ok(());
        }
        publish_ready_file(app, state, job_id, path)?;
        if input.output_profile == super::OutputProfile::Xrbazaar {
            let ffmpeg = tools::find_tool(app, "ffmpeg")
                .ok_or("FFmpeg is required for XRBAZAAR preparation.")?;
            let ffprobe = tools::find_tool(app, "ffprobe")
                .ok_or("ffprobe is required for XRBAZAAR verification.")?;
            update_phase(
                app,
                state,
                job_id,
                JobStatus::Postprocessing,
                95.0,
                "Preparing for XRBAZAAR",
            )?;
            let result = super::xrbazaar::prepare_video(
                &ffmpeg,
                &ffprobe,
                Path::new(path),
                cancel_flag,
                &mut |pid| state.set_process(job_id, pid.unwrap_or(download_pid)),
                &mut |percent| {
                    update_phase(
                        app,
                        state,
                        job_id,
                        JobStatus::Postprocessing,
                        95.0 + percent * 0.04,
                        &format!("Preparing for XRBAZAAR · {}%", percent as u32),
                    )
                },
            );
            state.set_process(job_id, download_pid)?;
            publish_ready_file(app, state, job_id, &result?.display().to_string())?;
        }
        Ok(())
    };

    let result = (|| loop {
        if cancel_flag.load(Ordering::SeqCst) {
            process_control::force_kill_process_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
            mark_canceled(app, state, job_id)?;
            return Ok(AttemptOutcome::Canceled);
        }

        drain_process_lines(
            app,
            state,
            job_id,
            &receiver,
            cancel_flag,
            Some(&mut attempt_logs),
            &mut file_ready,
            64,
        )?;

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
            for reader in reader_threads.drain(..) {
                let _ = reader.join();
            }
            while let Ok(line) = receiver.try_recv() {
                attempt_logs.push(sanitized_process_line_message(&line));
                if let Some(path) = ready_path_from_line(&line) {
                    file_ready(&path)?;
                } else {
                    handle_process_line(app, state, job_id, line)?;
                }
            }

            if cancel_flag.load(Ordering::SeqCst) {
                mark_canceled(app, state, job_id)?;
                return Ok(AttemptOutcome::Canceled);
            }

            if status.success() {
                return Ok(AttemptOutcome::Succeeded);
            }

            let error = yt_dlp_failure_hint_from_messages(&attempt_logs)
                .unwrap_or_else(|| format!("yt-dlp exited with status {status}"));
            return Ok(AttemptOutcome::Failed(error));
        }

        thread::sleep(Duration::from_millis(180));
    })();
    // On cancellation or preparation failure, keep every fully downloaded file
    // already reported by yt-dlp, including records buffered during conversion.
    if !matches!(child.try_wait(), Ok(Some(_))) {
        process_control::force_kill_process_group(child.id());
        let _ = child.kill();
        let _ = child.wait();
    }
    for reader in reader_threads {
        let _ = reader.join();
    }
    for line in receiver.try_iter() {
        if let Some(path) = ready_path_from_line(&line) {
            if Path::new(&path).is_file() {
                publish_ready_file(app, state, job_id, &path)?;
            }
        }
    }
    result
}

fn drain_process_lines(
    app: &Runtime,
    state: &backend::AppState,
    job_id: &str,
    receiver: &Receiver<ProcessLine>,
    cancel_flag: &Arc<AtomicBool>,
    mut captured: Option<&mut Vec<String>>,
    file_ready: &mut dyn FnMut(&str) -> Result<(), String>,
    limit: usize,
) -> Result<(), String> {
    for _ in 0..limit {
        if cancel_flag.load(Ordering::SeqCst) {
            break;
        }

        match receiver.try_recv() {
            Ok(line) => {
                if let Some(captured) = captured.as_mut() {
                    captured.push(sanitized_process_line_message(&line));
                }
                if let Some(path) = ready_path_from_line(&line) {
                    file_ready(&path)?;
                } else {
                    handle_process_line(app, state, job_id, line)?;
                }
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
    Ok(())
}

struct ProcessRegistration<'a> {
    state: &'a backend::AppState,
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
        "--ignore-config".to_string(),
        "--newline".to_string(),
        "--no-color".to_string(),
        "--progress".to_string(),
        "--no-simulate".to_string(),
        "--print".to_string(),
        "after_move:__DOWNLOADER_READY__%(filepath)j".to_string(),
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct XArticleVideo {
    media_id: String,
    url: String,
    bitrate: Option<i64>,
}

#[derive(Debug, Clone)]
struct XCookies {
    header: String,
    csrf_token: Option<String>,
}

fn is_x_article_preset(preset: &Preset) -> bool {
    preset.id == "x-article-video-highest"
}

fn is_x_article_url(input: &str) -> bool {
    let Ok(url) = Url::parse(input) else {
        return false;
    };
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_ascii_lowercase();

    (host == "x.com" || host == "twitter.com")
        && url.path_segments().is_some_and(|segments| {
            segments
                .collect::<Vec<_>>()
                .windows(2)
                .any(|pair| pair[0] == "article")
        })
}

fn x_article_tweet_id(input: &str) -> Option<String> {
    let url = Url::parse(input).ok()?;
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    if host != "x.com" && host != "twitter.com" {
        return None;
    }

    let segments = url.path_segments()?.collect::<Vec<_>>();
    let article_index = segments.iter().position(|segment| *segment == "article")?;
    if article_index == 0 || segments.get(article_index - 1).copied() == Some("i") {
        return None;
    }

    segments
        .get(article_index + 1)
        .and_then(|segment| segment.split('-').next())
        .filter(|segment| !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit()))
        .map(ToString::to_string)
}

fn resolve_x_article_video_urls(
    yt_dlp: &Path,
    page_url: &str,
    auth: &AuthSource,
) -> Result<Vec<XArticleVideo>, String> {
    let tweet_id = x_article_tweet_id(page_url).ok_or_else(|| {
        "X article resolver needs an article URL in the form /<screen>/article/<tweet-id>."
            .to_string()
    })?;
    let cookies = x_cookies_for_auth(yt_dlp, page_url, auth)?;
    let bearer = fetch_x_bearer_token(Some(&cookies.header))?;
    let api_url = x_tweet_result_api_url(&tweet_id)?;

    let mut request = ureq::get(&api_url)
        .header("User-Agent", BROWSER_USER_AGENT)
        .header("Accept", "*/*")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Authorization", bearer)
        .header("Cookie", cookies.header)
        .header("Referer", page_url)
        .header("X-Twitter-Active-User", "yes")
        .header("X-Twitter-Auth-Type", "OAuth2Session")
        .header("X-Twitter-Client-Language", "en");
    if let Some(csrf_token) = cookies.csrf_token.as_ref() {
        request = request.header("X-Csrf-Token", csrf_token);
    }

    let mut response = request
        .call()
        .map_err(|error| format!("X article API request failed: {error}"))?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("Could not read X article API response: {error}"))?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|error| format!("Could not parse X article API response: {error}"))?;

    Ok(extract_x_article_videos(&value))
}

fn x_cookies_for_auth(
    yt_dlp: &Path,
    page_url: &str,
    auth: &AuthSource,
) -> Result<XCookies, String> {
    match auth {
        AuthSource::None => {
            Err("X article extraction requires browser cookies or a cookies.txt file.".to_string())
        }
        AuthSource::CookieFile { path } if !path.trim().is_empty() => {
            parse_x_cookie_file(Path::new(path))
        }
        AuthSource::CookieFile { .. } => Err("X cookies.txt path is empty.".to_string()),
        AuthSource::Browser { .. } => {
            let source = browser_sources(auth)
                .into_iter()
                .next()
                .ok_or_else(|| "No browser cookie source is configured for X.".to_string())?;
            let cookie_path =
                std::env::temp_dir().join(format!("downloader-x-cookies-{}.txt", x_temp_suffix()));
            let browser_arg = browser_cookie_arg(&source);
            let cookie_path_arg = cookie_path.display().to_string();
            let output = Command::new(yt_dlp)
                .arg("--ignore-config")
                .args([
                    "--cookies-from-browser".to_string(),
                    browser_arg,
                    "--cookies".to_string(),
                    cookie_path_arg,
                    "--force-generic-extractor".to_string(),
                    "--skip-download".to_string(),
                    "--no-playlist".to_string(),
                    "--no-color".to_string(),
                    "--socket-timeout".to_string(),
                    "20".to_string(),
                    page_url.to_string(),
                ])
                .output()
                .map_err(|error| format!("Could not export X browser cookies: {error}"))?;

            let has_cookie_file = cookie_path
                .metadata()
                .map(|metadata| metadata.len() > 0)
                .unwrap_or(false);
            if !has_cookie_file {
                let stderr =
                    redaction::sanitize_log_line(String::from_utf8_lossy(&output.stderr).trim());
                let _ = fs::remove_file(&cookie_path);
                return Err(if stderr.is_empty() {
                    "Could not export X browser cookies.".to_string()
                } else {
                    format!("Could not export X browser cookies: {stderr}")
                });
            }

            let cookies = parse_x_cookie_file(&cookie_path);
            let _ = fs::remove_file(&cookie_path);
            cookies
        }
    }
}

fn parse_x_cookie_file(path: &Path) -> Result<XCookies, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("Could not read X cookie file {}: {error}", path.display()))?;
    let mut cookies = BTreeMap::<String, String>::new();
    let mut csrf_token = None;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.starts_with('#') {
            continue;
        }

        let parts = line.split('\t').collect::<Vec<_>>();
        if parts.len() < 7 {
            continue;
        }

        let domain = parts[0].to_ascii_lowercase();
        if !is_x_cookie_domain(&domain) {
            continue;
        }

        let name = parts[5].trim();
        let value = parts[6].trim();
        if name.is_empty() || value.is_empty() {
            continue;
        }
        if name == "ct0" {
            csrf_token = Some(value.to_string());
        }
        cookies.insert(name.to_string(), value.to_string());
    }

    let header = cookies
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");

    if header.is_empty() {
        Err("X cookie file did not contain x.com or twitter.com cookies.".to_string())
    } else {
        Ok(XCookies { header, csrf_token })
    }
}

fn is_x_cookie_domain(domain: &str) -> bool {
    let domain = domain.trim_start_matches('.');
    domain == "x.com"
        || domain.ends_with(".x.com")
        || domain == "twitter.com"
        || domain.ends_with(".twitter.com")
}

fn fetch_x_bearer_token(cookie_header: Option<&str>) -> Result<String, String> {
    let homepage = http_get_x_text("https://x.com/", cookie_header, &[])?;
    let main_js_url = find_x_main_js_url(&homepage)
        .ok_or_else(|| "Could not find X web client bundle URL.".to_string())?;
    let main_js = http_get_x_text(&main_js_url, cookie_header, &[])?;
    let bearer = Regex::new(r#"Bearer ([A-Za-z0-9%._-]+)"#)
        .ok()
        .and_then(|regex| {
            regex
                .captures(&main_js)
                .and_then(|captures| captures.get(1).map(|match_| match_.as_str().to_string()))
        })
        .ok_or_else(|| "Could not find X web bearer token in client bundle.".to_string())?;

    Ok(format!("Bearer {bearer}"))
}

fn find_x_main_js_url(html: &str) -> Option<String> {
    Regex::new(r#"https://abs\.twimg\.com/responsive-web/client-web/main\.[A-Za-z0-9]+\.js"#)
        .ok()
        .and_then(|regex| regex.find(html).map(|match_| match_.as_str().to_string()))
}

fn http_get_x_text(
    url: &str,
    cookie_header: Option<&str>,
    extra_headers: &[(&str, &str)],
) -> Result<String, String> {
    let mut request = ureq::get(url)
        .header("User-Agent", BROWSER_USER_AGENT)
        .header("Accept", "*/*")
        .header("Accept-Language", "en-US,en;q=0.9");
    if let Some(cookie_header) = cookie_header.filter(|value| !value.is_empty()) {
        request = request.header("Cookie", cookie_header);
    }
    for (key, value) in extra_headers {
        if !value.is_empty() {
            request = request.header(*key, *value);
        }
    }

    let mut response = request
        .call()
        .map_err(|error| format!("HTTP request to {url} failed: {error}"))?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("Could not read HTTP response from {url}: {error}"))
}

fn x_tweet_result_api_url(tweet_id: &str) -> Result<String, String> {
    let mut url = Url::parse(&format!(
        "https://x.com/i/api/graphql/{X_TWEET_RESULT_QUERY_ID}/{X_TWEET_RESULT_OPERATION}"
    ))
    .map_err(|error| format!("Could not build X article API URL: {error}"))?;
    let variables = json!({
        "tweetId": tweet_id,
        "withCommunity": false,
        "includePromotedContent": false,
        "withVoice": false
    })
    .to_string();
    let features = x_graphql_features().to_string();
    let field_toggles = json!({
        "withArticleRichContentState": true,
        "withArticlePlainText": false,
        "withArticleSummaryText": true,
        "withArticleVoiceOver": true,
        "withGrokAnalyze": true,
        "withDisallowedReplyControls": true,
        "withPayments": true,
        "withAuxiliaryUserLabels": true
    })
    .to_string();

    url.query_pairs_mut()
        .append_pair("variables", &variables)
        .append_pair("features", &features)
        .append_pair("fieldToggles", &field_toggles);
    Ok(url.to_string())
}

fn x_graphql_features() -> Value {
    let mut features = serde_json::Map::new();
    for key in X_GRAPHQL_FEATURES {
        features.insert((*key).to_string(), Value::Bool(true));
    }
    Value::Object(features)
}

fn extract_x_article_videos(value: &Value) -> Vec<XArticleVideo> {
    let Some(article) = value.pointer("/data/tweetResult/result/article/article_results/result")
    else {
        return Vec::new();
    };

    let media_entities = article
        .get("media_entities")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut videos = Vec::new();
    let mut seen = HashSet::<String>::new();

    if let Some(cover_media) = article.get("cover_media") {
        push_x_article_video(cover_media, &mut videos, &mut seen);
    }

    let mut media_ids = Vec::new();
    if let Some(content_state) = article.get("content_state") {
        collect_x_media_ids(content_state, &mut media_ids);
    }
    for media_id in media_ids {
        if let Some(entity) = media_entities.iter().find(|entity| {
            x_entity_media_id(entity)
                .as_deref()
                .is_some_and(|entity_media_id| entity_media_id == media_id.as_str())
        }) {
            push_x_article_video(entity, &mut videos, &mut seen);
        }
    }

    for entity in &media_entities {
        push_x_article_video(entity, &mut videos, &mut seen);
    }

    videos
}

fn push_x_article_video(
    entity: &Value,
    videos: &mut Vec<XArticleVideo>,
    seen: &mut HashSet<String>,
) {
    let Some(video) = x_article_video_from_entity(entity) else {
        return;
    };
    let key = if video.media_id.is_empty() {
        format!("url:{}", video.url)
    } else {
        format!("id:{}", video.media_id)
    };
    if seen.insert(key) {
        videos.push(video);
    }
}

fn x_article_video_from_entity(entity: &Value) -> Option<XArticleVideo> {
    let media_info = entity.get("media_info")?;
    let typename = media_info
        .get("__typename")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(typename, "ApiVideo" | "ApiGif") {
        return None;
    }

    let variants = media_info.get("variants").and_then(Value::as_array)?;
    let best_variant = variants
        .iter()
        .filter(|variant| x_variant_url(variant).is_some() && x_variant_is_mp4(variant))
        .max_by_key(|variant| x_variant_bitrate(variant).unwrap_or_default())
        .or_else(|| {
            variants
                .iter()
                .find(|variant| x_variant_url(variant).is_some() && x_variant_is_hls(variant))
        })?;

    Some(XArticleVideo {
        media_id: x_entity_media_id(entity).unwrap_or_default(),
        url: x_variant_url(best_variant)?,
        bitrate: x_variant_bitrate(best_variant),
    })
}

fn x_variant_url(variant: &Value) -> Option<String> {
    variant.get("url").and_then(Value::as_str).map(|url| {
        url.replace("\\/", "/")
            .trim_end_matches([',', ';', ')', ']', '}'])
            .to_string()
    })
}

fn x_variant_bitrate(variant: &Value) -> Option<i64> {
    variant
        .get("bit_rate")
        .or_else(|| variant.get("bitrate"))
        .and_then(Value::as_i64)
}

fn x_variant_is_mp4(variant: &Value) -> bool {
    let content_type = variant
        .get("content_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    content_type == "video/mp4"
        || x_variant_url(variant)
            .as_deref()
            .is_some_and(|url| url.contains(".mp4"))
}

fn x_variant_is_hls(variant: &Value) -> bool {
    let content_type = variant
        .get("content_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    content_type == "application/x-mpegurl"
        || content_type == "application/vnd.apple.mpegurl"
        || x_variant_url(variant)
            .as_deref()
            .is_some_and(|url| url.contains(".m3u8"))
}

fn x_entity_media_id(entity: &Value) -> Option<String> {
    entity
        .get("media_id")
        .or_else(|| entity.get("id_str"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn collect_x_media_ids(value: &Value, media_ids: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(media_id) = map.get("mediaId").and_then(Value::as_str) {
                if !media_ids.iter().any(|existing| existing == media_id) {
                    media_ids.push(media_id.to_string());
                }
            }
            for child in map.values() {
                collect_x_media_ids(child, media_ids);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_x_media_ids(item, media_ids);
            }
        }
        _ => {}
    }
}

fn x_temp_suffix() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}-{millis}", std::process::id())
}

fn is_linkedin_video_post_preset(preset: &Preset) -> bool {
    matches!(
        preset.id.as_str(),
        "linkedin-post-video-highest" | "linkedin-feed-update-video-highest"
    )
}

fn is_generic_page_video_preset(preset: &Preset) -> bool {
    preset.id == "generic-page-video-highest"
}

fn should_resolve_embedded_page(input: &str) -> bool {
    matches!(super::sites::detect_site(input), SiteKind::Generic)
}

fn page_media_filename(input: &str) -> Option<String> {
    let url = Url::parse(input).ok()?;
    let slug = url
        .path_segments()?
        .filter(|part| !part.is_empty())
        .next_back()?;
    let stem = slug
        .strip_suffix(".html")
        .or_else(|| slug.strip_suffix(".htm"))
        .unwrap_or(slug);
    let filename = stem
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .collect::<String>();

    (!filename.is_empty()).then_some(filename)
}

fn resolve_embedded_page_video_url(page_url: &str) -> Result<Option<String>, String> {
    Ok(resolve_embedded_page_video_urls(page_url)?
        .into_iter()
        .next())
}

fn resolve_embedded_page_video_urls(page_url: &str) -> Result<Vec<String>, String> {
    let html = http_get_page_text(page_url)?;
    Ok(extract_embedded_page_video_urls(&html, page_url))
}

fn resolve_page_video_target(yt_dlp: &Path, page_url: &str, auth: &AuthSource) -> Option<String> {
    resolve_embedded_page_video_url(page_url)
        .ok()
        .flatten()
        .or_else(|| {
            resolve_embedded_page_video_url_from_dump(yt_dlp, page_url, auth)
                .ok()
                .flatten()
        })
}

fn resolve_embedded_page_video_url_from_dump(
    yt_dlp: &Path,
    page_url: &str,
    auth: &AuthSource,
) -> Result<Option<String>, String> {
    Ok(
        resolve_embedded_page_video_urls_from_dump(yt_dlp, page_url, auth)?
            .into_iter()
            .next(),
    )
}

fn resolve_embedded_page_video_urls_from_dump(
    yt_dlp: &Path,
    page_url: &str,
    auth: &AuthSource,
) -> Result<Vec<String>, String> {
    let mut args = vec![
        "--dump-pages".to_string(),
        "--skip-download".to_string(),
        "--no-playlist".to_string(),
        "--no-color".to_string(),
        "--socket-timeout".to_string(),
        "20".to_string(),
    ];
    append_auth_args(&mut args, auth);
    args.push(page_url.to_string());

    let output = Command::new(yt_dlp)
        .arg("--ignore-config")
        .args(args)
        .output()
        .map_err(|error| format!("could not start page-data inspection: {error}"))?;

    Ok(extract_embedded_page_video_urls_from_dump(
        &output.stdout,
        page_url,
    ))
}

#[cfg(test)]
fn extract_embedded_page_video_url_from_dump(stdout: &[u8], page_url: &str) -> Option<String> {
    extract_embedded_page_video_urls_from_dump(stdout, page_url)
        .into_iter()
        .next()
}

fn extract_embedded_page_video_urls_from_dump(stdout: &[u8], page_url: &str) -> Vec<String> {
    let output = String::from_utf8_lossy(stdout);
    let mut targets = Vec::new();
    for line in output.lines().map(str::trim) {
        if !looks_like_base64_dump_line(line) {
            continue;
        }
        let Ok(decoded) = BASE64_STANDARD.decode(line) else {
            continue;
        };
        let page = String::from_utf8_lossy(&decoded);
        for target in extract_embedded_page_video_urls(&page, page_url) {
            push_unique_page_target(&mut targets, target);
        }
    }

    for target in extract_embedded_page_video_urls(&output, page_url) {
        push_unique_page_target(&mut targets, target);
    }
    targets
}

fn http_get_page_text(url: &str) -> Result<String, String> {
    let mut response = ureq::get(url)
        .header("User-Agent", BROWSER_USER_AGENT)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "en-US,en;q=0.9")
        .call()
        .map_err(|error| format!("page request failed: {error}"))?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("could not read the page response: {error}"))
}

fn normalize_embedded_media_html(html: &str) -> String {
    html.replace("\\\"", "\"")
        .replace("\\/", "/")
        .replace("\\u002F", "/")
        .replace("\\u002f", "/")
        .replace("\\u003A", ":")
        .replace("\\u003a", ":")
        .replace("\\u0026", "&")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#x22;", "\"")
        .replace("&#X22;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
fn extract_embedded_page_video_url(html: &str, page_url: &str) -> Option<String> {
    extract_embedded_page_video_urls(html, page_url)
        .into_iter()
        .next()
}

#[derive(Debug, Clone)]
struct PageVideoCandidate {
    position: usize,
    score: i32,
    identity: String,
    url: String,
}

fn extract_embedded_page_video_urls(html: &str, page_url: &str) -> Vec<String> {
    let normalized = normalize_embedded_media_html(html);
    let mut candidates = Vec::<PageVideoCandidate>::new();

    for (position, url) in extract_squarespace_video_urls(&normalized) {
        push_page_candidate(&mut candidates, url, 1_300, position);
    }

    let source_regex = Regex::new(
        r#"(?is)<(?:video|source)\b[^>]*\b(?:src|data-src|data-video-url|data-hls|data-dash|data-mp4)\s*=\s*(?:\"([^\"]+)\"|'([^']+)'|([^\s>]+))"#,
    )
    .expect("valid native video source regex");
    let video_block_regex =
        Regex::new(r#"(?is)<video\b[^>]*>.*?</video\s*>"#).expect("valid video block regex");
    let mut video_block_ranges = Vec::new();
    for video_block in video_block_regex.find_iter(&normalized) {
        video_block_ranges.push(video_block.start()..video_block.end());
        let mut block_candidates = Vec::<PageVideoCandidate>::new();
        for captures in source_regex.captures_iter(video_block.as_str()) {
            let candidate =
                (1..=3).find_map(|index| captures.get(index).map(|value| value.as_str()));
            if let Some(url) = candidate.and_then(|value| resolve_media_candidate(page_url, value))
            {
                let position =
                    video_block.start() + captures.get(0).map_or(0, |capture| capture.start());
                push_page_candidate(&mut block_candidates, url, 1_000, position);
            }
        }
        if let Some(best) = block_candidates
            .into_iter()
            .max_by_key(|candidate| candidate.score)
        {
            push_page_candidate(&mut candidates, best.url, 1_000, video_block.start());
        }
    }

    for captures in source_regex.captures_iter(&normalized) {
        let position = captures.get(0).map_or(0, |capture| capture.start());
        if video_block_ranges
            .iter()
            .any(|range| range.contains(&position))
        {
            continue;
        }
        let candidate = (1..=3).find_map(|index| captures.get(index).map(|value| value.as_str()));
        if let Some(url) = candidate.and_then(|value| resolve_media_candidate(page_url, value)) {
            push_page_candidate(&mut candidates, url, 1_000, position);
        }
    }

    let structured_regex = Regex::new(
        r#"(?i)\"(contentUrl|embedUrl|playerUrl|playbackUrl|manifestUrl|hlsUrl|dashUrl|file)\"\s*:\s*\"([^\"]+)\""#,
    )
    .expect("valid structured video descriptor regex");
    for captures in structured_regex.captures_iter(&normalized) {
        let Some(key) = captures
            .get(1)
            .map(|value| value.as_str().to_ascii_lowercase())
        else {
            continue;
        };
        let Some(value) = captures.get(2).map(|value| value.as_str()) else {
            continue;
        };
        let position = captures.get(0).map_or(0, |capture| capture.start());
        let candidate = resolve_media_candidate(page_url, value)
            .or_else(|| resolve_stream_descriptor_candidate(page_url, value, &key));
        if let Some(url) = candidate {
            push_page_candidate(&mut candidates, url, 900, position);
        } else if matches!(key.as_str(), "embedurl" | "playerurl") {
            if let Some(url) = resolve_embed_candidate(page_url, value) {
                push_page_candidate(&mut candidates, url, 850, position);
            }
        }
    }

    let mut meta_candidates = Vec::<PageVideoCandidate>::new();
    let meta_regex = Regex::new(r#"(?is)<meta\b[^>]*>"#).expect("valid meta tag regex");
    let meta_kind_regex =
        Regex::new(r#"(?is)\b(?:property|name)\s*=\s*(?:\"([^\"]+)\"|'([^']+)'|([^\s>]+))"#)
            .expect("valid meta kind regex");
    let content_regex = Regex::new(r#"(?is)\bcontent\s*=\s*(?:\"([^\"]+)\"|'([^']+)'|([^\s>]+))"#)
        .expect("valid meta content regex");
    for meta_match in meta_regex.find_iter(&normalized) {
        let tag = meta_match.as_str();
        let kind = meta_kind_regex
            .captures(tag)
            .and_then(|captures| first_optional_capture(&captures))
            .map(str::to_ascii_lowercase);
        if !kind.as_deref().is_some_and(|kind| {
            matches!(
                kind,
                "og:video"
                    | "og:video:url"
                    | "og:video:secure_url"
                    | "twitter:player"
                    | "twitter:player:stream"
            )
        }) {
            continue;
        }
        let Some(value) = content_regex
            .captures(tag)
            .and_then(|captures| first_optional_capture(&captures))
        else {
            continue;
        };
        if let Some(url) = resolve_media_candidate(page_url, value) {
            push_page_candidate(&mut meta_candidates, url, 950, meta_match.start());
        } else if let Some(url) = resolve_embed_candidate(page_url, value) {
            push_page_candidate(&mut meta_candidates, url, 800, meta_match.start());
        }
    }

    let iframe_regex = Regex::new(
        r#"(?is)<iframe\b[^>]*\b(?:src|data-src)\s*=\s*(?:\"([^\"]+)\"|'([^']+)'|([^\s>]+))"#,
    )
    .expect("valid iframe regex");
    for captures in iframe_regex.captures_iter(&normalized) {
        if let Some(url) = first_optional_capture(&captures)
            .and_then(|value| resolve_embed_candidate(page_url, value))
        {
            let position = captures.get(0).map_or(0, |capture| capture.start());
            push_page_candidate(&mut candidates, url, 800, position);
        }
    }

    if candidates.is_empty() {
        candidates = meta_candidates;
    } else {
        for meta_candidate in meta_candidates {
            if candidates
                .iter()
                .any(|candidate| candidate.identity == meta_candidate.identity)
            {
                let base_score =
                    meta_candidate.score - page_candidate_quality_score(&meta_candidate.url);
                push_page_candidate(
                    &mut candidates,
                    meta_candidate.url,
                    base_score,
                    meta_candidate.position,
                );
            }
        }
    }

    let media_url_regex = Regex::new(
        r#"(?i)https?://[^\s\"'<>\\]+\.(?:m3u8|mpd|mp4|mov|m4v|webm|mkv)(?:\?[^\s\"'<>\\]*)?"#,
    )
    .expect("valid media URL regex");
    if candidates.is_empty() {
        for value in media_url_regex.find_iter(&normalized) {
            if video_block_ranges
                .iter()
                .any(|range| range.contains(&value.start()))
            {
                continue;
            }
            if let Some(url) = resolve_media_candidate(page_url, value.as_str()) {
                push_page_candidate(&mut candidates, url, 400, value.start());
            }
        }
    }

    candidates.sort_by(|left, right| {
        left.position
            .cmp(&right.position)
            .then_with(|| right.score.cmp(&left.score))
            .then_with(|| left.url.cmp(&right.url))
    });
    candidates
        .into_iter()
        .map(|candidate| candidate.url)
        .collect()
}

fn first_optional_capture<'a>(captures: &regex::Captures<'a>) -> Option<&'a str> {
    (1..captures.len()).find_map(|index| captures.get(index).map(|value| value.as_str()))
}

fn push_page_candidate(
    candidates: &mut Vec<PageVideoCandidate>,
    url: String,
    base_score: i32,
    position: usize,
) {
    let identity = page_video_identity(&url);
    if let Some(existing) = candidates
        .iter_mut()
        .find(|candidate| candidate.identity == identity)
    {
        existing.position = existing.position.min(position);
        let score = base_score + page_candidate_quality_score(&url);
        if score > existing.score {
            existing.score = score;
            existing.url = url;
        }
        return;
    }
    let score = base_score + page_candidate_quality_score(&url);
    candidates.push(PageVideoCandidate {
        position,
        score,
        identity,
        url,
    });
}

fn push_unique_page_target(targets: &mut Vec<String>, url: String) {
    let identity = page_video_identity(&url);
    if !targets
        .iter()
        .any(|target| page_video_identity(target) == identity)
    {
        targets.push(url);
    }
}

fn page_video_identity(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return url.to_ascii_lowercase();
    };
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let mut segments = parsed
        .path_segments()
        .map(|segments| segments.map(str::to_ascii_lowercase).collect::<Vec<_>>())
        .unwrap_or_default();

    let mut is_file_media = false;
    if let Some(last) = segments.last_mut() {
        is_file_media = ["m3u8", "mpd", "mp4", "mov", "m4v", "webm", "mkv"]
            .iter()
            .any(|extension| last.ends_with(&format!(".{extension}")));
        let stem = last
            .rsplit_once('.')
            .map_or(last.as_str(), |(stem, _)| stem)
            .to_string();
        if matches!(stem.as_str(), "master" | "manifest" | "playlist" | "index") {
            segments.pop();
        } else {
            let quality_suffix = Regex::new(
                r#"(?i)(?:[-_.](?:2160|1440|1080|720|540|480|360|240)(?:p)?|[-_.](?:uhd|fhd|hd|sd))$"#,
            )
            .expect("valid quality suffix regex");
            *last = quality_suffix.replace(&stem, "").to_string();
        }
    }

    let mut identity = format!("{host}/{}", segments.join("/"));
    if !is_file_media {
        let stable_query = parsed
            .query_pairs()
            .filter(|(key, _)| {
                !matches!(
                    key.to_ascii_lowercase().as_str(),
                    "token"
                        | "sig"
                        | "signature"
                        | "expires"
                        | "expiry"
                        | "policy"
                        | "key-pair-id"
                        | "e"
                        | "t"
                )
            })
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>();
        if !stable_query.is_empty() {
            identity.push('?');
            identity.push_str(&stable_query.join("&"));
        }
    }
    identity
}

fn page_candidate_quality_score(url: &str) -> i32 {
    let lower = url.to_ascii_lowercase();
    let mut score = if lower.contains(".m3u8") || lower.contains(".mpd") {
        200
    } else {
        100
    };
    if lower.contains("master") || lower.contains("manifest") || lower.contains("playlist") {
        score += 80;
    }
    if lower.contains("subtitle") || lower.contains("/sub/") || lower.contains("st=audio") {
        score -= 300;
    }
    for (label, bonus) in [
        ("2160", 60),
        ("1440", 50),
        ("1080", 40),
        ("720", 30),
        ("480", 20),
    ] {
        if lower.contains(label) {
            score += bonus;
            break;
        }
    }
    score
}

fn resolve_embed_candidate(page_url: &str, candidate: &str) -> Option<String> {
    let resolved = resolve_http_candidate(page_url, candidate)?;
    let host = resolved.host_str()?.to_ascii_lowercase();
    let path = resolved.path().to_ascii_lowercase();
    let looks_like_player = host.starts_with("player.")
        || path.contains("/embed/")
        || path.contains("/player/")
        || path.contains("/video/");
    looks_like_player.then(|| resolved.to_string())
}

fn resolve_stream_descriptor_candidate(
    page_url: &str,
    candidate: &str,
    descriptor_key: &str,
) -> Option<String> {
    if !matches!(
        descriptor_key,
        "playbackurl" | "manifesturl" | "hlsurl" | "dashurl"
    ) {
        return None;
    }
    let resolved = resolve_http_candidate(page_url, candidate)?;
    let path = resolved.path().to_ascii_lowercase();
    let looks_like_stream = path.contains("/playlist/")
        || path.contains("/manifest")
        || path.contains("/dash/")
        || path.contains("/hls/")
        || path.contains("/dynamic/");
    looks_like_stream.then(|| resolved.to_string())
}

fn extract_squarespace_video_urls(normalized_html: &str) -> Vec<(usize, String)> {
    let regex =
        Regex::new(r#""alexandriaUrl"\s*:\s*"([^"]+)""#).expect("valid Squarespace video regex");
    let mut targets = Vec::new();
    for captures in regex.captures_iter(normalized_html) {
        let Some(base_url) = captures.get(1).map(|capture| capture.as_str()) else {
            continue;
        };
        if !base_url.contains("{variant}") {
            continue;
        }

        let playlist_url = base_url.replace("{variant}", "playlist.m3u8");
        let Ok(parsed) = Url::parse(&playlist_url) else {
            continue;
        };
        let host = parsed
            .host_str()
            .unwrap_or_default()
            .trim_start_matches("www.")
            .to_ascii_lowercase();
        if matches!(parsed.scheme(), "http" | "https")
            && (host == "squarespace-cdn.com" || host.ends_with(".squarespace-cdn.com"))
            && parsed.path().ends_with("/playlist.m3u8")
        {
            let position = captures.get(0).map_or(0, |capture| capture.start());
            if !targets
                .iter()
                .any(|(_, existing)| existing == &playlist_url)
            {
                targets.push((position, playlist_url));
            }
        }
    }
    targets
}

fn resolve_media_candidate(page_url: &str, candidate: &str) -> Option<String> {
    let resolved = resolve_http_candidate(page_url, candidate)?;
    let path = resolved.path().to_ascii_lowercase();
    const MEDIA_EXTENSIONS: &[&str] = &[".m3u8", ".mpd", ".mp4", ".mov", ".m4v", ".webm", ".mkv"];
    MEDIA_EXTENSIONS
        .iter()
        .any(|extension| path.ends_with(extension))
        .then(|| resolved.to_string())
}

fn resolve_http_candidate(page_url: &str, candidate: &str) -> Option<Url> {
    let candidate = candidate
        .trim()
        .trim_matches(|character| matches!(character, '"' | '\'' | '<' | '>'))
        .trim_end_matches([',', ';', ')', ']', '}']);
    let page = Url::parse(page_url).ok()?;
    let resolved = Url::parse(candidate)
        .or_else(|_| page.join(candidate))
        .ok()?;
    matches!(resolved.scheme(), "http" | "https").then_some(resolved)
}

fn linkedin_feed_update_url(input: &str) -> Option<String> {
    let Ok(url) = Url::parse(input) else {
        return None;
    };
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_ascii_lowercase();

    if host != "linkedin.com" && !host.ends_with(".linkedin.com") {
        return None;
    }

    let path = url.path();
    let feed_match = Regex::new(r"(?i)^/feed/update/urn:li:(activity|ugcpost):(\d+)/?$")
        .ok()?
        .captures(path);
    let post_match = Regex::new(r"(?i)^/posts/.+-(activity|ugcpost)-(\d+)-[^/]+/?$")
        .ok()?
        .captures(path);
    let captures = feed_match.or(post_match)?;
    let urn_kind = if captures.get(1)?.as_str().eq_ignore_ascii_case("ugcpost") {
        "ugcPost"
    } else {
        "activity"
    };
    let id = captures.get(2)?.as_str();

    Some(format!(
        "https://www.linkedin.com/feed/update/urn:li:{urn_kind}:{id}/"
    ))
}

fn resolve_linkedin_feed_stream_url(
    yt_dlp: &Path,
    page_url: &str,
    auth: &AuthSource,
) -> Result<String, String> {
    let mut args = vec![
        "--dump-pages".to_string(),
        "--skip-download".to_string(),
        "--no-playlist".to_string(),
        "--no-color".to_string(),
        "--socket-timeout".to_string(),
        "20".to_string(),
    ];
    append_auth_args(&mut args, auth);
    args.push(page_url.to_string());

    let output = Command::new(yt_dlp)
        .arg("--ignore-config")
        .args(args)
        .output()
        .map_err(|error| format!("Could not start LinkedIn stream resolver: {error}"))?;

    if let Some(stream_url) = extract_linkedin_stream_url_from_dump(&output.stdout) {
        return Ok(stream_url);
    }

    let stderr = redaction::sanitize_log_line(String::from_utf8_lossy(&output.stderr).trim());
    if stderr.is_empty() {
        Err("LinkedIn page did not expose a DASH or HLS playlist URL.".to_string())
    } else {
        Err(format!("LinkedIn stream resolver failed: {stderr}"))
    }
}

fn resolve_linkedin_feed_stream_url_with_cookie_file_http(
    page_url: &str,
    cookie_path: &Path,
) -> Result<String, String> {
    let cookie_header = linkedin_cookie_header_from_file(cookie_path)?;
    let html = http_get_linkedin_text(page_url, &cookie_header)?;
    choose_linkedin_stream_url(extract_linkedin_stream_urls(&html)).ok_or_else(|| {
        "LinkedIn authenticated page did not expose a DASH or HLS playlist URL.".to_string()
    })
}

fn http_get_linkedin_text(url: &str, cookie_header: &str) -> Result<String, String> {
    let mut response = ureq::get(url)
        .header("User-Agent", BROWSER_USER_AGENT)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Cookie", cookie_header)
        .call()
        .map_err(|error| format!("HTTP request to LinkedIn failed: {error}"))?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("Could not read LinkedIn HTTP response: {error}"))
}

fn linkedin_cookie_header_from_file(path: &Path) -> Result<String, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Could not read temporary Chrome cookie file: {error}"))?;
    let cookies = text
        .lines()
        .filter_map(linkedin_cookie_from_netscape_line)
        .collect::<Vec<_>>();

    if cookies.is_empty() {
        Err("Temporary Chrome cookie file did not contain LinkedIn cookies.".to_string())
    } else {
        Ok(cookies.join("; "))
    }
}

fn linkedin_cookie_from_netscape_line(line: &str) -> Option<String> {
    if line.trim().is_empty() || (line.starts_with('#') && !line.starts_with("#HttpOnly_")) {
        return None;
    }

    let columns = line.split('\t').collect::<Vec<_>>();
    if columns.len() < 7 {
        return None;
    }

    let domain = columns[0].trim_start_matches("#HttpOnly_");
    if !is_linkedin_cookie_domain(domain) {
        return None;
    }

    let name = columns[5].trim();
    let value = columns[6].trim();
    if name.is_empty() {
        None
    } else {
        Some(format!("{name}={value}"))
    }
}

fn is_linkedin_cookie_domain(domain: &str) -> bool {
    let domain = domain.trim_start_matches('.').to_ascii_lowercase();
    domain == "linkedin.com" || domain.ends_with(".linkedin.com")
}

fn extract_linkedin_stream_url_from_dump(stdout: &[u8]) -> Option<String> {
    let output = String::from_utf8_lossy(stdout);
    let mut candidates = Vec::new();

    for line in output.lines().map(str::trim) {
        if !looks_like_base64_dump_line(line) {
            continue;
        }

        let Ok(decoded) = BASE64_STANDARD.decode(line) else {
            continue;
        };
        let page = String::from_utf8_lossy(&decoded);
        candidates.extend(extract_linkedin_stream_urls(&page));
    }

    if candidates.is_empty() {
        candidates.extend(extract_linkedin_stream_urls(&output));
    }

    choose_linkedin_stream_url(candidates)
}

fn looks_like_base64_dump_line(line: &str) -> bool {
    line.len() > 120
        && line
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
}

fn extract_linkedin_stream_urls(text: &str) -> Vec<String> {
    let normalized = normalize_linkedin_metadata_text(text);
    let Ok(regex) =
        Regex::new(r#"https://dms\.licdn\.com/playlist/vid/(?:dash|dynamic)/[^\s"'<>\\]+"#)
    else {
        return Vec::new();
    };

    regex
        .find_iter(&normalized)
        .map(|match_| clean_linkedin_stream_url(match_.as_str()))
        .filter(|url| Url::parse(url).is_ok())
        .collect()
}

fn normalize_linkedin_metadata_text(text: &str) -> String {
    text.replace("\\/", "/")
        .replace("\\u0026", "&")
        .replace("\\u003D", "=")
        .replace("\\u003d", "=")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#x22;", "\"")
        .replace("&#X22;", "\"")
        .replace("&#61;", "=")
        .replace("&#x3D;", "=")
        .replace("&#x3d;", "=")
        .replace("&#X3D;", "=")
        .replace("&amp;", "&")
}

fn clean_linkedin_stream_url(input: &str) -> String {
    input
        .trim_end_matches([',', ';', ')', ']', '}'])
        .to_string()
}

fn choose_linkedin_stream_url(urls: Vec<String>) -> Option<String> {
    let mut unique = Vec::<String>::new();
    for url in urls {
        if !unique.contains(&url) {
            unique.push(url);
        }
    }

    unique
        .iter()
        .find(|url| url.contains("/playlist/vid/dynamic/"))
        .cloned()
        .or_else(|| unique.into_iter().next())
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

pub(crate) fn browser_cookie_arg(source: &BrowserAuthSource) -> String {
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

fn sanitized_process_line_message(line: &ProcessLine) -> String {
    let raw = match line {
        ProcessLine::Stdout(line) | ProcessLine::Stderr(line) => line,
    };
    redaction::sanitize_log_line(raw)
}

fn handle_process_line(
    app: &Runtime,
    state: &backend::AppState,
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

    backend::emit_job(app, &job, Some(log));
    Ok(())
}

pub(crate) fn parse_output_path(line: &str) -> Option<String> {
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
    app: &Runtime,
    state: &backend::AppState,
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
    backend::emit_job(app, &job, None);
    Ok(())
}

fn log(
    app: &Runtime,
    state: &backend::AppState,
    job_id: &str,
    level: &str,
    message: &str,
) -> Result<(), String> {
    let clean = redaction::sanitize_log_line(message);
    let log = state.append_log(job_id, level, &clean)?;
    if let Some(job) = state.get_job(job_id)? {
        backend::emit_job(app, &job, Some(log));
    }
    Ok(())
}

fn failure_hint_from_logs(logs: &[JobLog]) -> Option<String> {
    let messages = logs
        .iter()
        .map(|log| log.message.as_str())
        .collect::<Vec<_>>();

    if let Some(hint) = yt_dlp_failure_hint_from_messages(&messages) {
        return Some(hint);
    }

    let joined = messages.join("\n").to_ascii_lowercase();

    if joined.contains("[reddit]") && joined.contains("no impersonate target is available") {
        return Some(
            "Reddit blocked extraction because yt-dlp has no available browser impersonation target. Install the app-managed yt-dlp from Settings > Tools, then retry.".to_string(),
        );
    }

    if joined.contains("[reddit]") && joined.contains("account authentication is required") {
        return Some(
            "Reddit requires authenticated Reddit cookies. Log in to Reddit in the selected browser or configure a cookies.txt file, then retry.".to_string(),
        );
    }

    if joined.contains("[linkedin]") && joined.contains("unable to extract video") {
        return Some(
            "LinkedIn did not expose a downloadable video to yt-dlp. Check that the selected browser is logged in to LinkedIn and use the LinkedIn Feed Update preset for /feed/update/ URLs.".to_string(),
        );
    }

    None
}

fn yt_dlp_failure_hint_from_messages<T: AsRef<str>>(messages: &[T]) -> Option<String> {
    browser_cookie_failure_hint_from_messages(messages)
}

fn is_generic_yt_dlp_exit_error(error: &str) -> bool {
    error.starts_with("yt-dlp exited with status")
}

fn browser_cookie_failure_summary(errors: &[(String, String)]) -> Option<String> {
    if errors.is_empty()
        || errors
            .iter()
            .any(|(_, error)| !is_browser_cookie_failure_error(error))
    {
        return None;
    }

    let details = errors
        .iter()
        .map(|(label, error)| format!("{label}: {}", compact_browser_cookie_failure(error)))
        .collect::<Vec<_>>()
        .join("; ");

    Some(format!(
        "Could not import cookies from any configured browser. {details}."
    ))
}

fn is_browser_cookie_failure_error(error: &str) -> bool {
    error.starts_with("Could not import browser cookies")
        || error.starts_with("Could not decrypt browser cookies")
}

fn compact_browser_cookie_failure(error: &str) -> &'static str {
    if error.contains("desktop keyring") {
        "desktop keyring could not provide the cookie decryption key"
    } else if error.contains("could not find that browser's cookie database") {
        "cookie database not found"
    } else if error.contains("Could not decrypt browser cookies") {
        "cookies could not be decrypted"
    } else {
        "cookie import failed"
    }
}

fn browser_cookie_failure_hint_from_messages<T: AsRef<str>>(messages: &[T]) -> Option<String> {
    let values = messages
        .iter()
        .map(|message| message.as_ref())
        .collect::<Vec<_>>();
    let joined = values.join("\n").to_ascii_lowercase();

    let saw_cookie_import = joined.contains("extracting cookies from")
        || joined.contains("cookies-from-browser")
        || joined.contains("extract_cookies_from_browser");
    if !saw_cookie_import {
        return None;
    }

    let source = browser_cookie_source_from_messages(&values)
        .unwrap_or_else(|| "the selected browser".to_string());

    if joined.contains("could not find") && joined.contains("cookies database") {
        return Some(format!(
            "Could not import browser cookies from {source}: yt-dlp could not find that browser's cookie database. Select a browser/profile that exists on this machine or configure a cookies.txt file."
        ));
    }

    if joined.contains("item does not exist")
        || joined.contains("itemnotfoundexception")
        || joined.contains("org.freedesktop.secrets")
        || joined.contains("secretstorage")
    {
        return Some(format!(
            "Could not import browser cookies from {source}. The browser cookie database was found, but the desktop keyring could not provide the cookie decryption key. Unlock or repair the OS keyring, select another browser/profile, or configure a cookies.txt file."
        ));
    }

    if joined.contains("could not decrypt")
        || joined.contains("cannot decrypt")
        || joined.contains("keyring")
    {
        return Some(format!(
            "Could not decrypt browser cookies from {source}. Unlock the OS keyring, select another browser/profile, or configure a cookies.txt file."
        ));
    }

    None
}

fn browser_cookie_source_from_messages(messages: &[&str]) -> Option<String> {
    let regex = Regex::new(r"(?i)extracting cookies from\s+([^\s:]+)").ok()?;
    messages.iter().find_map(|message| {
        regex
            .captures(message)
            .and_then(|captures| captures.get(1))
            .map(|match_| match_.as_str().trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn mark_failed(
    app: &Runtime,
    state: &backend::AppState,
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
    backend::emit_job(app, &job, Some(log));
    Ok(())
}

fn mark_canceled(app: &Runtime, state: &backend::AppState, job_id: &str) -> Result<(), String> {
    let log = state.append_log(job_id, "warn", "Canceled by user.")?;
    let job = state.update_job(job_id, |job| {
        job.status = JobStatus::Canceled;
        job.phase = "Canceled".to_string();
        job.speed = None;
        job.eta = None;
    })?;
    backend::emit_job(app, &job, Some(log));
    Ok(())
}

fn fail_job(app: &Runtime, state: &backend::AppState, job_id: &str, error: &str) {
    let clean = redaction::sanitize_log_line(error);
    let _ = state.append_log(job_id, "error", &clean);
    if let Ok(job) = state.update_job(job_id, |job| {
        job.status = JobStatus::Failed;
        job.phase = "Failed".to_string();
        job.error_message = Some(clean.clone());
    }) {
        backend::emit_job(app, &job, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_squarespace_playlist_from_any_page() {
        let html = r#"
            <div
              data-config-video="{&quot;systemDataSourceType&quot;:&quot;mp4&quot;,&quot;alexandriaUrl&quot;:&quot;https://video.squarespace-cdn.com/content/v1/site-id/video-id/{variant}&quot;}"
            ></div>
        "#;

        assert_eq!(
            extract_embedded_page_video_url(html, "https://portfolio.example/work").as_deref(),
            Some("https://video.squarespace-cdn.com/content/v1/site-id/video-id/playlist.m3u8")
        );
    }

    #[test]
    fn rejects_non_squarespace_variant_templates() {
        let html = r#"&quot;alexandriaUrl&quot;:&quot;https://example.com/video/{variant}&quot;"#;

        assert_eq!(
            extract_embedded_page_video_url(html, "https://portfolio.example/work"),
            None
        );
    }

    #[test]
    fn resolves_relative_html_video_sources() {
        let html = r#"<video controls><source src="../media/master.m3u8?token=abc"></video>"#;

        assert_eq!(
            extract_embedded_page_video_url(html, "https://portfolio.example/case-studies/work/")
                .as_deref(),
            Some("https://portfolio.example/case-studies/media/master.m3u8?token=abc")
        );
    }

    #[test]
    fn resolves_embedded_player_iframes_without_provider_rules() {
        let html =
            r#"<iframe loading="lazy" src="https://player.example/video/123?autoplay=0"></iframe>"#;

        assert_eq!(
            extract_embedded_page_video_url(html, "https://portfolio.example/work").as_deref(),
            Some("https://player.example/video/123?autoplay=0")
        );
    }

    #[test]
    fn resolves_open_graph_video_metadata_regardless_of_attribute_order() {
        let html = r#"<meta content="/media/launch-1080.mp4" property="og:video:secure_url">"#;

        assert_eq!(
            extract_embedded_page_video_url(html, "https://portfolio.example/work").as_deref(),
            Some("https://portfolio.example/media/launch-1080.mp4")
        );
    }

    #[test]
    fn resolves_extensionless_structured_stream_descriptors() {
        let html = r#"{"playbackUrl":"https://cdn.example/playlist/dynamic/asset?token=abc"}"#;

        assert_eq!(
            extract_embedded_page_video_url(html, "https://portfolio.example/work").as_deref(),
            Some("https://cdn.example/playlist/dynamic/asset?token=abc")
        );
    }

    #[test]
    fn prefers_higher_resolution_native_sources() {
        let html = r#"
            <video>
              <source src="https://cdn.example/movie-480.mp4">
              <source src="https://cdn.example/movie-1080.mp4">
            </video>
        "#;

        assert_eq!(
            extract_embedded_page_video_url(html, "https://portfolio.example/work").as_deref(),
            Some("https://cdn.example/movie-1080.mp4")
        );
    }

    #[test]
    fn extracts_each_native_video_but_collapses_its_quality_variants() {
        let html = r#"
            <video>
              <source src="https://cdn.example/launch-480.mp4">
              <source src="https://cdn.example/launch-1080.mp4">
            </video>
            <video src="https://cdn.example/interview-720.mp4">
              <source src="https://cdn.example/interview-1440.mp4">
            </video>
        "#;

        assert_eq!(
            extract_embedded_page_video_urls(html, "https://portfolio.example/work"),
            vec![
                "https://cdn.example/launch-1080.mp4",
                "https://cdn.example/interview-1440.mp4",
            ]
        );
    }

    #[test]
    fn extracts_multiple_squarespace_video_descriptors_in_page_order() {
        let html = r#"
            {"alexandriaUrl":"https://video.squarespace-cdn.com/content/v1/site/first/{variant}"}
            {"alexandriaUrl":"https://video.squarespace-cdn.com/content/v1/site/second/{variant}"}
        "#;

        assert_eq!(
            extract_embedded_page_video_urls(html, "https://portfolio.example/work"),
            vec![
                "https://video.squarespace-cdn.com/content/v1/site/first/playlist.m3u8",
                "https://video.squarespace-cdn.com/content/v1/site/second/playlist.m3u8",
            ]
        );
    }

    #[test]
    fn keeps_player_urls_with_different_stable_query_ids_separate() {
        let html = r#"
            <iframe src="https://player.example/embed/video?id=first"></iframe>
            <iframe src="https://player.example/embed/video?id=second"></iframe>
        "#;

        assert_eq!(
            extract_embedded_page_video_urls(html, "https://portfolio.example/work"),
            vec![
                "https://player.example/embed/video?id=first",
                "https://player.example/embed/video?id=second",
            ]
        );
    }

    #[test]
    fn treats_open_graph_video_as_fallback_when_explicit_players_exist() {
        let html = r#"
            <meta property="og:video" content="https://cdn.example/social-preview.mp4">
            <iframe src="https://player.example/embed/primary"></iframe>
            <iframe src="https://player.example/embed/secondary"></iframe>
        "#;

        assert_eq!(
            extract_embedded_page_video_urls(html, "https://portfolio.example/work"),
            vec![
                "https://player.example/embed/primary",
                "https://player.example/embed/secondary",
            ]
        );
    }

    #[test]
    fn collapses_unlabelled_raw_quality_variants_when_they_are_the_only_targets() {
        let html = r#"
            "https://cdn.example/feature-480.mp4"
            "https://cdn.example/feature-1080.mp4"
        "#;

        assert_eq!(
            extract_embedded_page_video_urls(html, "https://portfolio.example/work"),
            vec!["https://cdn.example/feature-1080.mp4"]
        );
    }

    #[test]
    fn indexes_page_video_filenames_without_overwriting_siblings() {
        assert_eq!(
            page_video_filename_template(None, "https://portfolio.example/case-study", 1, 3)
                .as_deref(),
            Some("case-study-video-02.%(ext)s")
        );
        assert_eq!(
            page_video_filename_template(Some("custom.%(ext)s"), "https://example.com", 0, 2)
                .as_deref(),
            Some("custom-video-01.%(ext)s")
        );
    }

    #[test]
    fn extracts_embedded_players_from_base64_page_dumps() {
        let html = format!(
            r#"<iframe src="https://player.example/embed/456" data-page-context="{}"></iframe>"#,
            "x".repeat(120)
        );
        let dump = BASE64_STANDARD.encode(html.as_bytes());

        assert_eq!(
            extract_embedded_page_video_url_from_dump(
                dump.as_bytes(),
                "https://portfolio.example/work"
            )
            .as_deref(),
            Some("https://player.example/embed/456")
        );
    }

    #[test]
    fn combines_distinct_targets_from_multiple_base64_page_dumps() {
        let first = BASE64_STANDARD.encode(
            format!(
                r#"<iframe src="https://player.example/embed/first"></iframe><!-- {} -->"#,
                "x".repeat(120)
            )
            .as_bytes(),
        );
        let second = BASE64_STANDARD.encode(
            format!(
                r#"<iframe src="https://player.example/embed/second"></iframe><!-- {} -->"#,
                "y".repeat(120)
            )
            .as_bytes(),
        );
        let dump = format!("{first}\n{second}");

        assert_eq!(
            extract_embedded_page_video_urls_from_dump(
                dump.as_bytes(),
                "https://portfolio.example/work"
            ),
            vec![
                "https://player.example/embed/first",
                "https://player.example/embed/second",
            ]
        );
    }

    #[test]
    fn extracts_json_escaped_dash_manifest() {
        let html = r#"{"stream":"https:\/\/cdn.example\/video\/master.mpd?token=abc&amp;v=2"}"#;

        assert_eq!(
            extract_embedded_page_video_url(html, "https://portfolio.example/work").as_deref(),
            Some("https://cdn.example/video/master.mpd?token=abc&v=2")
        );
    }

    #[test]
    fn ignores_non_media_html_sources() {
        let html = r#"<video src="javascript:alert(1)"></video><source src="/image.jpg">"#;

        assert_eq!(
            extract_embedded_page_video_url(html, "https://portfolio.example/work"),
            None
        );
    }

    #[test]
    fn derives_page_filename_from_the_last_path_segment() {
        assert_eq!(
            page_media_filename("https://portfolio.example/case-study-old-navy.html?source=test")
                .as_deref(),
            Some("case-study-old-navy")
        );
    }

    #[test]
    fn reports_chrome_keyring_cookie_import_failure() {
        let messages = vec![
            "[LinkedIn] 7166603687367327744: Downloading webpage".to_string(),
            "Extracting cookies from chrome".to_string(),
            "ERROR: Item does not exist!".to_string(),
        ];

        let hint = yt_dlp_failure_hint_from_messages(&messages).unwrap();

        assert!(hint.contains("Could not import browser cookies from chrome"));
        assert!(hint.contains("desktop keyring"));
    }

    #[test]
    fn expands_multiple_browser_cookie_sources() {
        let auth = AuthSource::Browser {
            browser: "chrome".to_string(),
            profile: None,
            browsers: vec![
                BrowserAuthSource {
                    browser: "chrome".to_string(),
                    profile: None,
                },
                BrowserAuthSource {
                    browser: "firefox".to_string(),
                    profile: Some("default-release".to_string()),
                },
            ],
        };

        let attempts = auth_attempts(&auth, None);

        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].label, "chrome cookies");
        assert_eq!(attempts[1].label, "firefox cookies (default-release)");
    }

    #[test]
    fn summarizes_all_browser_cookie_failures() {
        let errors = vec![
            (
                "chrome cookies".to_string(),
                "Could not import browser cookies from chrome. The browser cookie database was found, but the desktop keyring could not provide the cookie decryption key. Unlock or repair the OS keyring, select another browser/profile, or configure a cookies.txt file.".to_string(),
            ),
            (
                "chromium cookies".to_string(),
                "Could not import browser cookies from chromium: yt-dlp could not find that browser's cookie database. Select a browser/profile that exists on this machine or configure a cookies.txt file.".to_string(),
            ),
            (
                "firefox cookies".to_string(),
                "Could not import browser cookies from firefox: yt-dlp could not find that browser's cookie database. Select a browser/profile that exists on this machine or configure a cookies.txt file.".to_string(),
            ),
        ];

        let summary = browser_cookie_failure_summary(&errors).unwrap();

        assert!(summary.contains("chrome cookies: desktop keyring"));
        assert!(summary.contains("chromium cookies: cookie database not found"));
        assert!(summary.contains("firefox cookies: cookie database not found"));
    }

    #[test]
    fn converts_linkedin_ugc_post_share_url_to_feed_update_url() {
        let url = "https://www.linkedin.com/posts/esther-choy-82a45658_ar-xr-mr-ugcPost-7483278057709985792-Em6y/?utm_source=social_share_send&utm_medium=ios_app";

        assert_eq!(
            linkedin_feed_update_url(url).as_deref(),
            Some("https://www.linkedin.com/feed/update/urn:li:ugcPost:7483278057709985792/")
        );
    }

    #[test]
    fn normalizes_linkedin_activity_feed_update_url() {
        let url = "https://linkedin.com/feed/update/urn:li:activity:7477578240379985920/?utm_source=share";

        assert_eq!(
            linkedin_feed_update_url(url).as_deref(),
            Some("https://www.linkedin.com/feed/update/urn:li:activity:7477578240379985920/")
        );
        assert_eq!(
            linkedin_feed_update_url(
                "https://notlinkedin.com/feed/update/urn:li:activity:7477578240379985920/"
            ),
            None
        );
    }

    #[test]
    fn parses_linkedin_netscape_cookie_lines() {
        assert_eq!(
            linkedin_cookie_from_netscape_line(
                "#HttpOnly_.www.linkedin.com\tTRUE\t/\tTRUE\t1784131200\tli_at\tabc"
            )
            .as_deref(),
            Some("li_at=abc")
        );
        assert_eq!(
            linkedin_cookie_from_netscape_line(
                ".example.com\tTRUE\t/\tTRUE\t1784131200\tli_at\tabc"
            ),
            None
        );
    }

    #[test]
    fn extracts_linkedin_dash_url_from_entity_encoded_metadata() {
        let html = r#"
            &quot;protocol&quot;:&quot;DASH&quot;,
            &quot;masterPlaylists&quot;:[{
                &quot;url&quot;:&quot;https://dms.licdn.com/playlist/vid/dash/D4E05AQF0i6gggM4d5A/CeJxzMnGNsgivKAtw9Qp1CdXVcULmR6Lxi9H4yWj8bFS-a6AuAFGZHGg?e&#61;1783972800&amp;v&#61;beta&amp;t&#61;2fgIFcWPyakv6zgHXAtkMjdb05EDKYDWUyUdopnwn8A&quot;,
                &quot;expiresAt&quot;:1783972800000
            }]
        "#;

        let urls = extract_linkedin_stream_urls(html);

        assert_eq!(
            urls,
            vec![
                "https://dms.licdn.com/playlist/vid/dash/D4E05AQF0i6gggM4d5A/CeJxzMnGNsgivKAtw9Qp1CdXVcULmR6Lxi9H4yWj8bFS-a6AuAFGZHGg?e=1783972800&v=beta&t=2fgIFcWPyakv6zgHXAtkMjdb05EDKYDWUyUdopnwn8A"
            ]
        );
    }

    #[test]
    fn prefers_dynamic_playlist_over_dash_for_yt_dlp() {
        let html = r#"
            &quot;url&quot;:&quot;https://dms.licdn.com/playlist/vid/dynamic/D4E05AQF0i6gggM4d5A/token?e&#61;1783972800&amp;v&#61;beta&amp;t&#61;hls&quot;
            &quot;url&quot;:&quot;https://dms.licdn.com/playlist/vid/dash/D4E05AQF0i6gggM4d5A/token?e&#61;1783972800&amp;v&#61;beta&amp;t&#61;dash&quot;
        "#;

        let stream_url = choose_linkedin_stream_url(extract_linkedin_stream_urls(html));

        assert_eq!(
            stream_url.as_deref(),
            Some(
                "https://dms.licdn.com/playlist/vid/dynamic/D4E05AQF0i6gggM4d5A/token?e=1783972800&v=beta&t=hls"
            )
        );
    }

    #[test]
    fn extracts_linkedin_stream_url_from_yt_dlp_base64_dump() {
        let html = r#"&quot;url&quot;:&quot;https://dms.licdn.com/playlist/vid/dash/asset/token?e&#61;1&amp;v&#61;beta&amp;t&#61;abc&quot;"#;
        let dump = BASE64_STANDARD.encode(html);

        let stream_url = extract_linkedin_stream_url_from_dump(dump.as_bytes());

        assert_eq!(
            stream_url.as_deref(),
            Some("https://dms.licdn.com/playlist/vid/dash/asset/token?e=1&v=beta&t=abc")
        );
    }

    #[test]
    fn parses_x_article_tweet_id() {
        assert_eq!(
            x_article_tweet_id("https://x.com/danizeres/article/2064352000054005945"),
            Some("2064352000054005945".to_string())
        );
        assert_eq!(
            x_article_tweet_id("https://x.com/danizeres/article/2064352000054005945-title"),
            Some("2064352000054005945".to_string())
        );
        assert_eq!(
            x_article_tweet_id("https://x.com/i/article/2063747475769319425"),
            None
        );
    }

    #[test]
    fn extracts_all_x_article_videos_and_prefers_best_mp4() {
        let response = serde_json::json!({
            "data": {
                "tweetResult": {
                    "result": {
                        "article": {
                            "article_results": {
                                "result": {
                                    "content_state": {
                                        "blocks": [
                                            {
                                                "data": {
                                                    "mediaItems": [
                                                        { "mediaId": "video-2" },
                                                        { "mediaId": "image-1" },
                                                        { "mediaId": "video-1" }
                                                    ]
                                                }
                                            }
                                        ]
                                    },
                                    "media_entities": [
                                        {
                                            "media_id": "video-1",
                                            "media_info": {
                                                "__typename": "ApiVideo",
                                                "variants": [
                                                    {
                                                        "content_type": "video/mp4",
                                                        "bit_rate": 256000,
                                                        "url": "https://video.twimg.com/amplify_video/video-1/vid/320x240/low.mp4?tag=1"
                                                    },
                                                    {
                                                        "content_type": "video/mp4",
                                                        "bit_rate": 832000,
                                                        "url": "https://video.twimg.com/amplify_video/video-1/vid/640x480/high.mp4?tag=1"
                                                    }
                                                ]
                                            }
                                        },
                                        {
                                            "media_id": "image-1",
                                            "media_info": {
                                                "__typename": "ApiImage"
                                            }
                                        },
                                        {
                                            "media_id": "video-2",
                                            "media_info": {
                                                "__typename": "ApiVideo",
                                                "variants": [
                                                    {
                                                        "content_type": "application/x-mpegURL",
                                                        "url": "https://video.twimg.com/amplify_video/video-2/pl/playlist.m3u8?tag=1"
                                                    },
                                                    {
                                                        "content_type": "video/mp4",
                                                        "bit_rate": 1024000,
                                                        "url": "https://video.twimg.com/amplify_video/video-2/vid/720x720/best.mp4?tag=1"
                                                    }
                                                ]
                                            }
                                        }
                                    ]
                                }
                            }
                        }
                    }
                }
            }
        });

        let videos = extract_x_article_videos(&response);

        assert_eq!(videos.len(), 2);
        assert_eq!(videos[0].media_id, "video-2");
        assert_eq!(
            videos[0].url,
            "https://video.twimg.com/amplify_video/video-2/vid/720x720/best.mp4?tag=1"
        );
        assert_eq!(videos[1].media_id, "video-1");
        assert_eq!(
            videos[1].url,
            "https://video.twimg.com/amplify_video/video-1/vid/640x480/high.mp4?tag=1"
        );
    }
}

fn ready_path_from_line(line: &ProcessLine) -> Option<String> {
    let ProcessLine::Stdout(line) = line else {
        return None;
    };
    serde_json::from_str(line.strip_prefix("__DOWNLOADER_READY__")?).ok()
}
fn publish_ready_file(
    app: &Runtime,
    state: &backend::AppState,
    job_id: &str,
    path: &str,
) -> Result<(), String> {
    if state
        .get_job(job_id)?
        .is_some_and(|job| job.ready_paths.iter().any(|p| p == path))
    {
        return Ok(());
    }
    if !Path::new(path).is_file() {
        return Err(format!(
            "The downloader reported a finished file that is missing: {path}"
        ));
    }
    let log = state.append_log(job_id, "info", &format!("File ready: {path}"))?;
    let job = state.update_job(job_id, |job| {
        if !job.ready_paths.iter().any(|p| p == path) {
            job.ready_paths.push(path.to_string());
        }
        job.output_path = Some(path.to_string());
    })?;
    backend::emit_job(app, &job, Some(log));
    Ok(())
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    #[test]
    fn accepts_only_explicit_final_file_records() {
        let path = "/tmp/a \"quoted\" & video.mp4";
        let record = ProcessLine::Stdout(format!(
            "__DOWNLOADER_READY__{}",
            serde_json::to_string(path).unwrap()
        ));
        assert_eq!(ready_path_from_line(&record).as_deref(), Some(path));
        assert!(ready_path_from_line(&ProcessLine::Stdout(
            "[download] Destination: unfinished.mp4".into()
        ))
        .is_none());
        assert!(ready_path_from_line(&ProcessLine::Stderr(
            "__DOWNLOADER_READY__\"wrong.mp4\"".into()
        ))
        .is_none());
    }

    #[test]
    #[ignore = "Requires yt-dlp and ffmpeg; downloads through the actual local media server"]
    fn real_download_reports_final_file_before_the_process_ends() {
        let root = std::env::temp_dir().join(format!("ready-event-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("source.mp4");
        assert!(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:size=160x90:rate=15",
                "-t",
                "1",
                "-c:v",
                "libx264"
            ])
            .arg(&source)
            .status()
            .unwrap()
            .success());
        let server = crate::media::MediaServer::start().unwrap();
        let url = server.register(source).unwrap();
        let input:StartDownloadRequest=serde_json::from_value(json!({"url":url,"presetId":"generic-page-video-highest","outputDir":root.join("downloads"),"auth":{"kind":"none"}})).unwrap();
        let preset = super::super::presets::find_preset(&input.preset_id).unwrap();
        let args = build_yt_dlp_args(&input, &preset, None);
        let mut command = Command::new("yt-dlp");
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        process_control::isolate_process_group(&mut command);
        let mut child = process_control::ChildGuard(command.spawn().unwrap());
        let reader = BufReader::new(child.stdout.take().unwrap());
        let mut ready = None;
        for line in reader.lines().map_while(Result::ok) {
            if let Some(path) = ready_path_from_line(&ProcessLine::Stdout(line)) {
                assert!(Path::new(&path).is_file());
                ready = Some(path);
                break;
            }
        }
        assert!(child.wait().unwrap().success());
        assert!(ready.is_some());
        fs::remove_dir_all(root).unwrap();
    }
}
