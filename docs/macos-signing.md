# macOS Ad-Hoc Releases

This project publishes macOS builds without a paid Apple Developer Program membership. The release workflow uses Apple's ad-hoc signing identity (`-`), which is required for reliable Apple Silicon binaries but does not identify a verified developer and does not notarize the app.

No Apple certificate, Team ID, Apple ID, or notarization secrets are required.

## Release Behavior

The workflow builds separate installers for:

- Apple Silicon Macs (`aarch64-apple-darwin`);
- Intel Macs (`x86_64-apple-darwin`).

It verifies that each app bundle:

- has a valid ad-hoc code signature;
- contains the expected CPU architecture;
- was packaged successfully as a DMG and updater archive.

Releases also include standalone `downloader-cli` archives for Apple Silicon and Intel. Each CLI is built and tested on a runner with the matching CPU, checked for Homebrew library dependencies, ad-hoc signed, extracted from its archive, and executed again before publication. CLI archives include their integration guide and Node adapter. See [CLI installation](cli.md#macos-and-linux) for download commands and first-run approval.

All platform artifacts remain in a draft GitHub release until every build and integrity check succeeds. The Tauri updater artifacts remain cryptographically signed with the existing, non-Apple updater secrets:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

## Gatekeeper Limitation

Ad-hoc signing is not Developer ID signing. Apple Gatekeeper will not identify the app as coming from a verified developer, and users must approve it manually the first time they open it.

After copying **Downloader** to Applications:

1. Try to open the app and dismiss the Apple verification warning.
2. Open **System Settings → Privacy & Security**.
3. Scroll to the Security section and click **Open Anyway** for Downloader.
4. Confirm **Open**. macOS remembers the choice for that app.

Do not tell users to disable Gatekeeper globally or run `xattr -cr` as the normal installation method.

## Publish a Release

Increment the version in `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, and the downloader entry in `src-tauri/Cargo.lock`. Add release notes at `docs/releases/<version>.md`, update the pinned download links in `docs/cli.md`, commit, and push the matching tag. For example, version `0.1.19` uses:

```bash
git tag app-v0.1.19
git push origin app-v0.1.19
```

Only after every desktop and CLI platform succeeds does the workflow upload all CLI archives, checksums, the guide, and adapter, then publish the draft release with the version's release notes.

For an independent package check, run the **mac release diagnostics** workflow and enter the new tag. It automatically downloads and validates every Mac DMG and updater tarball. This validates packaging and the ad-hoc signature; it does not and cannot prove Gatekeeper acceptance.

## Optional Future Upgrade

If the project later joins the paid Apple Developer Program, replace ad-hoc signing with a `Developer ID Application` certificate and Apple notarization. That removes the manual Gatekeeper approval for users.

## References

- [Tauri macOS code signing and ad-hoc signing](https://v2.tauri.app/distribute/sign/macos/)
- [Apple: safely open apps on your Mac](https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unidentified-developer-mh40616/mac)
- [Apple Developer membership comparison](https://developer.apple.com/support/compare-memberships/)
