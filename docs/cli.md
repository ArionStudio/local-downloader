# Downloader CLI and agent integration

`downloader-cli` runs the desktop app's download engine without a window, display server, or WebKit dependency. It supports the same presets, cookie handling, stream selection, trimming, channel catalogues, tool installers, and XRBAZAAR preparation. Every operation returns JSON; downloads can stream job events as newline-delimited JSON.

## Download and install

Download a standalone binary from [GitHub Releases](https://github.com/ArionStudio/local-downloader/releases/latest). No Rust, Node, source checkout, or desktop app is required. The following links and commands pin **0.1.22**, so your agent can reproduce its installation.

| Machine | Release archive |
| --- | --- |
| macOS Apple Silicon | [aarch64-apple-darwin.tar.gz](https://github.com/ArionStudio/local-downloader/releases/download/app-v0.1.22/downloader-cli-0.1.22-aarch64-apple-darwin.tar.gz) |
| macOS Intel | [x86_64-apple-darwin.tar.gz](https://github.com/ArionStudio/local-downloader/releases/download/app-v0.1.22/downloader-cli-0.1.22-x86_64-apple-darwin.tar.gz) |
| Linux x64 | [x86_64-unknown-linux-gnu.tar.gz](https://github.com/ArionStudio/local-downloader/releases/download/app-v0.1.22/downloader-cli-0.1.22-x86_64-unknown-linux-gnu.tar.gz) |
| Linux ARM64 | [aarch64-unknown-linux-gnu.tar.gz](https://github.com/ArionStudio/local-downloader/releases/download/app-v0.1.22/downloader-cli-0.1.22-aarch64-unknown-linux-gnu.tar.gz) |
| Windows x64 | [x86_64-pc-windows-msvc.zip](https://github.com/ArionStudio/local-downloader/releases/download/app-v0.1.22/downloader-cli-0.1.22-x86_64-pc-windows-msvc.zip) |

Use macOS 14 or newer, Linux with glibc 2.35 or newer, or Windows 10/11 x64. Linux needs the D-Bus runtime library, typically already installed on desktops (`libdbus-1-3` on Debian/Ubuntu). Alpine/musl and Windows ARM64 native binaries are not included. macOS builds are native for each CPU; Apple Silicon does not need Rosetta.

### macOS and Linux

Run this in Terminal. It detects your OS and CPU, downloads the matching archive, verifies its checksum, and installs into your user account without sudo:

```bash
(
  set -eu
  version=0.1.22
  case "$(uname -s)" in
    Darwin) platform=apple-darwin ;;
    Linux) platform=unknown-linux-gnu ;;
    *) echo 'Use the Windows instructions below.' >&2; exit 1 ;;
  esac
  # Detect Apple Silicon even if Terminal itself is running under Rosetta.
  machine="$(uname -m)"
  if [ "$platform" = apple-darwin ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then
    machine=arm64
  fi
  case "$machine" in
    arm64|aarch64) architecture=aarch64 ;;
    x86_64|amd64) architecture=x86_64 ;;
    *) echo "Unsupported CPU: $machine" >&2; exit 1 ;;
  esac
  asset="downloader-cli-$version-$architecture-$platform.tar.gz"
  base="https://github.com/ArionStudio/local-downloader/releases/download/app-v$version"
  temporary="$(mktemp -d)"
  cd "$temporary"
  curl --fail --location --retry 3 --output "$asset" "$base/$asset"
  curl --fail --location --retry 3 --output "$asset.sha256" "$base/$asset.sha256"
  if [ "$platform" = apple-darwin ]; then
    shasum -a 256 -c "$asset.sha256"
  else
    sha256sum -c "$asset.sha256"
  fi
  tar -xzf "$asset"
  mkdir -p "$HOME/.local/bin"
  install -m 755 downloader-cli "$HOME/.local/bin/downloader-cli"
  "$HOME/.local/bin/downloader-cli" --version
  echo "Guide and adapter extracted to: $temporary"
)
export PATH="$HOME/.local/bin:$PATH"
downloader-cli tools install all
```

Add `export PATH="$HOME/.local/bin:$PATH"` to `~/.zshrc` on macOS or your Linux shell configuration to keep it for later terminals. Agents launched as services should use the absolute executable path or configure their own PATH.

Mac binaries are ad-hoc signed, not Apple notarized. If a browser-downloaded executable is blocked, try running it, then allow that specific executable under **System Settings → Privacy & Security → Open Anyway** and retry. Do not disable Gatekeeper globally. Browser-cookie access may separately request Keychain permission; run under the account that owns the browser profile.

### Windows

Run in PowerShell:

```powershell
$ErrorActionPreference = 'Stop'
$version = '0.1.22'
$asset = "downloader-cli-$version-x86_64-pc-windows-msvc.zip"
$base = "https://github.com/ArionStudio/local-downloader/releases/download/app-v$version"
$temporary = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $temporary | Out-Null
$archive = Join-Path $temporary $asset
Invoke-WebRequest "$base/$asset" -OutFile $archive
Invoke-WebRequest "$base/$asset.sha256" -OutFile "$archive.sha256"
$expected = ((Get-Content "$archive.sha256" -Raw).Trim() -split '\s+')[0]
if ((Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
    throw 'Download checksum mismatch'
}
$destination = Join-Path $env:LOCALAPPDATA 'Programs\downloader-cli'
Expand-Archive $archive -DestinationPath $destination -Force
$env:Path = "$destination;$env:Path"
downloader-cli --version
downloader-cli tools install all
```

The executable is `%LOCALAPPDATA%\Programs\downloader-cli\downloader-cli.exe`. Configure that absolute path in your agent, or add its directory to your user PATH in Windows Environment Variables. The executable is not Authenticode signed; Windows may show a publisher warning. The ZIP also contains `docs\cli.md` and `examples\adminhub-downloader.mjs`.

### Download tools and check setup

Runtime downloads require yt-dlp; merging and XRBAZAAR preparation require FFmpeg and ffprobe. `tools install all` downloads and verifies the correct native versions into CLI-managed storage. Existing tools on PATH are also supported. Homebrew, Python, and a separate FFmpeg installation are not required for this setup.

```bash
downloader-cli tools platform
downloader-cli tools status
downloader-cli tools install all
```

`ffmpeg` installation includes ffprobe. Installers select and validate binaries for the executing machine's OS and architecture. macOS uses [Martin Riedl's FFmpeg builds](https://ffmpeg.martin-riedl.de/), Linux/Windows use [BtbN builds](https://github.com/BtbN/FFmpeg-Builds), and yt-dlp comes from its [upstream releases](https://github.com/yt-dlp/yt-dlp/releases). Downloads are checksum-verified before execution.

Browser cookies must be accessible to the OS account running the CLI. An SSH session or background agent may need access to that account's unlocked Keychain or Linux Secret Service. Cookie export does not transfer decryption keys between accounts or systems.

## Agent setup checklist

Give your agent this guide and the following task:

> Integrate `downloader-cli` using the standalone binary from this repository's GitHub release `app-v0.1.22`. Detect the machine's OS and CPU, download the matching CLI archive, verify its SHA-256 checksum, and install the executable. Do not build from source. Run `tools install all`, then discover capabilities with `schema`, `presets`, and `--help`. Invoke the executable directly, send download requests as JSON through stdin, and parse newline-delimited events plus the final result even on nonzero exit. Configure cookie access explicitly for the account running the agent. Preserve completed files after cancellation. For AdminHub, use the included Node adapter or implement its download-then-prepare flow; upload only verified prepared files through the existing AdminHub client. Read `CHANGELOG.md` at the repository/archive root before upgrading, and implement the update policy and user-requested update flow in this guide.

1. Run `--version`, `tools platform`, and `tools status`; install missing tools with `tools install all`.
2. Choose a persistent private CLI data directory with `DOWNLOADER_DATA_DIR`. Each concurrently running batch needs its own directory.
3. Read `schema` and `presets`. Select browser cookies, a cookie file, or explicit no-cookie mode for each request.
4. Try one selected video, check the returned file exists, and verify its metadata before connecting the upload step.
5. Handle progress, timeout, cancellation, and partial success as described below. Keep the returned originals and source URLs in the agent's research record.
6. Read the repository-root [CHANGELOG.md](../CHANGELOG.md). It is also at `CHANGELOG.md` in each extracted CLI archive and is a separate GitHub release asset. Follow [Updates for agents](#updates-for-agents); support requests such as “check for Downloader updates” and “update Downloader to the latest stable release.”

No Node runtime is required for direct CLI integration. The optional adapter below uses Node built-ins; it has no npm dependencies.

## Updates for agents

The agent updates the installed **Downloader CLI**, not its own agent software. The CLI has no built-in self-update command. Replace its standalone executable using verified GitHub release assets. Desktop updates are separate and can be installed through **Settings → App update**.

### When to check and update

- On initial integration, record `downloader-cli --version`, the executable path, the release tag, and its checksum.
- On “check for updates,” compare the installed version with the latest stable release and summarize the relevant `CHANGELOG.md` entries. This request only checks; it does not replace the binary.
- On “update Downloader” or a request for a specific release, carry out the upgrade below. That request authorizes the update; do not ask for the same permission again.
- Keep routine runs pinned to the recorded version. Do not silently replace a running download process. If the user has explicitly enabled automatic updates, check at agent startup or before a new batch, at most once per day, and apply a verified stable update while idle.
- If a download fails, inspect the actual source/tool error first. Check release notes for a matching fix; do not repeatedly upgrade or retry as a substitute for authentication.

### Find the release and changelog

The public GitHub endpoint returns `tag_name`, `draft`, `prerelease`, `html_url`, and `assets` with exact names and download URLs:

```sh
curl --fail --silent --show-error \
  https://api.github.com/repos/ArionStudio/local-downloader/releases/latest
```

PowerShell equivalent:

```powershell
$release = Invoke-RestMethod 'https://api.github.com/repos/ArionStudio/local-downloader/releases/latest'
$release.tag_name
$release.html_url
```

Parse the JSON rather than executing it. Require a stable `app-vMAJOR.MINOR.PATCH` tag and compare numeric version components. Read `CHANGELOG.md` from the matching release asset or tagged repository revision. If the installed version is already current, report that and leave it installed. An offline check or GitHub rate limit should retain the working version and report that the check could not complete.

### Apply an authorized update

1. Wait until this CLI installation has no active batch. Record the current binary path and version. Keep a copy of the old executable for rollback; preserve the CLI state directory, browser profiles, cookie files, and downloads.
2. Select the matching OS/CPU archive from the release's `assets`. Use the same detection as [Download and install](#download-and-install), including Apple Silicon detection under Rosetta. Pin the selected tag for the entire update so metadata and checksums cannot refer to different releases.
3. Download the archive and its matching `.sha256` file to a temporary directory. Verify the checksum before extracting or running the new binary. Use the commands above with `version` set to the selected release number.
4. Before replacement, run the extracted binary with `--version`, `tools platform`, and `schema` using a temporary `--data-dir`. Require the expected version, machine architecture, and supported `schemaVersion`. On macOS, also run `codesign --verify --strict ./downloader-cli`.
5. Replace only the agent's configured executable. On macOS/Linux, install to a temporary sibling such as `downloader-cli.new`, then rename it over the old executable while idle. On Windows, stop processes using that executable before replacing it. Keep the extracted `CHANGELOG.md`, `docs/cli.md`, and adapter alongside the agent's integration materials.
6. Run the installed binary with `--version`, `schema`, and `tools status`. Install missing tools with `tools install all`; tool updates are separate from CLI replacement. Perform a small authorized download or preparation check, then report the old/new versions, release link, changelog changes, and check results.
7. If verification fails, restore the previous executable and report the failed check. Do not delete state, cookies, or output files. For a release that documents a state migration, follow its migration/rollback instructions before attempting a binary downgrade.

Example staging check on macOS/Linux, from the extracted archive:

```sh
check_state="$(mktemp -d)"
./downloader-cli --version
./downloader-cli --data-dir "$check_state" tools platform
./downloader-cli --data-dir "$check_state" schema
```

This is the same flow for a user-requested update and an explicitly enabled automatic update. Never source scripts from release notes or disable OS security controls to make an update pass.

## Downloads

```bash
downloader-cli presets
downloader-cli analyze 'https://example.com/project'
downloader-cli formats 'https://example.com/project' --browser firefox

downloader-cli download 'https://example.com/project' \
  --browser firefox --browser 'chrome:Default' --output ./media --events

downloader-cli download 'https://example.com/video.mp4' \
  --profile xrbazaar --no-cookies --output ./media

downloader-cli download 'https://example.com/project' --audio-only
downloader-cli download 'https://example.com/project' --video-only --format-id 137
downloader-cli download 'https://example.com/project' --start 12.5 --end 24
```

Other flags include `--preset`, `--filename-template`, `--cookie-file`, and `--timeout SECONDS`. Use `download --help` for the complete list. Multiple positional URLs run as a batch. Channel catalogues combine all supplied channel URLs into one named export:

```bash
downloader-cli download 'https://youtube.com/@first' 'https://youtube.com/@second' \
  --preset youtube-channel-catalogue --export-name research --catalogue-content all
```

Catalogue content can be `all`, `videos`, or `shorts`. Completed exports are not overwritten; interrupted exports retain their checkpoints.

## Cookies and settings

`--browser NAME[:PROFILE]` reads that browser's local cookies. Repeated flags define ordered fallbacks. Browser aliases, including Zen and Helium, use the same profile discovery as the desktop app. Browser cookie values are never printed in CLI results.

```bash
downloader-cli cookies browsers
downloader-cli cookies export 'https://www.example.com/project' \
  --browser firefox --output ./example-cookies.txt
downloader-cli download 'https://www.example.com/project' --cookie-file ./example-cookies.txt
```

Export creates a new file containing only cookies applicable to the requested host. It never overwrites an existing file. On Unix it creates the file with mode 0600. Treat it as a credential file. Exporting is optional; downloads can read the browser directly. `--no-cookies` disables saved fallback too. Sites that require authentication will report an error instead of silently using cookies.

CLI jobs and settings live under the platform data directory in `dev.local.downloader.cli`. Set `DOWNLOADER_DATA_DIR` or pass `--data-dir` for another location. The CLI refuses the default desktop state directory. Copy the desktop configuration explicitly:

```bash
downloader-cli settings import-desktop
downloader-cli settings show
downloader-cli settings set --request settings.json
```

Import copies the download folder, cookie configuration, and opaque YouTube API-key IDs. It leaves desktop jobs untouched. `--from DIRECTORY` selects another desktop data directory. Key IDs still refer to the current OS account's credential vault, so copying them to another machine does not copy the keys.

```bash
downloader-cli keys list
downloader-cli keys add --stdin < /private/path/youtube-api-key.txt
downloader-cli keys remove KEY_ID
```

Secret key values are accepted only through stdin and saved in the same OS credential vault used by the app. Do not put them into agent prompts or command arguments.

## Agent JSON contract

`schema` returns JSON Schemas for the complete desktop download request, settings, and job objects. This includes every request option, including ordered browser profiles and catalogue settings.

```bash
downloader-cli schema
downloader-cli download --request request.json --events
# Or pipe JSON into: downloader-cli download --request - --events
```

Example request:

```json
{
  "url": "https://example.com/project",
  "presetId": "generic-page-video-highest",
  "outputDir": "/absolute/path/media",
  "outputProfile": "xrbazaar",
  "allowSavedAuth": true,
  "auth": {
    "kind": "browser",
    "browsers": [{ "browser": "firefox", "profile": null }]
  },
  "advanced": {
    "format": { "kind": "best" },
    "segment": { "enabled": false, "startSeconds": 0, "endSeconds": null }
  }
}
```

XRBAZAAR requires the best video with original audio, so custom stream selection and audio-only requests are rejected for that profile. Set `allowSavedAuth: false` and `auth: {"kind":"none"}` to forbid cookie access in a JSON request.

Without `--events`, stdout contains one result envelope:

```json
{
  "schemaVersion": 1,
  "type": "result",
  "ok": true,
  "exitCode": 0,
  "result": { "jobs": [], "files": [] }
}
```

With `--events`, preceding lines have `type: "job"`, with the current `job` and an optional `log`. `job.readyPaths` updates as each finished file becomes available, including during another video's conversion. The final envelope includes file paths, byte lengths, MIME types, and SHA-256 hashes. Failed operations return `ok: false`; failed or canceled downloads retain their job and file results. Parse the final envelope even when the process exits nonzero.

| Exit | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Operation or request validation failed |
| 2 | Command-line syntax is invalid |
| 124 | Download or preparation timeout |
| 130 | Download or preparation canceled |

`--json` is accepted globally for integrations that pass it explicitly. Help and version commands print ordinary text. The CLI uses no shell to invoke external tools. yt-dlp configuration files are ignored so the supplied request and saved app settings determine cookie access and output behavior.

## XRBAZAAR and AdminHub

```bash
downloader-cli prepare ./source.mp4 --events
```

Preparation writes a separate file and verifies MP4, H.264/AVC, yuv420p, original aspect ratio without upscaling, a longest edge no larger than 1280 pixels, at most 30 FPS, fast-start metadata, and a maximum size of 100 MiB. Audio is preserved when present as AAC at 48 kHz, mono or stereo, below the 128 kbps limit. Verification includes decoding the resulting video. Original files remain intact.

`prepare` returns a verified `file` with `path`, `contentType`, `byteLength`, and `sha256`. An XRBAZAAR **download** returns both originals and prepared copies; use `prepare` when an integration needs an unambiguous verified output.

[The Node adapter](../examples/adminhub-downloader.mjs) exposes `runDownloader` and `downloadForAdminHub`. The latter downloads originals and prepares each locally, returning only verified copies in `files`, with originals listed separately. Pass a complete video request and an optional `onEvent` callback or `AbortSignal`. The adapter invokes the executable directly and sends request JSON through stdin.

For example, from the extracted release directory or repository root with Node 20 or newer, replace `VIDEO_URL` with your selected source:

```bash
node --input-type=module - 'VIDEO_URL' <<'JS'
import { resolve } from 'node:path'
import { downloadForAdminHub } from './examples/adminhub-downloader.mjs'

try {
  const result = await downloadForAdminHub({
    url: process.argv[2],
    presetId: 'generic-page-video-highest',
    outputDir: resolve('media'),
    allowSavedAuth: false,
    auth: { kind: 'none' },
  }, {
    executable: process.env.DOWNLOADER_BIN || 'downloader-cli',
    signal: AbortSignal.timeout(30 * 60 * 1000),
    onEvent: event => console.error(JSON.stringify(event)),
  })
  console.log(JSON.stringify(result, null, 2))
} catch (error) {
  // A failed or canceled download can still have finished files.
  console.error(JSON.stringify({ message: error.message, result: error.result }))
  process.exitCode = error.exitCode || 1
}
JS
```

For authenticated sources, change `auth` to `{"kind":"browser","browsers":[{"browser":"firefox","profile":null}]}` or `{"kind":"cookie_file","path":"/private/path/cookies.txt"}`. Explicit auth still works with `allowSavedAuth: false`; that field only disables fallback to saved settings. The adapter inherits `DOWNLOADER_DATA_DIR` and the running account's environment. You can copy the adapter into your agent project and adjust its import path.

For the AdminHub agent workflow discussed in “Design Admin Hub Access Control,” keep the media source and selected gallery order in the agent's research record. Pass each verified file's content type, byte length, and hash to the existing `/agent/media/videos` operation. Upload the bytes through the returned signed storage form, complete and inspect that operation, and carry its returned signed lease into the next gallery operation. The CLI prepares local files; it does not authenticate to AdminHub, upload, or publish an artwork. The agent retains its existing permission, reservation, operation ID, and publication workflow.

## Progress and cancellation

```bash
downloader-cli history list
downloader-cli history show JOB_ID --logs
downloader-cli files --hashes
downloader-cli files --job JOB_ID
downloader-cli cancel JOB_ID
```

Ctrl+C and termination signals cancel a running download or preparation. `cancel` targets a CLI download in the same data directory. Finished files remain in history after cancellation, failure, and restart. Partial downloads are not reported as ready. `--timeout` provides a bounded run for agents.

Only one download batch runs per CLI data directory. History reads and cancellation work from another process. Use separate `--data-dir` directories for parallel batches. The desktop app and CLI do not cancel one another's jobs. GUI-only functions such as in-app playback, window settings, and desktop self-updates are not CLI operations; agents receive local file paths instead.

## Build from source for contributors

Users and agents should install the release binaries above. To change the CLI itself, use a current stable Rust toolchain and a native C/C++ compiler. Linux also needs `pkg-config` and D-Bus development headers. From the repository root:

```bash
cargo build --release --locked --manifest-path src-tauri/Cargo.toml \
  --no-default-features --bin downloader-cli
```

The result is `src-tauri/target/release/downloader-cli`, or `downloader-cli.exe` on Windows. Node, pnpm, GTK, and WebKit are not required for this headless build.
