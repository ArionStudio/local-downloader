use crate::runtime::Runtime;
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const USER_AGENT: &str = concat!("Downloader/", env!("CARGO_PKG_VERSION"));
const YT_DLP_CHECKSUMS_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS";
const FFMPEG_CHECKSUMS_URL: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/checksums.sha256";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUpdate {
    pub tool: String,
    pub status: String,
    pub current_version: Option<String>,
    pub available_version: Option<String>,
    pub path: Option<String>,
    pub message: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPlatform {
    os: &'static str,
    arch: &'static str,
    yt_dlp_asset: Option<&'static str>,
    ffmpeg_asset: Option<&'static str>,
}

pub fn tool_platform() -> ToolPlatform {
    ToolPlatform {
        os: env::consts::OS,
        arch: env::consts::ARCH,
        yt_dlp_asset: yt_dlp_asset().ok().map(|asset| asset.name),
        ffmpeg_asset: ffmpeg_asset().ok().map(|asset| asset.name),
    }
}

pub fn find_tool(app: &Runtime, name: &str) -> Option<PathBuf> {
    let candidates = updated_tool_path(app, name)
        .into_iter()
        .chain(bundled_tool_path(app, name))
        .chain(path_candidates(name));
    find_working_tool(name, candidates)
}

fn find_working_tool(name: &str, candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates
        .into_iter()
        .find(|path| is_executable(path) && tool_version_from_path(name, path).is_some())
}

pub fn has_available_impersonation_target(app: &Runtime) -> bool {
    let Some(tool) = find_tool(app, "yt-dlp") else {
        return false;
    };
    has_available_impersonation_target_at(&tool)
}

pub fn install_tool_update(app: &Runtime, tool: &str) -> Result<(), String> {
    match tool {
        "yt-dlp" => install_yt_dlp(app),
        "ffmpeg" => install_ffmpeg(app),
        other => Err(format!("Unsupported tool: {other}")),
    }
}

fn has_available_impersonation_target_at(tool: &Path) -> bool {
    let Ok(output) = Command::new(tool)
        .arg("--list-impersonate-targets")
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| !line.contains("(unavailable)") && line.contains("curl_cffi"))
}

pub fn check_tool_updates(app: &Runtime) -> Vec<ToolUpdate> {
    ["yt-dlp", "ffmpeg"]
        .iter()
        .map(|tool| {
            let path = find_tool(app, tool);
            let current = path
                .as_ref()
                .and_then(|tool_path| tool_version_from_path(tool, tool_path));

            if let Some(tool_path) = path {
                let unsupported_impersonation =
                    *tool == "yt-dlp" && !has_available_impersonation_target_at(&tool_path);
                let (status, message) = if *tool == "ffmpeg" && find_tool(app, "ffprobe").is_none() {
                    ("unsupported", "FFmpeg is installed, but ffprobe is missing. Reinstall the FFmpeg package to enable XRBAZAAR verification.".to_string())
                } else if unsupported_impersonation {
                    (
                        "unsupported",
                        "Installed, but Reddit/LinkedIn impersonation support is unavailable. Install the app-managed yt-dlp so the PATH copy is not used.".to_string(),
                    )
                } else {
                    (
                        "installed",
                        "Installed and available to the downloader.".to_string(),
                    )
                };

                ToolUpdate {
                    tool: tool.to_string(),
                    status: status.to_string(),
                    current_version: current,
                    available_version: None,
                    path: Some(tool_path.display().to_string()),
                    message,
                }
            } else {
                ToolUpdate {
                    tool: tool.to_string(),
                    status: "missing".to_string(),
                    current_version: None,
                    available_version: None,
                    path: None,
                    message: "Missing. Downloads that require this tool will fail until it is installed or found on the system path.".to_string(),
                }
            }
        })
        .collect()
}

fn tool_version_from_path(tool: &str, path: &Path) -> Option<String> {
    let version_arg = if matches!(tool, "ffmpeg" | "ffprobe") {
        "-version"
    } else {
        "--version"
    };
    let output = Command::new(path).arg(version_arg).output().ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout)
        .ok()
        .and_then(|version| version.lines().next().map(str::to_string))
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty())
}

struct DownloadAsset {
    name: &'static str,
    url: String,
}

struct FfmpegAsset {
    name: &'static str,
    url: String,
    archive_root: &'static str,
}

