use crate::runtime::Runtime;
use crate::{
    download::{
        engine, presets, sites, AnalyzeResult, AuthRequirement, AuthSource, Job, JobLog, JobStatus,
        StartDownloadRequest,
    },
    process_control,
    storage::Storage,
    tools,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use uuid::Uuid;

type CommandResult<T> = Result<T, String>;

#[derive(Clone)]
pub struct AppState {
    storage: Arc<Storage>,
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    cancels: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    processes: Arc<Mutex<HashMap<String, u32>>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadJobEvent {
    pub job: Job,
    pub log: Option<JobLog>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub default_output_dir: Option<String>,
    pub auth: AuthSource,
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
    pub fn new(data_dir: &Path, recover: bool) -> CommandResult<Self> {
        let storage = Arc::new(Storage::new(&data_dir.join("downloader.sqlite3"))?);
        if recover {
            storage.recover_interrupted_jobs()?;
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
        self.storage.upsert_job(&updated)?;
        drop(jobs);
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

pub fn analyze_url(url: &str) -> CommandResult<AnalyzeResult> {
    let normalized_url = sites::normalize_url(url)?;
    let site_kind = sites::detect_site(&normalized_url);
    let matching_presets = presets::matching_presets_for_url(&site_kind, &normalized_url);
    let warnings = if matching_presets
        .first()
        .is_some_and(|preset| preset.id == "youtube-channel-catalogue")
    {
        vec![
            "Choose whether the catalogue includes channel videos, Shorts, or both; livestream tabs are not included."
                .to_string(),
        ]
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

pub struct DownloadTask {
    pub job: Job,
    input: StartDownloadRequest,
    preset: crate::download::Preset,
    fallback_auth: Option<AuthSource>,
    cancel_flag: Arc<AtomicBool>,
}
impl DownloadTask {
    pub fn run(self, app: Runtime, state: AppState) {
        engine::run_download(
            app,
            state,
            self.job.id,
            self.input,
            self.preset,
            self.fallback_auth,
            self.cancel_flag,
        );
    }
}
pub fn prepare_download(
    app: &Runtime,
    state: &AppState,
    mut input: StartDownloadRequest,
    allow_saved_auth: bool,
) -> CommandResult<DownloadTask> {
    let normalized_url = sites::normalize_url(&input.url)?;
    let site = sites::detect_site(&normalized_url);
    let preset = presets::find_preset(&input.preset_id)
        .ok_or_else(|| format!("Unknown preset '{}'.", input.preset_id))?;
    if input.output_profile == crate::download::OutputProfile::Xrbazaar {
        if !matches!(preset.output_kind, crate::download::OutputKind::Video) {
            return Err("XRBAZAAR preparation is available for videos only.".into());
        }
        if !matches!(
            input.advanced.as_ref().map(|a| &a.format),
            None | Some(crate::download::FormatSelection::Best)
        ) {
            return Err("XRBAZAAR downloads use the best video with its original audio. Choose the standard download for a custom format.".into());
        }
        for tool in ["ffmpeg", "ffprobe"] {
            if tools::find_tool(&app, tool).is_none() {
                return Err(format!("XRBAZAAR preparation needs {tool}. Install the FFmpeg package in Settings > Tools."));
            }
        }
    }
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
    let mut settings = settings_with_defaults(app, state)?;
    if !allow_saved_auth || !input.allow_saved_auth {
        settings.auth = AuthSource::None;
    }
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
        ready_paths: Vec::new(),
        progress: 0.0,
        phase: "Queued".to_string(),
        speed: None,
        eta: None,
        error_message: None,
    };

    let job = state.insert_job(job)?;
    let cancel_flag = state.add_cancel_flag(&job.id)?;
    emit_job(&app, &job, None);

    Ok(DownloadTask {
        job,
        input,
        preset,
        fallback_auth,
        cancel_flag,
    })
}
pub fn settings_with_defaults(app: &Runtime, state: &AppState) -> CommandResult<Settings> {
    let mut settings = state.get_settings()?;
    if settings.default_output_dir.is_none() {
        settings.default_output_dir = Some(app.download_dir.display().to_string());
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

pub fn emit_job(app: &Runtime, job: &Job, log: Option<JobLog>) {
    (app.on_event)(DownloadJobEvent {
        job: job.clone(),
        log,
    });
}
