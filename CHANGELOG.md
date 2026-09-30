# Changelog

Changes are listed newest first. Desktop and standalone CLI releases share the `app-vVERSION` tag. Installation, agent integration, and update instructions are in [docs/cli.md](docs/cli.md#updates-for-agents).

## 0.1.22 — 2026-09-30

- Fix HTTPS trimming on macOS by supplying the system certificate bundle to FFmpeg without disabling TLS verification.
- Show thumbnail posters before video playback and load previews as they become visible.
- Start preview audio at 10%; share volume and mute across videos and remember them after restart. Add Settings → Preview audio.
- Remove audio from video-only downloads when the source contains combined video/audio streams. Announce readiness after processing.
- Show source error messages, including login requirements, instead of only a generic exit code.
- Add native Mac verification and a regression test for video-only output. Run subprocess integration tests sequentially with bounded cancellation timeouts.
- Include this changelog in every standalone CLI archive and as a release asset. Document agent update checks, user-requested updates, verification, and rollback.

[Release assets and notes](https://github.com/ArionStudio/local-downloader/releases/tag/app-v0.1.22) · [Mac verification](docs/verification/macos-0.1.22.md)

## 0.1.20 — 2026-09-29

- Publish standalone CLI archives for macOS Apple Silicon/Intel, Linux x64/ARM64, and Windows x64, with checksums, an integration guide, and a Node adapter.
- Expose download options, browser cookies/profiles, settings, channel catalogues, history, cancellation, and XRBAZAAR preparation through the headless CLI.
- Select and verify native yt-dlp, FFmpeg, and ffprobe for the executing OS and CPU.
- Improve desktop preview playback and access to downloaded files; display completed files during a batch and retain them after cancellation.
- Add the branded XRBAZAAR action, preserve originals, and create separate verified prepared videos.
- Reorganize Download, Activity, Files, and Settings and remove the introductory empty-state block.

[Release assets and notes](https://github.com/ArionStudio/local-downloader/releases/tag/app-v0.1.20)

Older releases predate this maintained changelog. See [GitHub Releases](https://github.com/ArionStudio/local-downloader/releases) for their published history.
