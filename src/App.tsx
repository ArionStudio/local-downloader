import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import type { FormEvent, ReactNode } from "react"
import {
  AlertCircle,
  ArrowDownToLine,
  History,
  Monitor,
  X,
  Check,
  ChevronRight,
  Clipboard,
  ClipboardPaste,
  Copy,
  Download,
  ExternalLink,
  FolderOpen,
  KeyRound,
  Film,
  Loader2,
  List,
  Music,
  Play,
  RefreshCw,
  Scissors,
  Search,
  Settings as SettingsIcon,
  Shield,
  SlidersHorizontal,
  Square,
  Trash2,
  Wrench,
} from "lucide-react"
import { useTheme } from "@/components/theme-provider"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Slider } from "@/components/ui/slider"
import { Switch } from "@/components/ui/switch"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import {
  analyzeFormats,
  addYoutubeApiKey,
  analyzeUrl,
  cancelJob,
  checkAppUpdate,
  checkToolUpdates,
  createVideoThumbnail,
  getAppInfo,
  getJob,
  getSettings,
  installAppUpdate,
  installToolUpdate,
  prepareMediaPreview,
  getToolPlatform,
  listJobs,
  listYoutubeApiKeys,
  localFilePreviewUrl,
  onDownloadJobEvent,
  openOutputPath,
  readClipboardText,
  removeYoutubeApiKey,
  revealOutputPath,
  selectDownloadDir,
  startDownload,
  updateSettings,
  writeClipboardText,
} from "@/lib/api"
import { defaultSettings } from "@/lib/fallback"
import { setPlayback, usePlayback } from "@/lib/playback"
import type {
  AnalyzeResult,
  AdvancedDownloadOptions,
  AppInfo,
  AppUpdate,
  AuthRequirement,
  AuthSource,
  BrowserAuthSource,
  BrowserKind,
  FormatAnalysis,
  FormatOption,
  FormatSelection,
  Job,
  JobLog,
  Preset,
  Settings as DownloaderSettings,
  SiteKind,
  StartDownloadRequest,
  OutputProfile,
  ToolUpdate,
  ToolPlatform,
  YoutubeCatalogueContent,
  YoutubeApiKeyInfo,
} from "@/lib/types"
import { cn } from "@/lib/utils"

type AppTab = "download" | "runs" | "downloaded" | "settings"

type AppUpdateState = {
  status:
    | "idle"
    | "checking"
    | "current"
    | "available"
    | "installing"
    | "restarting"
    | "failed"
  update: AppUpdate | null
  checkedAt: string | null
  message: string
}

type ToolCheckState = {
  status: "idle" | "checking" | "installing" | "ready" | "issues" | "failed"
  tools: ToolUpdate[]
  checkedAt: string | null
  message: string
}

type DownloadAsset = {
  path: string
  job: Job
}

type DownloadNavigatorItem = {
  id: string
  index: number
  sourceUrl: string
  siteLabel: string
  title: string
  status: string
}

const siteLabels: Record<SiteKind, string> = {
  generic: "Generic",
  reddit: "Reddit",
  linkedin: "LinkedIn",
  crunchyroll: "Crunchyroll",
  youtube: "YouTube",
  x: "X",
  vimeo: "Vimeo",
  direct_hls: "HLS",
  direct_file: "File",
}

const authLabels: Record<AuthRequirement, string> = {
  none: "No auth",
  optional: "Optional auth",
  recommended: "Auth helps",
  required: "Auth required",
}

const browsers: BrowserKind[] = [
  "firefox",
  "zen",
  "helium",
  "chrome",
  "chromium",
  "brave",
  "edge",
  "safari",
  "vivaldi",
  "opera",
  "whale",
]

const defaultAdvancedOptions: AdvancedDownloadOptions = {
  format: { kind: "best" },
  segment: {
    enabled: false,
    startSeconds: 0,
    endSeconds: null,
  },
}

const autoQualityValue = "__auto__"
const allRunPresetsValue = "__all_presets__"
const runsPanelDomId = "runs-panel"

function youtubeExportNameError(value: string): string | null {
  const name = value.trim()
  if (!name) return "Enter a name for this export."
  if ([...name].length > 80) return "Use 80 characters or fewer."
  if (
    name === "." ||
    name === ".." ||
    name.endsWith(".") ||
    [...name].some(
      (character) =>
        character.charCodeAt(0) < 32 || '<>:"/\\|?*'.includes(character)
    )
  ) {
    return 'Do not use path separators or < > : " | ? *, or end with a period.'
  }
  if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(name)) {
    return "Choose a different name; this one is reserved by Windows."
  }
  return null
}

