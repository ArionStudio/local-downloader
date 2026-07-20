use crate::{
    download::{
        engine, presets, sites, AnalyzeResult, AuthRequirement, AuthSource, FormatAnalysis, Job,
        JobDetail, JobLog, JobStatus, StartDownloadRequest,
    },
    process_control,
    storage::Storage,
    tools, youtube_api_keys,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::UpdaterExt;
use uuid::Uuid;

type CommandResult<T> = Result<T, String>;

#[derive(Clone)]
pub struct AppState {
    storage: Arc<Storage>,
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    cancels: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    processes: Arc<Mutex<HashMap<String, u32>>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeInput {
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelJobInput {
    job_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetJobInput {
    job_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallToolUpdateInput {
    tool: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathInput {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddYoutubeApiKeyInput {
    api_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveYoutubeApiKeyInput {
    id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YoutubeApiKeyInfo {
    id: String,
    label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeFormatsInput {
    url: String,
    auth: Option<AuthSource>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadJobEvent {
    job: Job,
    log: Option<JobLog>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdate {
    version: String,
    notes: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    name: String,
    version: String,
    updater_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    default_output_dir: Option<String>,
    auth: AuthSource,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_output_dir: None,
            auth: AuthSource::Browser {
                browser: "firefox".to_string(),
                profile: None,
                browsers: vec![],
            },
        }
    }
}

impl AppState {
    pub fn new(app: &AppHandle) -> CommandResult<Self> {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?;
        let storage = Arc::new(Storage::new(&data_dir.join("downloader.sqlite3"))?);
        let recovered_jobs = storage.recover_interrupted_jobs()?;
        if recovered_jobs > 0 {
            log::warn!("Marked {recovered_jobs} interrupted download job(s) as canceled");
        }
        let jobs = storage
            .list_jobs()?
            .into_iter()
            .map(|job| (job.id.clone(), job))
            .collect();

        Ok(Self {
            storage,
            jobs: Arc::new(Mutex::new(jobs)),
            cancels: Arc::new(Mutex::new(HashMap::new())),
            processes: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn insert_job(&self, job: Job) -> CommandResult<Job> {
        self.storage.upsert_job(&job)?;
        self.jobs
            .lock()
            .map_err(|error| error.to_string())?
            .insert(job.id.clone(), job.clone());
        Ok(job)
    }

    pub fn update_job<F>(&self, job_id: &str, update: F) -> CommandResult<Job>
    where
        F: FnOnce(&mut Job),
    {
        let mut jobs = self.jobs.lock().map_err(|error| error.to_string())?;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| "Job not found.".to_string())?;
        update(job);
        job.updated_at = Utc::now().to_rfc3339();
        let updated = job.clone();
        drop(jobs);
        self.storage.upsert_job(&updated)?;
        Ok(updated)
    }

    pub fn get_job(&self, job_id: &str) -> CommandResult<Option<Job>> {
        if let Some(job) = self
            .jobs
            .lock()
            .map_err(|error| error.to_string())?
            .get(job_id)
            .cloned()
        {
            return Ok(Some(job));
        }

        self.storage.get_job(job_id)
    }

    pub fn list_jobs(&self) -> CommandResult<Vec<Job>> {
        let mut jobs: Vec<Job> = self
            .jobs
            .lock()
            .map_err(|error| error.to_string())?
            .values()
            .cloned()
            .collect();
        jobs.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(jobs)
    }

    pub fn append_log(&self, job_id: &str, level: &str, message: &str) -> CommandResult<JobLog> {
        self.storage.append_log(job_id, level, message)
    }

    pub fn logs_for_job(&self, job_id: &str) -> CommandResult<Vec<JobLog>> {
        self.storage.logs_for_job(job_id)
    }

    pub fn get_settings(&self) -> CommandResult<Settings> {
        self.storage.get_json("settings")?.map_or_else(
            || Ok(Settings::default()),
            |value| serde_json::from_value(value).map_err(|error| error.to_string()),
        )
    }

    pub fn update_settings(&self, settings: &Settings) -> CommandResult<()> {
        let value = serde_json::to_value(settings).map_err(|error| error.to_string())?;
        self.storage.set_json("settings", &value)
    }

    pub fn youtube_api_key_ids(&self) -> CommandResult<Vec<String>> {
        self.storage.get_json("youtube_api_key_ids")?.map_or_else(
            || Ok(Vec::new()),
            |value| serde_json::from_value(value).map_err(|error| error.to_string()),
        )
    }

    pub fn set_youtube_api_key_ids(&self, ids: &[String]) -> CommandResult<()> {
        let value = serde_json::to_value(ids).map_err(|error| error.to_string())?;
        self.storage.set_json("youtube_api_key_ids", &value)
    }

    pub fn add_cancel_flag(&self, job_id: &str) -> CommandResult<Arc<AtomicBool>> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancels
            .lock()
            .map_err(|error| error.to_string())?
            .insert(job_id.to_string(), flag.clone());
        Ok(flag)
    }

    pub fn set_process(&self, job_id: &str, process_id: u32) -> CommandResult<()> {
        self.processes
            .lock()
            .map_err(|error| error.to_string())?
            .insert(job_id.to_string(), process_id);
        Ok(())
    }

    pub fn clear_process(&self, job_id: &str) {
        if let Ok(mut processes) = self.processes.lock() {
            processes.remove(job_id);
        }
    }

    pub fn cancel(&self, job_id: &str) -> CommandResult<bool> {
        let had_cancel_flag = if let Some(flag) = self
            .cancels
            .lock()
            .map_err(|error| error.to_string())?
            .get(job_id)
        {
            flag.store(true, Ordering::SeqCst);
            true
        } else {
            false
        };

        let process_id = self
            .processes
            .lock()
            .map_err(|error| error.to_string())?
            .get(job_id)
            .copied();
        if let Some(process_id) = process_id {
            process_control::terminate_process_group(process_id);
        }

        Ok(had_cancel_flag || process_id.is_some())
    }

    pub fn remove_cancel_flag(&self, job_id: &str) {
        if let Ok(mut cancels) = self.cancels.lock() {
            cancels.remove(job_id);
        }
    }

    pub fn stop_all_processes(&self) {
        if let Ok(processes) = self.processes.lock() {
            for process_id in processes.values().copied() {
                process_control::force_kill_process_group(process_id);
            }
        }
    }
}

#[tauri::command]
pub async fn analyze_url(input: AnalyzeInput) -> CommandResult<AnalyzeResult> {
    let normalized_url = sites::normalize_url(&input.url)?;
    let site_kind = sites::detect_site(&normalized_url);
    let matching_presets = presets::matching_presets_for_url(&site_kind, &normalized_url);
    let warnings = if matching_presets
        .first()
        .is_some_and(|preset| preset.id == "youtube-channel-catalogue")
    {
        vec!["The catalogue includes the channel's standard Videos tab, not Shorts or livestream tabs.".to_string()]
    } else {
        sites::warnings_for_site(&site_kind)
    };

    Ok(AnalyzeResult {
        normalized_url,
        site_kind,
        presets: matching_presets,
        warnings,
    })
}

#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    state: State<'_, AppState>,
    mut input: StartDownloadRequest,
) -> CommandResult<Job> {
    let normalized_url = sites::normalize_url(&input.url)?;
    let site = sites::detect_site(&normalized_url);
    let preset = presets::find_preset(&input.preset_id)
        .ok_or_else(|| format!("Unknown preset '{}'.", input.preset_id))?;
    if matches!(
        preset.pipeline,
        crate::download::Pipeline::YoutubeChannelExport
    ) {
        let supplied_urls = if input.channel_urls.is_empty() {
            vec![normalized_url.clone()]
        } else {
            input.channel_urls.clone()
        };
        let mut seen = HashSet::new();
        input.channel_urls = supplied_urls
            .iter()
            .map(|url| sites::youtube_channel_videos_url(url))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|url| seen.insert(url.clone()))
            .collect();
        input.url = input.channel_urls[0].clone();
    } else {
        input.url = normalized_url.clone();
    }
    if site == crate::download::SiteKind::Reddit && !tools::has_available_impersonation_target(&app)
    {
        return Err(
            "Reddit currently needs yt-dlp browser impersonation support, but no impersonation target is available. Install the app-managed yt-dlp from Settings > Tools, then retry.".to_string(),
        );
    }
    let settings = settings_with_defaults(&app, &state)?;
    if input.output_dir.as_deref().unwrap_or_default().is_empty() {
        input.output_dir.clone_from(&settings.default_output_dir);
    }
    if matches!(
        preset.pipeline,
        crate::download::Pipeline::YoutubeChannelExport
    ) {
        let export_name =
            crate::download::normalized_youtube_export_name(input.export_name.as_deref())?;
        let export_dir = Path::new(input.output_dir.as_deref().unwrap_or("."))
            .join("youtube_export")
            .join(&export_name);
        if ["youtube_videos.json", "youtube_videos.xlsx"]
            .iter()
            .any(|filename| export_dir.join(filename).exists())
        {
            return Err(format!(
                "An export named '{export_name}' already exists at {}. Choose a different name to preserve the existing files.",
                export_dir.display()
            ));
        }
        input.export_name = Some(export_name);
    }
    if matches!(input.auth, AuthSource::None) && preset.auth == AuthRequirement::Required {
        input.auth = settings.auth.clone();
    }
    if preset.auth == AuthRequirement::Required && !auth_is_configured(&input.auth) {
        return Err("Configure browser cookies or a cookies.txt file in Settings before running this preset.".to_string());
    }
    let fallback_auth = if preset.auth != AuthRequirement::Required
        && preset.auth != AuthRequirement::None
        && auth_is_configured(&settings.auth)
    {
        Some(settings.auth.clone())
    } else {
        None
    };
    let now = Utc::now().to_rfc3339();
    let job = Job {
        id: Uuid::new_v4().to_string(),
        created_at: now.clone(),
        updated_at: now,
        status: JobStatus::Queued,
        site,
        preset_id: preset.id.clone(),
        source_url: normalized_url,
        output_path: None,
        progress: 0.0,
        phase: "Queued".to_string(),
        speed: None,
        eta: None,
        error_message: None,
    };

    let job = state.insert_job(job)?;
    let cancel_flag = state.add_cancel_flag(&job.id)?;
    emit_job(&app, &job, None);

    let state_for_task = state.inner().clone();
    let app_for_task = app.clone();
    let job_id = job.id.clone();

    tauri::async_runtime::spawn_blocking(move || {
        engine::run_download(
            app_for_task,
            state_for_task,
            job_id,
            input,
            preset,
            fallback_auth,
            cancel_flag,
        );
    });

    Ok(job)
}

#[tauri::command]
pub async fn cancel_job(
    app: AppHandle,
    state: State<'_, AppState>,
    input: CancelJobInput,
) -> CommandResult<()> {
    let active_process = state.cancel(&input.job_id)?;
    if let Some(job) = state.get_job(&input.job_id)? {
        if !job.status.is_terminal() {
            let updated = state.update_job(&input.job_id, |job| {
                if active_process {
                    job.phase = "Cancel requested".to_string();
                } else {
                    job.status = JobStatus::Canceled;
                    job.phase = "Canceled".to_string();
                    job.speed = None;
                    job.eta = None;
                    job.error_message = None;
                }
            })?;
            emit_job(&app, &updated, None);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn list_jobs(state: State<'_, AppState>) -> CommandResult<Vec<Job>> {
    state.list_jobs()
}

#[tauri::command]
pub async fn get_job(state: State<'_, AppState>, input: GetJobInput) -> CommandResult<JobDetail> {
    let job = state
        .get_job(&input.job_id)?
        .ok_or_else(|| "Job not found.".to_string())?;
    let logs = state.logs_for_job(&input.job_id)?;
    Ok(JobDetail { job, logs })
}

#[tauri::command]
pub async fn analyze_formats(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AnalyzeFormatsInput,
) -> CommandResult<FormatAnalysis> {
    let normalized_url = sites::normalize_url(&input.url)?;
    let settings = settings_with_defaults(&app, &state)?;
    let auth = input.auth.unwrap_or(settings.auth);
    tauri::async_runtime::spawn_blocking(move || {
        engine::analyze_formats(&app, &normalized_url, &auth)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn open_output_path(input: PathInput) -> CommandResult<()> {
    open_path(&input.path, false)
}

#[tauri::command]
pub async fn reveal_output_path(input: PathInput) -> CommandResult<()> {
    open_path(&input.path, true)
}

#[tauri::command]
pub async fn create_video_thumbnail(
    app: AppHandle,
    input: PathInput,
) -> CommandResult<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || create_thumbnail(&app, &input.path))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn select_download_dir() -> CommandResult<Option<String>> {
    Ok(None)
}

#[tauri::command]
pub async fn check_app_update(app: AppHandle) -> CommandResult<Option<AppUpdate>> {
    let update = app
        .updater()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;

    Ok(update.map(|update| AppUpdate {
        version: update.version,
        notes: update.body.unwrap_or_default(),
    }))
}

#[tauri::command]
pub async fn install_app_update(app: AppHandle) -> CommandResult<()> {
    if let Some(update) = app
        .updater()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?
    {
        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|error| error.to_string())?;
        app.restart();
    }

    Ok(())
}

#[tauri::command]
pub fn get_app_info() -> AppInfo {
    AppInfo {
        name: "Downloader".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        updater_endpoint:
            "https://github.com/ArionStudio/local-downloader/releases/latest/download/latest.json"
                .to_string(),
    }
}

#[tauri::command]
pub async fn check_tool_updates(app: AppHandle) -> CommandResult<Vec<tools::ToolUpdate>> {
    Ok(tools::check_tool_updates(&app))
}

#[tauri::command]
pub async fn install_tool_update(
    app: AppHandle,
    input: InstallToolUpdateInput,
) -> CommandResult<()> {
    let tool = input.tool;
    tauri::async_runtime::spawn_blocking(move || tools::install_tool_update(&app, &tool))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn get_settings(app: AppHandle, state: State<'_, AppState>) -> CommandResult<Settings> {
    settings_with_defaults(&app, &state)
}

#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    input: Settings,
) -> CommandResult<Settings> {
    state.update_settings(&input)?;
    settings_with_defaults(&app, &state)
}

#[tauri::command]
pub async fn list_youtube_api_keys(
    state: State<'_, AppState>,
) -> CommandResult<Vec<YoutubeApiKeyInfo>> {
    let ids = state.youtube_api_key_ids()?;
    Ok(ids
        .into_iter()
        .enumerate()
        .map(|(index, id)| YoutubeApiKeyInfo {
            id,
            label: format!("YouTube API key {}", index + 1),
        })
        .collect())
}

#[tauri::command]
pub async fn add_youtube_api_key(
    state: State<'_, AppState>,
    input: AddYoutubeApiKeyInput,
) -> CommandResult<Vec<YoutubeApiKeyInfo>> {
    let api_key = input.api_key.trim();
    if api_key.is_empty() {
        return Err("Enter a YouTube API key.".to_string());
    }
    let mut ids = state.youtube_api_key_ids()?;
    let existing_keys = youtube_api_keys::load_all(&ids)?;
    if existing_keys.iter().any(|existing| existing == api_key) {
        return Err("That YouTube API key is already saved.".to_string());
    }

    let id = Uuid::new_v4().to_string();
    youtube_api_keys::store(&id, api_key)?;
    ids.push(id.clone());
    if let Err(error) = state.set_youtube_api_key_ids(&ids) {
        let _ = youtube_api_keys::remove(&id);
        return Err(error);
    }
    list_youtube_api_keys(state).await
}

#[tauri::command]
pub async fn remove_youtube_api_key(
    state: State<'_, AppState>,
    input: RemoveYoutubeApiKeyInput,
) -> CommandResult<Vec<YoutubeApiKeyInfo>> {
    let mut ids = state.youtube_api_key_ids()?;
    if !ids.iter().any(|id| id == &input.id) {
        return Err("YouTube API key not found.".to_string());
    }
    let api_key = youtube_api_keys::load_optional(&input.id)?;
    youtube_api_keys::remove(&input.id)?;
    ids.retain(|id| id != &input.id);
    if let Err(error) = state.set_youtube_api_key_ids(&ids) {
        if let Some(api_key) = api_key {
            let _ = youtube_api_keys::store(&input.id, &api_key);
        }
        return Err(error);
    }
    list_youtube_api_keys(state).await
}

fn settings_with_defaults(app: &AppHandle, state: &AppState) -> CommandResult<Settings> {
    let mut settings = state.get_settings()?;
    if settings.default_output_dir.is_none() {
        settings.default_output_dir = app
            .path()
            .download_dir()
            .ok()
            .map(|path| path.display().to_string());
    }
    Ok(settings)
}

pub fn auth_is_configured(auth: &AuthSource) -> bool {
    match auth {
        AuthSource::None => false,
        AuthSource::Browser {
            browser, browsers, ..
        } => {
            !browser.trim().is_empty()
                || browsers
                    .iter()
                    .any(|source| !source.browser.trim().is_empty())
        }
        AuthSource::CookieFile { path } => !path.trim().is_empty(),
    }
}

pub fn emit_job(app: &AppHandle, job: &Job, log: Option<JobLog>) {
    let _ = app.emit(
        "download:job-event",
        DownloadJobEvent {
            job: job.clone(),
            log,
        },
    );
}

fn open_path(path: &str, reveal: bool) -> CommandResult<()> {
    let path = Path::new(path);
    if !path.exists() {
        return Err("File does not exist.".to_string());
    }

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        if reveal {
            command.arg("-R");
        }
        command.arg(path);
        command
    };

    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("xdg-open");
        if reveal {
            command.arg(path.parent().unwrap_or(path));
        } else {
            command.arg(path);
        }
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer");
        if reveal {
            command.arg(format!("/select,{}", path.display()));
        } else {
            command.arg(path);
        }
        command
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open path: {error}"))
}

fn create_thumbnail(app: &AppHandle, path: &str) -> CommandResult<Option<String>> {
    let path = Path::new(path);
    if !path.exists() {
        return Ok(None);
    }

    let Some(ffmpeg) = tools::find_tool(app, "ffmpeg") else {
        return Ok(None);
    };

    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?
        .join("thumbnails");
    std::fs::create_dir_all(&cache_dir).map_err(|error| error.to_string())?;

    let output = cache_dir.join(format!("{}.jpg", stable_path_hash(path)));
    if output.exists() {
        return Ok(Some(output.display().to_string()));
    }

    let status = Command::new(ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-ss", "1"])
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-vf", "scale=360:-1"])
        .arg(&output)
        .status()
        .map_err(|error| format!("Could not run ffmpeg: {error}"))?;

    if status.success() && output.exists() {
        Ok(Some(output.display().to_string()))
    } else {
        Ok(None)
    }
}

fn stable_path_hash(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}
