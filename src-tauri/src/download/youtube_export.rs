use super::{sites, JobStatus, StartDownloadRequest, YoutubeCatalogueContent};
use crate::{commands, process_control, redaction, tools};
use chrono::{DateTime, NaiveDate, Utc};
use regex::Regex;
use rust_xlsxwriter::{Format, FormatAlign, Url as XlsxUrl, Workbook};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    env,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, OnceLock,
    },
    thread,
    time::Duration,
};
use tauri::AppHandle;
use url::Url;

const YOUTUBE_API_ROOT: &str = "https://www.googleapis.com/youtube/v3";
const YOUTUBE_API_BATCH_SIZE: usize = 50;
const YOUTUBE_API_WORKERS: usize = 4;
const YOUTUBE_API_RETRIES: usize = 4;
const DISCOVERY_SLEEP_REQUESTS: f64 = 0.25;
const FALLBACK_SLEEP_REQUESTS: f64 = 2.0;

const FIELDS: [&str; 12] = [
    "channel_name",
    "title",
    "video_url",
    "description",
    "published_at",
    "duration",
    "tags",
    "categories",
    "language",
    "view_count",
    "subscriber_count",
    "content_type",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VideoRecord {
    channel_name: Option<String>,
    title: Option<String>,
    video_url: String,
    description: String,
    published_at: Option<String>,
    duration: Option<String>,
    tags: Vec<String>,
    categories: Vec<String>,
    language: Option<String>,
    view_count: Option<i64>,
    subscriber_count: Option<i64>,
    #[serde(default)]
    content_type: YoutubeContentType,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum YoutubeContentType {
    #[default]
    Video,
    Short,
}

impl YoutubeContentType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Short => "short",
        }
    }

    fn video_url(self, video_id: &str) -> String {
        match self {
            Self::Video => format!("https://www.youtube.com/watch?v={video_id}"),
            Self::Short => format!("https://www.youtube.com/shorts/{video_id}"),
        }
    }
}

#[derive(Debug, Serialize)]
struct ExportError {
    url: String,
    error: String,
}

enum ProcessResult {
    Finished {
        success: bool,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    Canceled,
}

#[derive(Clone)]
struct YouTubeApiClient {
    api_key: String,
    agent: ureq::Agent,
}

impl YouTubeApiClient {
    fn new(api_key: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self { api_key, agent }
    }

    fn validate(&self, cancel_flag: &Arc<AtomicBool>) -> Result<(), String> {
        let result = self.get(
            "videos",
            &[
                ("part", "id".to_string()),
                ("id", "dQw4w9WgXcQ".to_string()),
                ("fields", "items(id)".to_string()),
            ],
            cancel_flag,
        )?;
        if result
            .get("items")
            .and_then(Value::as_array)
            .map_or(true, Vec::is_empty)
        {
            return Err("YouTube API validation returned no public video.".to_string());
        }
        Ok(())
    }