function App() {
  const startingRequests = useRef(new Set<string>())
  const [startingUrls, setStartingUrls] = useState<string[]>([])
  const [startingProfiles, setStartingProfiles] = useState<
    Record<string, OutputProfile>
  >({})
  const [savingSettings, setSavingSettings] = useState(false)
  const [settingsMessage, setSettingsMessage] = useState("")
  const [url, setUrl] = useState("")
  const [youtubeExportName, setYoutubeExportName] = useState("")
  const [youtubeCatalogueContent, setYoutubeCatalogueContent] =
    useState<YoutubeCatalogueContent>("all")
  const [activeTab, setActiveTab] = useState<AppTab>("download")
  const [runPresetFilter, setRunPresetFilter] = useState(allRunPresetsValue)
  const [analysesByUrl, setAnalysesByUrl] = useState<
    Record<string, AnalyzeResult>
  >({})
  const [selectedPresetByUrl, setSelectedPresetByUrl] = useState<
    Record<string, string>
  >({})
  const [settings, setSettings] = useState<DownloaderSettings>(defaultSettings)
  const [draftSettings, setDraftSettings] =
    useState<DownloaderSettings>(defaultSettings)
  const [advancedByPreset, setAdvancedByPreset] = useState<
    Record<string, AdvancedDownloadOptions>
  >({})
  const [formatsByPreset, setFormatsByPreset] = useState<
    Record<string, FormatAnalysis>
  >({})
  const [loadingFormatsKey, setLoadingFormatsKey] = useState<string | null>(
    null
  )
  const [analyzingUrls, setAnalyzingUrls] = useState<Record<string, boolean>>(
    {}
  )
  const [assetPathsByJob, setAssetPathsByJob] = useState<
    Record<string, string[]>
  >({})
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null)
  const [jobs, setJobs] = useState<Job[]>([])
  const [jobLogs, setJobLogs] = useState<Record<string, JobLog[]>>({})
  const [sessionLogs, setSessionLogs] = useState<string[]>([])
  const [error, setError] = useState<string | null>(null)
  const [appUpdateState, setAppUpdateState] = useState<AppUpdateState>({
    status: "idle",
    update: null,
    checkedAt: null,
    message: "Not checked in this session.",
  })
  const [toolCheckState, setToolCheckState] = useState<ToolCheckState>({
    status: "idle",
    tools: [],
    checkedAt: null,
    message: "Not checked in this session.",
  })
  const analysesByUrlRef = useRef<Record<string, AnalyzeResult>>({})
  const analyzingUrlSet = useRef(new Set<string>())

  const pushSessionLog = useCallback((line: string) => {
    setSessionLogs((current) => [...current.slice(-199), line])
  }, [])

  const inputUrls = useMemo(() => extractUrls(url), [url])
  const inputUrlsKey = inputUrls.join("\n")
  const isAnalyzing = Object.values(analyzingUrls).some(Boolean)
  const channelCatalogueUrls = useMemo(
    () =>
      inputUrls.filter((inputUrl) => {
        const analysis = analysesByUrl[inputUrl]
        const presetId =
          selectedPresetByUrl[inputUrl] ?? analysis?.presets[0]?.id
        return presetId === "youtube-channel-catalogue"
      }),
    [analysesByUrl, inputUrls, selectedPresetByUrl]
  )
  const channelCatalogueLeadUrl = channelCatalogueUrls[0] ?? null
  const resultUrls = useMemo(() => {
    const groupedUrls = new Set(channelCatalogueUrls)
    return inputUrls.filter(
      (inputUrl) =>
        !groupedUrls.has(inputUrl) || inputUrl === channelCatalogueLeadUrl
    )
  }, [channelCatalogueLeadUrl, channelCatalogueUrls, inputUrls])

  useEffect(() => {
    getAppInfo()
      .then((info) => setAppInfo(info))
      .catch(() => undefined)

    checkToolUpdates()
      .then((tools) => setToolCheckState(toolCheckStateFromTools(tools)))
      .catch((reason) =>
        setToolCheckState({
          status: "failed",
          tools: [],
          checkedAt: new Date().toISOString(),
          message: reason instanceof Error ? reason.message : String(reason),
        })
      )

    getSettings()
      .then((loaded) => {
        setSettings(loaded)
        setDraftSettings(loaded)
      })
      .catch(() => undefined)

    listJobs()
      .then((initialJobs) => setJobs(initialJobs))
      .catch(() => undefined)

    const refreshInterval = window.setInterval(() => {
      listJobs()
        .then((updatedJobs) => setJobs(updatedJobs))
        .catch(() => undefined)
    }, 2500)

    const unlistenPromise = onDownloadJobEvent(({ job, log }) => {
      setJobs((current) => upsertJob(current, job))
      if (log) {
        setJobLogs((current) => ({
          ...current,
          [job.id]: [...(current[job.id] ?? []), log],
        }))
        pushSessionLog(
          `${log.createdAt} ${log.level.toUpperCase()} ${job.presetId}: ${log.message}`
        )
      }
    })

    return () => {
      window.clearInterval(refreshInterval)
      unlistenPromise.then((unlisten) => unlisten()).catch(() => undefined)
    }
  }, [pushSessionLog])

  useEffect(() => {
    analysesByUrlRef.current = analysesByUrl
  }, [analysesByUrl])

  useEffect(() => {
    let canceled = false
    const jobsToLoad = jobs.filter(
      (job) => assetPathsByJob[job.id] === undefined
    )

    jobsToLoad.forEach((job) => {
      getJob(job.id)
        .then((detail) => {
          if (canceled) return
          setAssetPathsByJob((current) => ({
            ...current,
            [job.id]: assetPathsFromJob(detail, detail.logs),
          }))
        })
        .catch(() => {
          if (canceled) return
          setAssetPathsByJob((current) => ({
            ...current,
            [job.id]: assetPathsFromJob(job, jobLogs[job.id] ?? []),
          }))
        })
    })

    return () => {
      canceled = true
    }
  }, [assetPathsByJob, jobLogs, jobs])

  const runJobs = jobs
  const knownPresetLabels = useMemo(() => {
    const labels: Record<string, string> = {}
    Object.values(analysesByUrl).forEach((analysis) => {
      analysis.presets.forEach((preset) => {
        labels[preset.id] = preset.label
      })
    })
    return labels
  }, [analysesByUrl])
  const runPresetOptions = useMemo(
    () => runPresetOptionsFromJobs(runJobs, knownPresetLabels),
    [knownPresetLabels, runJobs]
  )
  const visibleRunJobs = useMemo(
    () =>
      runPresetFilter === allRunPresetsValue
        ? runJobs
        : runJobs.filter((job) => job.presetId === runPresetFilter),
    [runJobs, runPresetFilter]
  )
  const assetsByJob = useMemo(
    () =>
      Object.fromEntries(
        jobs.map((job) => [
          job.id,
          uniqueStrings([
            ...(assetPathsByJob[job.id] ?? []),
            ...assetPathsFromJob(job, jobLogs[job.id] ?? []),
          ]),
        ])
      ),
    [assetPathsByJob, jobLogs, jobs]
  )
  const downloadedAssets = useMemo(
    () =>
      jobs
        .flatMap((job) =>
          (assetsByJob[job.id] ?? []).map((path) => ({ path, job }))
        )
        .filter(
          (asset, index, all) =>
            all.findIndex((other) => other.path === asset.path) === index
        ),
    [assetsByJob, jobs]
  )
  const downloadNavItems = useMemo(
    () =>
      resultUrls.map((inputUrl, index) => {
        const analysis = analysesByUrl[inputUrl]
        const isChannelCatalogueGroup = inputUrl === channelCatalogueLeadUrl
        return {
          id: downloadItemDomId(inputUrl),
          index: index + 1,
          sourceUrl: inputUrl,
          siteLabel: analysis ? siteLabels[analysis.siteKind] : "Link",
          title:
            isChannelCatalogueGroup && channelCatalogueUrls.length > 1
              ? `${channelCatalogueUrls.length} channels`
              : compactUrlLabel(inputUrl),
          status: analyzingUrls[inputUrl]
            ? "Inspecting"
            : analysis
              ? "Ready"
              : "Queued",
        }
      }),
    [
      analysesByUrl,
      analyzingUrls,
      channelCatalogueLeadUrl,
      channelCatalogueUrls.length,
      resultUrls,
    ]
  )

  const runAnalysis = useCallback(
    async (inputUrl: string, normalizeInput: boolean) => {
      const cleanUrl = inputUrl.trim()
      if (!looksLikeUrl(cleanUrl)) return
      if (
        analysesByUrlRef.current[cleanUrl] ||
        analyzingUrlSet.current.has(cleanUrl)
      ) {
        return
      }

      analyzingUrlSet.current.add(cleanUrl)
      setAnalyzingUrls((current) => ({ ...current, [cleanUrl]: true }))
      setError(null)
      try {
        pushSessionLog(`${new Date().toISOString()} INFO analyze: ${cleanUrl}`)
        const result = await analyzeUrl(cleanUrl)
        setAnalysesByUrl((current) => ({
          ...current,
          [cleanUrl]: result,
          [result.normalizedUrl]: result,
        }))
        setSelectedPresetByUrl((current) => {
          const firstPresetId = result.presets[0]?.id
          if (!firstPresetId) return current
          return {
            ...current,
            [cleanUrl]: current[cleanUrl] ?? firstPresetId,
            [result.normalizedUrl]:
              current[result.normalizedUrl] ??
              current[cleanUrl] ??
              firstPresetId,
          }
        })
        if (normalizeInput) setUrl(result.normalizedUrl)
        pushSessionLog(
          `${new Date().toISOString()} INFO analyze: ${siteLabels[result.siteKind]} ${result.presets.length} presets`
        )
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : String(reason))
      } finally {
        analyzingUrlSet.current.delete(cleanUrl)
        setAnalyzingUrls((current) => {
          const next = { ...current }
          delete next[cleanUrl]
          return next
        })
      }
    },
    [pushSessionLog]
  )

  useEffect(() => {
    if (inputUrls.length === 0) return

    const timeout = window.setTimeout(() => {
      inputUrls.forEach((inputUrl) => {
        void runAnalysis(inputUrl, false)
      })
    }, 300)

    return () => window.clearTimeout(timeout)
  }, [inputUrls, inputUrlsKey, runAnalysis])

  async function handleAnalyze(event?: FormEvent) {
    event?.preventDefault()
    if (inputUrls.length === 0) {
      setError("Paste one or more http(s) links.")
      return
    }
    setActiveTab("download")
    await Promise.all(
      inputUrls.map((inputUrl) => runAnalysis(inputUrl, inputUrls.length === 1))
    )
  }

  async function handlePaste() {
    try {
      const text = await readClipboardText()
      const cleanText = text.trim()
      if (!cleanText) {
        setError("Clipboard is empty.")
        return
      }
      const pastedUrls = extractUrls(cleanText)
      handleUrlChange(pastedUrls.length > 1 ? pastedUrls.join("\n") : cleanText)
      if (pastedUrls.length > 0) setActiveTab("download")
    } catch {
      setError(
        "Paste was blocked. Use Ctrl+V or check app clipboard permissions."
      )
    }
  }

  async function handleStart(
    sourceUrl: string,
    preset: Preset,
    outputProfile: OutputProfile = "original"
  ) {
    const requestKey = advancedKey(sourceUrl, preset.id)
    if (
      startingRequests.current.has(requestKey) ||
      jobs.some(
        (job) =>
          job.sourceUrl ===
            (analysesByUrl[sourceUrl]?.normalizedUrl ?? sourceUrl) &&
          job.presetId === preset.id &&
          !["completed", "failed", "canceled"].includes(job.status)
      )
    )
      return false
    const analysis = analysesByUrl[sourceUrl]
    if (!analysis) return false

    if (preset.id === "youtube-channel-catalogue") {
      const exportNameError = youtubeExportNameError(youtubeExportName)
      if (exportNameError) {
        setError(exportNameError)
        return false
      }
    }

    const key = advancedKey(analysis.normalizedUrl, preset.id)
    const auth = authForPreset(preset, settings.auth)
    if (preset.auth === "required" && !isAuthConfigured(auth)) {
      setError("Configure browser cookies or cookies.txt in Settings first.")
      setActiveTab("settings")
      return false
    }

    const advanced = advancedByPreset[key] ?? defaultAdvancedOptions
    const request: StartDownloadRequest = {
      outputProfile,
      url: analysis.normalizedUrl,
      channelUrls:
        preset.id === "youtube-channel-catalogue"
          ? inputUrls
              .map((inputUrl) => analysesByUrl[inputUrl])
              .filter((item): item is AnalyzeResult =>
                Boolean(
                  item?.presets.some(
                    (candidate) => candidate.id === "youtube-channel-catalogue"
                  )
                )
              )
              .map((item) => item.normalizedUrl)
          : undefined,
      youtubeCatalogueContent:
        preset.id === "youtube-channel-catalogue"
          ? youtubeCatalogueContent
          : undefined,
      presetId: preset.id,
      outputDir: settings.defaultOutputDir,
      exportName:
        preset.id === "youtube-channel-catalogue"
          ? youtubeExportName.trim()
          : undefined,
      filenameTemplate: "%(title).180B [%(id)s].%(ext)s",
      auth,
      advanced:
        outputProfile === "xrbazaar"
          ? { ...advanced, format: { kind: "best" } }
          : advanced,
    }
    pushSessionLog(
      `${new Date().toISOString()} INFO start: ${preset.id} ${analysis.normalizedUrl}`
    )
    startingRequests.current.add(requestKey)
    setStartingUrls((current) => [...current, sourceUrl])
    setStartingProfiles((current) => ({
      ...current,
      [sourceUrl]: outputProfile,
    }))
    try {
      const job = await startDownload(request)
      setJobs((current) => upsertJob(current, job))
      return true
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
      return false
    } finally {
      startingRequests.current.delete(requestKey)
      setStartingUrls((current) => current.filter((url) => url !== sourceUrl))
    }
  }

  async function handleCancel(jobId: string) {
    try {
      await cancelJob(jobId)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    }
  }

  function scrollToDownloadItem(sourceUrl: string) {
    document.getElementById(downloadItemDomId(sourceUrl))?.scrollIntoView({
      behavior: "smooth",
      block: "start",
    })
  }

  function openRunsForPreset(presetId: string) {
    setRunPresetFilter(presetId)
    setActiveTab("runs")
    window.setTimeout(() => {
      document.getElementById(runsPanelDomId)?.scrollIntoView({
        behavior: "smooth",
        block: "start",
      })
    }, 0)
  }

  async function handleStartAll() {
    const startable = inputUrls
      .map((sourceUrl) => {
        const analysis = analysesByUrl[sourceUrl]
        const presetId = selectedPresetByUrl[sourceUrl]
        const preset = analysis?.presets.find((item) => item.id === presetId)
        return analysis && preset ? { sourceUrl, preset } : null
      })
      .filter((item): item is { sourceUrl: string; preset: Preset } =>
        Boolean(item)
      )

    const channelExports = startable.filter(
      (item) => item.preset.id === "youtube-channel-catalogue"
    )
    if (channelExports.length > 0) {
      const first = channelExports[0]
      const started = await handleStart(first.sourceUrl, first.preset)
      if (!started) return
    }

    for (const item of startable.filter(
      (item) => item.preset.id !== "youtube-channel-catalogue"
    )) {
      const started = await handleStart(item.sourceUrl, item.preset)
      if (!started) break
    }
  }

  async function handleLoadFormats(sourceUrl: string, preset: Preset) {
    const analysis = analysesByUrl[sourceUrl]
    if (!analysis) return

    const key = advancedKey(analysis.normalizedUrl, preset.id)
    setLoadingFormatsKey(key)
    setError(null)
    try {
      const result = await analyzeFormats(
        analysis.normalizedUrl,
        authForPreset(preset, settings.auth)
      )
      setFormatsByPreset((current) => ({ ...current, [key]: result }))
      setAdvancedByPreset((current) => ({
        ...current,
        [key]: normalizeAdvancedForDuration(
          current[key] ?? defaultAdvancedOptions,
          result.duration ?? null
        ),
      }))
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setLoadingFormatsKey(null)
    }
  }

  function handleAdvancedChange(
    sourceUrl: string,
    preset: Preset,
    nextOptions: AdvancedDownloadOptions
  ) {
    const analysis = analysesByUrl[sourceUrl]
    if (!analysis) return
    const key = advancedKey(analysis.normalizedUrl, preset.id)
    setAdvancedByPreset((current) => ({ ...current, [key]: nextOptions }))
  }

  async function handleSaveSettings() {
    setSavingSettings(true)
    setSettingsMessage("")
    try {
      const saved = await updateSettings(draftSettings)
      setSettings(saved)
      setDraftSettings(saved)
      setSettingsMessage("Settings saved.")
      pushSessionLog(`${new Date().toISOString()} INFO settings: saved`)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setSavingSettings(false)
    }
  }

  async function handlePickFolder() {
    const selected = await selectDownloadDir()
    if (selected) {
      setDraftSettings((current) => ({
        ...current,
        defaultOutputDir: selected,
      }))
    }
  }

  async function handleCheckAppUpdate() {
    setAppUpdateState((current) => ({
      ...current,
      status: "checking",
      message: "Checking GitHub release metadata.",
    }))
    try {
      const update = await checkAppUpdate()
      if (!update) {
        setAppUpdateState({
          status: "current",
          update: null,
          checkedAt: new Date().toISOString(),
          message: "Installed version is current for the configured channel.",
        })
        return
      }

      setAppUpdateState({
        status: "available",
        update,
        checkedAt: new Date().toISOString(),
        message: `Version ${update.version} is available.`,
      })
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason)
      setAppUpdateState({
        status: "failed",
        update: null,
        checkedAt: new Date().toISOString(),
        message,
      })
      setError(message)
    }
  }

  async function handleInstallAppUpdate() {
    setAppUpdateState((current) => ({
      ...current,
      status: "installing",
      message: current.update
        ? `Installing version ${current.update.version}.`
        : "Checking and installing the latest available update.",
    }))
    try {
      await installAppUpdate()
      setAppUpdateState((current) => ({
        ...current,
        status: "restarting",
        checkedAt: new Date().toISOString(),
        message: "Update installed. Restarting the app.",
      }))
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason)
      setAppUpdateState((current) => ({
        ...current,
        status: "failed",
        checkedAt: new Date().toISOString(),
        message,
      }))
      setError(reason instanceof Error ? reason.message : String(reason))
    }
  }

  async function handleCheckTools() {
    setToolCheckState((current) => ({
      ...current,
      status: "checking",
      message:
        "Checking yt-dlp and ffmpeg on app data, bundled resources, and PATH.",
    }))
    try {
      const tools = await checkToolUpdates()
      setToolCheckState(toolCheckStateFromTools(tools))
    } catch (reason) {
      setToolCheckState({
        status: "failed",
        tools: [],
        checkedAt: new Date().toISOString(),
        message: reason instanceof Error ? reason.message : String(reason),
      })
    }
  }

  async function handleInstallTool(tool: ToolUpdate["tool"]) {
    setToolCheckState((current) => ({
      ...current,
      status: "installing",
      message: `Installing ${tool} into app data tools.`,
    }))
    setError(null)
    try {
      await installToolUpdate(tool)
      await handleCheckTools()
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason)
      setToolCheckState((current) => ({
        ...current,
        status: "failed",
        checkedAt: new Date().toISOString(),
        message,
      }))
      setError(message)
    }
  }

  function handleUrlChange(nextUrl: string) {
    setUrl(nextUrl)
    if (extractUrls(nextUrl).length === 0) setError(null)
  }

  function handlePresetChange(
    sourceUrl: string,
    presetId: string | null,
    groupedSourceUrls: string[] = [sourceUrl]
  ) {
    if (!presetId) return
    setSelectedPresetByUrl((current) => {
      const next = { ...current }
      groupedSourceUrls.forEach((url) => {
        next[url] = presetId
      })
      return next
    })
  }

  async function copyAllLogs() {
    await copyText(sessionLogs.join("\n") || "No logs yet.")
  }

  async function copyJobLogs(job: Job) {
    let logs = jobLogs[job.id] ?? []
    try {
      const detail = await getJob(job.id)
      logs = detail.logs
    } catch {
      // Browser preview only has in-memory event logs.
    }

    const lines = [
      `job=${job.id}`,
      `status=${job.status}`,
      `site=${job.site}`,
      `preset=${job.presetId}`,
      `url=${job.sourceUrl}`,
      `phase=${job.phase}`,
      job.errorMessage ? `error=${job.errorMessage}` : "",
      ...logs.map(
        (log) => `${log.createdAt} ${log.level.toUpperCase()} ${log.message}`
      ),
    ].filter(Boolean)

    await copyText(lines.join("\n"))
  }

  return (
    <main className="min-h-svh bg-background text-foreground">
      <a href="#workspace" className="skip-link">
        Skip to content
      </a>
      <header className="border-b bg-card">
        <div className="mx-auto flex h-16 max-w-6xl items-center justify-between gap-4 px-5 sm:px-8">
          <div className="flex items-center gap-3 font-semibold">
            <div className="flex size-9 items-center justify-center rounded-lg bg-primary text-primary-foreground">
              <ArrowDownToLine className="size-5" />
            </div>
            Downloader
          </div>
          <span className="text-xs text-muted-foreground">
            {appInfo ? `v${appInfo.version}` : "Desktop app"}
          </span>
        </div>
      </header>
      <Tabs
        value={activeTab}
        onValueChange={(value) => setActiveTab(value as AppTab)}
        className="gap-0"
      >
        <div className="border-b bg-card">
          <div className="mx-auto max-w-6xl px-5 sm:px-8">
            <TabsList
              variant="line"
              aria-label="Main navigation"
              className="app-navigation"
            >
              <TabsTrigger value="download">
                <Download />
                Download
              </TabsTrigger>
              <TabsTrigger value="runs">
                <History />
                Activity
                {runJobs.length > 0 ? (
                  <span className="nav-count">{runJobs.length}</span>
                ) : null}
              </TabsTrigger>
              <TabsTrigger value="downloaded">
                <FolderOpen />
                Files
                {downloadedAssets.length > 0 ? (
                  <span className="nav-count">{downloadedAssets.length}</span>
                ) : null}
              </TabsTrigger>
              <TabsTrigger value="settings">
                <SettingsIcon />
                Settings
              </TabsTrigger>
            </TabsList>
          </div>
        </div>
        <div
          id="workspace"
          tabIndex={-1}
          className="mx-auto w-full max-w-6xl px-5 py-6 outline-none sm:px-8 sm:py-8"
        >
          <div className="mb-5">
            <h1 className="text-xl font-semibold tracking-tight">
              {
                {
                  download: "New download",
                  runs: "Download activity",
                  downloaded: "Your files",
                  settings: "Settings",
                }[activeTab]
              }
            </h1>
            <p className="mt-2 text-sm text-muted-foreground">
              {
                {
                  download:
                    "Save videos, audio, or channel catalogues from a link.",
                  runs: "Follow download progress and review past attempts.",
                  downloaded:
                    "Finished files stay here, including files from canceled downloads.",
                  settings:
                    "Choose where downloads go and how the app connects to sites.",
                }[activeTab]
              }
            </p>
          </div>
          {error ? (
            <div
              role="alert"
              className="mb-5 flex items-start gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive"
            >
              <AlertCircle className="mt-0.5 size-4 shrink-0" />
              <span className="min-w-0 flex-1 break-words">{error}</span>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Dismiss error"
                onClick={() => setError(null)}
              >
                <X />
              </Button>
            </div>
          ) : null}
          <TabsContent value="download" className="space-y-6">
            <form
              onSubmit={handleAnalyze}
              className="overflow-hidden rounded-xl border bg-card"
            >
              <div className="px-5 pt-5 sm:px-6">
                <div className="mb-3 flex items-center justify-between gap-3">
                  <Label
                    htmlFor="download-links"
                    className="text-sm font-medium"
                  >
                    Video or page links
                  </Label>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={isAnalyzing}
                    onClick={handlePaste}
                  >
                    <ClipboardPaste />
                    Paste links
                  </Button>
                </div>
                <textarea
                  id="download-links"
                  aria-label="Video or page links"
                  value={url}
                  onChange={(event) => handleUrlChange(event.target.value)}
                  onKeyDown={(event) => {
                    if (
                      (event.metaKey || event.ctrlKey) &&
                      event.key === "Enter"
                    ) {
                      event.preventDefault()
                      void handleAnalyze()
                    }
                  }}
                  aria-describedby="download-links-help"
                  spellCheck={false}
                  autoCapitalize="none"
                  placeholder="https://www.youtube.com/watch?v=…"
                  rows={2}
                  className="min-h-20 w-full resize-y rounded-lg border bg-background p-3 text-sm leading-6 placeholder:text-muted-foreground"
                />
                <div className="flex flex-wrap items-center justify-between gap-3 py-3">
                  <p
                    id="download-links-help"
                    className="text-xs text-muted-foreground"
                  >
                    Add one link per line. Links are checked automatically.
                  </p>
                  <div className="flex items-center gap-2">
                    {url ? (
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        onClick={() => handleUrlChange("")}
                      >
                        Clear
                      </Button>
                    ) : null}
                    <Button
                      type="submit"
                      size="sm"
                      variant="secondary"
                      disabled={isAnalyzing || !url.trim()}
                    >
                      {isAnalyzing ? (
                        <Loader2 className="animate-spin" />
                      ) : (
                        <Search />
                      )}
                      {isAnalyzing ? "Checking links…" : "Check links"}
                    </Button>
                  </div>
                </div>
              </div>
              <div className="flex flex-wrap items-center justify-between gap-3 border-t bg-muted/40 px-5 py-2 sm:px-6">
                <div className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
                  <FolderOpen className="size-4 shrink-0" />
                  <span
                    className="truncate"
                    title={settings.defaultOutputDir ?? "Downloads"}
                  >
                    Save to{" "}
                    <span className="font-medium text-foreground">
                      {settings.defaultOutputDir ?? "Downloads"}
                    </span>
                  </span>
                </div>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={() => setActiveTab("settings")}
                >
                  Change folder
                </Button>
              </div>
            </form>
            {inputUrls.length > 0 ? (
              <div className="space-y-3">
                <div className="flex flex-wrap items-center justify-between gap-3 text-sm">
                  <div className="text-muted-foreground">
                    {resultUrls.length} result
                    {resultUrls.length === 1 ? "" : "s"}
                    {inputUrls.length !== resultUrls.length
                      ? ` · ${inputUrls.length} links`
                      : ""}
                  </div>
                  {resultUrls.length > 1 ? (
                    <Button
                      disabled={
                        inputUrls.some(
                          (inputUrl) => !analysesByUrl[inputUrl]
                        ) || isAnalyzing
                      }
                      onClick={handleStartAll}
                    >
                      <Download />
                      Download all
                    </Button>
                  ) : null}
                </div>
                <div
                  className={cn(
                    "grid gap-4",
                    resultUrls.length > 1 &&
                      "lg:grid-cols-[168px_minmax(0,1fr)]"
                  )}
                >
                  {resultUrls.length > 1 ? (
                    <DownloadItemMenu
                      items={downloadNavItems}
                      onSelect={scrollToDownloadItem}
                    />
                  ) : null}

                  <div className="space-y-3">
                    {resultUrls.map((inputUrl) => {
                      const analysis = analysesByUrl[inputUrl]
                      const isChannelCatalogueGroup =
                        inputUrl === channelCatalogueLeadUrl
                      const presetId =
                        selectedPresetByUrl[inputUrl] ??
                        analysis?.presets[0]?.id
                      const preset =
                        analysis?.presets.find(
                          (item) => item.id === presetId
                        ) ??
                        analysis?.presets[0] ??
                        null
                      const key =
                        analysis && preset
                          ? advancedKey(analysis.normalizedUrl, preset.id)
                          : inputUrl

                      return (
                        <div
                          key={inputUrl}
                          id={downloadItemDomId(inputUrl)}
                          className="scroll-mt-24"
                        >
                          <DownloadLinkCard
                            sourceUrl={inputUrl}
                            starting={startingUrls.includes(inputUrl)}
                            startingProfile={startingProfiles[inputUrl]}
                            assetsByJob={assetsByJob}
                            displayLabel={
                              isChannelCatalogueGroup &&
                              channelCatalogueUrls.length > 1
                                ? `${channelCatalogueUrls.length} YouTube channels`
                                : undefined
                            }
                            analysis={analysis ?? null}
                            analyzing={
                              Boolean(analyzingUrls[inputUrl]) ||
                              (preset?.id === "youtube-channel-catalogue" &&
                                isAnalyzing)
                            }
                            preset={preset}
                            selectedPresetId={presetId ?? null}
                            jobs={jobs}
                            outputDir={settings.defaultOutputDir}
                            exportName={youtubeExportName}
                            catalogueContent={youtubeCatalogueContent}
                            auth={
                              preset
                                ? authForPreset(preset, settings.auth)
                                : settings.auth
                            }
                            advancedOptions={
                              advancedByPreset[key] ?? defaultAdvancedOptions
                            }
                            formatInfo={formatsByPreset[key] ?? null}
                            loadingFormats={loadingFormatsKey === key}
                            onPresetChange={(nextPresetId) =>
                              handlePresetChange(
                                inputUrl,
                                nextPresetId,
                                isChannelCatalogueGroup
                                  ? channelCatalogueUrls
                                  : undefined
                              )
                            }
                            onExportNameChange={setYoutubeExportName}
                            onCatalogueContentChange={
                              setYoutubeCatalogueContent
                            }
                            onStart={() =>
                              preset ? handleStart(inputUrl, preset) : undefined
                            }
                            onStartXrbazaar={() =>
                              preset
                                ? handleStart(inputUrl, preset, "xrbazaar")
                                : undefined
                            }
                            onCancel={handleCancel}
                            onCopyLogs={copyJobLogs}
                            onViewRun={openRunsForPreset}
                            onAdvancedChange={(nextOptions) =>
                              preset
                                ? handleAdvancedChange(
                                    inputUrl,
                                    preset,
                                    nextOptions
                                  )
                                : undefined
                            }
                            onLoadFormats={() =>
                              preset
                                ? handleLoadFormats(inputUrl, preset)
                                : undefined
                            }
                          />
                        </div>
                      )
                    })}
                  </div>
                </div>
              </div>
            ) : null}
          </TabsContent>

          <TabsContent value="runs" className="mt-0" id={runsPanelDomId}>
            {runJobs.length > 0 ? (
              <div className="space-y-3">
                <div className="flex flex-wrap items-end justify-between gap-3 rounded-lg border bg-card px-3 py-3">
                  <div className="min-w-0">
                    <div className="text-sm font-medium">Download history</div>
                    <div className="mt-1 text-xs text-muted-foreground">
                      Newest first
                      {runPresetFilter !== allRunPresetsValue
                        ? ` - ${visibleRunJobs.length} matching`
                        : ""}
                    </div>
                  </div>
                  <div className="w-full space-y-1 sm:w-72">
                    <Label className="text-xs text-muted-foreground">
                      Preset
                    </Label>
                    <Select
                      value={runPresetFilter}
                      onValueChange={(value) =>
                        setRunPresetFilter(value ?? allRunPresetsValue)
                      }
                      items={[
                        {
                          value: allRunPresetsValue,
                          label: "All presets",
                        },
                        ...runPresetOptions.map((option) => ({
                          value: option.id,
                          label: `${option.label} (${option.count})`,
                        })),
                      ]}
                    >
                      <SelectTrigger
                        aria-label="Filter by download type"
                        className="h-9 w-full bg-background"
                      >
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent align="start">
                        <SelectItem value={allRunPresetsValue}>
                          All presets
                        </SelectItem>
                        {runPresetOptions.map((option) => (
                          <SelectItem key={option.id} value={option.id}>
                            {option.label} ({option.count})
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                </div>

                {visibleRunJobs.length > 0 ? (
                  visibleRunJobs.map((job) => (
                    <div
                      key={job.id}
                      id={jobRunDomId(job.id)}
                      className="scroll-mt-24"
                    >
                      <JobRunItem
                        job={job}
                        onCancel={() => handleCancel(job.id)}
                        assets={
                          assetsByJob[job.id]?.map((path) => ({
                            path,
                            job,
                          })) ?? []
                        }
                        onCopyLogs={() => copyJobLogs(job)}
                      />
                    </div>
                  ))
                ) : (
                  <EmptyPanel message="No runs match this preset." />
                )}
              </div>
            ) : (
              <EmptyPanel
                icon={<History />}
                title="No downloads started"
                message="Start a download to see its progress, status, and any errors here."
                action={
                  <Button onClick={() => setActiveTab("download")}>
                    <Download />
                    New download
                  </Button>
                }
              />
            )}
          </TabsContent>

          <TabsContent value="downloaded" className="mt-0">
            {downloadedAssets.length > 0 ? (
              <div className="grid gap-3 lg:grid-cols-2">
                {downloadedAssets.map((asset) => (
                  <DownloadedAssetItem
                    key={`${asset.job.id}:${asset.path}`}
                    asset={asset}
                    onCopyLogs={() => copyJobLogs(asset.job)}
                  />
                ))}
              </div>
            ) : (
              <EmptyPanel
                icon={<FolderOpen />}
                title="Your downloads will appear here"
                message="Completed files are ready to preview, open, or reveal in their folder."
                action={
                  <Button onClick={() => setActiveTab("download")}>
                    <Download />
                    New download
                  </Button>
                }
              />
            )}
          </TabsContent>

          <TabsContent value="settings" className="mt-0">
            <SettingsPage
              appInfo={appInfo}
              appUpdateState={appUpdateState}
              toolCheckState={toolCheckState}
              settings={draftSettings}
              savedSettings={settings}
              sessionLogs={sessionLogs}
              onChange={setDraftSettings}
              onPickFolder={handlePickFolder}
              onSave={handleSaveSettings}
              saving={savingSettings}
              saveMessage={settingsMessage}
              onCheckAppUpdate={handleCheckAppUpdate}
              onInstallAppUpdate={handleInstallAppUpdate}
              onCheckTools={handleCheckTools}
              onInstallTool={handleInstallTool}
              onCopyLogs={copyAllLogs}
            />
          </TabsContent>
        </div>
      </Tabs>
    </main>
  )
}

type SettingsPageProps = {
  appInfo: AppInfo | null
  appUpdateState: AppUpdateState
  toolCheckState: ToolCheckState
  settings: DownloaderSettings
  savedSettings: DownloaderSettings
  sessionLogs: string[]
  onChange: (settings: DownloaderSettings) => void
  onPickFolder: () => void
  onSave: () => void
  saving: boolean
  saveMessage: string
  onCheckAppUpdate: () => void
  onInstallAppUpdate: () => void
  onCheckTools: () => void
  onInstallTool: (tool: ToolUpdate["tool"]) => void
  onCopyLogs: () => void
}

function SettingsPage({
  appInfo,
  appUpdateState,
  toolCheckState,
  settings,
  savedSettings,
  sessionLogs,
  onChange,
  onPickFolder,
  onSave,
  saving,
  saveMessage,
  onCheckAppUpdate,
  onInstallAppUpdate,
  onCheckTools,
  onInstallTool,
  onCopyLogs,
}: SettingsPageProps) {
  const { theme, setTheme } = useTheme()
  const playback = usePlayback()
  const [platform, setPlatform] = useState<ToolPlatform | null>(null)
  useEffect(() => {
    getToolPlatform()
      .then(setPlatform)
      .catch(() => undefined)
  }, [])
  const [youtubeApiKeys, setYoutubeApiKeys] = useState<YoutubeApiKeyInfo[]>([])
  const [newYoutubeApiKey, setNewYoutubeApiKey] = useState("")
  const [youtubeApiKeyBusy, setYoutubeApiKeyBusy] = useState(false)
  const [youtubeApiKeyError, setYoutubeApiKeyError] = useState<string | null>(
    null
  )
  const authMode = settings.auth.kind
  const selectedBrowsers =
    settings.auth.kind === "browser" ? browserSources(settings.auth) : []
  const cookieFile =
    settings.auth.kind === "cookie_file" ? settings.auth.path : ""
  const hasUnsavedSettings =
    JSON.stringify(settings) !== JSON.stringify(savedSettings)
  const issueTools = toolCheckState.tools.filter(
    (tool) => tool.status !== "installed"
  )

  useEffect(() => {
    listYoutubeApiKeys()
      .then(setYoutubeApiKeys)
      .catch((reason) =>
        setYoutubeApiKeyError(
          reason instanceof Error ? reason.message : String(reason)
        )
      )
  }, [])

  async function saveYoutubeApiKey() {
    if (!newYoutubeApiKey.trim()) return
    setYoutubeApiKeyBusy(true)
    setYoutubeApiKeyError(null)
    try {
      setYoutubeApiKeys(await addYoutubeApiKey(newYoutubeApiKey))
      setNewYoutubeApiKey("")
    } catch (reason) {
      setYoutubeApiKeyError(
        reason instanceof Error ? reason.message : String(reason)
      )
    } finally {
      setYoutubeApiKeyBusy(false)
    }
  }

  async function deleteYoutubeApiKey(id: string) {
    setYoutubeApiKeyBusy(true)
    setYoutubeApiKeyError(null)
    try {
      setYoutubeApiKeys(await removeYoutubeApiKey(id))
    } catch (reason) {
      setYoutubeApiKeyError(
        reason instanceof Error ? reason.message : String(reason)
      )
    } finally {
      setYoutubeApiKeyBusy(false)
    }
  }

  function setAuth(auth: AuthSource) {
    onChange({ ...settings, auth })
  }

  function setAuthMode(nextMode: string | null) {
    if (nextMode === "browser") {
      setAuth({
        kind: "browser",
        browser: selectedBrowsers[0]?.browser ?? "firefox",
        browsers:
          selectedBrowsers.length > 0
            ? selectedBrowsers
            : [{ browser: "firefox" }],
      })
    } else if (nextMode === "cookie_file") {
      setAuth({ kind: "cookie_file", path: cookieFile })
    } else {
      setAuth({ kind: "none" })
    }
  }

  function setBrowserEnabled(browser: BrowserKind, enabled: boolean) {
    const nextBrowsers = enabled
      ? [...selectedBrowsers, { browser }]
      : selectedBrowsers.filter((source) => source.browser !== browser)
    const cleanBrowsers = nextBrowsers.length > 0 ? nextBrowsers : []
    setAuth({
      kind: "browser",
      browser: cleanBrowsers[0]?.browser ?? "firefox",
      browsers: cleanBrowsers,
    })
  }

  return (
    <div className="grid gap-4">
      <section className="rounded-xl border bg-card p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <div className="flex items-center gap-2 text-sm font-medium">
              <SettingsIcon className="size-4" />
              Downloader settings
            </div>
            <div role="status" className="mt-1 text-xs text-muted-foreground">
              {hasUnsavedSettings
                ? "You have unsaved changes."
                : saveMessage || "Default folder and site access."}
            </div>
          </div>
          <div className="flex gap-2">
            {hasUnsavedSettings ? (
              <Button
                variant="ghost"
                disabled={saving}
                onClick={() => onChange(savedSettings)}
              >
                Discard changes
              </Button>
            ) : null}
            <Button
              type="button"
              onClick={onSave}
              disabled={!hasUnsavedSettings || saving}
            >
              {saving ? <Loader2 className="animate-spin" /> : <Check />}
              {saving ? "Saving…" : "Save settings"}
            </Button>
          </div>
        </div>

        <fieldset disabled={saving} className="min-w-0">
          <div className="mt-5 grid items-center gap-3 sm:grid-cols-[minmax(0,1fr)_auto]">
            <div className="min-w-0 rounded-md border bg-background px-3 py-2 text-sm">
              <div className="text-xs text-muted-foreground">
                Download folder
              </div>
              <div className="mt-0.5 truncate">
                {settings.defaultOutputDir ?? "Downloads"}
              </div>
            </div>
            <Button
              type="button"
              variant="outline"
              className="gap-2"
              onClick={onPickFolder}
            >
              <FolderOpen className="size-4" />
              Choose folder
            </Button>
          </div>

          <div className="mt-3 grid gap-3 sm:grid-cols-3">
            <div className="space-y-1">
              <Label
                htmlFor="site-access"
                className="text-xs text-muted-foreground"
              >
                Site access
              </Label>
              <Select
                value={authMode}
                onValueChange={setAuthMode}
                items={[
                  { value: "browser", label: "Browser cookies" },
                  { value: "cookie_file", label: "cookies.txt" },
                  { value: "none", label: "None" },
                ]}
              >
                <SelectTrigger
                  id="site-access"
                  aria-label="Site access"
                  className="h-9 w-full bg-background"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent align="start">
                  <SelectItem value="browser">Browser cookies</SelectItem>
                  <SelectItem value="cookie_file">cookies.txt</SelectItem>
                  <SelectItem value="none">None</SelectItem>
                </SelectContent>
              </Select>
            </div>

            {authMode === "browser" ? (
              <div className="space-y-2 text-xs text-muted-foreground sm:col-span-2">
                Use cookies from these browsers
                <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                  {browsers.map((browserName) => {
                    const checked = selectedBrowsers.some(
                      (source) => source.browser === browserName
                    )
                    return (
                      <label
                        key={browserName}
                        className={cn(
                          "flex h-8 items-center gap-2 rounded px-2 text-xs text-foreground capitalize hover:bg-muted",
                          authMode !== "browser" && "opacity-50"
                        )}
                      >
                        <Checkbox
                          aria-label={`${browserName} cookies`}
                          checked={checked}
                          disabled={authMode !== "browser"}
                          onCheckedChange={(nextChecked) =>
                            setBrowserEnabled(browserName, Boolean(nextChecked))
                          }
                        />
                        {browserName}
                      </label>
                    )
                  })}
                </div>
              </div>
            ) : null}

            {authMode === "cookie_file" ? (
              <label className="space-y-1 text-xs text-muted-foreground sm:col-span-2">
                Cookie file
                <input
                  value={cookieFile}
                  disabled={authMode !== "cookie_file"}
                  onChange={(event) =>
                    setAuth({ kind: "cookie_file", path: event.target.value })
                  }
                  placeholder="/path/to/cookies.txt"
                  className="h-9 w-full rounded-md border bg-background px-2 text-sm text-foreground outline-none disabled:opacity-50"
                />
              </label>
            ) : null}
          </div>
        </fieldset>
      </section>

      <section className="rounded-xl border bg-card p-5">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h2 className="flex items-center gap-2 text-sm font-medium">
              <Monitor className="size-4" />
              Appearance
            </h2>
            <p className="mt-1 text-xs text-muted-foreground">
              Use a light, dark, or system theme. Changes apply immediately.
            </p>
          </div>
          <Select
            value={theme}
            onValueChange={(value) => {
              if (value === "light" || value === "dark" || value === "system")
                setTheme(value)
            }}
            items={[
              { value: "system", label: "System" },
              { value: "light", label: "Light" },
              { value: "dark", label: "Dark" },
            ]}
          >
            <SelectTrigger aria-label="Appearance" className="w-36">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="system">System</SelectItem>
              <SelectItem value="light">Light</SelectItem>
              <SelectItem value="dark">Dark</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </section>

      <section className="rounded-xl border bg-card p-5">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h2 className="text-sm font-medium">Preview audio</h2>
            <p className="mt-1 text-xs text-muted-foreground">
              Shared by all videos and remembered next time. Starts at 10%.
            </p>
          </div>
          <div className="flex w-full items-center gap-3 sm:w-72">
            <input
              type="range"
              aria-label="Preview volume"
              min="0"
              max="100"
              step="1"
              value={Math.round(playback.volume * 100)}
              onChange={(event) =>
                setPlayback(Number(event.target.value) / 100, playback.muted)
              }
              className="min-w-0 flex-1 accent-primary"
            />
            <output className="w-10 text-right text-xs tabular-nums">
              {Math.round(playback.volume * 100)}%
            </output>
            <Button
              variant="outline"
              size="sm"
              aria-pressed={playback.muted}
              onClick={() => setPlayback(playback.volume, !playback.muted)}
            >
              {playback.muted ? "Unmute previews" : "Mute previews"}
            </Button>
          </div>
        </div>
      </section>

      <section className="rounded-xl border bg-card p-5">
        <div className="flex items-start gap-2">
          <KeyRound className="mt-0.5 size-4" />
          <div>
            <div className="text-sm font-medium">YouTube Data API keys</div>
            <div className="mt-1 text-xs text-muted-foreground">
              Add a key for faster YouTube channel exports. Keys are stored
              securely on your device.
            </div>
          </div>
        </div>

        <div className="mt-4 grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto]">
          <input
            type="password"
            value={newYoutubeApiKey}
            disabled={youtubeApiKeyBusy}
            autoComplete="off"
            spellCheck={false}
            placeholder="YouTube Data API key"
            aria-label="YouTube Data API key"
            className="h-9 min-w-0 rounded-md border bg-background px-3 text-sm text-foreground outline-none disabled:opacity-50"
            onChange={(event) => setNewYoutubeApiKey(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault()
                void saveYoutubeApiKey()
              }
            }}
          />
          <Button
            type="button"
            disabled={youtubeApiKeyBusy || !newYoutubeApiKey.trim()}
            onClick={saveYoutubeApiKey}
          >
            {youtubeApiKeyBusy ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              "Add key"
            )}
          </Button>
        </div>

        {youtubeApiKeyError ? (
          <div className="mt-2 text-xs text-destructive">
            {youtubeApiKeyError}
          </div>
        ) : null}

        <div className="mt-3 grid gap-2">
          {youtubeApiKeys.length > 0 ? (
            youtubeApiKeys.map((apiKey) => (
              <div
                key={apiKey.id}
                className="flex items-center justify-between gap-3 rounded-md border bg-background px-3 py-2"
              >
                <div className="min-w-0">
                  <div className="truncate text-sm">{apiKey.label}</div>
                  <div className="text-xs text-muted-foreground">
                    Stored securely · value hidden
                  </div>
                </div>
                <Button
                  type="button"
                  size="xs"
                  variant="outline"
                  disabled={youtubeApiKeyBusy}
                  aria-label={`Remove ${apiKey.label}`}
                  onClick={() => deleteYoutubeApiKey(apiKey.id)}
                >
                  <Trash2 className="size-3" />
                  Remove
                </Button>
              </div>
            ))
          ) : (
            <div className="rounded-md border bg-background px-3 py-3 text-sm text-muted-foreground">
              No API keys saved. Channel exports will use the slower yt-dlp
              metadata fallback.
            </div>
          )}
        </div>
      </section>

      <div className="grid items-start gap-4 lg:grid-cols-2">
        <section className="rounded-xl border bg-card p-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-sm font-medium">
                <RefreshCw className="size-4" />
                App update
              </div>
              <div className="mt-1 text-xs text-muted-foreground">
                {appInfo?.name ?? "Downloader"}{" "}
                {appInfo ? `v${appInfo.version}` : "version loading"}
              </div>
            </div>
            <StatusBadge status={appUpdateState.status}>
              {appUpdateStatusLabel(appUpdateState)}
            </StatusBadge>
          </div>

          <div className="mt-4 grid gap-3">
            <InfoRow
              label="Current version"
              value={appInfo ? `v${appInfo.version}` : "Loading"}
            />
            <InfoRow
              label="Update feed"
              value={appInfo?.updaterEndpoint ?? "Loading"}
              mono
            />
            <InfoRow
              label="Last check"
              value={formatCheckedAt(appUpdateState.checkedAt)}
            />
            <InfoRow label="State" value={appUpdateState.message} />
            {appUpdateState.update ? (
              <>
                <InfoRow
                  label="Available version"
                  value={`v${appUpdateState.update.version}`}
                />
                <InfoRow
                  label="Release notes"
                  value={appUpdateState.update.notes || "No release notes."}
                />
              </>
            ) : null}
          </div>

          <div className="mt-4 flex flex-wrap gap-2">
            <Button
              type="button"
              variant="outline"
              className="gap-1.5"
              disabled={appUpdateState.status === "checking"}
              onClick={onCheckAppUpdate}
            >
              {appUpdateState.status === "checking" ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <RefreshCw className="size-3.5" />
              )}
              Check app
            </Button>
            <Button
              type="button"
              className="gap-1.5"
              disabled={
                appUpdateState.status !== "available" &&
                appUpdateState.status !== "failed"
              }
              onClick={onInstallAppUpdate}
            >
              <Download className="size-3.5" />
              Install app update
            </Button>
          </div>
        </section>

        <section className="rounded-xl border bg-card p-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-sm font-medium">
                <Wrench className="size-4" />
                Tools
              </div>
              <div className="mt-1 text-xs text-muted-foreground">
                {toolCheckState.tools.length > 0
                  ? `${toolCheckState.tools.length - issueTools.length} ready, ${issueTools.length} issue${issueTools.length === 1 ? "" : "s"}`
                  : "Status not loaded"}
              </div>
            </div>
            <StatusBadge status={toolCheckState.status}>
              {toolStatusLabel(toolCheckState)}
            </StatusBadge>
          </div>

          <div className="mt-4 grid gap-2">
            {toolCheckState.tools.length > 0 ? (
              toolCheckState.tools.map((tool) => (
                <ToolStatusItem
                  key={tool.tool}
                  tool={tool}
                  installing={toolCheckState.status === "installing"}
                  platform={platform}
                  onInstall={() => onInstallTool(tool.tool)}
                />
              ))
            ) : (
              <div className="rounded-md border bg-background px-3 py-3 text-sm text-muted-foreground">
                {toolCheckState.message}
              </div>
            )}
          </div>

          <div className="mt-4 grid gap-2 text-xs text-muted-foreground">
            <div>yt-dlp downloads media. ffmpeg combines video and audio.</div>
            <div>Missing tools can be installed here.</div>
            <div>Last check: {formatCheckedAt(toolCheckState.checkedAt)}</div>
          </div>

          <div className="mt-4 flex">
            <Button
              type="button"
              variant="outline"
              className="gap-1.5"
              disabled={
                toolCheckState.status === "checking" ||
                toolCheckState.status === "installing"
              }
              onClick={onCheckTools}
            >
              {["checking", "installing"].includes(toolCheckState.status) ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <RefreshCw className="size-3.5" />
              )}
              Refresh tools
            </Button>
          </div>
        </section>
      </div>

      <details className="rounded-xl border bg-card p-5">
        <summary className="cursor-pointer text-sm font-medium">
          Session log{" "}
          <span className="ml-2 font-normal text-muted-foreground">
            {sessionLogs.length} lines
          </span>
        </summary>
        <div className="mt-4 flex justify-end">
          <Button variant="outline" onClick={onCopyLogs}>
            <Clipboard />
            Copy logs
          </Button>
        </div>
        <div className="mt-3 max-h-56 overflow-auto rounded-md border bg-background p-3 font-mono text-xs">
          {sessionLogs.length > 0 ? (
            sessionLogs.slice(-80).map((line, index) => (
              <div key={`${index}:${line}`} className="whitespace-pre-wrap">
                {line}
              </div>
            ))
          ) : (
            <div className="text-muted-foreground">No logs yet.</div>
          )}
        </div>
      </details>
    </div>
  )
}

