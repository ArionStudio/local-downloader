export type SiteKind =
  | "generic"
  | "reddit"
  | "linkedin"
  | "crunchyroll"
  | "youtube"
  | "x"
  | "vimeo"
  | "sawhorse"
  | "direct_hls"
  | "direct_file"

export type OutputKind = "video"

export type Pipeline = "yt_dlp" | "ffmpeg_hls" | "http_resolve_then_download"

export type AuthRequirement = "none" | "optional" | "recommended" | "required"

export type Preset = {
  id: string
  siteKinds: SiteKind[]
  label: string
  description: string
  outputKind: OutputKind
  pipeline: Pipeline
  auth: AuthRequirement
}

export type BrowserKind =
  | "firefox"
  | "zen"
  | "helium"
  | "chrome"
  | "chromium"
  | "brave"
  | "edge"
  | "safari"
  | "vivaldi"
  | "opera"
  | "whale"

export type BrowserAuthSource = {
  browser: BrowserKind
  profile?: string | null
}

export type AuthSource =
  | { kind: "none" }
  | {
      kind: "browser"
      browser?: BrowserKind
      profile?: string | null
      browsers?: BrowserAuthSource[]
    }
  | { kind: "cookie_file"; path: string }

export type AnalyzeResult = {
  normalizedUrl: string
  siteKind: SiteKind
  presets: Preset[]
  warnings: string[]
}

export type JobStatus =
  | "queued"
  | "resolving"
  | "downloading"
  | "postprocessing"
  | "completed"
  | "failed"
  | "canceled"

export type Job = {
  id: string
  createdAt: string
  updatedAt: string
  status: JobStatus
  site: SiteKind
  presetId: string
  sourceUrl: string
  outputPath?: string | null
  progress: number
  phase: string
  speed?: string | null
  eta?: string | null
  errorMessage?: string | null
}

export type JobDetail = Job & {
  logs: JobLog[]
}

export type JobLog = {
  id: number
  jobId: string
  createdAt: string
  level: "info" | "warn" | "error"
  message: string
}

export type StartDownloadRequest = {
  url: string
  presetId: string
  outputDir?: string | null
  filenameTemplate?: string | null
  auth: AuthSource
  advanced?: AdvancedDownloadOptions | null
}

export type FormatSelection =
  | { kind: "best" }
  | { kind: "format"; formatId: string }
  | { kind: "audio_only" }
  | { kind: "video_only"; formatId?: string | null }

export type SegmentSelection = {
  enabled: boolean
  startSeconds: number
  endSeconds?: number | null
}

export type AdvancedDownloadOptions = {
  format: FormatSelection
  segment?: SegmentSelection | null
}

export type FormatOption = {
  formatId: string
  label: string
  ext?: string | null
  width?: number | null
  height?: number | null
  fps?: number | null
  tbr?: number | null
  filesize?: number | null
  vcodec?: string | null
  acodec?: string | null
  hasVideo: boolean
  hasAudio: boolean
}

export type FormatAnalysis = {
  title?: string | null
  duration?: number | null
  formats: FormatOption[]
}

export type AppUpdate = {
  version: string
  notes: string
}

export type AppInfo = {
  name: string
  version: string
  updaterEndpoint: string
}

export type ToolUpdate = {
  tool: "yt-dlp" | "ffmpeg"
  status: "installed" | "missing"
  currentVersion?: string | null
  availableVersion?: string | null
  path?: string | null
  message: string
}

export type Settings = {
  defaultOutputDir?: string | null
  auth: AuthSource
}

export type DownloadJobEvent = {
  job: Job
  log?: JobLog | null
}
