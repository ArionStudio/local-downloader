import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import type { FormEvent, ReactNode } from "react"
import { AnimatePresence, motion } from "framer-motion"
import {
  AlertCircle,
  Check,
  ChevronRight,
  Clipboard,
  ClipboardPaste,
  Copy,
  Download,
  FolderOpen,
  Film,
  Loader2,
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
import {
  analyzeFormats,
  analyzeUrl,
  cancelJob,
  checkAppUpdate,
  checkToolUpdates,
  createVideoThumbnail,
  getJob,
  getSettings,
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
  AuthRequirement,
  AuthSource,
  BrowserAuthSource,
  BrowserKind,
  FormatAnalysis,
  Job,
  JobLog,
  Preset,
  Settings as DownloaderSettings,
  SiteKind,
  StartDownloadRequest,
} from "@/lib/types"
import { cn } from "@/lib/utils"

type ScreenState = "idle" | "analyzed" | "configuring" | "running"

const siteLabels: Record<SiteKind, string> = {
  generic: "Generic",
  reddit: "Reddit",
  linkedin: "LinkedIn",
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

function App() {
  const [url, setUrl] = useState("")
  const [analysis, setAnalysis] = useState<AnalyzeResult | null>(null)
  const [selectedPresetId, setSelectedPresetId] = useState<string | null>(null)
  const [settings, setSettings] = useState<DownloaderSettings>(defaultSettings)
  const [draftSettings, setDraftSettings] =
    useState<DownloaderSettings>(defaultSettings)
  const [advancedByPreset, setAdvancedByPreset] = useState<
    Record<string, AdvancedDownloadOptions>
  >({})
  const [formatsByPreset, setFormatsByPreset] = useState<
    Record<string, FormatAnalysis>
  >({})
  const [loadingFormatsKey, setLoadingFormatsKey] = useState<string | null>(null)
  const [showSettings, setShowSettings] = useState(false)
  const [jobs, setJobs] = useState<Job[]>([])
  const [jobLogs, setJobLogs] = useState<Record<string, JobLog[]>>({})
  const [sessionLogs, setSessionLogs] = useState<string[]>([])
  const [isAnalyzing, setIsAnalyzing] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [appUpdateLabel, setAppUpdateLabel] = useState("Up to date")
  const [toolLabel, setToolLabel] = useState("Tools")
  const lastAnalyzedUrl = useRef("")

  const pushSessionLog = useCallback((line: string) => {
    setSessionLogs((current) => [...current.slice(-199), line])
  }, [])

  useEffect(() => {
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
          `${log.createdAt} ${log.level.toUpperCase()} ${job.presetId}: ${log.message}`,
        )
      }
    })

    return () => {
      window.clearInterval(refreshInterval)
      unlistenPromise.then((unlisten) => unlisten()).catch(() => undefined)
    }
  }, [pushSessionLog])

  const selectedPreset = useMemo(
    () =>
      analysis?.presets.find((preset) => preset.id === selectedPresetId) ??
      null,
    [analysis, selectedPresetId],
  )

  const activeJob = useMemo(
    () =>
      jobs.find(
        (job) =>
          job.sourceUrl === analysis?.normalizedUrl &&
          job.presetId === selectedPresetId &&
          !["completed", "failed", "canceled"].includes(job.status),
      ) ?? null,
    [analysis?.normalizedUrl, jobs, selectedPresetId],
  )

  const recentJobs = jobs.slice(0, 8)
  const completedJobs = jobs.filter((job) => job.status === "completed")
  const screenState: ScreenState = activeJob
    ? "running"
    : selectedPreset
      ? "configuring"
      : analysis
        ? "analyzed"
        : "idle"

  const runAnalysis = useCallback(
    async (inputUrl: string, normalizeInput: boolean) => {
      if (lastAnalyzedUrl.current === inputUrl) return

      setIsAnalyzing(true)
      setError(null)
      try {
        pushSessionLog(`${new Date().toISOString()} INFO analyze: ${inputUrl}`)
        const result = await analyzeUrl(inputUrl)
        lastAnalyzedUrl.current = inputUrl
        setAnalysis(result)
        setSelectedPresetId(null)
        if (normalizeInput) setUrl(result.normalizedUrl)
        pushSessionLog(
          `${new Date().toISOString()} INFO analyze: ${siteLabels[result.siteKind]} ${result.presets.length} presets`,
        )
      } catch (reason) {
        setAnalysis(null)
        setError(reason instanceof Error ? reason.message : String(reason))
      } finally {
        setIsAnalyzing(false)
      }
    },
    [pushSessionLog],
  )

  useEffect(() => {
    const cleanUrl = url.trim()
    if (!looksLikeUrl(cleanUrl)) return

    const timeout = window.setTimeout(() => {
      void runAnalysis(cleanUrl, false)
    }, 300)

    return () => window.clearTimeout(timeout)
  }, [runAnalysis, url])

  async function handleAnalyze(event?: FormEvent) {
    event?.preventDefault()
    const cleanUrl = url.trim()
    if (!cleanUrl) return
    await runAnalysis(cleanUrl, true)
  }

  async function handlePaste() {
    try {
      const text = await readClipboardText()
      const cleanText = text.trim()
      if (!cleanText) {
        setError("Clipboard is empty.")
        return
      }
      handleUrlChange(cleanText)
    } catch {
      setError("Paste was blocked. Use Ctrl+V or check app clipboard permissions.")
    }
  }

  async function handleStart(preset: Preset) {
    if (!analysis) return

    const key = advancedKey(analysis.normalizedUrl, preset.id)
    const auth = authForPreset(preset, settings.auth)
    if (preset.auth === "required" && !isAuthConfigured(auth)) {
      setError("Configure browser cookies or cookies.txt in Settings first.")
      setShowSettings(true)
      return
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
      `${new Date().toISOString()} INFO start: ${preset.id} ${analysis.normalizedUrl}`,
    )
    const job = await startDownload(request)
    setJobs((current) => upsertJob(current, job))
  }

  async function handleLoadFormats(preset: Preset) {
    if (!analysis) return

    const key = advancedKey(analysis.normalizedUrl, preset.id)
    setLoadingFormatsKey(key)
    setError(null)
    try {
      const result = await analyzeFormats(
        analysis.normalizedUrl,
        authForPreset(preset, settings.auth),
      )
      setFormatsByPreset((current) => ({ ...current, [key]: result }))
      setAdvancedByPreset((current) => ({
        ...current,
        [key]: normalizeAdvancedForDuration(
          current[key] ?? defaultAdvancedOptions,
          result.duration ?? null,
        ),
      }))
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setLoadingFormatsKey(null)
    }
  }

  function handleAdvancedChange(
    preset: Preset,
    nextOptions: AdvancedDownloadOptions,
  ) {
    if (!analysis) return
    const key = advancedKey(analysis.normalizedUrl, preset.id)
    setAdvancedByPreset((current) => ({ ...current, [key]: nextOptions }))
  }

  async function handleSaveSettings() {
    const saved = await updateSettings(draftSettings)
    setSettings(saved)
    setDraftSettings(saved)
    setShowSettings(false)
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
    setAppUpdateLabel("Checking")
    const update = await checkAppUpdate()
    setAppUpdateLabel(update ? `v${update.version}` : "Up to date")
  }

  async function handleCheckTools() {
    setToolLabel("Checking")
    const updates = await checkToolUpdates()
    setToolLabel(updates.length > 0 ? `${updates.length} update` : "Ready")
  }

  function handleUrlChange(nextUrl: string) {
    setUrl(nextUrl)
    if (!looksLikeUrl(nextUrl.trim())) {
      lastAnalyzedUrl.current = ""
      setAnalysis(null)
      setSelectedPresetId(null)
    }
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
        (log) => `${log.createdAt} ${log.level.toUpperCase()} ${log.message}`,
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
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              variant="outline"
              className="h-8 gap-1.5 px-2.5 text-xs"
              onClick={copyAllLogs}
            >
              <Clipboard className="size-3.5" />
              Logs
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-8 gap-1.5 px-2.5 text-xs"
              onClick={handleCheckTools}
            >
              <Wrench className="size-3.5" />
              {toolLabel}
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-8 gap-1.5 px-2.5 text-xs"
              onClick={handleCheckAppUpdate}
            >
              <RefreshCw className="size-3.5" />
              {appUpdateLabel}
            </Button>
            <Button
              size="icon"
              variant="outline"
              className="size-8"
              aria-label="Settings"
              onClick={() => setShowSettings((current) => !current)}
            >
              <SettingsIcon className="size-4" />
            </Button>
          </div>
        </header>

        <AnimatePresence initial={false}>
          {showSettings ? (
            <SettingsPanel
              settings={draftSettings}
              onChange={setDraftSettings}
              onPickFolder={handlePickFolder}
              onSave={handleSaveSettings}
            />
          ) : null}
        </AnimatePresence>

        <motion.section
          layout
          className={cn(
            "flex flex-1 flex-col transition-[padding] duration-300",
            screenState === "idle" && !showSettings
              ? "justify-center pb-24"
              : "justify-start pt-12",
          )}
        >
          <motion.form
            layout
            onSubmit={handleAnalyze}
            className="mx-auto w-full max-w-3xl"
          >
            <div className="group flex h-14 items-center gap-3 rounded-lg border bg-card px-4 shadow-sm transition-shadow focus-within:shadow-md">
              {isAnalyzing ? (
                <Loader2 className="size-5 shrink-0 animate-spin text-muted-foreground" />
              ) : (
                <Search className="size-5 shrink-0 text-muted-foreground" />
              )}
              <input
                value={url}
                onChange={(event) => handleUrlChange(event.target.value)}
                placeholder="Paste URL"
                className="h-full min-w-0 flex-1 bg-transparent text-base outline-none placeholder:text-muted-foreground"
              />
              <Button
                type="button"
                size="sm"
                disabled={isAnalyzing}
                className="gap-1.5"
                onClick={handlePaste}
              >
                <ClipboardPaste className="size-3.5" />
                Paste
              </Button>
            </div>
          </motion.form>

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

            {analysis ? (
              <motion.div
                layout
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: 12 }}
                className="mx-auto mt-5 w-full max-w-3xl"
              >
                <div className="mb-3 flex items-center justify-between gap-3">
                  <div className="flex items-center gap-2 text-sm">
                    <span className="rounded-md border bg-muted px-2 py-1 font-medium">
                      {siteLabels[analysis.siteKind]}
                    </span>
                    <span className="text-muted-foreground">
                      {analysis.presets.length} presets
                    </span>
                  </div>
                  {analysis.warnings.length > 0 ? (
                    <span className="flex items-center gap-1.5 text-xs text-amber-700">
                      <Shield className="size-3.5" />
                      {analysis.warnings[0]}
                    </span>
                  ) : null}
                </div>

                <div className="space-y-2">
                  {analysis.presets.map((preset) => {
                    const key = advancedKey(analysis.normalizedUrl, preset.id)
                    return (
                      <PresetRow
                        key={preset.id}
                        preset={preset}
                        selected={preset.id === selectedPresetId}
                        job={
                          jobs.find(
                            (job) =>
                              job.sourceUrl === analysis.normalizedUrl &&
                              job.presetId === preset.id,
                          ) ?? null
                        }
                        outputDir={settings.defaultOutputDir}
                        auth={settings.auth}
                        advancedOptions={
                          advancedByPreset[key] ?? defaultAdvancedOptions
                        }
                        formatInfo={formatsByPreset[key] ?? null}
                        loadingFormats={loadingFormatsKey === key}
                        onSelect={() => setSelectedPresetId(preset.id)}
                        onStart={() => handleStart(preset)}
                        onCancel={(jobId) => cancelJob(jobId)}
                        onCopyLogs={copyJobLogs}
                        onAdvancedChange={(nextOptions) =>
                          handleAdvancedChange(preset, nextOptions)
                        }
                        onLoadFormats={() => handleLoadFormats(preset)}
                      />
                    )
                  })}
                </div>
              </motion.div>
            ) : recentJobs.length > 0 ? (
              <motion.div
                layout
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: 12 }}
                className="mx-auto mt-8 w-full max-w-3xl"
              >
                <div className="mb-2 text-sm font-medium">Recent</div>
                <div className="space-y-2">
                  {recentJobs.map((job) => (
                    <JobLine
                      key={job.id}
                      job={job}
                      onCopyLogs={() => copyJobLogs(job)}
                    />
                  ))}
                </div>
              </motion.div>
            ) : null}
          </AnimatePresence>

          {completedJobs.length > 0 ? (
            <motion.div layout className="mx-auto mt-8 w-full max-w-3xl">
              <div className="mb-2 flex items-center justify-between gap-3">
                <div className="text-sm font-medium">Downloaded</div>
                <div className="text-xs text-muted-foreground">
                  {completedJobs.length} items
                </div>
              </div>
              <div className="space-y-2">
                {completedJobs.map((job) => (
                  <DownloadedItem
                    key={job.id}
                    job={job}
                    onCopyLogs={() => copyJobLogs(job)}
                  />
                ))}
              </div>
            </motion.div>
          ) : null}
        </motion.section>
      </div>
    </main>
  )
}

