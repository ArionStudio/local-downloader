# Downloader CLI and agent integration

`downloader-cli` runs the desktop app's download engine without a window, display server, or WebKit dependency. It supports the same presets, cookie handling, stream selection, trimming, channel catalogues, tool installers, and XRBAZAAR preparation. Every operation returns JSON; downloads can stream job events as newline-delimited JSON.

## Build and install

The CLI is currently distributed as source on `t3code/improve-user-interface`, not as a standalone GitHub release asset. These shell commands are for Linux/macOS; Linux builds and runtime behavior have been verified. Use a current stable Rust toolchain and a native C/C++ compiler. Linux also needs `pkg-config` and D-Bus development headers (Debian/Ubuntu packages: `build-essential`, `pkg-config`, `libdbus-1-dev`). Node, pnpm, GTK, and WebKit are not needed to build the headless binary.

```bash
git clone --branch t3code/improve-user-interface --single-branch \
  https://github.com/ArionStudio/local-downloader.git
cd local-downloader
cargo build --release --locked --manifest-path src-tauri/Cargo.toml \
  --no-default-features --bin downloader-cli
mkdir -p "$HOME/.local/bin"
install -m755 src-tauri/target/release/downloader-cli ~/.local/bin/downloader-cli
export PATH="$HOME/.local/bin:$PATH"
downloader-cli --version
git rev-parse HEAD
```

Record the commit printed by `git rev-parse HEAD` and pin that revision in your integration. If you already cloned the repository, check out the feature branch and build from its root. For an agent launched by a service, configure its PATH or pass the absolute executable path; interactive shell PATH changes may not reach the service.

Linux uses D-Bus and the OS secret service for browser-cookie decryption and API keys. Runtime downloads require yt-dlp; merging and XRBAZAAR preparation require FFmpeg and ffprobe. Browser cookies must be accessible to the account running the CLI. An SSH session may need access to that account's unlocked credential vault.

```bash
downloader-cli tools platform
downloader-cli tools status
downloader-cli tools install all
```

`ffmpeg` installation includes ffprobe. Installers select and validate binaries for the executing machine's OS and architecture.

## Agent setup checklist

Give your agent this guide and the following task:

> Integrate `downloader-cli` from this repository's `t3code/improve-user-interface` branch. Build the headless binary using the instructions above and pin the source commit. Discover capabilities with `schema`, `presets`, and `--help`. Invoke the executable directly, send download requests as JSON through stdin, and parse newline-delimited events plus the final result even on nonzero exit. Configure cookie access explicitly for the account running the agent. Preserve completed files after cancellation. For AdminHub, use the included Node adapter or implement its download-then-prepare flow; upload only verified prepared files through the existing AdminHub client.

1. Run `--version`, `tools platform`, and `tools status`; install missing tools with `tools install all`.
2. Choose a persistent private CLI data directory with `DOWNLOADER_DATA_DIR`. Each concurrently running batch needs its own directory.
3. Read `schema` and `presets`. Select browser cookies, a cookie file, or explicit no-cookie mode for each request.
4. Try one selected video, check the returned file exists, and verify its metadata before connecting the upload step.
5. Handle progress, timeout, cancellation, and partial success as described below. Keep the returned originals and source URLs in the agent's research record.

No Node runtime is required for direct CLI integration. The optional adapter below uses Node built-ins; it has no npm dependencies.

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

For example, from the repository root with Node 20 or newer, replace `VIDEO_URL` with your selected source:

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
