import type {
  AnalyzeResult,
  AuthSource,
  Job,
  Preset,
  Settings,
  SiteKind,
  StartDownloadRequest,
} from "@/lib/types"

export const presets: Preset[] = [
  {
    id: "generic-page-video-highest",
    siteKinds: ["generic"],
    label: "Generic Page Video",
    description: "Find the highest quality video available on a standard page.",
    outputKind: "video",
    pipeline: "yt_dlp",
    auth: "optional",
  },
  {
    id: "generic-direct-stream-highest",
    siteKinds: ["generic", "direct_hls", "sawhorse"],
    label: "Direct Stream Video",
    description: "Save the highest quality HLS or DASH stream as mp4.",
    outputKind: "video",
    pipeline: "http_resolve_then_download",
    auth: "optional",
  },
  {
    id: "reddit-post-video-highest",
    siteKinds: ["reddit"],
    label: "Reddit Post Video",
    description:
      "Download the highest quality video from a single Reddit post.",
    outputKind: "video",
    pipeline: "yt_dlp",
    auth: "optional",
  },
  {
    id: "reddit-multiple-media-highest",
    siteKinds: ["reddit"],
    label: "Reddit Multiple Media",
    description: "Download the highest quality videos from a multi-media post.",
    outputKind: "video",
    pipeline: "yt_dlp",
    auth: "optional",
  },
  {
    id: "youtube-video-highest",
    siteKinds: ["youtube"],
    label: "YouTube Video",
    description:
      "Download the highest quality YouTube video and retry with saved auth only if needed.",
    outputKind: "video",
    pipeline: "yt_dlp",
    auth: "recommended",
  },
  {
    id: "linkedin-post-video-highest",
    siteKinds: ["linkedin"],
    label: "LinkedIn Post Video",
    description: "Use local cookies to save the highest quality post video.",
    outputKind: "video",
    pipeline: "yt_dlp",
    auth: "required",
  },
  {
    id: "linkedin-article-video-highest",
    siteKinds: ["linkedin"],
    label: "LinkedIn Article Video",
    description: "Use local cookies to resolve video embedded in an article.",
    outputKind: "video",
    pipeline: "http_resolve_then_download",
    auth: "required",
  },
  {
    id: "linkedin-feed-update-video-highest",
    siteKinds: ["linkedin"],
    label: "LinkedIn Feed Update",
    description:
      "Use local cookies to resolve the DASH stream from feed update metadata.",
    outputKind: "video",
    pipeline: "http_resolve_then_download",
    auth: "required",
  },
  {
    id: "crunchyroll-video-highest",
    siteKinds: ["crunchyroll"],
    label: "Crunchyroll Video",
    description:
      "Download the highest quality stream available to yt-dlp with your account cookies.",
    outputKind: "video",
    pipeline: "yt_dlp",
    auth: "required",
  },
  {
    id: "x-article-video-highest",
    siteKinds: ["x"],
    label: "X Article Video",
    description:
      "Resolve every video embedded in an X article and download the highest MP4 variant.",
    outputKind: "video",
    pipeline: "http_resolve_then_download",
    auth: "required",
  },
  {
    id: "vimeo-video-highest",
    siteKinds: ["vimeo"],
    label: "Vimeo Video",
    description: "Download the highest quality Vimeo video.",
    outputKind: "video",
    pipeline: "yt_dlp",
    auth: "optional",
  },
  {
    id: "sawhorse-portfolio-video-highest",
    siteKinds: ["sawhorse"],
    label: "Sawhorse Portfolio Video",
    description: "Resolve embedded portfolio video at the highest quality.",
    outputKind: "video",
    pipeline: "http_resolve_then_download",
    auth: "optional",
  },
]

export function detectSite(input: string): SiteKind {
  try {
    const url = new URL(input.trim())
    const host = url.hostname.replace(/^www\./, "").toLowerCase()
    const path = url.pathname.toLowerCase()

    if (/\.(m3u8|mpd)$/.test(path)) return "direct_hls"
    if (/\.(mp4|mov|m4v|webm|mkv)$/.test(path)) return "direct_file"
    if (host === "redd.it" || host.endsWith("reddit.com")) return "reddit"
    if (host.endsWith("linkedin.com")) return "linkedin"
    if (host.endsWith("crunchyroll.com")) return "crunchyroll"
    if (host === "youtu.be" || host.endsWith("youtube.com")) return "youtube"
    if (host === "x.com" || host === "twitter.com") return "x"
    if (host.endsWith("vimeo.com")) return "vimeo"
    if (host.endsWith("sawhorsela.com")) return "sawhorse"

    return "generic"
  } catch {
    return "generic"
  }
}

export function analyzeLocally(input: string): AnalyzeResult {
  const normalizedUrl = input.trim()
  const siteKind = detectSite(normalizedUrl)
  const matching = presets.filter((preset) =>
    preset.siteKinds.includes(siteKind)
  )
  const warnings = warningsForSite(siteKind)

  return {
    normalizedUrl,
    siteKind,
    presets:
      matching.length > 0
        ? matching
        : presets.filter((preset) => preset.siteKinds.includes("generic")),
    warnings,
  }
}

function warningsForSite(siteKind: SiteKind): string[] {
  if (siteKind === "linkedin") {
    return [
      "LinkedIn usually needs browser cookies from the same local machine.",
    ]
  }
  if (siteKind === "crunchyroll") {
    return [
      "Crunchyroll needs your own account cookies; DRM-protected streams are not bypassed.",
    ]
  }
  if (siteKind === "reddit") {
    return [
      "Reddit may require logged-in cookies and yt-dlp browser impersonation support.",
    ]
  }
  if (siteKind === "youtube") {
    return [
      "YouTube will try without cookies first, then retry with saved auth if needed.",
    ]
  }
  if (siteKind === "x")
    return ["X article/media extraction may require cookies."]
  return []
}

export function createFallbackJob(input: StartDownloadRequest): Job {
  const now = new Date().toISOString()
  return {
    id: crypto.randomUUID(),
    createdAt: now,
    updatedAt: now,
    status: "failed",
    site: detectSite(input.url),
    presetId: input.presetId,
    sourceUrl: input.url,
    outputPath: null,
    progress: 0,
    phase: "Tauri backend unavailable in browser preview",
    speed: null,
    eta: null,
    errorMessage:
      "Run the desktop app with pnpm tauri dev to start real downloads.",
  }
}

export const defaultAuth: AuthSource = { kind: "none" }

export const defaultSettings: Settings = {
  defaultOutputDir: null,
  auth: {
    kind: "browser",
    browser: "firefox",
    browsers: [{ browser: "firefox" }],
  },
}
