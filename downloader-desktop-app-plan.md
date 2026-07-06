# Downloader Desktop App Plan

## Summary
Build a local desktop downloader app with **Tauri v2 + React + Vite + TypeScript + shadcn/ui Base UI**. The app’s main screen is a large URL/search input with matching presets listed below. Selecting a preset fluidly expands into download setup/progress on the same screen, with no modal-based workflow.

Primary platforms:
- macOS Apple Silicon DMG
- Linux x86_64 AppImage as the first-class artifact for Ubuntu and CachyOS
- In-app update button through Tauri updater + GitHub Releases

## Confirmed Decisions
- Frontend: React, Vite, TypeScript, shadcn/ui, Base UI.
- Package manager: `pnpm`.
- UI: single fluid screen, no sidebar-first layout, no popup-heavy flow.
- Sites in v1: Generic, Reddit, LinkedIn.
- Auth in v1: browser-cookie import plus manual `cookies.txt`.
- Resolver depth in v1: `yt-dlp` first, then HTTP/HTML resolver for media URLs and `m3u8`/`mpd`; no headless browser automation.
- Tools: bundle `yt-dlp` and `ffmpeg`; support separate tool updates through an app-managed signed/checksummed tool manifest.
- History: local sanitized history.
- Linux install/update: AppImage primary because it best matches the in-app update requirement.

## Setup Choice From The Third Screenshot
Use the shadcn setup as:
- Mode: `New Project`
- Template: `Vite`, not Next.js
- Base: `Base UI`
- Package manager: `pnpm`
- Use pointer on buttons: `on`
- Create monorepo: `off`
- RTL support: `off`

Implementation command path:
```bash
pnpm dlx shadcn@latest init -t vite
pnpm add -D @tauri-apps/cli
pnpm tauri init
```

Tauri init answers:
- App name: `Downloader`
- Window title: `Downloader`
- Web assets: `../dist`
- Dev server: `http://localhost:5173`
- Frontend dev command: `pnpm dev`
- Frontend build command: `pnpm build`

Add shadcn components:
```bash
pnpm dlx shadcn@latest add button input input-group badge progress scroll-area separator checkbox switch native-select tooltip skeleton label
pnpm add lucide-react framer-motion
```

## UI Design
Main app states:
1. `Idle`
   - Centered URL/search input.
   - Small top-right status cluster: app update, tools status, settings icon.
   - Recent sanitized downloads below if any.

2. `Analyzed`
   - Input moves upward with a smooth layout transition.
   - Preset rows appear below, similar to the screenshot list style.
   - Rows show site, preset name, expected output, auth requirement, and a chevron/action button.

3. `Configure`
   - Selected preset expands inline.
   - Shows output folder, filename preview, quality/profile options, auth source, and start button.
   - No modal. Folder picker may use the native OS dialog only when explicitly clicked.

4. `Running`
   - Same row becomes a progress row.
   - Shows percentage, speed, ETA, current phase, cancel button, and compact logs underneath.

5. `Completed/Failed`
   - Completed row shows output path, reveal button, retry button.
   - Failed row shows short error plus expandable sanitized logs.

Use `framer-motion` layout animations for transitions between states.

Visual rules:
- Compact SaaS/tooling style inspired by the screenshots.
- White/light neutral UI, restrained borders, 6-8px radius.
- No large marketing hero.
- No card-inside-card layout.
- Use lucide icons for search, folder, download, cancel, refresh, settings, alert, check.

## Frontend Structure
```text
src/
  app/
    App.tsx
    providers.tsx
  components/
    search-url-input.tsx
    preset-list.tsx
    preset-row.tsx
    job-row.tsx
    inline-auth-selector.tsx
    output-folder-picker.tsx
    update-status-button.tsx
    tool-status-button.tsx
  lib/
    api.ts
    types.ts
    url.ts
    format.ts
```

Frontend state:
- Use React reducer for the main screen state.
- No global state library in v1.
- Tauri events update job progress.

## Backend Structure
```text
src-tauri/src/
  lib.rs
  commands.rs
  storage.rs
  redaction.rs
  download/
    engine.rs
    jobs.rs
    progress.rs
    presets.rs
    sites/
      mod.rs
      generic.rs
      reddit.rs
      linkedin.rs
    pipelines/
      yt_dlp.rs
      ffmpeg.rs
      http_resolver.rs
  tools/
    locator.rs
    updater.rs
    manifest.rs
  auth.rs
```

## Public Types
Core frontend/backend contract:

```ts
type SiteKind = "generic" | "reddit" | "linkedin" | "direct_hls" | "direct_file";

type AuthSource =
  | { kind: "none" }
  | { kind: "browser"; browser: "chrome" | "chromium" | "brave" | "edge" | "firefox" | "safari"; profile?: string }
  | { kind: "cookie_file"; path: string };

type Preset = {
  id: string;
  siteKinds: SiteKind[];
  label: string;
  description: string;
  outputKind: "video";
  pipeline: "yt_dlp" | "ffmpeg_hls" | "http_resolve_then_download";
  auth: "none" | "optional" | "recommended" | "required";
};

type JobStatus = "queued" | "resolving" | "downloading" | "postprocessing" | "completed" | "failed" | "canceled";
```

Tauri commands:
```ts
analyze_url(input: { url: string }): Promise<AnalyzeResult>
start_download(input: StartDownloadRequest): Promise<Job>
cancel_job(input: { jobId: string }): Promise<void>
list_jobs(): Promise<Job[]>
get_job(input: { jobId: string }): Promise<JobDetail>
select_download_dir(): Promise<string | null>
check_app_update(): Promise<AppUpdate | null>
install_app_update(): Promise<void>
check_tool_updates(): Promise<ToolUpdate[]>
install_tool_update(input: { tool: "yt-dlp" | "ffmpeg" }): Promise<void>
get_settings(): Promise<Settings>
update_settings(input: Partial<Settings>): Promise<Settings>
```

