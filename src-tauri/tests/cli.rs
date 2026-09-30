use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let base = std::env::var_os("DOWNLOADER_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        fs::create_dir_all(&base).unwrap();
        let p = base.join(format!("downloader-cli-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn cli(root: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_downloader-cli"));
    c.arg("--data-dir")
        .arg(root.join("state"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    c
}
fn result(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stdout)))
}
#[test]
fn schema_and_presets_work_without_a_display() {
    let t = Temp::new();
    for command in ["schema", "presets"] {
        let out = cli(&t.0).arg(command).output().unwrap();
        assert!(out.status.success());
        let v = result(&out);
        assert_eq!(v["ok"], true);
        if command == "schema" {
            assert!(v["result"]["downloadRequest"]["properties"]["allowSavedAuth"].is_object());
        }
    }
}
#[test]
fn invalid_commands_are_json_errors() {
    let t = Temp::new();
    let out = cli(&t.0)
        .args([
            "download",
            "--audio-only",
            "--video-only",
            "https://example.com",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(result(&out)["error"]["code"], "invalid_arguments");
}
#[test]
fn settings_roundtrip_and_cookie_failure_do_not_expose_values() {
    let t = Temp::new();
    let path = t.0.join("settings.json");
    fs::write(
        &path,
        json!({"defaultOutputDir":t.0.join("media"),"auth":{"kind":"none"}}).to_string(),
    )
    .unwrap();
    let out = cli(&t.0)
        .args(["settings", "set", "--request"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = cli(&t.0).args(["settings", "show"]).output().unwrap();
    assert_eq!(result(&out)["result"]["auth"]["kind"], "none");
    let out = cli(&t.0)
        .args(["download", "https://example.com/video.mp4", "--cookie-file"])
        .arg(t.0.join("missing-cookie-file"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(result(&out)["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Cookie file does not exist"));
}

struct Fixture {
    url: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    saw_cookie: Arc<AtomicBool>,
}
impl Fixture {
    fn start(bytes: Vec<u8>) -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let saw_cookie = Arc::new(AtomicBool::new(false));
        let observed = saw_cookie.clone();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                if let Some(request) = server.recv_timeout(Duration::from_millis(100)).unwrap() {
                    if request.headers().iter().any(|h| {
                        h.field.equiv("Cookie") && h.value.as_str().contains("session=fixture")
                    }) {
                        observed.store(true, Ordering::SeqCst);
                    }
                    if request.url().contains("slow") {
                        for _ in 0..100 {
                            if flag.load(Ordering::SeqCst) {
                                break;
                            }
                            thread::sleep(Duration::from_millis(50));
                        }
                    }
                    let response = tiny_http::Response::from_data(bytes.clone()).with_header(
                        tiny_http::Header::from_bytes("Content-Type", "video/mp4").unwrap(),
                    );
                    let _ = request.respond(response);
                }
            }
        });
        Self {
            url,
            stop,
            handle: Some(handle),
            saw_cookie,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
fn media(root: &Path) -> Vec<u8> {
    let path = root.join("source.mp4");
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:size=320x180:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac"
        ])
        .arg(&path)
        .status()
        .unwrap()
        .success());
    fs::read(path).unwrap()
}
#[test]
#[ignore = "Requires real yt-dlp, FFmpeg and ffprobe"]
fn video_only_removes_audio_from_combined_sources_before_ready() {
    let t = Temp::new();
    let original = media(&t.0);
    let server = Fixture::start(original.clone());
    let mut child = cli(&t.0)
        .args([
            "download",
            &format!("{}/video.mp4", server.url),
            "--video-only",
            "--no-cookies",
            "--events",
            "--timeout",
            "90",
            "--output",
        ])
        .arg(t.0.join("media"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = false;
    let mut final_result = Value::Null;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let event: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if event["type"] == "result" {
            final_result = event.clone();
        }
        if let Some(paths) = event["job"]["readyPaths"].as_array() {
            for path in paths {
                let probe = Command::new("ffprobe")
                    .args(["-v", "error", "-show_streams", "-of", "json"])
                    .arg(path.as_str().unwrap())
                    .output()
                    .unwrap();
                assert!(probe.status.success());
                let info: Value = serde_json::from_slice(&probe.stdout).unwrap();
                let streams = info["streams"].as_array().unwrap();
                assert_eq!(
                    streams.len(),
                    1,
                    "A ready video-only file must have no audio"
                );
                assert_eq!(streams[0]["codec_type"], "video");
                assert_eq!(streams[0]["codec_name"], "h264");
                ready = true;
            }
        }
    }
    assert!(child.wait().unwrap().success(), "{final_result}");
    assert!(ready);
    assert_eq!(fs::read(t.0.join("source.mp4")).unwrap(), original);
}

#[test]
#[ignore = "Requires real yt-dlp and FFmpeg"]
fn real_headless_cookie_download_and_xrbazaar_preparation() {
    let t = Temp::new();
    let server = Fixture::start(media(&t.0));
    let cookie = t.0.join("cookies.txt");
    fs::write(
        &cookie,
        "# Netscape HTTP Cookie File\n127.0.0.1\tFALSE\t/\tFALSE\t2147483647\tsession\tfixture\n",
    )
    .unwrap();
    let out = cli(&t.0)
        .args([
            "download",
            &format!("{}/video.mp4", server.url),
            "--cookie-file",
        ])
        .arg(&cookie)
        .arg("--output")
        .arg(t.0.join("media"))
        .args(["--profile", "xrbazaar"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let value = result(&out);
    let files = value["result"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert!(server.saw_cookie.load(Ordering::SeqCst));
    for file in files {
        assert!(Path::new(file["path"].as_str().unwrap()).is_file());
        assert_eq!(file["sha256"].as_str().unwrap().len(), 64);
    }
    let out = cli(&t.0).args(["files", "--hashes"]).output().unwrap();
    assert_eq!(result(&out)["result"].as_array().unwrap().len(), 2);
}
#[test]
#[ignore = "Requires real yt-dlp and FFmpeg"]
fn cancellation_from_another_process_keeps_finished_files() {
    let t = Temp::new();
    let server = Fixture::start(media(&t.0));
    let mut child = cli(&t.0)
        .args([
            "download",
            &format!("{}/first.mp4", server.url),
            &format!("{}/slow.mp4", server.url),
            "--no-cookies",
            "--events",
            "--timeout",
            "120",
            "--output",
        ])
        .arg(t.0.join("media"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut saw_ready = false;
    let mut canceled = false;
    let mut final_result = None;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let event: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if event["type"] == "result" {
            final_result = Some(event);
            continue;
        }
        if event["job"]["readyPaths"]
            .as_array()
            .is_some_and(|p| !p.is_empty())
        {
            saw_ready = true;
        }
        if saw_ready
            && !canceled
            && event["job"]["sourceUrl"]
                .as_str()
                .is_some_and(|u| u.contains("slow"))
        {
            let id = event["job"]["id"].as_str().unwrap();
            let out = cli(&t.0).args(["cancel", id]).output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
            canceled = true;
        }
    }
    assert_eq!(child.wait().unwrap().code(), Some(130));
    assert!(saw_ready && canceled);
    let final_value = final_result.unwrap();
    assert_eq!(final_value["result"]["files"].as_array().unwrap().len(), 1);
    let out = cli(&t.0).arg("files").output().unwrap();
    assert_eq!(result(&out)["result"].as_array().unwrap().len(), 1);
}
#[test]
#[ignore = "Requires real yt-dlp and FFmpeg"]
fn browser_cookie_export_is_scoped_private_and_never_overwrites() {
    let t = Temp::new();
    let server = Fixture::start(media(&t.0));
    let profile = t.0.join("firefox-profile");
    fs::create_dir(&profile).unwrap();
    let db = rusqlite::Connection::open(profile.join("cookies.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE moz_cookies (host TEXT,path TEXT,isSecure INTEGER,expiry INTEGER,name TEXT,value TEXT); INSERT INTO moz_cookies VALUES ('127.0.0.1','/',0,2147483647,'session','fixture'),('.unrelated.example','/',0,2147483647,'private','do-not-export');").unwrap();
    drop(db);
    let output = t.0.join("scoped.txt");
    let exported = cli(&t.0)
        .args([
            "cookies",
            "export",
            &format!("{}/video.mp4", server.url),
            "--browser",
            &format!("firefox:{}", profile.display()),
            "--output",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stdout)
    );
    assert_eq!(result(&exported)["result"]["cookieCount"], 1);
    let jar = fs::read_to_string(&output).unwrap();
    assert!(jar.contains("session\tfixture"));
    assert!(!jar.contains("do-not-export"));
    assert!(!String::from_utf8_lossy(&exported.stdout).contains("fixture"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let again = cli(&t.0)
        .args([
            "cookies",
            "export",
            &format!("{}/video.mp4", server.url),
            "--browser",
            "firefox",
            "--output",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(!again.status.success());
    assert_eq!(fs::read_to_string(&output).unwrap(), jar);
}
#[test]
#[ignore = "Requires FFmpeg and ffprobe"]
fn prepare_returns_verified_upload_metadata_without_a_display() {
    let t = Temp::new();
    media(&t.0);
    let output = cli(&t.0)
        .arg("prepare")
        .arg(t.0.join("source.mp4"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value = result(&output);
    assert_eq!(value["result"]["profile"], "xrbazaar");
    assert_eq!(value["result"]["file"]["contentType"], "video/mp4");
    assert_eq!(
        value["result"]["file"]["sha256"].as_str().unwrap().len(),
        64
    );
    assert!(t.0.join("source.mp4").is_file());
}
#[test]
fn json_request_rejects_unknown_options_before_downloading() {
    let t = Temp::new();
    let path = t.0.join("request.json");
    fs::write(&path,json!({"url":"https://example.com/video.mp4","presetId":"generic-page-video-highest","auth":{"kind":"none"},"outputProfil":"xrbazaar"}).to_string()).unwrap();
    let output = cli(&t.0)
        .args(["download", "--request"])
        .arg(path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(result(&output)["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown field"));
}

#[test]
fn json_request_can_forbid_saved_cookie_fallback() {
    let t = Temp::new();
    let path = t.0.join("request.json");
    fs::write(&path,json!({"url":"https://www.linkedin.com/posts/example","presetId":"linkedin-post-video-highest","allowSavedAuth":false,"auth":{"kind":"none"}}).to_string()).unwrap();
    let output = cli(&t.0)
        .args(["download", "--request"])
        .arg(path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let v = result(&output);
    assert_eq!(v["result"]["jobs"].as_array().unwrap().len(), 0);
    assert!(v["result"]["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("Configure browser cookies"));
}
