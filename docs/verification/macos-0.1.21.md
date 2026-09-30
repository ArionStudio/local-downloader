# macOS verification for Downloader 0.1.21

Verification ran on an Apple Silicon MacBook Air with macOS 26.6.2 using SSH, native Accessibility controls, and window screenshots. The checklist includes requests from the Downloader UI thread and the media requirements in “Design Admin Hub Access Control.” Tests used an isolated CLI state directory and generated media/cookies; existing user downloads were preserved.

The final native headless test run passed **72 unit tests and 10 CLI integration tests**, including real downloads, cookies, preparation, and cancellation. Frontend production build and lint also passed.

## Coverage

| Area | Evidence |
| --- | --- |
| Native tool installation | The released CLI installed macOS yt-dlp and ARM64 FFmpeg/ffprobe. File architecture, executable versions, and checksums were checked. |
| CLI contract | Help, schema, presets, URL analysis, formats, JSON requests, settings persistence/import, history, file hashes, invalid inputs, and timeout results exercised on the Mac. |
| Download options | Direct and embedded-page downloads, custom filenames, audio-only, video-only, and time ranges exercised with a real audio/video source. Video-only previously retained audio on combined sources; the regression test now probes each ready file and requires a video stream without audio. |
| Cookies | A generated Firefox profile exported a host-scoped Netscape file with mode 0600. Both browser-profile and cookie-file downloads accessed a controlled authenticated source. Unauthenticated access produced no files. No personal cookie contents were logged. |
| XRBAZAAR | A 1920×1080, 60 fps source was downloaded and prepared as 1280×720, 30 fps H.264/yuv420p MP4 with AAC audio and faststart. FFprobe, full decoding, size checks, and source hashes verified the result and original preservation. |
| Cancellation | A two-file job announced the first file while the second was downloading. Cancellation from another CLI process retained the first file in history. Concurrent batches sharing a data directory were rejected. Native regression tests cover these behaviors. |
| Desktop download | A real download was started through the Mac UI and appeared in Files with Open video, Show in folder, Copy path, and Logs actions. |
| Preview posters | Actual Mac window inspection confirmed visible thumbnail images before playback. Previews load as they approach the visible area. |
| Playback | Native video playback works. Changing mute in one player updates another player. Shared volume defaults to 10%; changing it to 11% through keyboard controls survived an app restart. The setting was restored to 10% after verification. |
| Interface | Download, Activity, Files, and Settings use the existing aligned layouts. The obsolete “Start with a link” block is absent. File actions and the branded XRBAZAAR action remain available. |
| Distribution | The release workflow builds separate CLI archives for both Mac CPUs, both Linux CPUs, and Windows x64, with SHA-256 manifests, the CLI guide, and Node adapter. Mac binaries are ad-hoc signed, not notarized. |

## External access limits

Controlled-source tests do not prove every third-party website is accessible. The public Vimeo sample required login. The Reddit sample reached the bounded timeout. A YouTube sample failed inside yt-dlp/FFmpeg; yt-dlp also reported that no supported JavaScript runtime was available. Those are failed or blocked public-source checks, not successful downloads. The app now retains useful source error messages instead of replacing them with a generic process exit code.

The YouTube catalogue timeout/cancellation path was exercised. A complete authenticated channel export and quota rotation against live YouTube Data API credentials were not verified. Adding and removing a dummy API key through the native UI succeeded. The SSH session could not save a dummy key because macOS Keychain disallowed interaction; CLI key storage still depends on the logged-in account's unlocked credential vault. No valid API key or subscription credentials were supplied for those tests. Protected LinkedIn, Crunchyroll, and other account-specific sources cannot be claimed as end-to-end passes.

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