type SettingsPanelProps = {
  settings: DownloaderSettings
  onChange: (settings: DownloaderSettings) => void
  onPickFolder: () => void
  onSave: () => void
}

function SettingsPanel({
  settings,
  onChange,
  onPickFolder,
  onSave,
}: SettingsPanelProps) {
  const authMode = settings.auth.kind
  const selectedBrowsers =
    settings.auth.kind === "browser" ? browserSources(settings.auth) : []
  const cookieFile =
    settings.auth.kind === "cookie_file" ? settings.auth.path : ""

  function setAuth(auth: AuthSource) {
    onChange({ ...settings, auth })
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
    <motion.section
      layout
      initial={{ opacity: 0, y: -8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -8 }}
      className="mx-auto mt-5 w-full max-w-3xl rounded-lg border bg-card px-4 py-4"
    >
      <div className="grid gap-3 sm:grid-cols-[1fr_auto]">
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
        <label className="space-y-1 text-xs text-muted-foreground">
          Auth
          <select
            value={authMode}
            onChange={(event) => {
              const next = event.target.value
              if (next === "browser") {
                setAuth({
                  kind: "browser",
                  browser: selectedBrowsers[0]?.browser ?? "firefox",
                  browsers:
                    selectedBrowsers.length > 0
                      ? selectedBrowsers
                      : [{ browser: "firefox" }],
                })
              }
              else if (next === "cookie_file") {
                setAuth({ kind: "cookie_file", path: cookieFile })
              } else setAuth({ kind: "none" })
            }}
            className="h-9 w-full rounded-md border bg-background px-2 text-sm text-foreground outline-none"
          >
            <option value="browser">Browser cookies</option>
            <option value="cookie_file">cookies.txt</option>
            <option value="none">None</option>
          </select>
        </label>

        <div className="space-y-1 text-xs text-muted-foreground sm:col-span-2">
          Browser fallback
          <div className="grid grid-cols-2 gap-2 rounded-md border bg-background p-2 sm:grid-cols-4">
            {browsers.map((browserName) => {
              const checked = selectedBrowsers.some(
                (source) => source.browser === browserName,
              )
              return (
                <label
                  key={browserName}
                  className={cn(
                    "flex h-8 items-center gap-2 rounded border px-2 text-xs capitalize text-foreground",
                    authMode !== "browser" && "opacity-50",
                  )}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    disabled={authMode !== "browser"}
                    onChange={(event) =>
                      setBrowserEnabled(browserName, event.target.checked)
                    }
                    className="size-3.5"
                  />
                  {browserName}
                </label>
              )
            })}
          </div>
        </div>

        <label className="space-y-1 text-xs text-muted-foreground">
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

      <div className="mt-4 flex justify-end">
        <Button type="button" onClick={onSave}>
          Save
        </Button>
      </div>
    </motion.section>
  )
}

