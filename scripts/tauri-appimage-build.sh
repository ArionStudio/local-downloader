#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

should_patch=false
for arg in "$@"; do
  case "$arg" in
    appimage | *appimage*) should_patch=true ;;
  esac
done

if [[ "$should_patch" != "true" ]]; then
  pnpm tauri "$@"
  exit 0
fi

find src-tauri/target -type d -path '*/release/bundle/appimage' -prune -exec rm -rf {} + 2>/dev/null || true

set +e
pnpm tauri "$@"
build_status=$?
set -e

find_appimagetool() {
  if command -v appimagetool >/dev/null 2>&1; then
    command -v appimagetool
    return
  fi

  local tools_dir="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/downloader-appimage-tools"
  local appimage="$tools_dir/appimagetool-x86_64.AppImage"
  local extracted_dir="$tools_dir/appimagetool"
  local apprun="$extracted_dir/squashfs-root/AppRun"

  if [[ ! -x "$apprun" ]]; then
    rm -rf "$extracted_dir"
    mkdir -p "$tools_dir" "$extracted_dir"
    curl -fsSL \
      -o "$appimage" \
      "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
    chmod +x "$appimage"
    (
      cd "$extracted_dir"
      APPIMAGELAUNCHER_DISABLE=1 "$appimage" --appimage-extract >/dev/null
    )
  fi

  echo "$apprun"
}

patch_apprun() {
  local appdir="$1"

  cat >"$appdir/AppRun" <<'APPRUN'
#!/usr/bin/env bash
set -e

appdir="$(readlink -f "$(dirname "$0")")"

unset LD_LIBRARY_PATH
export XDG_DATA_DIRS="$appdir/usr/share:/usr/share:${XDG_DATA_DIRS:-}"

cd "$appdir/usr"
exec "$appdir/usr/bin/Downloader" "$@"
APPRUN

  chmod 0755 "$appdir/AppRun"
}

patch_host_runtime() {
  local appdir="$1"
  local executable="$appdir/usr/bin/Downloader"

  if [[ ! -x "$executable" ]]; then
    echo "AppImage executable not found: $executable" >&2
    exit 1
  fi

  patchelf --remove-rpath "$executable"
  if [[ -n "$(patchelf --print-rpath "$executable")" ]]; then
    echo "Failed to remove the AppImage executable RUNPATH." >&2
    exit 1
  fi
}

app_config_value() {
  local expression="$1"
  node -e "const config = require('./src-tauri/tauri.conf.json'); console.log($expression);"
}

default_appimage_name() {
  local app_name version arch
  app_name="$(app_config_value 'config.productName')"
  version="$(app_config_value 'config.version')"

  case "$(uname -m)" in
    x86_64) arch="amd64" ;;
    i?86) arch="i386" ;;
    aarch64 | arm64) arch="aarch64" ;;
    *) arch="$(uname -m)" ;;
  esac

  echo "${app_name}_${version}_${arch}.AppImage"
}

mapfile -t appdirs < <(find src-tauri/target -type d -path '*/release/bundle/appimage/*.AppDir' | sort)
if (( ${#appdirs[@]} == 0 )); then
  if (( build_status != 0 )); then
    exit "$build_status"
  fi

  echo "No AppDir found under src-tauri/target." >&2
  exit 1
fi

if (( build_status != 0 )); then
  echo "Tauri AppImage bundling failed, continuing with AppDir repack." >&2
fi

appimagetool="$(find_appimagetool)"

for appdir in "${appdirs[@]}"; do
  appdir="$(realpath "$appdir")"
  bundle_dir="$(realpath "$(dirname "$appdir")")"
  mapfile -t appimages < <(find "$bundle_dir" -maxdepth 1 -type f -name '*.AppImage' | sort)

  if (( ${#appimages[@]} == 0 )); then
    appimage="$bundle_dir/$(default_appimage_name)"
  elif (( ${#appimages[@]} == 1 )); then
    appimage="${appimages[0]}"
  else
    echo "Expected at most one AppImage in $bundle_dir, found ${#appimages[@]}." >&2
    exit 1
  fi
  patched="$appimage.patched"

  echo "Patching AppImage launcher: $appimage"
  patch_apprun "$appdir"
  patch_host_runtime "$appdir"

  rm -f "$patched"
  ARCH=x86_64 "$appimagetool" "$appdir" "$patched"
  mv "$patched" "$appimage"

  if [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
    rm -f "$appimage.sig"
    pnpm tauri signer sign "$appimage"
    test -s "$appimage.sig"
  else
    rm -f "$appimage.sig"
    echo "TAURI_SIGNING_PRIVATE_KEY is not set; skipped AppImage updater signature." >&2
  fi
done
