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
    pub current_version: Option<String>,
    pub available_version: String,
}

pub fn find_tool(app: &AppHandle, name: &str) -> Option<PathBuf> {
    updated_tool_path(app, name)
        .filter(|path| is_executable(path))
        .or_else(|| bundled_tool_path(app, name).filter(|path| is_executable(path)))
        .or_else(|| find_on_path(name))
}

pub fn tool_version(app: &AppHandle, name: &str) -> Option<String> {
    let tool = find_tool(app, name)?;
    let output = Command::new(tool).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout)
        .ok()
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty())
}

pub fn check_tool_updates(app: &AppHandle) -> Vec<ToolUpdate> {
    ["yt-dlp", "ffmpeg"]
        .iter()
        .filter_map(|tool| {
            let current = tool_version(app, tool);
            if current.is_none() {
                Some(ToolUpdate {
                    tool: tool.to_string(),
                    current_version: None,
                    available_version: "not installed".to_string(),
                })
            } else {
                None
            }
        })
        .collect()
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
