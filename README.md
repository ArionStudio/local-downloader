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

The current app can run real downloads when `yt-dlp` is available on the system path or provided later as a bundled/updated tool. `ffmpeg` is detected the same way and passed to `yt-dlp` when present.

Crunchyroll support uses `yt-dlp` with the user's configured browser cookies or cookies.txt file. It does not include the upstream project's Widevine/PlayReady CDM or DRM-decryption paths.

## Test Links

Seed URLs for resolver work are stored in [docs/example-links.md](docs/example-links.md).
