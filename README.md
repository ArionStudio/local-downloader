# Downloader

Local desktop downloader app built with Tauri, React, Vite, TypeScript, and shadcn/ui Base UI.

## Command line and agents

A headless `downloader-cli` uses the same download, browser-cookie, catalogue, and XRBAZAAR preparation engine. It runs without the desktop app or a display server and returns JSON for agent integrations.

**[Download the standalone CLI from GitHub Releases](https://github.com/ArionStudio/local-downloader/releases/latest).** Choose the `downloader-cli` archive for macOS Apple Silicon, macOS Intel, Linux x64/ARM64, or Windows x64. You do not need Rust, Node, or the desktop app to run it.

**Give your agent [the CLI integration guide](docs/cli.md#agent-setup-checklist).** It includes direct release downloads, installation commands, browser cookies and profiles, all download options, JSON schemas and progress events, cancellation, and the AdminHub upload handoff. A dependency-free [Node adapter](examples/adminhub-downloader.mjs) is included in each archive.

After [downloading and installing the CLI](docs/cli.md#download-and-install):

```bash
downloader-cli tools install all
downloader-cli schema
downloader-cli download 'VIDEO_URL' --browser firefox --profile xrbazaar --events
```

Each release includes SHA-256 checksums. Mac binaries are ad-hoc signed; [installation instructions](docs/cli.md#macos-and-linux) cover first-run approval when macOS requests it.

## Development

```bash
pnpm install
pnpm dev
```

Open the web preview at `http://localhost:5173/`.

For the desktop shell:

```bash
pnpm tauri dev
```

## Checks

```bash
pnpm build
pnpm lint
cd src-tauri && cargo check
```

## Build

```bash
pnpm tauri build --no-bundle
```

GitHub releases include ad-hoc-signed builds for Apple Silicon and Intel Macs. They do not require an Apple Developer membership, but users must approve the app under **System Settings → Privacy & Security → Open Anyway** on first launch. See [docs/macos-signing.md](docs/macos-signing.md) for release and installation details.

Presets describe page shapes such as a general page video, LinkedIn post, LinkedIn article, Reddit post, Reddit multiple media, or Crunchyroll video. All presets target the highest quality video available.

The default **Page Video** preset combines the reusable behavior from the specialized downloaders: native HTML video, structured video metadata, embedded players, direct media, HLS/DASH manifests, escaped page data, extractor page dumps, optional cookies, highest-quality selection, and standard `yt-dlp` extraction. Provider-specific API workflows remain separate, but every regular video site also offers **Page Video** as a fallback.

YouTube channel links (`/@handle`, `/channel/...`, `/c/...`, and `/user/...`) also offer a **YouTube Channel Catalogue** preset. Paste one or several channel links into the main input; they are grouped into one result and one **Export all channels** action. Before starting, enter an export name and choose **All**, **Videos only**, or **Shorts only**; the combined catalogue is written to `youtube_export/<export name>/youtube_videos.json` and `youtube_export/<export name>/youtube_videos.xlsx`. Each exported item includes a `content_type` value of `video` or `short`, and Shorts use their `youtube.com/shorts/...` link. A completed export with the same name is never overwritten. The export reads the selected Videos and/or Shorts tabs, excludes livestream tabs, and checkpoints completed video metadata so interrupted runs can resume.

YouTube Data API keys can be added under **Settings → YouTube Data API keys**. Secret values are stored by the operating-system credential vault (Secret Service on Linux, Keychain on macOS, and Credential Manager on Windows); only opaque key IDs are kept in app settings. Multiple keys are rotated between API batches and tried in sequence when a key is invalid, rate-limited, or out of quota. Without a saved key, the same export schema is populated through the slower yt-dlp metadata fallback.

The current app can run real downloads when `yt-dlp` is available on the system path or provided later as a bundled/updated tool. `ffmpeg` is detected the same way and passed to `yt-dlp` when present.

Crunchyroll support uses `yt-dlp` with the user's configured browser cookies or cookies.txt file. It does not include the upstream project's Widevine/PlayReady CDM or DRM-decryption paths.

## Test Links

Seed URLs for resolver work are stored in [docs/example-links.md](docs/example-links.md).