type PresetRowProps = {
  preset: Preset
  selected: boolean
  job: Job | null
  outputDir?: string | null
  auth: AuthSource
  advancedOptions: AdvancedDownloadOptions
  formatInfo: FormatAnalysis | null
  loadingFormats: boolean
  onSelect: () => void
  onStart: () => void
  onCancel: (jobId: string) => void
  onCopyLogs: (job: Job) => void
  onAdvancedChange: (options: AdvancedDownloadOptions) => void
  onLoadFormats: () => void
}

function PresetRow({
  preset,
  selected,
  job,
  outputDir,
  auth,
  advancedOptions,
  formatInfo,
  loadingFormats,
  onSelect,
  onStart,
  onCancel,
  onCopyLogs,
  onAdvancedChange,
  onLoadFormats,
}: PresetRowProps) {
  const running = job && !["completed", "failed", "canceled"].includes(job.status)
  const canUseAuth = isAuthConfigured(auth)

  return (
    <motion.div
      layout
      className={cn(
        "overflow-hidden rounded-lg border bg-card transition-colors",
        selected && "border-foreground/25",
      )}
    >
      <button
        type="button"
        onClick={onSelect}
        className="grid w-full grid-cols-[1fr_auto] items-center gap-3 px-4 py-3 text-left"
      >
        <div className="min-w-0">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <span className="truncate text-sm font-medium">{preset.label}</span>
            <span className="rounded border bg-muted px-1.5 py-0.5 text-[11px] uppercase tracking-normal text-muted-foreground">
              video
            </span>
            <span className="rounded border bg-background px-1.5 py-0.5 text-[11px] text-muted-foreground">
              {authLabels[preset.auth]}
            </span>
          </div>
          <p className="mt-1 line-clamp-1 text-xs text-muted-foreground">
            {preset.description}
          </p>
        </div>
        <ChevronRight
          className={cn(
            "size-4 text-muted-foreground transition-transform",
            selected && "rotate-90",
          )}
        />
      </button>

      <AnimatePresence initial={false}>
        {selected ? (
          <motion.div
            layout
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            className="border-t"
          >
            <div className="space-y-3 px-4 py-4">
              {job ? (
                <JobProgress job={job} onCopyLogs={() => onCopyLogs(job)} />
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
                    <Button type="button" className="gap-2" onClick={onStart}>
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
          </motion.div>
        ) : null}
      </AnimatePresence>
    </motion.div>
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
  const videoFormats = (formatInfo?.formats ?? []).filter((format) => format.hasVideo)
  const selectedFormatId =
    options.format.kind === "format"
      ? options.format.formatId
      : options.format.kind === "video_only"
        ? (options.format.formatId ?? "")
        : ""

  function setFormat(format: AdvancedDownloadOptions["format"]) {
    onChange({ ...options, format })
  }

  function setSegment(nextSegment: NonNullable<AdvancedDownloadOptions["segment"]>) {
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
          Formats
        </Button>
      </div>

      <div className="grid grid-cols-4 gap-1 rounded-md border bg-muted p-1">
        <ModeButton
          active={options.format.kind === "best"}
          icon={<Download className="size-3.5" />}
          label="Best"
          onClick={() => setFormat({ kind: "best" })}
        />
        <ModeButton
          active={options.format.kind === "format"}
          icon={<Film className="size-3.5" />}
          label="Quality"
          onClick={() =>
            setFormat({
              kind: "format",
              formatId: videoFormats[0]?.formatId ?? selectedFormatId,
            })
          }
        />
        <ModeButton
          active={options.format.kind === "audio_only"}
          icon={<Music className="size-3.5" />}
          label="Audio"
          onClick={() => setFormat({ kind: "audio_only" })}
        />
        <ModeButton
          active={options.format.kind === "video_only"}
          icon={<Film className="size-3.5" />}
          label="Video"
          onClick={() =>
            setFormat({
              kind: "video_only",
              formatId: videoFormats[0]?.formatId ?? selectedFormatId,
            })
          }
        />
      </div>

      {options.format.kind === "format" || options.format.kind === "video_only" ? (
        <label className="mt-3 block space-y-1 text-xs text-muted-foreground">
          Quality
          <select
            value={selectedFormatId}
            onChange={(event) => {
              const formatId = event.target.value
              if (options.format.kind === "video_only") {
                setFormat({ kind: "video_only", formatId })
              } else {
                setFormat({ kind: "format", formatId })
              }
            }}
            className="h-9 w-full rounded-md border bg-card px-2 text-sm text-foreground outline-none"
          >
            {videoFormats.length === 0 ? (
              <option value="">Load formats first</option>
            ) : null}
            {videoFormats.map((format) => (
              <option key={format.formatId} value={format.formatId}>
                {format.label}
              </option>
            ))}
          </select>
        </label>
      ) : null}

      <div className="mt-3 rounded-md border bg-card p-3">
        <label className="flex items-center justify-between gap-3 text-sm">
          <span className="flex items-center gap-2 font-medium">
            <Scissors className="size-4" />
            Segment
          </span>
          <input
            type="checkbox"
            checked={Boolean(segment?.enabled)}
            onChange={(event) => enableSegment(event.target.checked)}
            className="size-4"
          />
        </label>

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
                      segment.endSeconds ?? duration ?? startSeconds,
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

function ModeButton({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean
  icon: ReactNode
  label: string
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex h-8 items-center justify-center gap-1.5 rounded px-2 text-xs",
        active ? "bg-background shadow-sm" : "text-muted-foreground",
      )}
    >
      {icon}
      {label}
    </button>
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
  const startPercent = (safeStart / safeDuration) * 100
  const endPercent = (safeEnd / safeDuration) * 100

  return (
    <div className="relative h-8">
      <div className="absolute left-0 right-0 top-3 h-2 rounded-full bg-muted" />
      <div
        className="absolute top-3 h-2 rounded-full bg-foreground"
        style={{
          left: `${startPercent}%`,
          width: `${Math.max(0, endPercent - startPercent)}%`,
        }}
      />
      <input
        type="range"
        min={0}
        max={safeDuration}
        step={0.1}
        value={safeStart}
        onChange={(event) =>
          onChange(Math.min(Number(event.target.value), safeEnd), safeEnd)
        }
        className="pointer-events-none absolute inset-x-0 top-0 h-8 w-full appearance-none bg-transparent [&::-moz-range-thumb]:pointer-events-auto [&::-moz-range-thumb]:size-4 [&::-webkit-slider-thumb]:pointer-events-auto [&::-webkit-slider-thumb]:size-4"
      />
      <input
        type="range"
        min={0}
        max={safeDuration}
        step={0.1}
        value={safeEnd}
        onChange={(event) =>
          onChange(safeStart, Math.max(Number(event.target.value), safeStart))
        }
        className="pointer-events-none absolute inset-x-0 top-0 h-8 w-full appearance-none bg-transparent [&::-moz-range-thumb]:pointer-events-auto [&::-moz-range-thumb]:size-4 [&::-webkit-slider-thumb]:pointer-events-auto [&::-webkit-slider-thumb]:size-4"
      />
    </div>
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
}: {
  job: Job
  onCopyLogs: () => void
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
            size="icon-xs"
            variant="outline"
            aria-label="Copy logs"
            onClick={onCopyLogs}
          >
            <Clipboard className="size-3" />
          </Button>
        </div>
      </div>
      <div className="mt-3 h-2 overflow-hidden rounded-full bg-muted">
        <div
          className={cn(
            "h-full rounded-full transition-all",
            failed || canceled ? "bg-destructive" : "bg-foreground",
          )}
          style={{ width: `${Math.max(0, Math.min(100, job.progress))}%` }}
        />
      </div>
      <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
        {job.speed ? <span>{job.speed}</span> : null}
        {job.eta ? <span>ETA {job.eta}</span> : null}
        {job.outputPath ? <span className="truncate">{job.outputPath}</span> : null}
        {job.errorMessage ? (
          <span className="text-destructive">{job.errorMessage}</span>
        ) : null}
      </div>
    </div>
  )
}

function JobLine({
  job,
  onCopyLogs,
}: {
  job: Job
  onCopyLogs: () => void
}) {
  return (
    <div className="grid grid-cols-[1fr_auto_auto] items-center gap-3 rounded-lg border bg-card px-4 py-3">
      <div className="min-w-0">
        <div className="truncate text-sm font-medium">
          {siteLabels[job.site]} - {job.presetId}
        </div>
        <div className="mt-1 truncate text-xs text-muted-foreground">
          {job.phase}
        </div>
      </div>
      <span className="rounded border bg-muted px-2 py-1 text-xs capitalize text-muted-foreground">
        {job.status}
      </span>
      <Button
        type="button"
        size="icon-xs"
        variant="outline"
        aria-label="Copy logs"
        onClick={onCopyLogs}
      >
        <Clipboard className="size-3" />
      </Button>
    </div>
  )
}

function DownloadedItem({
  job,
  onCopyLogs,
}: {
  job: Job
  onCopyLogs: () => void
}) {
  const [thumbnail, setThumbnail] = useState<{
    sourcePath: string
    thumbnailPath: string | null
  } | null>(null)
  const previewUrl = job.outputPath ? pathToPreviewUrl(job.outputPath) : null
  const thumbnailPath =
    thumbnail && thumbnail.sourcePath === job.outputPath
      ? thumbnail.thumbnailPath
      : null
  const thumbnailUrl = thumbnailPath ? localFilePreviewUrl(thumbnailPath) : null

  useEffect(() => {
    let canceled = false
    if (!job.outputPath || !isLikelyVideoPath(job.outputPath)) return

    const sourcePath = job.outputPath
    createVideoThumbnail(sourcePath)
      .then((path) => {
        if (!canceled) setThumbnail({ sourcePath, thumbnailPath: path })
      })
      .catch(() => undefined)

    return () => {
      canceled = true
    }
  }, [job.outputPath])

  return (
    <div className="grid gap-3 rounded-lg border bg-card p-3 sm:grid-cols-[168px_1fr_auto]">
      <button
        type="button"
        disabled={!job.outputPath}
        onClick={() => job.outputPath && revealOutputPath(job.outputPath)}
        className="relative flex aspect-video items-center justify-center overflow-hidden rounded-md border bg-muted text-left disabled:cursor-default"
      >
        {thumbnailUrl ? (
          <img
            src={thumbnailUrl}
            className="h-full w-full object-cover"
            alt=""
            draggable={false}
          />
        ) : previewUrl ? (
          <video src={previewUrl} className="h-full w-full object-cover" muted />
        ) : (
          <Download className="size-5 text-muted-foreground" />
        )}
        <span className="absolute bottom-2 right-2 flex size-6 items-center justify-center rounded bg-background/90 text-muted-foreground shadow-sm">
          <FolderOpen className="size-3.5" />
        </span>
      </button>

      <div className="min-w-0 self-center">
        <div className="truncate text-sm font-medium">
          {siteLabels[job.site]} - {job.presetId}
        </div>
        <div className="mt-1 truncate text-xs text-muted-foreground">
          {job.outputPath ?? job.sourceUrl}
        </div>
        <div className="mt-2 flex flex-wrap gap-2 text-xs text-muted-foreground">
          <span className="rounded border bg-muted px-1.5 py-0.5">video</span>
          <span className="rounded border bg-muted px-1.5 py-0.5">
            completed
          </span>
        </div>
      </div>

      <div className="flex items-start justify-end gap-2 sm:flex-col">
        {job.outputPath ? (
          <>
            <Button
              type="button"
              size="icon-xs"
              variant="outline"
              aria-label="Show in folder"
              onClick={() => revealOutputPath(job.outputPath ?? "")}
            >
              <FolderOpen className="size-3" />
            </Button>
            <Button
              type="button"
              size="icon-xs"
              variant="outline"
              aria-label="Open in default app"
              onClick={() => openOutputPath(job.outputPath ?? "")}
            >
              <Play className="size-3" />
            </Button>
            <Button
              type="button"
              size="icon-xs"
              variant="outline"
              aria-label="Copy file path"
              onClick={() => copyText(job.outputPath ?? "")}
            >
              <Copy className="size-3" />
            </Button>
          </>
        ) : null}
        <Button
          type="button"
          size="icon-xs"
          variant="outline"
          aria-label="Copy logs"
          onClick={onCopyLogs}
        >
          <Clipboard className="size-3" />
        </Button>
      </div>
    </div>
  )
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

function advancedKey(url: string, presetId: string): string {
  return `${url}::${presetId}`
}

function normalizeAdvancedForDuration(
  options: AdvancedDownloadOptions,
  duration: number | null,
): AdvancedDownloadOptions {
  if (!options.segment?.enabled || !duration) return options
  const startSeconds = clamp(options.segment.startSeconds, 0, duration)
  const endSeconds = clamp(options.segment.endSeconds ?? duration, startSeconds, duration)
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
    right.updatedAt.localeCompare(left.updatedAt),
  )
}

export default App