fn install_yt_dlp(app: &Runtime) -> Result<(), String> {
    let asset = yt_dlp_asset()?;
    let tools_dir = app_tools_dir(app)?;
    fs::create_dir_all(&tools_dir).map_err(|error| error.to_string())?;
    let download_path = temp_path(&tools_dir, "yt-dlp.download");
    let result = (|| {
        download_and_verify(&asset.url, YT_DLP_CHECKSUMS_URL, asset.name, &download_path)?;
        set_executable(&download_path)?;
        verify_download(&download_path, &["--version"], "yt-dlp")?;
        if !has_available_impersonation_target_at(&download_path) {
            return Err("Downloaded yt-dlp has no browser impersonation support. The existing installation was kept.".to_string());
        }
        replace_file(&download_path, &updated_tool_path_result(app, "yt-dlp")?)
    })();
    let _ = fs::remove_file(download_path);
    result
}

fn install_ffmpeg(app: &Runtime) -> Result<(), String> {
    let asset = ffmpeg_asset()?;
    let tools_dir = app_tools_dir(app)?;
    fs::create_dir_all(&tools_dir).map_err(|error| error.to_string())?;

    let archive_path = temp_path(&tools_dir, "ffmpeg.tar.xz");
    let extract_dir = temp_path(&tools_dir, "ffmpeg.extract");
    let staged_path = temp_path(&tools_dir, "ffmpeg.download");
    let staged_probe = temp_path(&tools_dir, "ffprobe.download");

    let result = (|| {
        download_and_verify(&asset.url, FFMPEG_CHECKSUMS_URL, asset.name, &archive_path)?;
        fs::create_dir_all(&extract_dir).map_err(|error| error.to_string())?;

        let status = Command::new("tar")
            .arg("-xJf")
            .arg(&archive_path)
            .arg("-C")
            .arg(&extract_dir)
            .status()
            .map_err(|error| format!("Could not run tar to extract ffmpeg: {error}"))?;
        if !status.success() {
            return Err(format!(
                "Could not extract ffmpeg archive: tar exited with {status}"
            ));
        }

        let extracted = extract_dir
            .join(asset.archive_root)
            .join("bin")
            .join(tool_executable_name("ffmpeg"));
        if !extracted.is_file() {
            return Err("Downloaded ffmpeg archive did not contain bin/ffmpeg.".to_string());
        }

        fs::copy(&extracted, &staged_path).map_err(|error| error.to_string())?;
        set_executable(&staged_path)?;

        let final_path = updated_tool_path_result(app, "ffmpeg")?;
        verify_download(&staged_path, &["-version"], "ffmpeg")?;
        let probe = extract_dir
            .join(asset.archive_root)
            .join("bin")
            .join(tool_executable_name("ffprobe"));
        fs::copy(&probe, &staged_probe).map_err(|e| format!("Could not prepare ffprobe: {e}"))?;
        set_executable(&staged_probe)?;
        verify_download(&staged_probe, &["-version"], "ffprobe")?;
        replace_file(&staged_probe, &updated_tool_path_result(app, "ffprobe")?)?;
        replace_file(&staged_path, &final_path)
    })();

    let _ = fs::remove_file(&archive_path);
    let _ = fs::remove_file(&staged_path);
    let _ = fs::remove_file(&staged_probe);
    let _ = fs::remove_dir_all(&extract_dir);

    result
}

fn yt_dlp_asset() -> Result<DownloadAsset, String> {
    yt_dlp_asset_for(env::consts::OS, env::consts::ARCH)
}

fn yt_dlp_asset_for(os: &str, arch: &str) -> Result<DownloadAsset, String> {
    let name = match (os, arch) {
        ("linux", "x86_64") => "yt-dlp_linux",
        ("linux", "aarch64") => "yt-dlp_linux_aarch64",
        ("macos", "x86_64" | "aarch64") => "yt-dlp_macos",
        ("windows", "x86_64") => "yt-dlp.exe",
        ("windows", "aarch64") => "yt-dlp_arm64.exe",
        ("windows", "x86") => "yt-dlp_x86.exe",
        _ => {
            return Err(format!(
                "yt-dlp installer does not support {} {}.",
                os, arch
            ));
        }
    };

    Ok(DownloadAsset {
        name,
        url: format!("https://github.com/yt-dlp/yt-dlp/releases/latest/download/{name}"),
    })
}