Events:
```ts
"download:job-event"
"tools:update-event"
"app:update-event"
```

## Presets
v1 hardcoded presets. Presets describe page/post shape; every preset targets the highest quality video available.

Generic:
- `generic-page-video-highest`
- `generic-direct-stream-highest`

Reddit:
- `reddit-post-video-highest`
- `reddit-multiple-media-highest`

LinkedIn:
- `linkedin-post-video-highest`
- `linkedin-article-video-highest`
- `linkedin-feed-update-video-highest`

X:
- `x-article-video-highest`

Vimeo:
- `vimeo-video-highest`

Sawhorse:
- `sawhorse-portfolio-video-highest`

All presets generate arguments from typed config. Users cannot enter arbitrary `yt-dlp` or `ffmpeg` arguments in v1.

## Download Pipeline
Order:
1. Detect site by URL host/path.
2. Show matching presets immediately.
3. On start, resolve using selected preset:
   - Try `yt-dlp` with generated args.
   - If preset is HLS/direct or `yt-dlp` fails with unsupported media, run HTTP resolver.
   - HTTP resolver fetches public HTML and extracts `.m3u8`, `.mpd`, `<video src>`, `og:video`.
   - Download direct HLS with `yt-dlp` or `ffmpeg`.
4. Stream stdout/stderr into parser.
5. Emit sanitized progress/log events to UI.
6. Persist final sanitized job record.

Cookie handling:
- Use `yt-dlp --cookies-from-browser ...` for browser import.
- Use `yt-dlp --cookies ...` for manual cookie file.
- Never store cookie contents.
- Redact cookie args, cookie paths, auth headers, and resolved signed media URLs from logs.

## Storage
Use Rust-managed SQLite via `rusqlite`.

Tables:
```sql
jobs(id, created_at, updated_at, status, site, preset_id, title, source_url, output_path, progress, speed, eta, error_code, error_message)
job_logs(id, job_id, created_at, level, message)
settings(key, value_json)
```

Do not persist:
- cookies
- request headers
- resolved direct media URLs
- raw command strings containing auth options

Default output directory:
- user Downloads folder.

## Tool Management
Bundle target-specific tools:
```text
src-tauri/binaries/
  yt-dlp-x86_64-unknown-linux-gnu
  ffmpeg-x86_64-unknown-linux-gnu
  yt-dlp-aarch64-apple-darwin
  ffmpeg-aarch64-apple-darwin
```

Runtime priority:
1. Use updated tool from app data if checksum is valid.
2. Fall back to bundled binary.

Tool updates:
- App reads `tools-manifest.json` from GitHub Releases or a static GitHub-hosted file.
- Manifest contains tool name, version, target triple, URL, SHA256.
- Download to temp file, verify SHA256, chmod executable, atomically replace.

## Distribution And Updates
GitHub Releases will host:
- macOS Apple Silicon DMG
- Linux x86_64 AppImage
- Tauri updater metadata/signatures
- tool manifest and tool binaries when changed

App update UI:
- Inline status button, not popup.
- “Update available” expands inline with release notes, progress, restart button.

Linux:
- AppImage is the v1 primary artifact.
- Add an in-app “Install to Applications” action that copies the AppImage to a user-local app folder and creates a desktop entry.
- `.deb` is out of scope for v1 because the update-button requirement is more important.

macOS:
- Apple Silicon only.
- For non-technical distribution, Developer ID signing and notarization are required. Without this, macOS users will see trust/Gatekeeper friction.

## Testing
Frontend:
- Vitest for URL parsing, reducer states, preset filtering.
- React Testing Library for search, preset selection, auth selector, progress row.
- Playwright screenshots for main screen at desktop and narrow widths.

Rust:
- Unit tests for site detection.
- Unit tests for preset-to-command argument generation.
- Unit tests for log redaction.
- Unit tests for progress parsing.
- HTTP resolver tests using local HTML fixtures.
- Storage tests using temp SQLite DB.

Integration:
- Fake `yt-dlp` and `ffmpeg` scripts that emit known progress and failures.
- Local fixture server serving HTML + m3u8 test streams.
- Manual smoke tests for one public Reddit URL, one direct m3u8 URL, and one LinkedIn URL with browser cookies.

Acceptance criteria:
- App runs with `pnpm tauri dev`.
- User can paste URL, see presets below, select preset, start download, cancel, retry.
- Progress updates without freezing UI.
- Cookies are never printed in logs.
- App builds AppImage on Linux.
- App update check appears inline.
- Tool update check appears inline.

## Out Of Scope For v1
- Windows builds.
- Headless browser/network-capture scraping.
- Arbitrary user-provided downloader arguments.
- Cloud sync.
- Multi-user profiles.
- Store publishing.
- `.deb` as primary Linux install path.

## Sources Used
- Tauri create project: https://v2.tauri.app/start/create-project/
- Tauri Vite frontend config: https://v2.tauri.app/start/frontend/vite/
- Tauri sidecar/external binaries: https://v2.tauri.app/develop/sidecar/
- Tauri updater: https://v2.tauri.app/plugin/updater/
- Tauri GitHub release pipeline: https://v2.tauri.app/distribute/pipelines/github/
- shadcn Vite install: https://ui.shadcn.com/docs/installation/vite
- yt-dlp cookies FAQ: https://github.com/yt-dlp/yt-dlp/wiki/FAQ
