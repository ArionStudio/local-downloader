use crate::{
    backend::{self, AppState, Settings},
    download::{
        engine, sites, AnalyzeResult, AuthSource, FormatAnalysis, Job, JobDetail, JobLog,
        JobStatus, StartDownloadRequest,
    },
    runtime::Runtime,
    tools, youtube_api_keys,
};
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Command, sync::Mutex};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::UpdaterExt;
use uuid::Uuid;

type CommandResult<T> = Result<T, String>;

#[tauri::command]
pub fn prepare_media_preview(
    app: AppHandle,
    server: State<'_, Mutex<Option<crate::media::MediaServer>>>,
    path: String,
) -> CommandResult<String> {
    let path = std::path::PathBuf::from(path)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !app.asset_protocol_scope().is_allowed(&path) {
        return Err("This video is outside the allowed preview folders.".into());
    }
    let mut server = server.lock().map_err(|e| e.to_string())?;
    if server.is_none() {
        *server = Some(crate::media::MediaServer::start()?);
    }
    server.as_ref().unwrap().register(path)
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

#[tauri::command]
pub async fn analyze_url(input: AnalyzeInput) -> CommandResult<AnalyzeResult> {
    backend::analyze_url(&input.url)
}

#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    state: State<'_, AppState>,
    input: StartDownloadRequest,
) -> CommandResult<Job> {
    let runtime = Runtime::from_app(&app)?;
    let task = backend::prepare_download(&runtime, &state, input, true)?;
    let job = task.job.clone();
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || task.run(runtime, state));
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
        engine::analyze_formats(&Runtime::from_app(&app)?, &normalized_url, &auth)
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
    tauri::async_runtime::spawn_blocking(move || {
        Runtime::from_app(&app).map(|runtime| tools::check_tool_updates(&runtime))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn get_tool_platform() -> tools::ToolPlatform {
    tools::tool_platform()
}

#[tauri::command]
pub async fn install_tool_update(
    app: AppHandle,
    input: InstallToolUpdateInput,
) -> CommandResult<()> {
    let tool = input.tool;
    tauri::async_runtime::spawn_blocking(move || {
        tools::install_tool_update(&Runtime::from_app(&app)?, &tool)
    })
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
    backend::settings_with_defaults(&Runtime::from_app(app)?, state)
}
fn emit_job(app: &AppHandle, job: &Job, log: Option<JobLog>) {
    if let Ok(runtime) = Runtime::from_app(app) {
        backend::emit_job(&runtime, job, log);
    }
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

    let Some(ffmpeg) = tools::find_tool(&Runtime::from_app(app)?, "ffmpeg") else {
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
