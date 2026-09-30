# macOS verification for Downloader 0.1.22

Verification ran on an Apple Silicon MacBook Air with macOS 26.6.2 using SSH, native Accessibility controls, and window screenshots. The checklist includes requests from the Downloader UI thread and the media requirements in “Design Admin Hub Access Control.” Tests used an isolated CLI state directory and generated media/cookies; existing user downloads were preserved.

The final native headless test run passed **73 unit tests and 10 CLI integration tests**, including real downloads, cookies, preparation, and cancellation. Frontend production build and lint also passed.

## Coverage

| Area | Evidence |
| --- | --- |
| Native tool installation | The released CLI installed macOS yt-dlp and ARM64 FFmpeg/ffprobe. File architecture, executable versions, and checksums were checked. |
| CLI contract | Help, schema, presets, URL analysis, formats, JSON requests, settings persistence/import, history, file hashes, invalid inputs, and timeout results exercised on the Mac. |
| Download options | Direct and embedded-page downloads, custom filenames, audio-only, video-only, and time ranges exercised with a real audio/video source. Video-only previously retained audio on combined sources; the regression test now probes each ready file and requires a video stream without audio. |
| Cookies | A generated Firefox profile exported a host-scoped Netscape file with mode 0600. Both browser-profile and cookie-file downloads accessed a controlled authenticated source. Unauthenticated access produced no files. No personal cookie contents were logged. |
| XRBAZAAR | A 1920×1080, 60 fps source was downloaded and prepared as 1280×720, 30 fps H.264/yuv420p MP4 with AAC audio and faststart. FFprobe, full decoding, size checks, and source hashes verified the result and original preservation. |
| Cancellation | Both the native UI and CLI announced the first file while a second file was still downloading. Cancellation from another CLI process retained the first file in history. Concurrent batches sharing a data directory were rejected. Native regression tests cover these behaviors. |
| Desktop download | Real original and XRBAZAAR downloads were started through the Mac UI. Open video launched QuickTime, Show in folder opened Finder at the output folder, and Copy path placed the exact path on the clipboard. |
| Preview posters | Actual Mac window inspection confirmed visible thumbnail images before playback, including a 0.2-second video and its XRBAZAAR copy. Previews load as they approach the visible area. |
| Playback | Native video playback works. Seeking worked, and changing mute in one player updated another player. Shared volume defaults to 10%; changing it to 11% through keyboard controls survived an app restart. The setting was restored to 10% after verification. |
| Interface | Download, Activity, Files, and Settings use the existing aligned layouts. Both light and system/dark themes were inspected. The obsolete “Start with a link” block is absent. File actions and the branded XRBAZAAR action remain available. |
| Node adapter | The Mac adapter downloaded and prepared a source, forwarded job/progress events, and returned a verified MP4 with upload metadata while retaining the original. |
| Distribution | The release workflow builds separate CLI archives for both Mac CPUs, both Linux CPUs, and Windows x64, with SHA-256 manifests, the CLI guide, and Node adapter. Mac binaries are ad-hoc signed, not notarized. |

## Published release and deployment

[Release 0.1.22](https://github.com/ArionStudio/local-downloader/releases/tag/app-v0.1.22) was published on September 30, 2026 after [all nine build/test jobs and publication checks passed](https://github.com/ArionStudio/local-downloader/actions/runs/36653540179). It contains 25 distribution assets, including all five standalone CLI archives, checksums, `CHANGELOG.md`, and the agent guide. The published changelog and guide match the release source.

The installed Mac app updated from 0.1.20 to 0.1.22 using **Settings → Install app update**. The updater downloaded, verified, installed, and launched the new version. The installed executable matches the published ARM64 app archive byte for byte; the GitHub asset digest and strict code signature verification also passed.

The standalone CLI was installed at `~/.local/bin/downloader-cli`, with the changelog, guide, and adapter under `~/.local/share/downloader-cli/`. The release checksum, native architecture, signature, version, and schema were verified before replacement. The installed CLI then installed native tools into its own data directory and passed these checks:

| Installed-release check | Result |
| --- | --- |
| Download and XRBAZAAR preparation | Preserved the 11,420,575-byte original and returned an 876,122-byte prepared MP4, H.264/yuv420p, 1280×720, 30 fps, AAC 48 kHz. |
| Public YouTube HTTPS trim | Completed successfully; FFprobe measured exactly 3.000 seconds, with H.264 video and AAC 48 kHz audio. Certificate verification remained enabled. |
| Native app launch | The installed window rendered version 0.1.22 and retained seven history entries and 37 completed files, including QA downloads. |

The Mac locked its screen during the final deployment checks. This was confirmed through `CGSSessionScreenIsLocked`, rather than inferred from an automation error. Screen capture had confirmed the installed app's initial window, but the final interactive check of the **published** app's previews could not continue while locked. The earlier native UI checks used the same preview/audio implementation and passed before the lock. An unlocked Mac is required to finish that final interactive check.

Desktop-clarke was offline on Tailscale, so deployment there was not performed. Existing Mac user downloads were preserved; the download directory was restored to Downloads, theme to System, and preview volume to 10% before the lock. QA API keys were removed. The previous app bundle was retained as a rollback copy in the private QA cache.

## External access limits

Controlled-source tests do not prove every third-party website is accessible. The public Vimeo sample required login. The Reddit sample reached the bounded timeout. A public YouTube video downloaded successfully in full. Its initial trimmed attempt exposed a missing CA bundle in standalone macOS FFmpeg; configuring the system certificate bundle fixed the same three-second trim. yt-dlp also warned that no supported JavaScript runtime was available, so some YouTube formats may remain unavailable. Vimeo and Reddit are blocked/failed checks, not successful downloads. The app now retains useful source error messages instead of replacing them with a generic process exit code.

The YouTube catalogue timeout/cancellation path was exercised. A complete authenticated channel export and quota rotation against live YouTube Data API credentials were not verified. Adding and removing a dummy API key through the native UI succeeded. The SSH session could not save a dummy key because macOS Keychain disallowed interaction; CLI key storage still depends on the logged-in account's unlocked credential vault. For SSH agents, macOS may also deny access to Downloads even when the desktop app can use it. Use an explicitly writable `--output` directory or grant the automation host the appropriate folder access. No valid API key or subscription credentials were supplied for those tests. Protected LinkedIn, Crunchyroll, and other account-specific sources cannot be claimed as end-to-end passes.

The CLI prepares media locally. AdminHub upload and publication remain the integrating agent's responsibility; this release does not change AdminHub backend behavior.

## Repeating the native CLI checks

With native yt-dlp, FFmpeg, and ffprobe on PATH:

```sh
pnpm build
pnpm lint
cargo test --locked --manifest-path src-tauri/Cargo.toml \
  --no-default-features -- --include-ignored --test-threads=1
```

Tests run sequentially because packaged yt-dlp startup on the Mac was slow enough for parallel subprocess tests to exhaust their deadlines. The cancellation test keeps a bounded 120-second timeout.

To verify the desktop, use the built Mac app with a combined audio/video file. Open Files, check the poster before playing, play and seek, change audio in one preview, open another preview, then restart the app and check Settings → Preview audio. Start a two-file download, wait for the first ready file, cancel, and confirm that file remains accessible.
