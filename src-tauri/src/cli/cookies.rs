use super::{absolute, private_dir};
use crate::{
    download::{chrome_cookies, engine, BrowserAuthSource},
    redaction,
    runtime::Runtime,
    tools,
};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};
struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn matching_cookies(raw: &str, host: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|line| {
            let content = line.strip_prefix("#HttpOnly_").unwrap_or(line);
            if content.starts_with('#') {
                return None;
            }
            let fields = content.split('\t').collect::<Vec<_>>();
            if fields.len() != 7 {
                return None;
            }
            let domain = fields[0].trim_start_matches('.').to_ascii_lowercase();
            let matches =
                host == domain || (fields[1] == "TRUE" && host.ends_with(&format!(".{domain}")));
            (matches && !domain.is_empty()).then(|| line.to_owned())
        })
        .collect()
}
pub fn export(
    app: &Runtime,
    url: &str,
    source: &BrowserAuthSource,
    output: &Path,
) -> Result<Value, String> {
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Cookie export needs an HTTP(S) URL.".into());
    }
    let host = parsed
        .host_str()
        .ok_or("URL needs a host.")?
        .to_ascii_lowercase();
    let output = absolute(output)?;
    if output.exists() {
        return Err("The cookie output file already exists; choose a new path.".into());
    }
    let temp = TempDir(
        app.data_dir
            .join(format!("downloader-cookie-export-{}", uuid::Uuid::new_v4())),
    );
    private_dir(&temp.0)?;
    let jar = temp.0.join("cookies.txt");
    let yt_dlp =
        tools::find_tool(app, "yt-dlp").ok_or("yt-dlp is required. Run tools install yt-dlp.")?;
    let result = Command::new(yt_dlp)
        .args(["--ignore-config", "--cookies-from-browser"])
        .arg(engine::browser_cookie_arg(source))
        .arg("--cookies")
        .arg(&jar)
        .args([
            "--skip-download",
            "--simulate",
            "--no-playlist",
            "--no-color",
            "--socket-timeout",
            "20",
            "--retries",
            "0",
            "--force-generic-extractor",
        ])
        .arg(url)
        .output()
        .map_err(|e| e.to_string())?;
    let mut raw = fs::read_to_string(&jar).unwrap_or_default();
    if matching_cookies(&raw, &host).is_empty() && chrome_cookies::can_export(source) {
        if let Ok(path) = chrome_cookies::export(source, url) {
            raw = fs::read_to_string(&path).unwrap_or_default();
            let _ = fs::remove_file(path);
        }
    }
    let selected = matching_cookies(&raw, &host);
    if selected.is_empty() {
        return Err(format!(
            "No cookies for {host} were extracted from {}. {}",
            source.browser,
            redaction::sanitize_log_line(&String::from_utf8_lossy(&result.stderr))
        ));
    }
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&output).map_err(|e| e.to_string())?;
    let written = (|| {
        writeln!(file, "# Netscape HTTP Cookie File\n# Exported for {host}")?;
        for line in &selected {
            writeln!(file, "{line}")?;
        }
        file.sync_all()
    })()
    .map_err(|e: std::io::Error| e.to_string());
    if let Err(e) = written {
        drop(file);
        let _ = fs::remove_file(&output);
        return Err(e);
    }
    Ok(json!({"path":output,"host":host,"browser":source.browser,"cookieCount":selected.len()}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_only_target_domains_and_preserves_httponly() {
        let jar="# Netscape HTTP Cookie File\n.example.com\tTRUE\t/\tTRUE\t0\ta\tsecret\n#HttpOnly_sub.example.com\tFALSE\t/\tTRUE\t0\tb\tsecret\n.evil-example.com\tTRUE\t/\tTRUE\t0\tc\tsecret\nother.example.com\tFALSE\t/\tTRUE\t0\td\tsecret\n";
        let selected = matching_cookies(jar, "sub.example.com");
        assert_eq!(selected.len(), 2);
        assert!(selected[1].starts_with("#HttpOnly_"));
    }
}