function InfoRow({
  label,
  value,
  mono = false,
}: {
  label: string
  value: string
  mono?: boolean
}) {
  return (
    <div className="grid gap-1 rounded-md border bg-background px-3 py-2 text-sm sm:grid-cols-[132px_minmax(0,1fr)]">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div
        className={cn("min-w-0 break-words", mono && "font-mono text-xs")}
        title={value}
      >
        {value}
      </div>
    </div>
  )
}

function ToolStatusItem({
  tool,
  installing,
  onInstall,
  platform,
}: {
  tool: ToolUpdate
  platform: ToolPlatform | null
  installing: boolean
  onInstall: () => void
}) {
  const installed = tool.status === "installed"
  const asset =
    tool.tool === "yt-dlp" ? platform?.ytDlpAsset : platform?.ffmpegAsset
  const installable = !installed && Boolean(asset)

  return (
    <div className="rounded-md border bg-background px-3 py-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <div className="font-mono text-sm font-medium">{tool.tool}</div>
            <StatusBadge status={tool.status}>
              {toolStatusItemLabel(tool.status)}
            </StatusBadge>
          </div>
          <div className="mt-2 grid gap-1 text-xs text-muted-foreground">
            <div className="break-words">
              Version: {tool.currentVersion ?? "Unavailable"}
            </div>
            <div className="break-words">Path: {tool.path ?? "Not found"}</div>
            <div className="break-words">{tool.message}</div>
            {platform ? (
              <div className="break-words">
                System: {platform.os} / {platform.arch}
                <br />
                {asset
                  ? `Installer: ${asset}`
                  : "Automatic installation is not available for this system."}
              </div>
            ) : null}
          </div>
        </div>
        {installable ? (
          <Button
            type="button"
            size="xs"
            variant="outline"
            className="gap-1.5"
            disabled={installing}
            onClick={onInstall}
          >
            {installing ? (
              <Loader2 className="size-3 animate-spin" />
            ) : (
              <Download className="size-3" />
            )}
            {installing ? "Installing" : "Install"}
          </Button>
        ) : null}
      </div>
    </div>
  )
}

