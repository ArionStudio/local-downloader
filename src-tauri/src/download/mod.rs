pub mod engine;
pub mod presets;
pub mod sites;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SiteKind {
    Generic,
    Reddit,
    Linkedin,
    Youtube,
    X,
    Vimeo,
    Sawhorse,
    DirectHls,
    DirectFile,
}

impl SiteKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SiteKind::Generic => "generic",
            SiteKind::Reddit => "reddit",
            SiteKind::Linkedin => "linkedin",
            SiteKind::Youtube => "youtube",
            SiteKind::X => "x",
            SiteKind::Vimeo => "vimeo",
            SiteKind::Sawhorse => "sawhorse",
            SiteKind::DirectHls => "direct_hls",
            SiteKind::DirectFile => "direct_file",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "reddit" => SiteKind::Reddit,
            "linkedin" => SiteKind::Linkedin,
            "youtube" => SiteKind::Youtube,
            "x" => SiteKind::X,
            "vimeo" => SiteKind::Vimeo,
            "sawhorse" => SiteKind::Sawhorse,
            "direct_hls" => SiteKind::DirectHls,
            "direct_file" => SiteKind::DirectFile,
            _ => SiteKind::Generic,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pipeline {
    YtDlp,
    FfmpegHls,
    HttpResolveThenDownload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthRequirement {
    None,
    Optional,
    Recommended,
    Required,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub site_kinds: Vec<SiteKind>,
    pub label: String,
    pub description: String,
    pub output_kind: OutputKind,
    pub pipeline: Pipeline,
    pub auth: AuthRequirement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeResult {
    pub normalized_url: String,
    pub site_kind: SiteKind,
    pub presets: Vec<Preset>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Resolving,
    Downloading,
    Postprocessing,
    Completed,
    Failed,
    Canceled,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Resolving => "resolving",
            JobStatus::Downloading => "downloading",
            JobStatus::Postprocessing => "postprocessing",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
            JobStatus::Canceled => "canceled",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "resolving" => JobStatus::Resolving,
            "downloading" => JobStatus::Downloading,
            "postprocessing" => JobStatus::Postprocessing,
            "completed" => JobStatus::Completed,
            "failed" => JobStatus::Failed,
            "canceled" => JobStatus::Canceled,
            _ => JobStatus::Queued,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobStatus::Completed | JobStatus::Failed | JobStatus::Canceled
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub status: JobStatus,
    pub site: SiteKind,
    pub preset_id: String,
    pub source_url: String,
    pub output_path: Option<String>,
    pub progress: f64,
    pub phase: String,
    pub speed: Option<String>,
    pub eta: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobLog {
    pub id: i64,
    pub job_id: String,
    pub created_at: String,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDetail {
    #[serde(flatten)]
    pub job: Job,
    pub logs: Vec<JobLog>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAuthSource {
    pub browser: String,
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthSource {
    None,
    Browser {
        #[serde(default)]
        browser: String,
        #[serde(default)]
        profile: Option<String>,
        #[serde(default)]
        browsers: Vec<BrowserAuthSource>,
    },
    CookieFile {
        path: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FormatSelection {
    Best,
    Format { format_id: String },
    AudioOnly,
    VideoOnly { format_id: Option<String> },
}

impl Default for FormatSelection {
    fn default() -> Self {
        Self::Best
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentSelection {
    pub enabled: bool,
    pub start_seconds: f64,
    pub end_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AdvancedDownloadOptions {
    #[serde(default)]
    pub format: FormatSelection,
    pub segment: Option<SegmentSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartDownloadRequest {
    pub url: String,
    pub preset_id: String,
    pub output_dir: Option<String>,
    pub filename_template: Option<String>,
    pub auth: AuthSource,
    pub advanced: Option<AdvancedDownloadOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatOption {
    pub format_id: String,
    pub label: String,
    pub ext: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub fps: Option<f64>,
    pub tbr: Option<f64>,
    pub filesize: Option<i64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub has_video: bool,
    pub has_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatAnalysis {
    pub title: Option<String>,
    pub duration: Option<f64>,
    pub formats: Vec<FormatOption>,
}