    fn videos(
        &self,
        video_ids: &[String],
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<Vec<Value>, String> {
        let result = self.get(
            "videos",
            &[
                ("part", "snippet,contentDetails,statistics".to_string()),
                ("id", video_ids.join(",")),
                (
                    "fields",
                    "items(id,snippet(publishedAt,channelId,channelTitle,title,description,tags,categoryId,defaultLanguage,defaultAudioLanguage),contentDetails(duration),statistics(viewCount))".to_string(),
                ),
            ],
            cancel_flag,
        )?;
        Ok(result
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    fn channel_subscribers(
        &self,
        channel_ids: &[String],
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<HashMap<String, Option<i64>>, String> {
        if channel_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let result = self.get(
            "channels",
            &[
                ("part", "statistics".to_string()),
                ("id", channel_ids.join(",")),
                (
                    "fields",
                    "items(id,statistics(subscriberCount,hiddenSubscriberCount))".to_string(),
                ),
            ],
            cancel_flag,
        )?;
        Ok(result
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| {
                let id = value_string(item, "id")?;
                let count = optional_int(item.get("statistics")?.get("subscriberCount"));
                Some((id, count))
            })
            .collect())
    }

    fn category_names(
        &self,
        category_ids: &[String],
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<HashMap<String, String>, String> {
        if category_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let result = self.get(
            "videoCategories",
            &[
                ("part", "snippet".to_string()),
                ("id", category_ids.join(",")),
                ("fields", "items(id,snippet(title))".to_string()),
            ],
            cancel_flag,
        )?;
        Ok(result
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| {
                Some((
                    value_string(item, "id")?,
                    value_string(item.get("snippet")?, "title")?,
                ))
            })
            .collect())
    }

    fn get(
        &self,
        resource: &str,
        params: &[(&str, String)],
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<Value, String> {
        let mut url = Url::parse(&format!("{YOUTUBE_API_ROOT}/{resource}"))
            .map_err(|_| "Could not construct YouTube API request.".to_string())?;
        {
            let mut query = url.query_pairs_mut();
            for (name, value) in params {
                query.append_pair(name, value);
            }
            query.append_pair("key", &self.api_key);
        }

        for attempt in 0..=YOUTUBE_API_RETRIES {
            if cancel_flag.load(Ordering::SeqCst) {
                return Err("Canceled by user.".to_string());
            }
            let response = self
                .agent
                .get(url.as_str())
                .header("User-Agent", "downloader-youtube-export/1.0")
                .call();
            match response {
                Ok(mut response) => {
                    let status = response.status().as_u16();
                    let body = response.body_mut().read_to_string().map_err(|_| {
                        format!("YouTube API {resource} returned an unreadable response.")
                    })?;
                    let value: Value = serde_json::from_str(&body)
                        .map_err(|_| format!("YouTube API {resource} returned invalid JSON."))?;
                    if status < 400 {
                        return Ok(value);
                    }
                    let (message, reasons) = api_error_details(&value);
                    let retryable = matches!(status, 429 | 500 | 502 | 503 | 504)
                        || reasons.iter().any(|reason| {
                            matches!(
                                reason.as_str(),
                                "rateLimitExceeded" | "userRateLimitExceeded"
                            )
                        });
                    if retryable && attempt < YOUTUBE_API_RETRIES {
                        thread::sleep(Duration::from_secs(1_u64 << attempt));
                        continue;
                    }
                    let safe_message = message.replace(&self.api_key, "[REDACTED]");
                    return Err(format!(
                        "YouTube API {resource} failed: HTTP {status}; reasons={reasons:?}; message={safe_message}"
                    ));
                }
                Err(_) if attempt < YOUTUBE_API_RETRIES => {
                    thread::sleep(Duration::from_secs(1_u64 << attempt));
                }
                Err(_) => {
                    return Err(format!("YouTube API {resource} network failure."));
                }
            }
        }
        Err(format!("YouTube API {resource} failed."))
    }
}

#[derive(Clone)]
struct YouTubeApiPool {
    clients: Vec<YouTubeApiClient>,
    cursor: Arc<AtomicUsize>,
}

impl YouTubeApiPool {
    fn new(api_keys: Vec<String>) -> Self {
        Self {
            clients: api_keys.into_iter().map(YouTubeApiClient::new).collect(),
            cursor: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn len(&self) -> usize {
        self.clients.len()
    }

    fn request<T>(
        &self,
        request: impl Fn(&YouTubeApiClient) -> Result<T, String>,
    ) -> Result<T, String> {
        let start = self.cursor.fetch_add(1, Ordering::Relaxed);
        let mut failures = Vec::new();
        for offset in 0..self.clients.len() {
            let client = &self.clients[(start + offset) % self.clients.len()];
            match request(client) {
                Ok(value) => return Ok(value),
                Err(error) => failures.push(error),
            }
        }
        Err(failures
            .into_iter()
            .next_back()
            .unwrap_or_else(|| "No YouTube API keys are configured.".to_string()))
    }

    fn validate(&self, cancel_flag: &Arc<AtomicBool>) -> Result<(), String> {
        self.request(|client| client.validate(cancel_flag))
    }

    fn videos(
        &self,
        video_ids: &[String],
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<Vec<Value>, String> {
        self.request(|client| client.videos(video_ids, cancel_flag))
    }

    fn channel_subscribers(
        &self,
        channel_ids: &[String],
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<HashMap<String, Option<i64>>, String> {
        self.request(|client| client.channel_subscribers(channel_ids, cancel_flag))
    }

    fn category_names(
        &self,
        category_ids: &[String],
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<HashMap<String, String>, String> {
        self.request(|client| client.category_names(category_ids, cancel_flag))
    }
}

pub fn run(
    app: &AppHandle,
    state: &commands::AppState,
    job_id: &str,
    input: &StartDownloadRequest,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    let yt_dlp = tools::find_tool(app, "yt-dlp").ok_or_else(|| {
        "yt-dlp was not found. Bundle it in src-tauri/binaries or install it locally.".to_string()
    })?;
    let mut api_keys = crate::youtube_api_keys::load_all(&state.youtube_api_key_ids()?)?;
    if let Some(environment_key) = env::var("YOUTUBE_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
    {
        api_keys.push(environment_key);
    }
    let mut seen_keys = HashSet::new();
    api_keys.retain(|key| seen_keys.insert(key.clone()));
    let api_client = (!api_keys.is_empty()).then(|| YouTubeApiPool::new(api_keys));
    let output_root = input
        .output_dir
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let export_name =
        crate::download::normalized_youtube_export_name(input.export_name.as_deref())?;
    let output_dir = output_root.join("youtube_export").join(export_name);
    fs::create_dir_all(&output_dir)
        .map_err(|error| format!("Could not create {}: {error}", output_dir.display()))?;

    let checkpoint_path = output_dir.join(".youtube_export_checkpoint.jsonl");
    let errors_path = output_dir.join("errors.json");
    let json_path = output_dir.join("youtube_videos.json");
    let excel_path = output_dir.join("youtube_videos.xlsx");
    let mut records_by_id = load_checkpoint(&checkpoint_path)?;
    let mut errors = Vec::new();
    let mut video_urls = Vec::new();
    let mut content_types_by_id = HashMap::new();

    update_phase(
        app,
        state,
        job_id,
        JobStatus::Resolving,
        2.0,
        "Discovering channel videos",
    )?;
    log(
        app,
        state,
        job_id,
        "info",
        &format!(
            "Metadata-only YouTube export started for {} channel(s); media download is disabled.",
            input.channel_urls.len()
        ),
    )?;
    if let Some(client) = api_client.as_ref() {
        log(
            app,
            state,
            job_id,
            "info",
            &format!("Validating {} saved YouTube Data API key(s).", client.len()),
        )?;
        if let Err(error) = client.validate(cancel_flag) {
            if cancel_flag.load(Ordering::SeqCst) {
                mark_canceled(app, state, job_id)?;
                return Ok(());
            }
            return Err(error);
        }
        log(
            app,
            state,
            job_id,
            "info",
            "YouTube Data API validation succeeded; metadata will use 50-video batches with four workers and key rotation.",
        )?;
    } else {
        log(
            app,
            state,
            job_id,
            "info",
            "No YouTube API keys are saved; metadata will use the yt-dlp fallback.",
        )?;
    }
    if !records_by_id.is_empty() {
        log(
            app,
            state,
            job_id,
            "info",
            &format!("Resuming with {} checkpointed videos.", records_by_id.len()),
        )?;
    }

    for (index, channel_url) in input.channel_urls.iter().enumerate() {
        for listing_url in channel_listing_urls(channel_url, &input.youtube_catalogue_content)? {
            let args = common_args(DISCOVERY_SLEEP_REQUESTS)
                .into_iter()
                .chain([
                    "--flat-playlist".to_string(),
                    "--dump-single-json".to_string(),
                    listing_url.clone(),
                ])
                .collect::<Vec<_>>();
            let result = run_process(yt_dlp.as_path(), &args, state, job_id, cancel_flag)?;
            let ProcessResult::Finished {
                success,
                stdout,
                stderr,
            } = result
            else {
                mark_canceled(app, state, job_id)?;
                return Ok(());
            };
            if !success {
                let error = process_error(&stderr);
                if is_missing_shorts_tab(&listing_url, &error) {
                    log(
                        app,
                        state,
                        job_id,
                        "info",
                        &format!(
                            "[{}/{}] Shorts tab is not present; skipping.",
                            index + 1,
                            input.channel_urls.len()
                        ),
                    )?;
                    continue;
                }
                errors.push(ExportError {
                    url: listing_url.clone(),
                    error: error.clone(),
                });
                log(
                    app,
                    state,
                    job_id,
                    "error",
                    &format!("Could not read {listing_url}: {error}"),
                )?;
                continue;
            }

            match serde_json::from_slice::<Value>(&stdout) {
                Ok(info) => {
                    let discovered = listing_video_urls(&info);
                    let content_type = listing_content_type(&listing_url);
                    for video_url in &discovered {
                        if let Some(video_id) = video_id_from_url(video_url) {
                            content_types_by_id
                                .entry(video_id)
                                .and_modify(|existing| {
                                    if content_type == YoutubeContentType::Short {
                                        *existing = YoutubeContentType::Short;
                                    }
                                })
                                .or_insert(content_type);
                        }
                    }
                    log(
                        app,
                        state,
                        job_id,
                        "info",
                        &format!(
                            "[{}/{}] {} discovery completed: {} videos.",
                            index + 1,
                            input.channel_urls.len(),
                            listing_name(&listing_url),
                            discovered.len()
                        ),
                    )?;
                    video_urls.extend(discovered);
                }
                Err(error) => errors.push(ExportError {
                    url: listing_url,
                    error: format!("Invalid yt-dlp channel metadata: {error}"),
                }),
            }
        }
        let progress = 5.0 + 25.0 * (index + 1) as f64 / input.channel_urls.len() as f64;
        update_phase(
            app,
            state,
            job_id,
            JobStatus::Resolving,
            progress,
            "Discovering channel videos",
        )?;
    }

    let mut seen_ids = HashSet::new();
    video_urls.retain(|url| {
        video_id_from_url(url)
            .map(|video_id| seen_ids.insert(video_id))
            .unwrap_or_else(|| seen_ids.insert(url.clone()))
    });
    for (video_id, content_type) in &content_types_by_id {
        let Some(record) = records_by_id.get_mut(video_id) else {
            continue;
        };
        let video_url = content_type.video_url(video_id);
        if record.content_type != *content_type || record.video_url != video_url {
            record.content_type = *content_type;
            record.video_url = video_url;
            append_checkpoint(&checkpoint_path, record)?;
        }
    }
    log(
        app,
        state,
        job_id,
        "info",
        &format!(
            "Fetching full metadata for {} unique videos.",
            video_urls.len()
        ),
    )?;

    if let Some(client) = api_client.as_ref() {
        if let Err(error) = fetch_api_metadata(
            app,
            state,
            job_id,
            client,
            &video_urls,
            &content_types_by_id,
            &mut records_by_id,
            &checkpoint_path,
            &mut errors,
            cancel_flag,
        ) {
            if cancel_flag.load(Ordering::SeqCst) {
                mark_canceled(app, state, job_id)?;
                return Ok(());
            }
            return Err(error);
        }
    } else {
        let total = video_urls.len().max(1);
        for (index, video_url) in video_urls.iter().enumerate() {
            if cancel_flag.load(Ordering::SeqCst) {
                mark_canceled(app, state, job_id)?;
                return Ok(());
            }
            let record_key = video_id_from_url(video_url).unwrap_or_else(|| video_url.clone());
            if !records_by_id.contains_key(&record_key) {
                let content_type = content_types_by_id
                    .get(&record_key)
                    .copied()
                    .unwrap_or_default();
                let args = common_args(FALLBACK_SLEEP_REQUESTS)
                    .into_iter()
                    .chain([
                        "--no-playlist".to_string(),
                        "--dump-single-json".to_string(),
                        video_url.clone(),
                    ])
                    .collect::<Vec<_>>();
                match run_process(yt_dlp.as_path(), &args, state, job_id, cancel_flag)? {
                    ProcessResult::Canceled => {
                        mark_canceled(app, state, job_id)?;
                        return Ok(());
                    }
                    ProcessResult::Finished {
                        success,
                        stdout,
                        stderr: _,
                    } if success => match serde_json::from_slice::<Value>(&stdout)
                        .map_err(|error| error.to_string())
                        .and_then(|info| video_record(&info, content_type))
                    {
                        Ok(record) => {
                            append_checkpoint(&checkpoint_path, &record)?;
                            records_by_id.insert(record_key.clone(), record);
                        }
                        Err(error) => errors.push(ExportError {
                            url: video_url.clone(),
                            error: format!("Invalid video metadata: {error}"),
                        }),
                    },
                    ProcessResult::Finished { stderr, .. } => errors.push(ExportError {
                        url: video_url.clone(),
                        error: process_error(&stderr),
                    }),
                }
            }

            let progress = 30.0 + 60.0 * (index + 1) as f64 / total as f64;
            update_phase(
                app,
                state,
                job_id,
                JobStatus::Downloading,
                progress,
                &format!("Reading video metadata {}/{}", index + 1, video_urls.len()),
            )?;
        }
    }

    if cancel_flag.load(Ordering::SeqCst) {
        mark_canceled(app, state, job_id)?;
        return Ok(());
    }

    update_phase(
        app,
        state,
        job_id,
        JobStatus::Postprocessing,
        94.0,
        "Writing JSON and Excel",
    )?;
    let mut records = records_by_id.into_values().collect::<Vec<_>>();
    records.sort_by(|left, right| right.published_at.cmp(&left.published_at));
    write_json(&json_path, &records)?;
    write_excel(&excel_path, &records)?;
    log(
        app,
        state,
        job_id,
        "info",
        &format!("[download] Destination: {}", json_path.display()),
    )?;
    log(
        app,
        state,
        job_id,
        "info",
        &format!("[download] Destination: {}", excel_path.display()),
    )?;

    if errors.is_empty() {
        remove_if_exists(&errors_path)?;
        remove_if_exists(&checkpoint_path)?;
    } else {
        write_json(&errors_path, &errors)?;
    }

    let job = state.update_job(job_id, |job| {
        job.output_path = Some(json_path.display().to_string());
        if errors.is_empty() {
            job.status = JobStatus::Completed;
            job.progress = 100.0;
            job.phase = format!("Exported {} videos", records.len());
            job.error_message = None;
        }
    })?;
    commands::emit_job(app, &job, None);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Exported {} videos with {} errors. Details: {}",
            records.len(),
            errors.len(),
            errors_path.display()
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn fetch_api_metadata(
    app: &AppHandle,
    state: &commands::AppState,
    job_id: &str,
    client: &YouTubeApiPool,
    video_urls: &[String],
    content_types_by_id: &HashMap<String, YoutubeContentType>,
    records_by_id: &mut HashMap<String, VideoRecord>,
    checkpoint_path: &Path,
    errors: &mut Vec<ExportError>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    let pending_urls = video_urls
        .iter()
        .filter(|url| {
            video_id_from_url(url)
                .map(|video_id| !records_by_id.contains_key(&video_id))
                .unwrap_or(true)
        })
        .cloned()
        .collect::<Vec<_>>();
    let pending = pending_urls
        .iter()
        .filter_map(|url| video_id_from_url(url).map(|id| (id, url.clone())))
        .collect::<Vec<_>>();
    for url in pending_urls
        .iter()
        .filter(|url| video_id_from_url(url).is_none())
    {
        errors.push(ExportError {
            url: url.clone(),
            error: "Cannot extract video ID from URL".to_string(),
        });
    }
    let url_by_id = pending.iter().cloned().collect::<HashMap<_, _>>();
    let video_ids = pending.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    let batches = video_ids
        .chunks(YOUTUBE_API_BATCH_SIZE)
        .map(<[String]>::to_vec)
        .collect::<Vec<_>>();
    if batches.is_empty() {
        log(
            app,
            state,
            job_id,
            "info",
            "All discovered videos are already present in the checkpoint.",
        )?;
        return Ok(());
    }

    log(
        app,
        state,
        job_id,
        "info",
        &format!(
            "Official API phase: pending={}, batches={}, batch_size={}, workers={}.",
            video_ids.len(),
            batches.len(),
            YOUTUBE_API_BATCH_SIZE,
            YOUTUBE_API_WORKERS
        ),
    )?;
    let mut items_by_id = HashMap::<String, Value>::new();
    let mut completed_batches = 0;
    for batch_group in batches.chunks(YOUTUBE_API_WORKERS) {
        if cancel_flag.load(Ordering::SeqCst) {
            return Ok(());
        }
        let results = thread::scope(|scope| {
            batch_group
                .iter()
                .map(|batch| {
                    let client = client.clone();
                    let cancel_flag = cancel_flag.clone();
                    scope.spawn(move || {
                        let result = client.videos(batch, &cancel_flag);
                        (batch.clone(), result)
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .map_err(|_| "YouTube API worker stopped unexpectedly.".to_string())
                })
                .collect::<Result<Vec<_>, _>>()
        })?;

        for (batch, result) in results {
            match result {
                Ok(items) => {
                    let returned_ids = items
                        .iter()
                        .filter_map(|item| value_string(item, "id"))
                        .collect::<HashSet<_>>();
                    for item in items {
                        if let Some(id) = value_string(&item, "id") {
                            items_by_id.insert(id, item);
                        }
                    }
                    for missing_id in batch
                        .iter()
                        .filter(|id| !returned_ids.contains(id.as_str()))
                    {
                        if let Some(url) = url_by_id.get(missing_id) {
                            errors.push(ExportError {
                                url: url.clone(),
                                error: "Video was not returned by the YouTube API".to_string(),
                            });
                        }
                    }
                }
                Err(error) => {
                    if cancel_flag.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                    for id in batch {
                        if let Some(url) = url_by_id.get(&id) {
                            errors.push(ExportError {
                                url: url.clone(),
                                error: error.clone(),
                            });
                        }
                    }
                }
            }
            completed_batches += 1;
        }
        let progress = 30.0 + 45.0 * completed_batches as f64 / batches.len() as f64;
        update_phase(
            app,
            state,
            job_id,
            JobStatus::Downloading,
            progress,
            &format!(
                "Reading API metadata batches {}/{}",
                completed_batches,
                batches.len()
            ),
        )?;
    }

    let channel_ids = sorted_unique_strings(
        items_by_id
            .values()
            .filter_map(|item| value_string(item.get("snippet")?, "channelId")),
    );
    let mut subscriber_counts = HashMap::new();
    for batch in channel_ids.chunks(YOUTUBE_API_BATCH_SIZE) {
        subscriber_counts.extend(client.channel_subscribers(batch, cancel_flag)?);
    }

    let subscriber_counts_by_channel_name = items_by_id
        .values()
        .filter_map(|item| {
            let snippet = item.get("snippet")?;
            let name = value_string(snippet, "channelTitle")?;
            let channel_id = value_string(snippet, "channelId")?;
            Some((name, subscriber_counts.get(&channel_id).copied().flatten()))
        })
        .collect::<HashMap<_, _>>();
    for record in records_by_id.values_mut() {
        let Some(channel_name) = record.channel_name.as_ref() else {
            continue;
        };
        let Some(current_count) = subscriber_counts_by_channel_name.get(channel_name) else {
            continue;
        };
        if record.subscriber_count != *current_count {
            record.subscriber_count = *current_count;
            append_checkpoint(checkpoint_path, record)?;
        }
    }

    let category_ids = sorted_unique_strings(
        items_by_id
            .values()
            .filter_map(|item| value_string(item.get("snippet")?, "categoryId")),
    );
    let mut category_names = HashMap::new();
    for batch in category_ids.chunks(YOUTUBE_API_BATCH_SIZE) {
        category_names.extend(client.category_names(batch, cancel_flag)?);
    }

    for video_id in video_ids {
        let Some(item) = items_by_id.get(&video_id) else {
            continue;
        };
        let content_type = content_types_by_id
            .get(&video_id)
            .copied()
            .unwrap_or_default();
        let record = api_video_record(item, &subscriber_counts, &category_names, content_type)?;
        append_checkpoint(checkpoint_path, &record)?;
        records_by_id.insert(video_id, record);
    }
    Ok(())
}

fn api_video_record(
    item: &Value,
    subscriber_counts: &HashMap<String, Option<i64>>,
    category_names: &HashMap<String, String>,
    content_type: YoutubeContentType,
) -> Result<VideoRecord, String> {
    let video_id = value_string(item, "id").ok_or_else(|| "Video ID missing".to_string())?;
    let snippet = item.get("snippet").unwrap_or(&Value::Null);
    let statistics = item.get("statistics").unwrap_or(&Value::Null);
    let content_details = item.get("contentDetails").unwrap_or(&Value::Null);
    let channel_id = value_string(snippet, "channelId").unwrap_or_default();
    let category = value_string(snippet, "categoryId")
        .and_then(|id| category_names.get(&id).cloned())
        .into_iter()
        .collect();
    Ok(VideoRecord {
        channel_name: value_string(snippet, "channelTitle"),
        title: value_string(snippet, "title"),
        video_url: content_type.video_url(&video_id),
        description: value_string(snippet, "description").unwrap_or_default(),
        published_at: value_string(snippet, "publishedAt"),
        duration: value_string(content_details, "duration")
            .and_then(|duration| api_duration(&duration)),
        tags: string_list(snippet.get("tags")),
        categories: category,
        language: value_string(snippet, "defaultAudioLanguage")
            .or_else(|| value_string(snippet, "defaultLanguage")),
        view_count: optional_int(statistics.get("viewCount")),
        subscriber_count: subscriber_counts.get(&channel_id).copied().flatten(),
        content_type,
    })
}

fn api_duration(value: &str) -> Option<String> {
    static DURATION_PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = DURATION_PATTERN.get_or_init(|| {
        Regex::new(
            r"^P(?:(?P<days>\d+)D)?T(?:(?P<hours>\d+)H)?(?:(?P<minutes>\d+)M)?(?:(?P<seconds>\d+(?:\.\d+)?)S)?$",
        )
        .expect("YouTube duration pattern must be valid")
    });
    let captures = pattern.captures(value)?;
    let part = |name| {
        captures
            .name(name)
            .and_then(|value| value.as_str().parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let seconds = part("days") * 86_400.0
        + part("hours") * 3_600.0
        + part("minutes") * 60.0
        + part("seconds");
    duration_value(Some(&Value::from(seconds)))
}

fn video_id_from_url(video_url: &str) -> Option<String> {
    let url = Url::parse(video_url).ok()?;
    if let Some(video_id) = url
        .query_pairs()
        .find_map(|(name, value)| (name == "v").then(|| value.into_owned()))
    {
        return Some(video_id);
    }
    let segments = url.path_segments()?.collect::<Vec<_>>();
    match segments.as_slice() {
        ["shorts", video_id, ..] if !video_id.is_empty() => Some((*video_id).to_string()),
        _ => None,
    }
}

fn sorted_unique_strings(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut values = values
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    values.sort();
    values
}

fn api_error_details(value: &Value) -> (String, Vec<String>) {
    let error = value.get("error").unwrap_or(&Value::Null);
    let message = value_string(error, "message").unwrap_or_else(|| "Unknown API error".to_string());
    let reasons = error
        .get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| value_string(item, "reason"))
        .collect();
    (message, reasons)
}

fn common_args(sleep_requests: f64) -> Vec<String> {
    let mut args = [
        "--quiet",
        "--no-warnings",
        "--skip-download",
        "--simulate",
        "--socket-timeout",
        "30",
        "--retries",
        "5",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    args.extend(["--sleep-requests".to_string(), sleep_requests.to_string()]);
    args
}

fn run_process(
    executable: &Path,
    args: &[String],
    state: &commands::AppState,
    job_id: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<ProcessResult, String> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process_control::isolate_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start yt-dlp: {error}"))?;
    state.set_process(job_id, child.id())?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not capture yt-dlp output.".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Could not capture yt-dlp errors.".to_string())?;
    let stdout_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let status = loop {
        if cancel_flag.load(Ordering::SeqCst) {
            process_control::force_kill_process_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
            state.clear_process(job_id);
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            return Ok(ProcessResult::Canceled);
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        thread::sleep(Duration::from_millis(180));
    };
    state.clear_process(job_id);
    let stdout = stdout_thread
        .join()
        .map_err(|_| "Could not read yt-dlp output.".to_string())?;
    let stderr = stderr_thread
        .join()
        .map_err(|_| "Could not read yt-dlp errors.".to_string())?;
    Ok(ProcessResult::Finished {
        success: status.success(),
        stdout,
        stderr,
    })
}

fn listing_video_urls(info: &Value) -> Vec<String> {
    info.get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(canonical_video_url)
        .collect()
}

fn channel_listing_urls(
    channel_url: &str,
    content: &YoutubeCatalogueContent,
) -> Result<Vec<String>, String> {
    match content {
        YoutubeCatalogueContent::All => Ok(vec![
            sites::youtube_channel_videos_url(channel_url)?,
            sites::youtube_channel_shorts_url(channel_url)?,
        ]),
        YoutubeCatalogueContent::Videos => {
            Ok(vec![sites::youtube_channel_videos_url(channel_url)?])
        }
        YoutubeCatalogueContent::Shorts => {
            Ok(vec![sites::youtube_channel_shorts_url(channel_url)?])
        }
    }
}

fn listing_name(listing_url: &str) -> &'static str {
    match listing_content_type(listing_url) {
        YoutubeContentType::Video => "Videos",
        YoutubeContentType::Short => "Shorts",
    }
}

fn listing_content_type(listing_url: &str) -> YoutubeContentType {
    if listing_url.ends_with("/shorts") {
        YoutubeContentType::Short
    } else {
        YoutubeContentType::Video
    }
}

fn is_missing_shorts_tab(listing_url: &str, error: &str) -> bool {
    listing_url.ends_with("/shorts")
        && error
            .to_ascii_lowercase()
            .contains("does not have a shorts tab")
}

fn canonical_video_url(info: &Value) -> Option<String> {
    value_string(info, "id")
        .map(|id| format!("https://www.youtube.com/watch?v={id}"))
        .or_else(|| value_string(info, "webpage_url"))
        .or_else(|| value_string(info, "url"))
}

fn video_record(info: &Value, content_type: YoutubeContentType) -> Result<VideoRecord, String> {
    let video_url = value_string(info, "id")
        .map(|video_id| content_type.video_url(&video_id))
        .or_else(|| canonical_video_url(info))
        .ok_or_else(|| "Video URL missing".to_string())?;
    Ok(VideoRecord {
        channel_name: value_string(info, "channel").or_else(|| value_string(info, "uploader")),
        title: value_string(info, "title"),
        video_url,
        description: value_string(info, "description").unwrap_or_default(),
        published_at: publication_value(info),
        duration: duration_value(info.get("duration")),
        tags: string_list(info.get("tags")),
        categories: string_list(info.get("categories")),
        language: value_string(info, "language"),
        view_count: optional_int(info.get("view_count")),
        subscriber_count: optional_int(info.get("channel_follower_count")),
        content_type,
    })
}

fn value_string(info: &Value, field: &str) -> Option<String> {
    info.get(field).and_then(Value::as_str).map(str::to_string)
}

fn publication_value(info: &Value) -> Option<String> {
    for field in ["release_timestamp", "timestamp"] {
        if let Some(timestamp) = number_value(info.get(field))
            .and_then(|value| DateTime::<Utc>::from_timestamp(value as i64, 0))
        {
            return Some(timestamp.format("%Y-%m-%dT%H:%M:%SZ").to_string());
        }
    }
    for field in ["release_date", "upload_date"] {
        if let Some(date) = info
            .get(field)
            .and_then(Value::as_str)
            .and_then(|value| NaiveDate::parse_from_str(value, "%Y%m%d").ok())
        {
            return Some(date.format("%Y-%m-%d").to_string());
        }
    }
    None
}

fn duration_value(value: Option<&Value>) -> Option<String> {
    let seconds = number_value(value)?.max(0.0) as u64;
    Some(format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    ))
}

fn number_value(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn optional_int(value: Option<&Value>) -> Option<i64> {
    number_value(value).map(|value| value as i64)
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| match item {
            Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
            Value::Null => None,
            other => {
                let text = other.to_string();
                (!text.trim().is_empty()).then_some(text)
            }
        })
        .collect()
}

fn load_checkpoint(path: &Path) -> Result<HashMap<String, VideoRecord>, String> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut records = HashMap::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if let Ok(record) = serde_json::from_str::<VideoRecord>(&line) {
            let record_key =
                video_id_from_url(&record.video_url).unwrap_or_else(|| record.video_url.clone());
            records.insert(record_key, record);
        }
    }
    Ok(records)
}

fn append_checkpoint(path: &Path, record: &VideoRecord) -> Result<(), String> {
    ensure_parent_directory(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut file, record).map_err(|error| error.to_string())?;
    file.write_all(b"\n").map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    ensure_parent_directory(path)?;
    let mut file = File::create(path).map_err(|error| error.to_string())?;
    serde_json::to_writer_pretty(&mut file, value).map_err(|error| error.to_string())?;
    file.write_all(b"\n").map_err(|error| error.to_string())
}

fn write_excel(path: &Path, records: &[VideoRecord]) -> Result<(), String> {
    let mut workbook = Workbook::new();
    let header = Format::new().set_bold();
    let wrapped = Format::new().set_text_wrap().set_align(FormatAlign::Top);
    let worksheet = workbook.add_worksheet();
    worksheet
        .set_name("Videos")
        .map_err(|error| error.to_string())?;
    for (column, field) in FIELDS.iter().enumerate() {
        worksheet
            .write_string_with_format(0, column as u16, *field, &header)
            .map_err(|error| error.to_string())?;
    }

    for (index, record) in records.iter().enumerate() {
        let row = index as u32 + 1;
        write_optional_string(worksheet, row, 0, record.channel_name.as_deref(), None)?;
        write_optional_string(worksheet, row, 1, record.title.as_deref(), None)?;
        worksheet
            .write(row, 2, XlsxUrl::new(&record.video_url))
            .map_err(|error| error.to_string())?;
        write_optional_string(worksheet, row, 3, Some(&record.description), Some(&wrapped))?;
        write_optional_string(worksheet, row, 4, record.published_at.as_deref(), None)?;
        write_optional_string(worksheet, row, 5, record.duration.as_deref(), None)?;
        write_optional_string(
            worksheet,
            row,
            6,
            Some(&record.tags.join(", ")),
            Some(&wrapped),
        )?;
        write_optional_string(worksheet, row, 7, Some(&record.categories.join(", ")), None)?;
        write_optional_string(worksheet, row, 8, record.language.as_deref(), None)?;
        if let Some(value) = record.view_count {
            worksheet
                .write_number(row, 9, value as f64)
                .map_err(|error| error.to_string())?;
        }
        if let Some(value) = record.subscriber_count {
            worksheet
                .write_number(row, 10, value as f64)
                .map_err(|error| error.to_string())?;
        }
        worksheet
            .write_string(row, 11, record.content_type.as_str())
            .map_err(|error| error.to_string())?;
    }

    for (column, width) in [28, 55, 48, 90, 24, 14, 55, 30, 14, 16, 18, 14]
        .into_iter()
        .enumerate()
    {
        worksheet
            .set_column_width(column as u16, width)
            .map_err(|error| error.to_string())?;
    }
    worksheet
        .set_freeze_panes(1, 0)
        .map_err(|error| error.to_string())?;
    worksheet
        .autofilter(0, 0, records.len() as u32, (FIELDS.len() - 1) as u16)
        .map_err(|error| error.to_string())?;
    ensure_parent_directory(path)?;
    workbook.save(path).map_err(|error| error.to_string())
}

fn ensure_parent_directory(path: &Path) -> Result<(), String> {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(());
    };
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create {}: {error}", parent.display()))
}

fn write_optional_string(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    row: u32,
    column: u16,
    value: Option<&str>,
    format: Option<&Format>,
) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    let safe = excel_safe(value);
    if let Some(format) = format {
        worksheet
            .write_string_with_format(row, column, &safe, format)
            .map_err(|error| error.to_string())?;
    } else {
        worksheet
            .write_string(row, column, &safe)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn excel_safe(value: &str) -> String {
    if value.starts_with(['=', '+', '-', '@']) {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

fn process_error(stderr: &[u8]) -> String {
    let error = String::from_utf8_lossy(stderr);
    let clean = redaction::sanitize_log_line(error.trim());
    if clean.is_empty() {
        "yt-dlp failed without an error message".to_string()
    } else {
        clean
    }
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
    let log = state.append_log(job_id, level, message)?;
    if let Some(job) = state.get_job(job_id)? {
        commands::emit_job(app, &job, Some(log));
    }
    Ok(())
}

fn mark_canceled(app: &AppHandle, state: &commands::AppState, job_id: &str) -> Result<(), String> {
    let job = state.update_job(job_id, |job| {
        job.status = JobStatus::Canceled;
        job.phase = "Canceled".to_string();
        job.speed = None;
        job.eta = None;
        job.error_message = None;
    })?;
    commands::emit_job(app, &job, None);
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn maps_metadata_to_the_export_contract() {
        let info = serde_json::json!({
            "id": "abc123",
            "channel": "Example",
            "title": "Title",
            "description": "Description",
            "timestamp": 1_700_000_000,
            "duration": 3661.9,
            "tags": ["one", "two"],
            "categories": ["Education"],
            "language": "en",
            "view_count": 42,
            "channel_follower_count": 1000
        });
        let record = video_record(&info, YoutubeContentType::Short).unwrap();

        assert_eq!(record.video_url, "https://www.youtube.com/shorts/abc123");
        assert_eq!(record.duration.as_deref(), Some("01:01:01"));
        assert_eq!(record.published_at.as_deref(), Some("2023-11-14T22:13:20Z"));
        assert_eq!(record.tags, ["one", "two"]);
        assert_eq!(record.view_count, Some(42));
        assert_eq!(record.subscriber_count, Some(1000));
        assert_eq!(record.content_type, YoutubeContentType::Short);
    }

    #[test]
    fn selects_the_requested_channel_tabs() {
        let channel_url = "https://www.youtube.com/@example/videos";

        assert_eq!(
            channel_listing_urls(channel_url, &YoutubeCatalogueContent::All).unwrap(),
            [
                "https://www.youtube.com/@example/videos",
                "https://www.youtube.com/@example/shorts",
            ]
        );
        assert_eq!(
            channel_listing_urls(channel_url, &YoutubeCatalogueContent::Videos).unwrap(),
            ["https://www.youtube.com/@example/videos"]
        );
        assert_eq!(
            channel_listing_urls(channel_url, &YoutubeCatalogueContent::Shorts).unwrap(),
            ["https://www.youtube.com/@example/shorts"]
        );
        assert_eq!(
            listing_content_type("https://www.youtube.com/@example/videos"),
            YoutubeContentType::Video
        );
        assert_eq!(
            listing_content_type("https://www.youtube.com/@example/shorts"),
            YoutubeContentType::Short
        );
        assert_eq!(
            video_id_from_url("https://www.youtube.com/shorts/abc123").as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn recognizes_a_missing_shorts_tab_as_an_empty_listing() {
        assert!(is_missing_shorts_tab(
            "https://www.youtube.com/@example/shorts",
            "ERROR: [youtube:tab] @example: This channel does not have a shorts tab"
        ));
        assert!(!is_missing_shorts_tab(
            "https://www.youtube.com/@example/videos",
            "ERROR: [youtube:tab] @example: This channel does not have a shorts tab"
        ));
    }

    #[test]
    fn maps_official_api_metadata_to_the_same_contract() {
        let item = serde_json::json!({
            "id": "abc123",
            "snippet": {
                "channelId": "channel-1",
                "channelTitle": "Example",
                "title": "Title",
                "description": "Description",
                "publishedAt": "2026-07-17T12:00:00Z",
                "tags": ["one", "two"],
                "categoryId": "27",
                "defaultAudioLanguage": "en-US"
            },
            "contentDetails": { "duration": "P1DT2H3M4S" },
            "statistics": { "viewCount": "42" }
        });
        let subscribers = HashMap::from([("channel-1".to_string(), Some(1_000))]);
        let categories = HashMap::from([("27".to_string(), "Education".to_string())]);

        let record =
            api_video_record(&item, &subscribers, &categories, YoutubeContentType::Video).unwrap();

        assert_eq!(record.video_url, "https://www.youtube.com/watch?v=abc123");
        assert_eq!(record.duration.as_deref(), Some("26:03:04"));
        assert_eq!(record.tags, ["one", "two"]);
        assert_eq!(record.categories, ["Education"]);
        assert_eq!(record.language.as_deref(), Some("en-US"));
        assert_eq!(record.view_count, Some(42));
        assert_eq!(record.subscriber_count, Some(1_000));
        assert_eq!(record.content_type, YoutubeContentType::Video);
    }

    #[test]
    fn rejects_invalid_official_api_durations() {
        assert_eq!(api_duration("PT1H2M3.9S").as_deref(), Some("01:02:03"));
        assert_eq!(api_duration("not-a-duration"), None);
    }

    #[test]
    fn protects_excel_cells_without_changing_json_values() {
        assert_eq!(excel_safe("=HYPERLINK(\"bad\")"), "'=HYPERLINK(\"bad\")");
        assert_eq!(excel_safe("Normal title"), "Normal title");
    }

    #[test]
    fn writes_the_json_and_excel_deliverables() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("downloader-youtube-export-{suffix}"));
        let record = VideoRecord {
            channel_name: Some("Example".to_string()),
            title: Some("=Unsafe title".to_string()),
            video_url: "https://www.youtube.com/shorts/abc123".to_string(),
            description: "Description".to_string(),
            published_at: Some("2026-07-17".to_string()),
            duration: Some("00:01:00".to_string()),
            tags: vec!["tag".to_string()],
            categories: vec!["Education".to_string()],
            language: Some("en".to_string()),
            view_count: Some(42),
            subscriber_count: Some(1000),
            content_type: YoutubeContentType::Short,
        };
        let json_path = directory.join("json").join("youtube_videos.json");
        let excel_path = directory.join("excel").join("youtube_videos.xlsx");

        write_json(&json_path, &vec![record.clone()]).unwrap();
        write_excel(&excel_path, &[record]).unwrap();

        let json = fs::read_to_string(&json_path).unwrap();
        assert!(json.find("\"channel_name\"").unwrap() < json.find("\"title\"").unwrap());
        assert!(json.contains("\"title\": \"=Unsafe title\""));
        assert!(json.contains("\"content_type\": \"short\""));
        assert_eq!(&fs::read(&excel_path).unwrap()[..2], b"PK");
        fs::remove_dir_all(directory).unwrap();
    }
}