fn ffmpeg_asset() -> Result<FfmpegAsset, String> {
    ffmpeg_asset_for(env::consts::OS, env::consts::ARCH)
}

fn ffmpeg_asset_for(os: &str, arch: &str) -> Result<FfmpegAsset, String> {
    let (name, archive_root) = match (os, arch) {
        ("linux", "x86_64") => (
            "ffmpeg-master-latest-linux64-gpl.tar.xz",
            "ffmpeg-master-latest-linux64-gpl",
        ),
        ("linux", "aarch64") => (
            "ffmpeg-master-latest-linuxarm64-gpl.tar.xz",
            "ffmpeg-master-latest-linuxarm64-gpl",
        ),
        _ => {
            return Err(format!(
                "ffmpeg installer does not support {} {} yet.",
                os, arch
            ));
        }
    };

    Ok(FfmpegAsset {
        name,
        archive_root,
        url: format!("https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/{name}"),
    })
}

fn download_and_verify(
    asset_url: &str,
    checksums_url: &str,
    asset_name: &str,
    destination: &Path,
) -> Result<(), String> {
    let checksums = download_text(checksums_url)?;
    let expected = expected_sha256(&checksums, asset_name)?;
    download_to_path(asset_url, destination)?;
    let actual = sha256_file(destination)?;

    if actual != expected {
        let _ = fs::remove_file(destination);
        return Err(format!(
            "Checksum mismatch for {asset_name}: expected {expected}, got {actual}."
        ));
    }

    Ok(())
}

fn download_text(url: &str) -> Result<String, String> {
    ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| format!("Could not download {url}: {error}"))?
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("Could not read {url}: {error}"))
}

fn download_to_path(url: &str, destination: &Path) -> Result<(), String> {
    let mut response = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| format!("Could not download {url}: {error}"))?;
    let mut file = File::create(destination).map_err(|error| error.to_string())?;
    let mut reader = response.body_mut().as_reader();
    io::copy(&mut reader, &mut file)
        .map(|_| ())
        .map_err(|error| format!("Could not save {}: {error}", destination.display()))
}

fn expected_sha256(checksums: &str, asset_name: &str) -> Result<String, String> {
    checksums
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts
                .next()?
                .trim_start_matches('*')
                .trim_start_matches("./");
            (name == asset_name).then(|| hash.to_ascii_lowercase())
        })
        .next()
        .filter(|hash| hash.len() == 64 && hash.chars().all(|char| char.is_ascii_hexdigit()))
        .ok_or_else(|| format!("Could not find a SHA-256 checksum for {asset_name}."))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(bytes_to_hex(&hasher.finalize()))
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn verify_download(path: &Path, args: &[&str], label: &str) -> Result<(), String> {
    let mut header = [0u8; 64];
    let count = File::open(path)
        .map_err(|e| e.to_string())?
        .read(&mut header)
        .map_err(|e| e.to_string())?;
    if !matches_platform(&header[..count], env::consts::OS, env::consts::ARCH) {
        return Err(format!("Downloaded {label} is not an executable for {} {}. The existing installation was kept.", env::consts::OS, env::consts::ARCH));
    }
    verify_tool_command(path, args, label)
}

fn matches_platform(header: &[u8], os: &str, arch: &str) -> bool {
    match os {
        "linux" => {
            if header.len() < 20 || !header.starts_with(b"\x7fELF") || header[5] != 1 {
                return false;
            }
            let machine = u16::from_le_bytes([header[18], header[19]]);
            matches!(
                (arch, header[4], machine),
                ("x86_64", 2, 62) | ("aarch64", 2, 183)
            )
        }
        "windows" => header.starts_with(b"MZ"),
        "macos" => [
            b"\xcf\xfa\xed\xfe",
            b"\xfe\xed\xfa\xcf",
            b"\xca\xfe\xba\xbe",
            b"\xca\xfe\xba\xbf",
        ]
        .iter()
        .any(|magic| header.starts_with(*magic)),
        _ => false,
    }
}

fn verify_tool_command(path: &Path, args: &[&str], label: &str) -> Result<(), String> {
    let output = Command::new(path)
        .args(args)
        .output()
        .map_err(|error| format!("Could not run installed {label}: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("Installed {label} exited with status {}.", output.status)
        } else {
            format!("Installed {label} failed: {stderr}")
        })
    }
}

