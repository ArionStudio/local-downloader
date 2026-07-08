import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import type { FormEvent, ReactNode } from "react"
import { AnimatePresence, motion } from "framer-motion"
import {
  AlertCircle,
  Check,
  ChevronLeft,
  ChevronRight,
  Clipboard,
  ClipboardPaste,
  Copy,
  Download,
  FolderOpen,
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
  Wrench,
} from "lucide-react"
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
import {
  analyzeFormats,
  analyzeUrl,
  cancelJob,
  checkAppUpdate,
  checkToolUpdates,
  getAppInfo,
  getJob,
  getSettings,
  installAppUpdate,
  installToolUpdate,
  localFilePreviewUrl,
  listJobs,
  onDownloadJobEvent,
  openOutputPath,
  readClipboardText,
  revealOutputPath,
  selectDownloadDir,
  startDownload,
  updateSettings,
  writeClipboardText,
} from "@/lib/api"
import { defaultSettings } from "@/lib/fallback"
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
  ToolUpdate,
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
  sawhorse: "Sawhorse",
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

function App() {
  const [url, setUrl] = useState("")
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
  const completedJobs = jobs.filter((job) => job.status === "completed")
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
      completedJobs.flatMap((job) =>
        (assetsByJob[job.id] ?? []).map((path) => ({ path, job }))
      ),
    [assetsByJob, completedJobs]
  )
  const downloadNavItems = useMemo(
    () =>
      inputUrls.map((inputUrl, index) => {
        const analysis = analysesByUrl[inputUrl]
        return {
          id: downloadItemDomId(inputUrl),
          index: index + 1,
          sourceUrl: inputUrl,
          siteLabel: analysis ? siteLabels[analysis.siteKind] : "Link",
          title: compactUrlLabel(inputUrl),
          status: analyzingUrls[inputUrl]
            ? "Inspecting"
            : analysis
              ? "Ready"
              : "Queued",
        }
      }),
    [analysesByUrl, analyzingUrls, inputUrls]
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

  async function handleStart(sourceUrl: string, preset: Preset) {
    const analysis = analysesByUrl[sourceUrl]
    if (!analysis) return false

    const key = advancedKey(analysis.normalizedUrl, preset.id)
    const auth = authForPreset(preset, settings.auth)
    if (preset.auth === "required" && !isAuthConfigured(auth)) {
      setError("Configure browser cookies or cookies.txt in Settings first.")
      setActiveTab("settings")
      return false
    }

    const request: StartDownloadRequest = {
      url: analysis.normalizedUrl,
      presetId: preset.id,
      outputDir: settings.defaultOutputDir,
      filenameTemplate: "%(title).180B [%(id)s].%(ext)s",
      auth,
      advanced: advancedByPreset[key] ?? defaultAdvancedOptions,
    }
    pushSessionLog(
      `${new Date().toISOString()} INFO start: ${preset.id} ${analysis.normalizedUrl}`
    )
    try {
      const job = await startDownload(request)
      setJobs((current) => upsertJob(current, job))
      return true
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
      return false
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

    for (const item of startable) {
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
    const saved = await updateSettings(draftSettings)
    setSettings(saved)
    setDraftSettings(saved)
    pushSessionLog(`${new Date().toISOString()} INFO settings: saved`)
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

  function handlePresetChange(sourceUrl: string, presetId: string | null) {
    if (!presetId) return
    setSelectedPresetByUrl((current) => ({
      ...current,
      [sourceUrl]: presetId,
    }))
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
      <div className="mx-auto flex min-h-svh w-full max-w-6xl flex-col px-4 py-5 sm:px-6 lg:px-8">
        <header className="flex h-10 items-center justify-between gap-3">
          <div className="flex items-center gap-2 text-sm font-medium">
            <div className="flex size-8 items-center justify-center rounded-md border bg-card">
              <Download className="size-4" />
            </div>
            <span>Downloader</span>
          </div>
          <div className="truncate text-xs text-muted-foreground">
            {appInfo ? `v${appInfo.version}` : "Desktop downloader"}
          </div>
        </header>

        <section className="flex flex-1 flex-col justify-start pt-8">
          <form onSubmit={handleAnalyze} className="mx-auto w-full max-w-5xl">
            <div className="group grid min-h-24 grid-cols-[auto_1fr_auto] items-start gap-3 rounded-lg border bg-card px-4 py-3 shadow-sm transition-shadow focus-within:shadow-md">
              {isAnalyzing ? (
                <Loader2 className="mt-1 size-5 shrink-0 animate-spin text-muted-foreground" />
              ) : (
                <Search className="mt-1 size-5 shrink-0 text-muted-foreground" />
              )}
              <textarea
                value={url}
                onChange={(event) => handleUrlChange(event.target.value)}
                placeholder="Paste one or more URLs"
                className="min-h-16 resize-none bg-transparent text-base leading-6 outline-none placeholder:text-muted-foreground"
              />
              <Button
                type="button"
                size="sm"
                disabled={isAnalyzing}
                className="gap-1.5 self-start"
                onClick={handlePaste}
              >
                <ClipboardPaste className="size-3.5" />
                Paste
              </Button>
            </div>
          </form>

          <AnimatePresence mode="popLayout">
            {error ? (
              <motion.div
                layout
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: 8 }}
                className="mx-auto mt-4 flex w-full max-w-3xl items-center gap-2 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
              >
                <AlertCircle className="size-4" />
                {error}
              </motion.div>
            ) : null}
          </AnimatePresence>

          <Tabs
            value={activeTab}
            onValueChange={(value) => setActiveTab(value as AppTab)}
            className="mx-auto mt-5 w-full max-w-5xl"
          >
            <div className="flex flex-wrap items-center justify-between gap-3">
              <TabsList>
                <TabsTrigger value="download">Download</TabsTrigger>
                <TabsTrigger value="runs">
                  Runs
                  {runJobs.length > 0 ? ` ${runJobs.length}` : ""}
                </TabsTrigger>
                <TabsTrigger value="downloaded">
                  Downloaded
                  {downloadedAssets.length > 0
                    ? ` ${downloadedAssets.length}`
                    : ""}
                </TabsTrigger>
                <TabsTrigger value="settings">
                  <SettingsIcon className="size-3.5" />
                  Settings
                </TabsTrigger>
              </TabsList>

              {activeTab === "download" && inputUrls.length > 1 ? (
                <Button
                  type="button"
                  size="sm"
                  className="gap-1.5"
                  disabled={
                    inputUrls.some((inputUrl) => !analysesByUrl[inputUrl]) ||
                    isAnalyzing
                  }
                  onClick={handleStartAll}
                >
                  <Download className="size-3.5" />
                  Start all
                </Button>
              ) : null}
            </div>

            <TabsContent value="download" className="mt-4">
              {inputUrls.length > 0 ? (
                <div className="space-y-3">
                  <div className="flex items-center justify-between gap-3 text-sm">
                    <div className="text-muted-foreground">
                      {inputUrls.length} link{inputUrls.length === 1 ? "" : "s"}
                    </div>
                  </div>
                  <div
                    className={cn(
                      "grid gap-3",
                      inputUrls.length > 1 &&
                        "lg:grid-cols-[168px_minmax(0,1fr)]"
                    )}
                  >
                    {inputUrls.length > 1 ? (
                      <DownloadItemMenu
                        items={downloadNavItems}
                        onSelect={scrollToDownloadItem}
                      />
                    ) : null}

                    <div className="space-y-3">
                      {inputUrls.map((inputUrl) => {
                        const analysis = analysesByUrl[inputUrl]
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
                              analysis={analysis ?? null}
                              analyzing={Boolean(analyzingUrls[inputUrl])}
                              preset={preset}
                              selectedPresetId={presetId ?? null}
                              jobs={jobs}
                              outputDir={settings.defaultOutputDir}
                              auth={settings.auth}
                              advancedOptions={
                                advancedByPreset[key] ?? defaultAdvancedOptions
                              }
                              formatInfo={formatsByPreset[key] ?? null}
                              loadingFormats={loadingFormatsKey === key}
                              onPresetChange={(nextPresetId) =>
                                handlePresetChange(inputUrl, nextPresetId)
                              }
                              onStart={() =>
                                preset
                                  ? handleStart(inputUrl, preset)
                                  : undefined
                              }
                              onCancel={(jobId) => cancelJob(jobId)}
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
              ) : (
                <EmptyPanel message="Paste one or more links to configure downloads." />
              )}
            </TabsContent>

            <TabsContent value="runs" className="mt-4" id={runsPanelDomId}>
              {runJobs.length > 0 ? (
                <div className="space-y-3">
                  <div className="flex flex-wrap items-end justify-between gap-3 rounded-lg border bg-card px-3 py-3">
                    <div className="min-w-0">
                      <div className="text-sm font-medium">Runs</div>
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
                        <SelectTrigger className="h-9 w-full bg-background">
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
                <EmptyPanel message="Started runs will appear here." />
              )}
            </TabsContent>

            <TabsContent value="downloaded" className="mt-4">
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
                <EmptyPanel message="Downloaded assets will appear here." />
              )}
            </TabsContent>

            <TabsContent value="settings" className="mt-4">
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
                onCheckAppUpdate={handleCheckAppUpdate}
                onInstallAppUpdate={handleInstallAppUpdate}
                onCheckTools={handleCheckTools}
                onInstallTool={handleInstallTool}
                onCopyLogs={copyAllLogs}
              />
            </TabsContent>
          </Tabs>
        </section>
      </div>
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
  onCheckAppUpdate,
  onInstallAppUpdate,
  onCheckTools,
  onInstallTool,
  onCopyLogs,
}: SettingsPageProps) {
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
      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(320px,420px)]">
        <section className="rounded-lg border bg-card p-4">
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

          <div className="mt-4 flex flex-wrap justify-end gap-2">
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

        <section className="rounded-lg border bg-card p-4">
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
            <div>
              Search order: app data tools, bundled resources, system PATH,
              `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`.
            </div>
            <div>
              Installer: downloads verified upstream release assets into app
              data tools.
            </div>
            <div>Last check: {formatCheckedAt(toolCheckState.checkedAt)}</div>
          </div>

          <div className="mt-4 flex justify-end">
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

      <section className="rounded-lg border bg-card p-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <div className="flex items-center gap-2 text-sm font-medium">
              <SettingsIcon className="size-4" />
              Downloader settings
            </div>
            <div className="mt-1 text-xs text-muted-foreground">
              {hasUnsavedSettings ? "Unsaved changes" : "Saved"}
            </div>
          </div>
          <Button type="button" onClick={onSave} disabled={!hasUnsavedSettings}>
            Save settings
          </Button>
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-[1fr_auto]">
          <div className="min-w-0 rounded-md border bg-background px-3 py-2 text-sm">
            <div className="text-xs text-muted-foreground">Download folder</div>
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
            Folder
          </Button>
        </div>

        <div className="mt-3 grid gap-3 sm:grid-cols-3">
          <div className="space-y-1">
            <Label className="text-xs text-muted-foreground">Auth</Label>
            <Select
              value={authMode}
              onValueChange={setAuthMode}
              items={[
                { value: "browser", label: "Browser cookies" },
                { value: "cookie_file", label: "cookies.txt" },
                { value: "none", label: "None" },
              ]}
            >
              <SelectTrigger className="h-9 w-full bg-background">
                <SelectValue />
              </SelectTrigger>
              <SelectContent align="start">
                <SelectItem value="browser">Browser cookies</SelectItem>
                <SelectItem value="cookie_file">cookies.txt</SelectItem>
                <SelectItem value="none">None</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-1 text-xs text-muted-foreground sm:col-span-2">
            Browser fallback
            <div className="grid grid-cols-2 gap-2 rounded-md border bg-background p-2 sm:grid-cols-4">
              {browsers.map((browserName) => {
                const checked = selectedBrowsers.some(
                  (source) => source.browser === browserName
                )
                return (
                  <label
                    key={browserName}
                    className={cn(
                      "flex h-8 items-center gap-2 rounded border px-2 text-xs text-foreground capitalize",
                      authMode !== "browser" && "opacity-50"
                    )}
                  >
                    <Checkbox
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

          <label className="space-y-1 text-xs text-muted-foreground sm:col-span-3">
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
        </div>
      </section>

      <section className="rounded-lg border bg-card p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <div className="flex items-center gap-2 text-sm font-medium">
              <Clipboard className="size-4" />
              Session log
            </div>
            <div className="mt-1 text-xs text-muted-foreground">
              {sessionLogs.length} line{sessionLogs.length === 1 ? "" : "s"}
            </div>
          </div>
          <Button
            type="button"
            variant="outline"
            className="gap-1.5"
            onClick={onCopyLogs}
          >
            <Clipboard className="size-3.5" />
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
      </section>
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
}: {
  tool: ToolUpdate
  installing: boolean
  onInstall: () => void
}) {
  const installed = tool.status === "installed"
  const installable = !installed

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
  analysis: AnalyzeResult | null
  analyzing: boolean
  preset: Preset | null
  selectedPresetId: string | null
  jobs: Job[]
  outputDir?: string | null
  auth: AuthSource
  advancedOptions: AdvancedDownloadOptions
  formatInfo: FormatAnalysis | null
  loadingFormats: boolean
  onPresetChange: (presetId: string | null) => void
  onStart: () => void
  onCancel: (jobId: string) => void
  onCopyLogs: (job: Job) => void
  onViewRun: (presetId: string) => void
  onAdvancedChange: (options: AdvancedDownloadOptions) => void
  onLoadFormats: () => void
}

function DownloadLinkCard({
  sourceUrl,
  analysis,
  analyzing,
  preset,
  selectedPresetId,
  jobs,
  outputDir,
  auth,
  advancedOptions,
  formatInfo,
  loadingFormats,
  onPresetChange,
  onStart,
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
            item.sourceUrl === analysis.normalizedUrl &&
            item.presetId === preset.id
        ) ?? null)
      : null
  const running =
    job && !["completed", "failed", "canceled"].includes(job.status)
  const presetAuth = preset?.auth ?? "none"
  const canUseAuth = isAuthConfigured(auth)

  return (
    <div className="rounded-lg border bg-card p-4">
      <div className="grid gap-3 lg:grid-cols-[1fr_220px]">
        <div className="min-w-0 space-y-2">
          <div className="flex min-w-0 flex-wrap items-center gap-2 text-sm">
            {analysis ? (
              <span className="rounded-md border bg-muted px-2 py-1 font-medium">
                {siteLabels[analysis.siteKind]}
              </span>
            ) : null}
            <span className="rounded border bg-muted px-1.5 py-0.5 text-[11px] tracking-normal text-muted-foreground uppercase">
              video
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
          <div className="truncate text-sm font-medium">{sourceUrl}</div>
          {analysis?.warnings.length ? (
            <div className="flex items-center gap-1.5 text-xs text-amber-700">
              <Shield className="size-3.5" />
              {analysis.warnings[0]}
            </div>
          ) : null}
        </div>

        <div className="space-y-1">
          <Label className="text-xs text-muted-foreground">Preset</Label>
          {analysis && analysis.presets.length > 0 ? (
            <Select
              value={selectedPresetId}
              onValueChange={onPresetChange}
              items={analysis.presets.map((item) => ({
                value: item.id,
                label: item.label,
              }))}
            >
              <SelectTrigger className="h-9 w-full bg-background">
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
        <div className="mt-4 space-y-3">
          {job ? (
            <JobProgress
              job={job}
              onCopyLogs={() => onCopyLogs(job)}
              onViewRun={() => preset && onViewRun(preset.id)}
            />
          ) : null}

          {!running ? (
            <>
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="min-w-0 rounded-md border bg-background px-3 py-2 text-sm">
                  <div className="text-xs text-muted-foreground">Output</div>
                  <div className="mt-0.5 truncate">
                    {outputDir ?? "Downloads"}
                  </div>
                </div>
                <div className="min-w-0 rounded-md border bg-background px-3 py-2 text-sm">
                  <div className="text-xs text-muted-foreground">Auth</div>
                  <div className="mt-0.5 truncate">
                    {authLabel(auth)}
                    {preset.auth === "required" && !canUseAuth
                      ? " required"
                      : ""}
                  </div>
                </div>
              </div>

              <AdvancedDownloadPanel
                options={advancedOptions}
                formatInfo={formatInfo}
                loadingFormats={loadingFormats}
                onChange={onAdvancedChange}
                onLoadFormats={onLoadFormats}
              />

              <div className="flex justify-end">
                <Button
                  type="button"
                  className="gap-2"
                  disabled={analyzing}
                  onClick={onStart}
                >
                  <Download className="size-4" />
                  Start
                </Button>
              </div>
            </>
          ) : (
            <div className="flex justify-end">
              <Button
                type="button"
                variant="outline"
                className="gap-2"
                onClick={() => onCancel(job.id)}
              >
                <Square className="size-3.5" />
                Stop
              </Button>
            </div>
          )}
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
    <div className="rounded-md border bg-background p-3">
      <div className="mb-3 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-sm font-medium">
          <SlidersHorizontal className="size-4" />
          Advanced
        </div>
      </div>

      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground">Streams</Label>
        <div className="grid gap-2 sm:grid-cols-2">
          <Label className="flex h-10 items-center gap-2 rounded-md border bg-card px-3 text-sm">
            <Checkbox
              checked={videoEnabled}
              disabled={videoEnabled && !audioEnabled}
              onCheckedChange={(checked) => setVideoEnabled(Boolean(checked))}
            />
            <Film className="size-3.5 text-muted-foreground" />
            Video
          </Label>
          <Label className="flex h-10 items-center gap-2 rounded-md border bg-card px-3 text-sm">
            <Checkbox
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
          <Label className="text-xs text-muted-foreground">Video quality</Label>
          <Select
            value={selectedVideoQualityValue}
            onValueChange={setVideoQuality}
            disabled={!videoEnabled}
            items={videoQualityItems}
          >
            <SelectTrigger className="h-9 w-full bg-card">
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
            <SelectTrigger className="h-9 w-full bg-card">
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
        <Label className="flex items-center justify-between gap-3 text-sm">
          <span className="flex items-center gap-2 font-medium">
            <Scissors className="size-4" />
            Segment
          </span>
          <Switch
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
  onViewRun: () => void
}) {
  const done = job.status === "completed"
  const failed = job.status === "failed"
  const canceled = job.status === "canceled"

  return (
    <div className="rounded-md border bg-background px-3 py-3">
      <div className="flex items-center justify-between gap-3 text-sm">
        <div className="flex min-w-0 items-center gap-2">
          {done ? (
            <Check className="size-4 text-emerald-600" />
          ) : failed || canceled ? (
            <AlertCircle className="size-4 text-destructive" />
          ) : (
            <Loader2 className="size-4 animate-spin text-muted-foreground" />
          )}
          <span className="truncate font-medium">{job.phase}</span>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <span className="text-xs text-muted-foreground">
            {Math.round(job.progress)}%
          </span>
          <Button
            type="button"
            size="xs"
            variant="outline"
            className="gap-1.5"
            onClick={onViewRun}
          >
            <List className="size-3" />
            Run
          </Button>
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
      <div className="mt-3 h-2 overflow-hidden rounded-full bg-muted">
        <div
          className={cn(
            "h-full rounded-full transition-all",
            failed || canceled ? "bg-destructive" : "bg-foreground"
          )}
          style={{ width: `${Math.max(0, Math.min(100, job.progress))}%` }}
        />
      </div>
      <div className="mt-2 grid min-w-0 gap-1 text-xs text-muted-foreground">
        {job.speed || job.eta ? (
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
            className="min-w-0 truncate text-destructive"
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
}: {
  job: Job
  assets: DownloadAsset[]
  onCopyLogs: () => void
}) {
  return (
    <div className="rounded-lg border bg-card p-4">
      <div className="grid gap-3 sm:grid-cols-[1fr_auto_auto] sm:items-center">
        <div className="min-w-0">
          <div className="truncate text-sm font-medium">
            {siteLabels[job.site]} - {job.presetId}
          </div>
          <div className="mt-1 truncate text-xs text-muted-foreground">
            {job.sourceUrl}
          </div>
        </div>
        <span className="w-fit rounded border bg-muted px-2 py-1 text-xs text-muted-foreground capitalize">
          {job.status}
        </span>
        <div className="flex items-center justify-end">
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

      <div className="mt-3 text-xs text-muted-foreground">{job.phase}</div>
      {assets.length > 0 ? (
        <div className="mt-3">
          <AssetCarousel assets={assets} />
        </div>
      ) : null}
    </div>
  )
}

function AssetCarousel({ assets }: { assets: DownloadAsset[] }) {
  const [index, setIndex] = useState(0)
  const safeIndex = clamp(index, 0, Math.max(assets.length - 1, 0))
  const asset = assets[safeIndex]

  if (!asset) return null

  return (
    <div className="rounded-md border bg-background p-3">
      <div className="grid gap-3 md:grid-cols-[280px_1fr]">
        <AssetPreview path={asset.path} />
        <div className="flex min-w-0 flex-col justify-between gap-3">
          <div className="min-w-0">
            <div
              className="truncate text-sm font-medium"
              title={fileNameFromPath(asset.path)}
            >
              {fileNameFromPath(asset.path)}
            </div>
            <div
              className="mt-1 truncate text-xs text-muted-foreground"
              title={asset.path}
            >
              {asset.path}
            </div>
          </div>
          <AssetActions path={asset.path} />
        </div>
      </div>

      {assets.length > 1 ? (
        <div className="mt-3 flex items-center justify-between gap-3">
          <Button
            type="button"
            size="xs"
            variant="outline"
            className="gap-1.5"
            onClick={() => setIndex(Math.max(0, safeIndex - 1))}
          >
            <ChevronLeft className="size-3" />
            Prev
          </Button>
          <div className="text-xs text-muted-foreground">
            {safeIndex + 1} / {assets.length}
          </div>
          <Button
            type="button"
            size="xs"
            variant="outline"
            className="gap-1.5"
            onClick={() => setIndex(Math.min(assets.length - 1, safeIndex + 1))}
          >
            Next
            <ChevronRight className="size-3" />
          </Button>
        </div>
      ) : null}
    </div>
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
        <AssetActions path={asset.path} />
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
  const previewUrl = pathToPreviewUrl(path)

  return (
    <div className="relative aspect-video overflow-hidden rounded-md border bg-muted">
      {previewUrl ? (
        <video
          src={previewUrl}
          className="h-full w-full object-contain"
          controls
          preload="metadata"
        />
      ) : (
        <div className="flex h-full items-center justify-center">
          <Download className="size-5 text-muted-foreground" />
        </div>
      )}
    </div>
  )
}

function AssetActions({ path }: { path: string }) {
  return (
    <div className="flex flex-wrap gap-2">
      <Button
        type="button"
        size="xs"
        variant="outline"
        className="gap-1.5"
        onClick={() => revealOutputPath(path)}
      >
        <FolderOpen className="size-3" />
        Show
      </Button>
      <Button
        type="button"
        size="xs"
        variant="outline"
        className="gap-1.5"
        onClick={() => openOutputPath(path)}
      >
        <Play className="size-3" />
        Open
      </Button>
      <Button
        type="button"
        size="xs"
        variant="outline"
        className="gap-1.5"
        onClick={() => copyText(path)}
      >
        <Copy className="size-3" />
        Path
      </Button>
    </div>
  )
}

function EmptyPanel({ message }: { message: string }) {
  return (
    <div className="rounded-lg border bg-card px-4 py-8 text-center text-sm text-muted-foreground">
      {message}
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
    const leaf = parsed.pathname.split("/").filter(Boolean).at(-1)
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

function pathToPreviewUrl(path: string): string | null {
  if (!isLikelyVideoPath(path)) return null
  return localFilePreviewUrl(path)
}

function isLikelyVideoPath(path: string): boolean {
  return /\.(mp4|m4v|mov|webm|mkv)$/i.test(path)
}

function upsertJob(jobs: Job[], next: Job): Job[] {
  const without = jobs.filter((job) => job.id !== next.id)
  return [next, ...without].sort((left, right) =>
    right.updatedAt.localeCompare(left.updatedAt)
  )
}

export default App
