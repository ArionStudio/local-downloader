# Downloader

Local desktop downloader app built with Tauri, React, Vite, TypeScript, and shadcn/ui Base UI.

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

Presets describe page shapes such as LinkedIn post, LinkedIn article, Reddit post, Reddit multiple media, or Crunchyroll video. All presets target the highest quality video available.

YouTube channel links (`/@handle`, `/channel/...`, `/c/...`, and `/user/...`) also offer a **YouTube Channel Catalogue** preset. Paste one or several channel links into the main input; they are grouped into one result and one **Export all channels** action. Before starting, enter an export name and choose **All**, **Videos only**, or **Shorts only**; the combined catalogue is written to `youtube_export/<export name>/youtube_videos.json` and `youtube_export/<export name>/youtube_videos.xlsx`. A completed export with the same name is never overwritten. The export reads the selected Videos and/or Shorts tabs, excludes livestream tabs, and checkpoints completed video metadata so interrupted runs can resume.

YouTube Data API keys can be added under **Settings → YouTube Data API keys**. Secret values are stored by the operating-system credential vault (Secret Service on Linux, Keychain on macOS, and Credential Manager on Windows); only opaque key IDs are kept in app settings. Multiple keys are rotated between API batches and tried in sequence when a key is invalid, rate-limited, or out of quota. Without a saved key, the same export schema is populated through the slower yt-dlp metadata fallback.

The current app can run real downloads when `yt-dlp` is available on the system path or provided later as a bundled/updated tool. `ffmpeg` is detected the same way and passed to `yt-dlp` when present.

Crunchyroll support uses `yt-dlp` with the user's configured browser cookies or cookies.txt file. It does not include the upstream project's Widevine/PlayReady CDM or DRM-decryption paths.

## Test Links

Seed URLs for resolver work are stored in [docs/example-links.md](docs/example-links.md).
