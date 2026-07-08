# macOS Signing and Notarization

macOS releases are built by `.github/workflows/release.yml`. The workflow signs with an Apple Developer ID certificate when the Apple secrets below are configured. If `APPLE_CERTIFICATE` is missing, it falls back to ad-hoc signing for internal testing.

## Required Apple Account

Use a paid Apple Developer Program account. The certificate type for distribution outside the Mac App Store is `Developer ID Application`.

## Create the Certificate

1. On a Mac, open Keychain Access.
2. Use Certificate Assistant to create a Certificate Signing Request.
3. In Apple Developer Certificates, IDs & Profiles, create a `Developer ID Application` certificate with that CSR.
4. Download and open the `.cer` file so it appears in Keychain Access under `My Certificates`.
5. Expand the certificate, confirm the private key is present, then export the certificate plus private key as a `.p12` file.
6. Give the `.p12` export a strong password.

Convert the `.p12` to a single-line base64 value:

```bash
openssl base64 -A -in DeveloperIDApplication.p12 -out apple-certificate-base64.txt
```

Find the signing identity locally:

```bash
security find-identity -v -p codesigning
```

The workflow finds the `Developer ID Application` identity automatically after importing the `.p12`.

## GitHub Secrets

Add these repository secrets:

```text
APPLE_CERTIFICATE            contents of apple-certificate-base64.txt
APPLE_CERTIFICATE_PASSWORD   password used when exporting the .p12
KEYCHAIN_PASSWORD            random CI-only keychain password
APPLE_ID                     Apple Developer account email
APPLE_PASSWORD               app-specific password for the Apple ID
APPLE_TEAM_ID                Apple Developer Team ID
```

Keep the existing Tauri updater signing secrets:

```text
TAURI_SIGNING_PRIVATE_KEY
TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

## Release and Verify

After adding secrets, create the next `app-v*` tag to run the release workflow.

Then run the `mac release diagnostics` workflow against the new tag. A notarized release should show:

```text
codesign --verify ... exit status: 0
spctl -a -vv --type execute ... accepted
TeamIdentifier=<your team id>
Signature=...
```

If `spctl` still says `rejected`, open the release job logs and search for notarization, staple, or Apple credential errors.

## References

- Tauri macOS signing: https://v2.tauri.app/distribute/sign/macos/
- Apple notarization overview: https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution
- Apple Developer ID: https://developer.apple.com/developer-id/