function StatusBadge({
  status,
  children,
}: {
  status: string
  children: ReactNode
}) {
  return (
    <span
      className={cn(
        "inline-flex h-6 shrink-0 items-center rounded-md border px-2 text-xs font-medium",
        statusBadgeClass(status)
      )}
    >
      {children}
    </span>
  )
}

function DownloadItemMenu({
  items,
  onSelect,
}: {
  items: DownloadNavigatorItem[]
  onSelect: (sourceUrl: string) => void
}) {
  return (
    <nav
      aria-label="Download items"
      className="min-w-0 lg:sticky lg:top-4 lg:self-start"
    >
      <div className="rounded-lg border bg-card p-2">
        <div className="mb-2 flex items-center gap-2 px-2 text-xs font-medium text-muted-foreground">
          <List className="size-3.5" />
          Items
        </div>
        <div className="flex gap-2 overflow-x-auto pb-1 lg:flex-col lg:overflow-visible lg:pb-0">
          {items.map((item) => (
            <Button
              key={item.id}
              type="button"
              variant="ghost"
              className="h-auto min-w-40 justify-start gap-2 px-2 py-2 lg:w-full lg:min-w-0"
              onClick={() => onSelect(item.sourceUrl)}
            >
              <span className="flex size-6 shrink-0 items-center justify-center rounded-md border bg-background text-xs font-medium">
                {item.index}
              </span>
              <span className="min-w-0 text-left">
                <span className="block truncate text-xs font-medium text-foreground">
                  {item.siteLabel}
                </span>
                <span className="block truncate text-[11px] text-muted-foreground">
                  {item.title}
                </span>
              </span>
              <span className="sr-only">{item.status}</span>
            </Button>
          ))}
        </div>
      </div>
    </nav>
  )
}