fn app_tools_dir(app: &Runtime) -> Result<PathBuf, String> {
    Ok(app.data_dir.join("tools"))
}

fn updated_tool_path_result(app: &Runtime, name: &str) -> Result<PathBuf, String> {
    app_tools_dir(app).map(|dir| dir.join(tool_executable_name(name)))
}

fn temp_path(parent: &Path, label: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    parent.join(format!(".{label}-{millis}-{}", std::process::id()))
}

fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if fs::symlink_metadata(destination).is_ok() {
        fs::remove_file(destination).map_err(|error| error.to_string())?;
    }
    fs::rename(source, destination).map_err(|error| error.to_string())
}

fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)
            .map_err(|error| error.to_string())?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn updated_tool_path(app: &Runtime, name: &str) -> Option<PathBuf> {
    Some(app.data_dir.join("tools").join(tool_executable_name(name)))
}

fn bundled_tool_path(app: &Runtime, name: &str) -> Option<PathBuf> {
    app.resource_dir
        .as_ref()
        .map(|dir| dir.join("binaries").join(tool_executable_name(name)))
}

fn path_candidates(name: &str) -> Vec<PathBuf> {
    let executable = tool_executable_name(name);
    let mut candidates: Vec<PathBuf> = env::var_os("PATH")
        .map(|paths| {
            env::split_paths(&paths)
                .map(|path| path.join(&executable))
                .collect()
        })
        .unwrap_or_default();

    candidates.extend([
        PathBuf::from("/opt/homebrew/bin").join(&executable),
        PathBuf::from("/usr/local/bin").join(&executable),
        PathBuf::from("/usr/bin").join(&executable),
    ]);

    if let Some(home) = env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".local/bin").join(&executable));
    }
    candidates
}

fn tool_executable_name(name: &str) -> String {
    if cfg!(windows) && !name.ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_installers_never_select_windows_or_macos_assets() {
        assert_eq!(
            yt_dlp_asset_for("linux", "x86_64").unwrap().name,
            "yt-dlp_linux"
        );
        assert_eq!(
            yt_dlp_asset_for("linux", "aarch64").unwrap().name,
            "yt-dlp_linux_aarch64"
        );
        assert_eq!(
            ffmpeg_asset_for("linux", "x86_64").unwrap().name,
            "ffmpeg-master-latest-linux64-gpl.tar.xz"
        );
        assert_eq!(
            ffmpeg_asset_for("linux", "aarch64").unwrap().name,
            "ffmpeg-master-latest-linuxarm64-gpl.tar.xz"
        );
        assert!(yt_dlp_asset_for("linux", "unknown").is_err());
        assert!(ffmpeg_asset_for("macos", "aarch64").is_err());
        assert_eq!(
            yt_dlp_asset_for("windows", "x86_64").unwrap().name,
            "yt-dlp.exe"
        );
        assert_eq!(
            yt_dlp_asset_for("macos", "aarch64").unwrap().name,
            "yt-dlp_macos"
        );
    }

    #[test]
    fn rejects_foreign_or_wrong_architecture_downloads() {
        assert!(!matches_platform(b"MZ windows", "linux", "x86_64"));
        assert!(!matches_platform(b"\xcf\xfa\xed\xfe", "linux", "x86_64"));
        let mut elf = [0; 64];
        elf[..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2;
        elf[5] = 1;
        elf[18] = 62;
        assert!(matches_platform(&elf, "linux", "x86_64"));
        assert!(!matches_platform(&elf, "linux", "aarch64"));
        elf[18] = 183;
        assert!(matches_platform(&elf, "linux", "aarch64"));
        assert!(!matches_platform(&elf, "linux", "x86_64"));
    }

    #[cfg(unix)]
    #[test]
    fn broken_managed_binary_does_not_hide_working_system_tool() {
        let dir = env::temp_dir().join(format!("downloader-tools-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let broken = dir.join("broken");
        let good = dir.join("good");
        fs::write(&broken, "#!/bin/sh\nexit 1\n").unwrap();
        fs::write(&good, "#!/bin/sh\necho test-version\n").unwrap();
        set_executable(&broken).unwrap();
        set_executable(&good).unwrap();
        assert_eq!(
            find_working_tool("yt-dlp", [broken, good.clone()]),
            Some(good)
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
