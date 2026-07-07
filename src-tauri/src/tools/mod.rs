use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};
use tauri::{AppHandle, Manager};

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

pub fn find_tool(app: &AppHandle, name: &str) -> Option<PathBuf> {
    updated_tool_path(app, name)
        .filter(|path| is_executable(path))
        .or_else(|| bundled_tool_path(app, name).filter(|path| is_executable(path)))
        .or_else(|| find_on_path(name))
}

pub fn has_available_impersonation_target(app: &AppHandle) -> bool {
    let Some(tool) = find_tool(app, "yt-dlp") else {
        return false;
    };
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

pub fn check_tool_updates(app: &AppHandle) -> Vec<ToolUpdate> {
    ["yt-dlp", "ffmpeg"]
        .iter()
        .map(|tool| {
            let path = find_tool(app, tool);
            let current = path
                .as_ref()
                .and_then(|tool_path| tool_version_from_path(tool_path));

            if let Some(tool_path) = path {
                let message = if *tool == "yt-dlp" && !has_available_impersonation_target(app) {
                    "Installed, but Reddit/LinkedIn impersonation support is unavailable. Install yt-dlp with curl_cffi support if those sites fail.".to_string()
                } else {
                    "Installed and available to the downloader.".to_string()
                };

                ToolUpdate {
                    tool: tool.to_string(),
                    status: "installed".to_string(),
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
                    message: "Missing. Downloads that require this tool will fail until it is installed or bundled by a signed tools release.".to_string(),
                }
            }
        })
        .collect()
}

fn tool_version_from_path(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout)
        .ok()
        .and_then(|version| version.lines().next().map(str::to_string))
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty())
}

fn updated_tool_path(app: &AppHandle, name: &str) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join("tools").join(name))
}

fn bundled_tool_path(app: &AppHandle, name: &str) -> Option<PathBuf> {
    app.path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("binaries").join(name))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = env::var_os("PATH")
        .map(|paths| {
            env::split_paths(&paths)
                .map(|path| path.join(name))
                .collect()
        })
        .unwrap_or_default();

    candidates.extend([
        PathBuf::from("/opt/homebrew/bin").join(name),
        PathBuf::from("/usr/local/bin").join(name),
        PathBuf::from("/usr/bin").join(name),
    ]);

    candidates.into_iter().find(|path| is_executable(path))
}

fn is_executable(path: &Path) -> bool {
    path.is_file()
}