type DownloadLinkCardProps = {
  sourceUrl: string
  starting: boolean
  startingProfile?: OutputProfile
  assetsByJob: Record<string, string[]>
  displayLabel?: string
  analysis: AnalyzeResult | null
  analyzing: boolean
  preset: Preset | null
  selectedPresetId: string | null
  jobs: Job[]
  outputDir?: string | null
  exportName: string
  catalogueContent: YoutubeCatalogueContent
  auth: AuthSource
  advancedOptions: AdvancedDownloadOptions
  formatInfo: FormatAnalysis | null
  loadingFormats: boolean
  onPresetChange: (presetId: string | null) => void
  onExportNameChange: (name: string) => void
  onCatalogueContentChange: (content: YoutubeCatalogueContent) => void
  onStart: () => void
  onStartXrbazaar: () => void
  onCancel: (jobId: string) => void
  onCopyLogs: (job: Job) => void
  onViewRun: (presetId: string) => void
  onAdvancedChange: (options: AdvancedDownloadOptions) => void
  onLoadFormats: () => void
}

function DownloadLinkCard({
  sourceUrl,
  starting,
  startingProfile,
  assetsByJob,
  displayLabel,
  analysis,
  analyzing,
  preset,
  selectedPresetId,
  jobs,
  outputDir,
  exportName,
  catalogueContent,
  auth,
  advancedOptions,
  formatInfo,
  loadingFormats,
  onPresetChange,
  onExportNameChange,
  onCatalogueContentChange,
  onStart,
  onStartXrbazaar,
  onCancel,
  onCopyLogs,
  onViewRun,
  onAdvancedChange,
  onLoadFormats,
}: DownloadLinkCardProps) {
  const job =
    analysis && preset
      ? (jobs.find(
          (item) =>
            item.presetId === preset.id &&
            (preset.id === "youtube-channel-catalogue" ||
              item.sourceUrl === analysis.normalizedUrl)
        ) ?? null)
      : null
  const readyAssets = jobs
    .filter(
      (item) =>
        analysis &&
        preset &&
        item.presetId === preset.id &&
        (preset.id === "youtube-channel-catalogue" ||
          item.sourceUrl === analysis.normalizedUrl)
    )
    .flatMap((item) =>
      (assetsByJob[item.id] ?? []).map((path) => ({ path, job: item }))
    )
    .filter(
      (asset, index, all) =>
        all.findIndex((other) => other.path === asset.path) === index
    )
  const running =
    job && !["completed", "failed", "canceled"].includes(job.status)
  const presetAuth = preset?.auth ?? "none"
  const canUseAuth = isAuthConfigured(auth)
  const exportNameError =
    preset?.pipeline === "youtube_channel_export"
      ? youtubeExportNameError(exportName)
      : null

  return (
    <div className="download-card">
      <div className="download-card-heading">
        <div className="min-w-0 space-y-2">
          <div className="flex min-w-0 flex-wrap items-center gap-2 text-sm">
            {analysis ? (
              <span className="rounded-md border bg-muted px-2 py-1 font-medium">
                {siteLabels[analysis.siteKind]}
              </span>
            ) : null}
            <span className="rounded border bg-muted px-1.5 py-0.5 text-[11px] tracking-normal text-muted-foreground uppercase">
              {preset?.outputKind ?? "video"}
            </span>
            <span className="rounded border bg-background px-1.5 py-0.5 text-[11px] text-muted-foreground">
              {authLabels[presetAuth]}
            </span>
            {analyzing ? (
              <span className="flex items-center gap-1 text-xs text-muted-foreground">
                <Loader2 className="size-3 animate-spin" />
                Inspecting
              </span>
            ) : null}
          </div>
          <div
            className="line-clamp-2 text-sm font-medium break-all"
            title={sourceUrl}
          >
            {displayLabel ?? sourceUrl}
          </div>
          {analysis?.warnings.length ? (
            <div className="flex items-center gap-1.5 text-xs text-amber-800 dark:text-amber-300">
              <Shield className="size-3.5 shrink-0" />
              {analysis.warnings[0]}
            </div>
          ) : null}
        </div>

        <div className="space-y-1">
          <Label className="text-xs text-muted-foreground">Download type</Label>
          {analysis && analysis.presets.length > 0 ? (
            <Select
              value={selectedPresetId}
              onValueChange={onPresetChange}
              items={analysis.presets.map((item) => ({
                value: item.id,
                label: item.label,
              }))}
            >
              <SelectTrigger
                aria-label="Download type"
                className="h-9 w-full bg-background"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent align="start">
                {analysis.presets.map((item) => (
                  <SelectItem key={item.id} value={item.id}>
                    {item.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : (
            <div className="flex h-9 items-center rounded-md border bg-background px-2 text-sm text-muted-foreground">
              {analyzing ? "Inspecting" : "No preset"}
            </div>
          )}
        </div>
      </div>
      {preset ? (
        <div className="download-actions">
          <div className="flex flex-wrap items-center gap-2">
            {!running ? (
              <>
                <Button
                  type="button"
                  variant="outline"
                  disabled={starting || analyzing || Boolean(exportNameError)}
                  onClick={onStart}
                >
                  {starting && startingProfile !== "xrbazaar" ? (
                    <Loader2 className="animate-spin" />
                  ) : (
                    <Download />
                  )}
                  {starting && startingProfile !== "xrbazaar"
                    ? "Starting…"
                    : preset.pipeline === "youtube_channel_export"
                      ? "Export all channels"
                      : "Download original"}
                </Button>
                {preset.outputKind === "video" ? (
                  <button
                    type="button"
                    className="xrbazaar-button"
                    disabled={starting || analyzing}
                    onClick={onStartXrbazaar}
                  >
                    {starting && startingProfile === "xrbazaar" ? (
                      <Loader2 className="size-4 animate-spin" />
                    ) : (
                      <span className="xrbazaar-signet" aria-hidden="true" />
                    )}
                    {starting && startingProfile === "xrbazaar"
                      ? "Starting…"
                      : "Download for XRBAZAAR"}
                  </button>
                ) : null}
              </>
            ) : (
              <Button variant="outline" onClick={() => onCancel(job.id)}>
                <Square />
                Cancel download
              </Button>
            )}
          </div>
          {preset.outputKind === "video" && !running ? (
            <span className="download-profile-note">
              XRBAZAAR: MP4 · 1280 px · up to 30 fps · 100 MiB
            </span>
          ) : null}
        </div>
      ) : null}

      {preset ? (
        <div className="mt-4 space-y-3">
          {job ? (
            <JobProgress
              job={job}
              onCopyLogs={() => onCopyLogs(job)}
              onViewRun={() => preset && onViewRun(preset.id)}
            />
          ) : null}

          {readyAssets.length > 0 ? <ReadyMedia assets={readyAssets} /> : null}

          {!running ? (
            <>
              {preset.pipeline === "youtube_channel_export" ? (
                <div className="space-y-3">
                  <div className="space-y-1">
                    <Label
                      htmlFor="youtube-export-name"
                      className="text-xs text-muted-foreground"
                    >
                      Export name
                    </Label>
                    <input
                      id="youtube-export-name"
                      value={exportName}
                      maxLength={80}
                      placeholder="e.g. AI channels July"
                      aria-invalid={Boolean(exportNameError)}
                      className="h-9 w-full rounded-md border bg-background px-3 text-sm text-foreground outline-none"
                      onChange={(event) =>
                        onExportNameChange(event.target.value)
                      }
                    />
                    <div
                      className={cn(
                        "text-xs",
                        exportNameError
                          ? "text-destructive"
                          : "text-muted-foreground"
                      )}
                    >
                      {exportNameError ??
                        "A completed export with the same name will never be overwritten."}
                    </div>
                  </div>

                  <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">
                      Channel content
                    </Label>
                    <ToggleGroup
                      value={[catalogueContent]}
                      variant="outline"
                      spacing={0}
                      aria-label="Channel content"
                      className="w-full"
                      onValueChange={(values) => {
                        const value = values[0]
                        if (
                          value === "all" ||
                          value === "videos" ||
                          value === "shorts"
                        ) {
                          onCatalogueContentChange(value)
                        }
                      }}
                    >
                      <ToggleGroupItem value="all" className="flex-1">
                        All
                      </ToggleGroupItem>
                      <ToggleGroupItem value="videos" className="flex-1">
                        Videos only
                      </ToggleGroupItem>
                      <ToggleGroupItem value="shorts" className="flex-1">
                        Shorts only
                      </ToggleGroupItem>
                    </ToggleGroup>
                  </div>
                </div>
              ) : null}

              <div className="download-details">
                <span
                  className="flex min-w-0 items-center gap-2"
                  title={outputDir ?? "Downloads"}
                >
                  <FolderOpen className="size-3.5 shrink-0" />
                  <span className="truncate">
                    {preset.pipeline === "youtube_channel_export"
                      ? `${outputDir ?? "Downloads"}/youtube_export/${exportName.trim() || "<export name>"}`
                      : (outputDir ?? "Downloads")}
                  </span>
                </span>
                <span className="flex items-center gap-2">
                  <Shield className="size-3.5" />
                  {authLabel(auth)}
                  {preset.auth === "required" && !canUseAuth ? " required" : ""}
                </span>
              </div>

              {preset.pipeline !== "youtube_channel_export" ? (
                <AdvancedDownloadPanel
                  options={advancedOptions}
                  formatInfo={formatInfo}
                  loadingFormats={loadingFormats}
                  onChange={onAdvancedChange}
                  onLoadFormats={onLoadFormats}
                />
              ) : null}
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function AdvancedDownloadPanel({
  options,
  formatInfo,
  loadingFormats,
  onChange,
  onLoadFormats,
}: {
  options: AdvancedDownloadOptions
  formatInfo: FormatAnalysis | null
  loadingFormats: boolean
  onChange: (options: AdvancedDownloadOptions) => void
  onLoadFormats: () => void
}) {
  const segment = options.segment ?? defaultAdvancedOptions.segment
  const duration = formatInfo?.duration ?? null
  const videoFormats = (formatInfo?.formats ?? []).filter(
    (format) => format.hasVideo
  )
  const selectedQualityValue = qualityValueFromFormat(options.format)
  const videoEnabled = videoEnabledFromFormat(options.format)
  const audioEnabled = audioEnabledFromFormat(options.format)
  const selectedVideoQualityValue = videoQualityValueFromFormat(
    selectedQualityValue,
    videoFormats
  )
  const selectedQualityLoaded =
    selectedQualityValue === autoQualityValue ||
    videoFormats.some((format) => format.formatId === selectedQualityValue)
  const videoQualityItems = videoQualitySelectItems(
    videoFormats,
    selectedVideoQualityValue,
    selectedQualityValue
  )
  const exactFormatItems = [
    { value: autoQualityValue, label: "Auto format" },
    ...(!selectedQualityLoaded && selectedQualityValue !== autoQualityValue
      ? [
          {
            value: selectedQualityValue,
            label: `Selected format ${selectedQualityValue}`,
          },
        ]
      : []),
    ...formatsForVideoQuality(videoFormats, selectedVideoQualityValue).map(
      (format) => ({
        value: format.formatId,
        label: format.label,
      })
    ),
  ]

  function setFormat(format: FormatSelection) {
    onChange({ ...options, format })
  }

  function setVideoEnabled(nextEnabled: boolean) {
    if (!nextEnabled && !audioEnabled) return
    setFormat(
      formatFromControls(nextEnabled, audioEnabled, selectedQualityValue)
    )
  }

  function setAudioEnabled(nextEnabled: boolean) {
    if (!nextEnabled && !videoEnabled) return
    setFormat(
      formatFromControls(videoEnabled, nextEnabled, selectedQualityValue)
    )
  }

  function setVideoQuality(nextQualityValue: string | null) {
    const qualityValue = nextQualityValue ?? autoQualityValue
    const nextFormat =
      qualityValue === autoQualityValue
        ? autoQualityValue
        : (bestFormatForVideoQuality(videoFormats, qualityValue)?.formatId ??
          autoQualityValue)

    setFormat(formatFromControls(videoEnabled, audioEnabled, nextFormat))
  }

  function setQuality(nextQualityValue: string | null) {
    setFormat(
      formatFromControls(
        videoEnabled,
        audioEnabled,
        nextQualityValue ?? autoQualityValue
      )
    )
  }

  function setSegment(
    nextSegment: NonNullable<AdvancedDownloadOptions["segment"]>
  ) {
    onChange({ ...options, segment: nextSegment })
  }

  function enableSegment(enabled: boolean) {
    const endSeconds = segment?.endSeconds ?? duration ?? 60
    setSegment({
      enabled,
      startSeconds: segment?.startSeconds ?? 0,
      endSeconds,
    })
  }

  return (
    <details className="download-options border-t pt-4">
      <summary className="flex cursor-pointer flex-wrap items-center gap-2 text-sm font-medium">
        <SlidersHorizontal className="size-4" />
        Download options
        <span className="ml-auto text-xs font-normal text-muted-foreground">
          {!videoEnabled
            ? "Audio only"
            : !audioEnabled
              ? "Video only"
              : "Video + audio"}
          {segment?.enabled ? " · Trimmed" : " · Full length"}
        </span>
        <ChevronRight className="disclosure-chevron size-4 text-muted-foreground" />
      </summary>
      <div className="pt-5">
        <div className="space-y-2">
          <Label className="text-xs text-muted-foreground">Streams</Label>
          <div className="grid gap-2 sm:grid-cols-2">
            <Label className="flex h-10 items-center gap-2 rounded-md border bg-card px-3 text-sm">
              <Checkbox
                aria-label="Include video"
                checked={videoEnabled}
                disabled={videoEnabled && !audioEnabled}
                onCheckedChange={(checked) => setVideoEnabled(Boolean(checked))}
              />
              <Film className="size-3.5 text-muted-foreground" />
              Video
            </Label>
            <Label className="flex h-10 items-center gap-2 rounded-md border bg-card px-3 text-sm">
              <Checkbox
                aria-label="Include audio"
                checked={audioEnabled}
                disabled={audioEnabled && !videoEnabled}
                onCheckedChange={(checked) => setAudioEnabled(Boolean(checked))}
              />
              <Music className="size-3.5 text-muted-foreground" />
              Audio
            </Label>
          </div>
        </div>

        <div className="mt-3 grid gap-3 sm:grid-cols-2">
          <div className="space-y-2">
            <Label className="flex h-7 items-center text-xs text-muted-foreground">
              Video quality
            </Label>
            <Select
              value={selectedVideoQualityValue}
              onValueChange={setVideoQuality}
              disabled={!videoEnabled}
              items={videoQualityItems}
            >
              <SelectTrigger
                aria-label="Video quality"
                className="h-9 w-full bg-card"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent align="start">
                {videoQualityItems.map((item) => (
                  <SelectItem key={item.value} value={item.value}>
                    {item.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-2">
            <div className="flex items-center justify-between gap-2">
              <Label className="text-xs text-muted-foreground">Format</Label>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-7 gap-1.5 px-2 text-xs"
                disabled={loadingFormats || !videoEnabled}
                onClick={onLoadFormats}
              >
                {loadingFormats ? (
                  <Loader2 className="size-3 animate-spin" />
                ) : (
                  <RefreshCw className="size-3" />
                )}
                Load
              </Button>
            </div>
            <Select
              value={selectedQualityValue}
              onValueChange={setQuality}
              disabled={!videoEnabled}
              items={exactFormatItems}
            >
              <SelectTrigger
                aria-label="Stream format"
                className="h-9 w-full bg-card"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent align="start">
                {exactFormatItems.map((item) => (
                  <SelectItem key={item.value} value={item.value}>
                    {item.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>

        {!videoEnabled ? (
          <div className="mt-3 text-xs text-muted-foreground">
            Audio uses the best available audio stream.
          </div>
        ) : videoFormats.length === 0 ? (
          <div className="mt-3 flex items-center justify-between gap-3 rounded-md border bg-card px-3 py-2 text-xs text-muted-foreground">
            <span>
              Load formats to choose exact video quality and stream format.
            </span>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 gap-1.5 px-2 text-xs"
              disabled={loadingFormats}
              onClick={onLoadFormats}
            >
              {loadingFormats ? (
                <Loader2 className="size-3 animate-spin" />
              ) : (
                <RefreshCw className="size-3" />
              )}
              Load formats
            </Button>
          </div>
        ) : null}

        <div className="mt-3 rounded-md border bg-card p-3">
          <Label className="flex flex-wrap items-center justify-between gap-3 text-sm">
            <span className="flex items-center gap-2 font-medium">
              <Scissors className="size-4" />
              Trim video
            </span>
            <Switch
              aria-label="Trim video"
              checked={Boolean(segment?.enabled)}
              onCheckedChange={enableSegment}
            />
          </Label>

          {segment?.enabled ? (
            <div className="mt-3 space-y-3">
              <SegmentRange
                duration={duration ?? Math.max(segment.endSeconds ?? 60, 60)}
                start={segment.startSeconds}
                end={segment.endSeconds ?? duration ?? 60}
                onChange={(startSeconds, endSeconds) =>
                  setSegment({
                    enabled: true,
                    startSeconds,
                    endSeconds,
                  })
                }
              />
              <div className="grid grid-cols-2 gap-2">
                <TimeInput
                  label="Start"
                  value={segment.startSeconds}
                  onChange={(startSeconds) =>
                    setSegment({
                      enabled: true,
                      startSeconds,
                      endSeconds: Math.max(
                        startSeconds,
                        segment.endSeconds ?? duration ?? startSeconds
                      ),
                    })
                  }
                />
                <TimeInput
                  label="End"
                  value={segment.endSeconds ?? duration ?? 60}
                  onChange={(endSeconds) =>
                    setSegment({
                      enabled: true,
                      startSeconds: Math.min(segment.startSeconds, endSeconds),
                      endSeconds,
                    })
                  }
                />
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </details>
  )
}

function SegmentRange({
  duration,
  start,
  end,
  onChange,
}: {
  duration: number
  start: number
  end: number
  onChange: (start: number, end: number) => void
}) {
  const safeDuration = Math.max(duration, 1)
  const safeStart = clamp(start, 0, safeDuration)
  const safeEnd = clamp(Math.max(end, safeStart), 0, safeDuration)

  return (
    <Slider
      value={[safeStart, safeEnd]}
      min={0}
      max={safeDuration}
      step={0.1}
      minStepsBetweenValues={0}
      thumbCollisionBehavior="none"
      className="py-3"
      onValueChange={(nextValue) => {
        const [nextStart = safeStart, nextEnd = safeEnd] = Array.isArray(
          nextValue
        )
          ? nextValue
          : [safeStart, safeEnd]
        onChange(Math.min(nextStart, nextEnd), Math.max(nextStart, nextEnd))
      }}
    />
  )
}

function TimeInput({
  label,
  value,
  onChange,
}: {
  label: string
  value: number
  onChange: (value: number) => void
}) {
  const [draft, setDraft] = useState<string | null>(null)
  const displayValue = draft ?? formatTime(value)

  return (
    <label className="space-y-1 text-xs text-muted-foreground">
      {label}
      <input
        value={displayValue}
        onFocus={() => setDraft(formatTime(value))}
        onChange={(event) => {
          const next = event.target.value
          setDraft(next)
          const parsed = parseTime(next)
          if (parsed !== null) onChange(parsed)
        }}
        onBlur={() => setDraft(null)}
        className="h-9 w-full rounded-md border bg-background px-2 font-mono text-sm text-foreground outline-none"
      />
    </label>
  )
}

function JobProgress({
  job,
  onCopyLogs,
  onViewRun,
}: {
  job: Job
  onCopyLogs: () => void
  onViewRun?: () => void
}) {
  const done = job.status === "completed"
  const failed = job.status === "failed"
  const canceled = job.status === "canceled"

  return (
    <div className="job-progress">
      <div className="flex flex-wrap items-center justify-between gap-3 text-sm">
        <div className="flex min-w-0 items-center gap-2">
          {done ? (
            <Check className="size-4 text-emerald-600" />
          ) : failed ? (
            <AlertCircle className="size-4 text-destructive" />
          ) : canceled ? (
            <Square className="size-4 text-muted-foreground" />
          ) : (
            <Loader2 className="size-4 animate-spin text-muted-foreground" />
          )}
          <span className="truncate font-medium">{job.phase}</span>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <span className="text-xs text-muted-foreground">
            {Math.round(job.progress)}%
          </span>
          {onViewRun ? (
            <Button
              type="button"
              size="xs"
              variant="outline"
              className="gap-1.5"
              onClick={onViewRun}
            >
              <List className="size-3" />
              Activity
            </Button>
          ) : null}
          <Button
            type="button"
            size="xs"
            variant="outline"
            className="gap-1.5"
            aria-label="Copy logs"
            onClick={onCopyLogs}
          >
            <Clipboard className="size-3" />
            Logs
          </Button>
        </div>
      </div>
      <div
        role="progressbar"
        aria-label="Download progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(clamp(job.progress, 0, 100))}
        className="mt-3 h-1.5 overflow-hidden rounded-full bg-muted"
      >
        <div
          className={cn(
            "h-full rounded-full transition-all",
            failed
              ? "bg-destructive"
              : canceled
                ? "bg-muted-foreground"
                : "bg-foreground"
          )}
          style={{ width: `${Math.max(0, Math.min(100, job.progress))}%` }}
        />
      </div>
      <div className="mt-2 grid min-w-0 gap-1 text-xs text-muted-foreground">
        {!done && !failed && !canceled && (job.speed || job.eta) ? (
          <div className="flex flex-wrap gap-x-4 gap-y-1">
            {job.speed ? <span>{job.speed}</span> : null}
            {job.eta ? <span>ETA {job.eta}</span> : null}
          </div>
        ) : null}
        {job.outputPath ? (
          <div className="min-w-0 truncate" title={job.outputPath}>
            {job.outputPath}
          </div>
        ) : null}
        {job.errorMessage ? (
          <div
            className="min-w-0 break-words text-destructive"
            title={job.errorMessage}
          >
            {job.errorMessage}
          </div>
        ) : null}
      </div>
    </div>
  )
}

function JobRunItem({
  job,
  assets,
  onCopyLogs,
  onCancel,
}: {
  job: Job
  assets: DownloadAsset[]
  onCopyLogs: () => void
  onCancel: () => void
}) {
  const running = !["completed", "failed", "canceled"].includes(job.status)
  return (
    <div className="rounded-xl border bg-card p-5">
      <div className="mb-4 flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-medium">
            {siteLabels[job.site]} · {humanPresetId(job.presetId)}
          </h2>
          <p className="mt-1 text-xs break-all text-muted-foreground">
            {job.sourceUrl}
          </p>
        </div>
        <div className="flex items-center gap-3">
          <StatusBadge
            status={job.status === "completed" ? "ready" : job.status}
          >
            {job.status}
          </StatusBadge>
          {running ? (
            <Button size="sm" variant="outline" onClick={onCancel}>
              <Square />
              Cancel download
            </Button>
          ) : null}
        </div>
      </div>
      <JobProgress job={job} onCopyLogs={onCopyLogs} />
      {assets.length > 0 ? (
        <div className="mt-4">
          <ReadyMedia assets={assets} />
        </div>
      ) : null}
    </div>
  )
}

function ReadyMedia({ assets }: { assets: DownloadAsset[] }) {
  return (
    <section className="ready-media" aria-label="Downloaded files">
      <div className="ready-media-heading">
        <span>
          {assets.length} {assets.length === 1 ? "file" : "files"} ready
        </span>
      </div>
      <div className="ready-media-grid">
        {assets.map((asset) => (
          <div key={asset.path} className="ready-media-item">
            <AssetPreview path={asset.path} />
            <div className="min-w-0 space-y-3">
              <p className="text-sm font-medium break-words" title={asset.path}>
                {fileNameFromPath(asset.path)}
              </p>
              <AssetActions key={asset.path} path={asset.path} />
            </div>
          </div>
        ))}
      </div>
    </section>
  )
}

function DownloadedAssetItem({
  asset,
  onCopyLogs,
}: {
  asset: DownloadAsset
  onCopyLogs: () => void
}) {
  return (
    <div className="rounded-lg border bg-card p-3">
      <AssetPreview path={asset.path} />
      <div className="mt-3 min-w-0">
        <div
          className="truncate text-sm font-medium"
          title={fileNameFromPath(asset.path)}
        >
          {fileNameFromPath(asset.path)}
        </div>
        <div className="mt-1 truncate text-xs text-muted-foreground">
          {siteLabels[asset.job.site]} - {asset.job.presetId}
        </div>
        <div
          className="mt-1 truncate text-xs text-muted-foreground"
          title={asset.path}
        >
          {asset.path}
        </div>
      </div>
      <div className="mt-3 flex flex-wrap gap-2">
        <AssetActions key={asset.path} path={asset.path} />
        <Button
          type="button"
          size="xs"
          variant="outline"
          className="gap-1.5"
          onClick={onCopyLogs}
        >
          <Clipboard className="size-3" />
          Logs
        </Button>
      </div>
    </div>
  )
}

function AssetPreview({ path }: { path: string }) {
  if (!isVideoPath(path)) {
    return (
      <div className="flex aspect-video items-center justify-center rounded-md border bg-muted">
        <Download className="size-5 text-muted-foreground" />
      </div>
    )
  }
  return <VideoPreview key={path} path={path} />
}

function VideoPreview({ path }: { path: string }) {
  const container = useRef<HTMLDivElement>(null)
  const video = useRef<HTMLVideoElement>(null)
  const playback = usePlayback()
  const [visible, setVisible] = useState(false)
  const [source, setSource] = useState<string | null>(null)
  const [poster, setPoster] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)
  useEffect(() => {
    if (!video.current) return
    video.current.volume = playback.volume
    video.current.muted = playback.muted
  }, [playback, source, error])
  useEffect(() => {
    const element = container.current
    if (!element) return
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setVisible(true)
          observer.disconnect()
        }
      },
      { rootMargin: "200px" }
    )
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  useEffect(() => {
    if (!visible) return
    let canceled = false
    prepareMediaPreview(path)
      .then((url) => {
        if (!canceled) setSource(url)
      })
      .catch((reason) => {
        if (!canceled)
          setError(reason instanceof Error ? reason.message : String(reason))
      })
    createVideoThumbnail(path)
      .then((thumbnail) => {
        if (!canceled && thumbnail) setPoster(localFilePreviewUrl(thumbnail))
      })
      .catch(() => {
        // Thumbnail generation must not prevent otherwise supported playback.
      })
    return () => {
      canceled = true
    }
  }, [path, attempt, visible])

  return (
    <div
      ref={container}
      className="relative flex aspect-video items-center justify-center overflow-hidden rounded-md border bg-muted"
    >
      {error ? (
        <div
          role="status"
          className="flex max-w-sm flex-col items-center gap-3 p-4 text-center"
        >
          <Film className="size-6 text-muted-foreground" />
          <p className="text-xs text-muted-foreground">{error}</p>
          <div className="flex flex-wrap justify-center gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                setError(null)
                setSource(null)
                setAttempt((n) => n + 1)
              }}
            >
              Retry preview
            </Button>
            <Button
              size="sm"
              onClick={() =>
                openOutputPath(path).catch((reason) => setError(String(reason)))
              }
            >
              <Play />
              Open in player
            </Button>
          </div>
        </div>
      ) : source ? (
        <video
          ref={video}
          key={attempt}
          src={source}
          poster={poster ?? undefined}
          aria-label={`Preview of ${fileNameFromPath(path)}`}
          className="h-full w-full object-contain"
          controls
          playsInline
          preload="metadata"
          onVolumeChange={(event) =>
            setPlayback(event.currentTarget.volume, event.currentTarget.muted)
          }
          onError={(event) =>
            setError(
              event.currentTarget.error?.code === 3 ||
                event.currentTarget.error?.code === 4
                ? "This video format cannot play in the app. Open it in your video player."
                : "The video could not be loaded. Retry or open it in your video player."
            )
          }
        />
      ) : (
        <div
          role="status"
          className="flex items-center gap-2 text-xs text-muted-foreground"
        >
          <Loader2 className="size-4 animate-spin" />
          Loading preview…
        </div>
      )}
    </div>
  )
}

function AssetActions({
  path,
  prominent = false,
}: {
  path: string
  prominent?: boolean
}) {
  const [error, setError] = useState<string | null>(null)
  const video = isVideoPath(path)

  async function runAction(action: () => Promise<void>) {
    setError(null)
    try {
      await action()
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    }
  }

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          size={prominent ? "sm" : "xs"}
          variant={prominent ? "default" : "outline"}
          className="gap-1.5"
          onClick={() => runAction(() => openOutputPath(path))}
        >
          {video ? (
            <Play className="size-3" />
          ) : (
            <ExternalLink className="size-3" />
          )}
          {video ? "Open video" : "Open file"}
        </Button>
        <Button
          type="button"
          size={prominent ? "sm" : "xs"}
          variant="outline"
          className="gap-1.5"
          onClick={() => runAction(() => revealOutputPath(path))}
        >
          <FolderOpen className="size-3" />
          Show in folder
        </Button>
        <Button
          type="button"
          size={prominent ? "sm" : "xs"}
          variant="outline"
          className="gap-1.5"
          onClick={() => runAction(() => copyText(path))}
        >
          <Copy className="size-3" />
          Copy path
        </Button>
      </div>
      {error ? (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}
    </div>
  )
}

function EmptyPanel({
  message,
  title,
  icon,
  action,
}: {
  message: string
  title?: string
  icon?: ReactNode
  action?: ReactNode
}) {
  return (
    <div className="flex flex-col items-center rounded-xl border bg-card px-6 py-16 text-center">
      {icon ? (
        <div className="mb-5 flex size-12 items-center justify-center rounded-xl bg-muted text-muted-foreground">
          {icon}
        </div>
      ) : null}
      {title ? <h2 className="text-base font-medium">{title}</h2> : null}
      <p className="mt-2 max-w-sm text-sm leading-6 text-muted-foreground">
        {message}
      </p>
      {action ? <div className="mt-6">{action}</div> : null}
    </div>
  )
}

function toolCheckStateFromTools(tools: ToolUpdate[]): ToolCheckState {
  const issues = tools.filter((tool) => tool.status !== "installed")
  return {
    status: issues.length > 0 ? "issues" : "ready",
    tools,
    checkedAt: new Date().toISOString(),
    message:
      issues.length > 0
        ? `${issues.length} required tool${issues.length === 1 ? "" : "s"} need attention.`
        : "Required tools are available.",
  }
}

function appUpdateStatusLabel(state: AppUpdateState): string {
  if (state.status === "checking") return "Checking"
  if (state.status === "current") return "Current"
  if (state.status === "available") return "Available"
  if (state.status === "installing") return "Installing"
  if (state.status === "restarting") return "Restarting"
  if (state.status === "failed") return "Failed"
  return "Not checked"
}

function toolStatusLabel(state: ToolCheckState): string {
  if (state.status === "checking") return "Checking"
  if (state.status === "installing") return "Installing"
  if (state.status === "ready") return "Ready"
  if (state.status === "issues") return "Needs attention"
  if (state.status === "failed") return "Failed"
  return "Not checked"
}

function toolStatusItemLabel(status: ToolUpdate["status"]): string {
  if (status === "installed") return "Installed"
  if (status === "unsupported") return "Unsupported"
  return "Missing"
}

function statusBadgeClass(status: string): string {
  if (["ready", "current", "installed"].includes(status)) {
    return "border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-300"
  }
  if (["available", "checking", "installing", "restarting"].includes(status)) {
    return "border-sky-200 bg-sky-50 text-sky-700 dark:border-sky-900 dark:bg-sky-950/40 dark:text-sky-300"
  }
  if (["issues", "missing", "unsupported"].includes(status)) {
    return "border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-300"
  }
  if (status === "failed") {
    return "border-destructive/30 bg-destructive/10 text-destructive"
  }
  return "border-border bg-muted text-muted-foreground"
}

function formatCheckedAt(value: string | null): string {
  if (!value) return "Not checked"
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return date.toLocaleString()
}

function authForPreset(preset: Preset, auth: AuthSource): AuthSource {
  if (preset.auth === "none") return { kind: "none" }
  if (preset.auth === "required") {
    return isAuthConfigured(auth) ? auth : { kind: "none" }
  }
  return { kind: "none" }
}

function isAuthConfigured(auth: AuthSource): boolean {
  if (auth.kind === "browser") return browserSources(auth).length > 0
  if (auth.kind === "cookie_file") return auth.path.trim().length > 0
  return false
}

function authLabel(auth: AuthSource): string {
  if (auth.kind === "browser") {
    const sources = browserSources(auth)
    return sources.length > 0
      ? `${sources.map((source) => source.browser).join(" -> ")} cookies`
      : "Browser cookies"
  }
  if (auth.kind === "cookie_file") return auth.path || "cookies.txt"
  return "None"
}

function browserSources(auth: AuthSource): BrowserAuthSource[] {
  if (auth.kind !== "browser") return []
  if (auth.browsers?.length) {
    return auth.browsers.filter((source) => source.browser.trim().length > 0)
  }
  return auth.browser ? [{ browser: auth.browser, profile: auth.profile }] : []
}

function extractUrls(input: string): string[] {
  const matches = input.match(/https?:\/\/[^\s<>"']+/g) ?? []
  return uniqueStrings(
    matches
      .map((match) => match.replace(/[),.;\]]+$/g, ""))
      .filter(looksLikeUrl)
  )
}

function downloadItemDomId(sourceUrl: string): string {
  return `download-item-${stableHash(sourceUrl)}`
}

function jobRunDomId(jobId: string): string {
  return `run-job-${jobId}`
}

function stableHash(value: string): string {
  let hash = 5381
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 33) ^ value.charCodeAt(index)
  }
  return (hash >>> 0).toString(36)
}

function compactUrlLabel(input: string): string {
  try {
    const parsed = new URL(input)
    const parts = parsed.pathname.split("/").filter(Boolean)
    const leaf =
      parsed.hostname.endsWith("youtube.com") &&
      ["videos", "shorts", "streams", "playlists", "about"].includes(
        parts.at(-1)?.toLowerCase() ?? ""
      )
        ? parts.at(-2)
        : parts.at(-1)
    return leaf ? `${parsed.hostname}/${leaf}` : parsed.hostname
  } catch {
    return input
  }
}

function uniqueStrings(values: string[]): string[] {
  const seen = new Set<string>()
  return values.filter((value) => {
    if (seen.has(value)) return false
    seen.add(value)
    return true
  })
}

function runPresetOptionsFromJobs(
  jobs: Job[],
  knownLabels: Record<string, string>
): Array<{ id: string; label: string; count: number }> {
  const counts = new Map<string, number>()
  jobs.forEach((job) => {
    counts.set(job.presetId, (counts.get(job.presetId) ?? 0) + 1)
  })

  return Array.from(counts.entries()).map(([id, count]) => ({
    id,
    count,
    label: knownLabels[id] ?? humanPresetId(id),
  }))
}

function humanPresetId(presetId: string): string {
  return presetId
    .split("-")
    .filter((part) => part.length > 0)
    .map((part) => part[0].toUpperCase() + part.slice(1))
    .join(" ")
}

function assetPathsFromJob(job: Job, logs: JobLog[] = []): string[] {
  if (job.readyPaths) return job.readyPaths
  if (job.status !== "completed") return []
  return uniqueStrings(
    [
      job.outputPath ?? "",
      ...logs.map((log) => parseOutputPathFromLog(log.message) ?? ""),
    ].filter(Boolean)
  )
}

function parseOutputPathFromLog(message: string): string | null {
  const patterns = [
    /^\[download\] Destination: (.+)$/,
    /^\[Merger\] Merging formats into "(.+)"$/,
    /^\[download\] (.+) has already been downloaded$/,
  ]

  for (const pattern of patterns) {
    const match = message.match(pattern)
    if (match?.[1]) return match[1]
  }
  return null
}

function fileNameFromPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path
}

function isVideoPath(path: string): boolean {
  return /\.(mp4|m4v|mov|webm|mkv)$/i.test(path)
}

function advancedKey(url: string, presetId: string): string {
  return `${url}::${presetId}`
}

function videoEnabledFromFormat(format: FormatSelection): boolean {
  return format.kind !== "audio_only"
}

function audioEnabledFromFormat(format: FormatSelection): boolean {
  return format.kind !== "video_only"
}

function qualityValueFromFormat(format: FormatSelection): string {
  if (format.kind === "format" && format.formatId.trim()) {
    return format.formatId
  }
  if (format.kind === "video_only" && format.formatId?.trim()) {
    return format.formatId
  }
  return autoQualityValue
}

function videoQualityValueFromFormat(
  formatId: string,
  formats: FormatOption[]
): string {
  if (formatId === autoQualityValue) return autoQualityValue
  const format = formats.find((item) => item.formatId === formatId)
  if (format?.height) return `height:${format.height}`
  return `format:${formatId}`
}

function videoQualitySelectItems(
  formats: FormatOption[],
  selectedQualityValue: string,
  selectedFormatId: string
): Array<{ value: string; label: string }> {
  const heights = uniqueStrings(
    formats
      .map((format) => format.height)
      .filter((height): height is number => Boolean(height))
      .sort((left, right) => right - left)
      .map((height) => String(height))
  )
  const items = [
    { value: autoQualityValue, label: "Auto best quality" },
    ...heights.map((height) => ({
      value: `height:${height}`,
      label: `${height}p`,
    })),
  ]

  if (
    selectedQualityValue.startsWith("format:") &&
    selectedFormatId !== autoQualityValue
  ) {
    items.splice(1, 0, {
      value: selectedQualityValue,
      label: `Selected format ${selectedFormatId}`,
    })
  }

  return items
}

function formatsForVideoQuality(
  formats: FormatOption[],
  qualityValue: string
): FormatOption[] {
  if (!qualityValue.startsWith("height:")) return formats
  const height = Number(qualityValue.slice("height:".length))
  return formats.filter((format) => format.height === height)
}

function bestFormatForVideoQuality(
  formats: FormatOption[],
  qualityValue: string
): FormatOption | null {
  const candidates = formatsForVideoQuality(formats, qualityValue)
  return (
    candidates.toSorted((left, right) => {
      const rightTbr = right.tbr ?? 0
      const leftTbr = left.tbr ?? 0
      return rightTbr - leftTbr
    })[0] ?? null
  )
}

function formatFromControls(
  videoEnabled: boolean,
  audioEnabled: boolean,
  qualityValue: string
): FormatSelection {
  const formatId = qualityValue === autoQualityValue ? "" : qualityValue.trim()

  if (audioEnabled && !videoEnabled) return { kind: "audio_only" }
  if (videoEnabled && !audioEnabled) {
    return { kind: "video_only", formatId: formatId || null }
  }
  if (formatId) return { kind: "format", formatId }
  return { kind: "best" }
}

function normalizeAdvancedForDuration(
  options: AdvancedDownloadOptions,
  duration: number | null
): AdvancedDownloadOptions {
  if (!options.segment?.enabled || !duration) return options
  const startSeconds = clamp(options.segment.startSeconds, 0, duration)
  const endSeconds = clamp(
    options.segment.endSeconds ?? duration,
    startSeconds,
    duration
  )
  return {
    ...options,
    segment: {
      enabled: true,
      startSeconds,
      endSeconds,
    },
  }
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value))
}

function formatTime(seconds: number): string {
  const safe = Math.max(0, seconds)
  const hours = Math.floor(safe / 3600)
  const minutes = Math.floor((safe % 3600) / 60)
  const wholeSeconds = Math.floor(safe % 60)
  const millis = Math.round((safe - Math.floor(safe)) * 1000)
  const base = [hours, minutes, wholeSeconds]
    .map((part) => String(part).padStart(2, "0"))
    .join(":")
  return millis > 0 ? `${base}.${String(millis).padStart(3, "0")}` : base
}

function parseTime(input: string): number | null {
  const clean = input.trim()
  if (!clean) return null
  if (/^\d+(?:\.\d+)?$/.test(clean)) return Number(clean)

  const parts = clean.split(":")
  if (parts.length < 2 || parts.length > 3) return null
  const numbers = parts.map(Number)
  if (numbers.some((part) => Number.isNaN(part) || part < 0)) return null
  if (numbers.length === 2) return numbers[0] * 60 + numbers[1]
  return numbers[0] * 3600 + numbers[1] * 60 + numbers[2]
}

function looksLikeUrl(input: string): boolean {
  try {
    const parsed = new URL(input)
    return parsed.protocol === "http:" || parsed.protocol === "https:"
  } catch {
    return false
  }
}

async function copyText(text: string) {
  await writeClipboardText(text)
}

function upsertJob(jobs: Job[], next: Job): Job[] {
  const without = jobs.filter((job) => job.id !== next.id)
  return [next, ...without].sort((left, right) =>
    right.updatedAt.localeCompare(left.updatedAt)
  )
}

export default App
