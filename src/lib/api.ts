import { convertFileSrc, invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-dialog"
import { listen } from "@tauri-apps/api/event"
import {
  readText as readNativeClipboardText,
  writeText as writeNativeClipboardText,
} from "@tauri-apps/plugin-clipboard-manager"
import {
  analyzeLocally,
  createFallbackJob,
  defaultSettings,
} from "@/lib/fallback"
import type {
  AnalyzeResult,
  AppInfo,
  AppUpdate,
  AuthSource,
  DownloadJobEvent,
  FormatAnalysis,
  Job,
  JobDetail,
  Settings,
  StartDownloadRequest,
  ToolUpdate,
} from "@/lib/types"

const isTauri = () => "__TAURI_INTERNALS__" in window

export async function analyzeUrl(url: string): Promise<AnalyzeResult> {
  if (!isTauri()) return analyzeLocally(url)
  return invoke("analyze_url", { input: { url } })
}

export async function analyzeFormats(
  url: string,
  auth?: AuthSource,
): Promise<FormatAnalysis> {
  if (!isTauri()) return { title: null, duration: null, formats: [] }
  return invoke("analyze_formats", { input: { url, auth } })
}

export async function startDownload(request: StartDownloadRequest): Promise<Job> {
  if (!isTauri()) return createFallbackJob(request)
  return invoke("start_download", { input: request })
}

export async function cancelJob(jobId: string): Promise<void> {
  if (!isTauri()) return
  return invoke("cancel_job", { input: { jobId } })
}

export async function listJobs(): Promise<Job[]> {
  if (!isTauri()) return []
  return invoke("list_jobs")
}

export async function getJob(jobId: string): Promise<JobDetail> {
  if (!isTauri()) throw new Error("Job details are available in the desktop app.")
  return invoke("get_job", { input: { jobId } })
}

export async function selectDownloadDir(): Promise<string | null> {
  if (!isTauri()) return null
  const selected = await open({ directory: true, multiple: false })
  return typeof selected === "string" ? selected : null
}

export async function openOutputPath(path: string): Promise<void> {
  if (!isTauri()) return
  return invoke("open_output_path", { input: { path } })
}

export async function revealOutputPath(path: string): Promise<void> {
  if (!isTauri()) return
  return invoke("reveal_output_path", { input: { path } })
}

export async function createVideoThumbnail(path: string): Promise<string | null> {
  if (!isTauri()) return null
  return invoke("create_video_thumbnail", { input: { path } })
}

export async function checkAppUpdate(): Promise<AppUpdate | null> {
  if (!isTauri()) return null
  return invoke("check_app_update")
}

export async function getAppInfo(): Promise<AppInfo> {
  if (!isTauri()) {
    return {
      name: "Downloader",
      version: "0.1.1",
      updaterEndpoint:
        "https://github.com/ArionStudio/local-downloader/releases/latest/download/latest.json",
    }
  }
  return invoke("get_app_info")
}

export async function installAppUpdate(): Promise<void> {
  if (!isTauri()) return
  return invoke("install_app_update")
}

export async function checkToolUpdates(): Promise<ToolUpdate[]> {
  if (!isTauri()) return []
  return invoke("check_tool_updates")
}

export async function installToolUpdate(tool: ToolUpdate["tool"]): Promise<void> {
  if (!isTauri()) return
  return invoke("install_tool_update", { input: { tool } })
}

export async function getSettings(): Promise<Settings> {
  if (!isTauri()) return defaultSettings
  return invoke("get_settings")
}

export async function updateSettings(input: Partial<Settings>): Promise<Settings> {
  if (!isTauri()) return { ...defaultSettings, ...input }
  const current = await getSettings()
  return invoke("update_settings", { input: { ...current, ...input } })
}

export async function readClipboardText(): Promise<string> {
  if (isTauri()) return readNativeClipboardText()
  return navigator.clipboard.readText()
}

export async function writeClipboardText(text: string): Promise<void> {
  if (isTauri()) return writeNativeClipboardText(text)
  return navigator.clipboard.writeText(text)
}

export function onDownloadJobEvent(
  callback: (event: DownloadJobEvent) => void,
): Promise<() => void> {
  if (!isTauri()) return Promise.resolve(() => undefined)
  return listen<DownloadJobEvent>("download:job-event", (event) => callback(event.payload))
}

export function localFilePreviewUrl(path: string): string {
  if (!isTauri()) return `file://${path}`
  return convertFileSrc(path)
}
