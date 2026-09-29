//! XRBAZAAR preparation policy, copied from the shared media guidance on 2026-09-29.
//! MP4/H.264/yuv420p, <=1280px, <=30fps, AAC <=128kbps/48kHz/1-2ch,
//! <=100MiB, fast start, original aspect ratio, no upscaling or source overwrite.
use crate::process_control::{self, ChildGuard};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub const MAX_BYTES: u64 = 100 * 1024 * 1024;
const AUDIO_RATE: u64 = 96_000;

struct Runner<'a> {
    cancel: &'a AtomicBool,
    process: &'a mut dyn FnMut(Option<u32>) -> Result<(), String>,
    progress: &'a mut dyn FnMut(f64) -> Result<(), String>,
    duration: f64,
}
impl Runner<'_> {
    fn run(&mut self, command: &mut Command) -> Result<String, String> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err("Canceled".into());
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        process_control::isolate_process_group(command);
        let mut child = ChildGuard(command.spawn().map_err(|e| e.to_string())?);
        (self.process)(Some(child.id()))?;
        let (tx, rx) = mpsc::channel();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let tx_out = tx.clone();
        let mut readers = Some([
            thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if tx_out.send((true, line)).is_err() {
                        break;
                    }
                }
            }),
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if tx.send((false, line)).is_err() {
                        break;
                    }
                }
            }),
        ]);
        let mut output = String::new();
        let mut errors = String::new();
        let mut reported = -1;
        let result = (|| loop {
            if self.cancel.load(Ordering::SeqCst) {
                return Err("Canceled".into());
            }
            let status = child.try_wait().map_err(|e| e.to_string())?;
            if status.is_some() {
                for reader in readers.take().unwrap() {
                    let _ = reader.join();
                }
            }
            while let Ok((stdout, line)) = rx.try_recv() {
                if let Some(time) = line
                    .strip_prefix("out_time_us=")
                    .and_then(|s| s.parse::<f64>().ok())
                {
                    let percent =
                        ((time / 1_000_000.0 / self.duration) * 100.0).clamp(0.0, 100.0) as i32;
                    if percent != reported {
                        (self.progress)(percent as f64)?;
                        reported = percent;
                    }
                } else {
                    let buffer = if stdout { &mut output } else { &mut errors };
                    if buffer.len() < 1024 * 1024 {
                        buffer.push_str(&line);
                        buffer.push('\n');
                    }
                }
            }
            if let Some(status) = status {
                return if status.success() {
                    Ok(output)
                } else {
                    Err(format!(
                        "Media preparation failed ({status}): {}",
                        errors.trim()
                    ))
                };
            }
            thread::sleep(Duration::from_millis(100));
        })();
        let _ = (self.process)(None);
        result
    }
    fn probe(&mut self, ffprobe: &Path, path: &Path) -> Result<Value, String> {
        let text = self.run(
            Command::new(ffprobe)
                .args([
                    "-v",
                    "error",
                    "-show_streams",
                    "-show_format",
                    "-of",
                    "json",
                ])
                .arg(path),
        )?;
        serde_json::from_str(&text).map_err(|e| format!("Cannot inspect video: {e}"))
    }
}

fn number(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.parse().ok())
}
fn ratio(v: &Value) -> Option<f64> {
    let s = v.as_str()?;
    let (a, b) = s.split_once('/').or_else(|| s.split_once(':'))?;
    let n = a.parse::<f64>().ok()? / b.parse::<f64>().ok()?;
    (n.is_finite() && n > 0.0).then_some(n)
}
fn video(info: &Value) -> Result<&Value, String> {
    info["streams"]
        .as_array()
        .and_then(|s| {
            s.iter()
                .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        })
        .ok_or_else(|| "The downloaded file has no video stream.".into())
}
fn audio(info: &Value) -> Option<&Value> {
    info["streams"]
        .as_array()?
        .iter()
        .find(|s| s["codec_type"] == "audio")
}
fn dimensions(v: &Value) -> Result<(f64, f64), String> {
    let mut w =
        number(&v["width"]).unwrap_or(0.0) * ratio(&v["sample_aspect_ratio"]).unwrap_or(1.0);
    let mut h = number(&v["height"]).unwrap_or(0.0);
    let rotation = v["side_data_list"]
        .as_array()
        .and_then(|list| list.iter().find_map(|s| number(&s["rotation"])))
        .or_else(|| number(&v["tags"]["rotate"]))
        .unwrap_or(0.0);
    if (rotation.abs() % 180.0 - 90.0).abs() < 0.1 {
        std::mem::swap(&mut w, &mut h);
    }
    if w < 2.0 || h < 2.0 || !w.is_finite() || !h.is_finite() {
        return Err("Invalid video dimensions.".into());
    }
    Ok((w, h))
}
fn target_dimensions(w: f64, h: f64) -> (u32, u32) {
    let scale = (1280.0 / w.max(h)).min(1.0);
    (
        ((w * scale / 2.0).floor() as u32) * 2,
        ((h * scale / 2.0).floor() as u32) * 2,
    )
}
fn frame_rate(v: &Value) -> Option<f64> {
    ratio(&v["avg_frame_rate"]).or_else(|| ratio(&v["r_frame_rate"]))
}
fn compatible(info: &Value) -> bool {
    let Ok(v) = video(info) else { return false };
    let Ok((w, h)) = dimensions(v) else {
        return false;
    };
    v["codec_name"] == "h264"
        && v["pix_fmt"] == "yuv420p"
        && w.max(h) <= 1280.0
        && number(&v["width"]).unwrap_or(f64::INFINITY) <= 1280.0
        && number(&v["height"]).unwrap_or(f64::INFINITY) <= 1280.0
        && frame_rate(v).is_some_and(|fps| fps <= 30.001)
        && ratio(&v["r_frame_rate"]).is_some_and(|fps| fps <= 30.001)
        && audio(info).is_none_or(|a| {
            a["codec_name"] == "aac"
                && number(&a["sample_rate"]) == Some(48000.0)
                && number(&a["channels"]).is_some_and(|n| n == 1.0 || n == 2.0)
                && number(&a["bit_rate"]).is_some_and(|n| n > 0.0 && n <= 128000.0)
        })
}
fn fast_start(path: &Path) -> Result<bool, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let length = f.metadata().map_err(|e| e.to_string())?.len();
    let mut pos = 0;
    while pos + 8 <= length {
        let mut h = [0u8; 8];
        f.read_exact(&mut h).map_err(|e| e.to_string())?;
        let mut size = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
        let mut header = 8;
        if size == 1 {
            let mut ext = [0u8; 8];
            f.read_exact(&mut ext).map_err(|e| e.to_string())?;
            size = u64::from_be_bytes(ext);
            header = 16;
        }
        if size == 0 {
            size = length - pos;
        }
        if size < header || size > length - pos {
            return Ok(false);
        }
        if &h[4..] == b"moov" {
            return Ok(true);
        }
        if &h[4..] == b"mdat" {
            return Ok(false);
        }
        pos += size;
        f.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
    }
    Ok(false)
}
struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn prepare_video(
    ffmpeg: &Path,
    ffprobe: &Path,
    source: &Path,
    cancel: &AtomicBool,
    process: &mut dyn FnMut(Option<u32>) -> Result<(), String>,
    progress: &mut dyn FnMut(f64) -> Result<(), String>,
) -> Result<PathBuf, String> {
    prepare_with_limit(
        ffmpeg, ffprobe, source, cancel, process, progress, MAX_BYTES,
    )
}
fn prepare_with_limit(
    ffmpeg: &Path,
    ffprobe: &Path,
    source: &Path,
    cancel: &AtomicBool,
    process: &mut dyn FnMut(Option<u32>) -> Result<(), String>,
    progress: &mut dyn FnMut(f64) -> Result<(), String>,
    limit: u64,
) -> Result<PathBuf, String> {
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let mut run = Runner {
        cancel,
        process,
        progress,
        duration: 1.0,
    };
    let input = run.probe(ffprobe, &source)?;
    let v = video(&input)?;
    let duration = number(&input["format"]["duration"])
        .filter(|d| d.is_finite() && *d > 0.0)
        .ok_or("Cannot determine video duration.")?;
    run.duration = duration;
    let (iw, ih) = dimensions(v)?;
    let (w, h) = target_dimensions(iw, ih);
    if w < 2 || h < 2 {
        return Err("The video aspect ratio cannot fit XRBAZAAR's limits.".into());
    }
    let dir = TempDir(
        source
            .parent()
            .unwrap()
            .join(format!(".downloader-{}", uuid::Uuid::new_v4())),
    );
    fs::create_dir(&dir.0).map_err(|e| e.to_string())?;
    let output = dir.0.join("compatible.mp4");
    let passlog = dir.0.join("encoding");
    let audio_rate = if audio(&input).is_some() {
        AUDIO_RATE
    } else {
        0
    };
    let mut budget = ((limit as f64 * 8.0 * 0.93 / duration) - audio_rate as f64)
        .floor().min(5_000_000.0);
    let can_copy =
        compatible(&input) && fs::metadata(&source).map_err(|e| e.to_string())?.len() < limit;
    // First preserve compatible streams, or encode with constant quality. Only
    // use two-pass bitrate encoding when the resulting complete file is too big.
    for attempt in 0..3 {
        if attempt > 0 {
            if budget < 100_000.0 {
                return Err("This video is too long to fit the XRBAZAAR size limit at useful quality. Choose a shorter segment in Download options.".into());
            }
        }
        let passes = if attempt == 0 { vec![0] } else { vec![1, 2] };
        for pass in passes {
            let mut command = Command::new(ffmpeg);
            command
                .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
                .arg(&source)
                .args([
                    "-map",
                    &format!("0:{}", v["index"]),
                    "-map_metadata",
                    "-1",
                    "-map_chapters",
                    "-1",
                ]);
            if let Some(a) = audio(&input).filter(|_| pass != 1) {
                command.args(["-map", &format!("0:{}", a["index"])]);
            }
            if can_copy && attempt == 0 {
                command.args(["-c", "copy"]);
            } else {
                command.args([
                    "-vf",
                    &format!("scale={w}:{h}:flags=lanczos,setsar=1"),
                    "-fpsmax",
                    "30",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "medium",
                    "-pix_fmt",
                    "yuv420p",
                ]);
                if pass == 0 {
                    command.args(["-crf", "23"]);
                } else {
                    command
                        .args([
                            "-b:v",
                            &(budget as u64).to_string(),
                            "-pass",
                            &pass.to_string(),
                            "-passlogfile",
                        ])
                        .arg(&passlog);
                }
                if let Some(a) = audio(&input).filter(|_| pass != 1) {
                    command.args([
                        "-c:a",
                        "aac",
                        "-b:a",
                        "96000",
                        "-ar",
                        "48000",
                        "-ac",
                        if a["channels"] == 1 { "1" } else { "2" },
                    ]);
                }
            }
            command.args(["-progress", "pipe:1", "-nostats"]);
            if pass == 1 {
                command.args([
                    "-an",
                    "-f",
                    "null",
                    if cfg!(windows) { "NUL" } else { "/dev/null" },
                ]);
            } else {
                command
                    .args(["-movflags", "+faststart", "-f", "mp4"])
                    .arg(&output);
            }
            run.run(&mut command)?;
        }
        let encoded_size = fs::metadata(&output).map_err(|e| e.to_string())?.len();
        if encoded_size <= limit {
            break;
        }
        if attempt > 0 {
            // Two-pass encoders can overshoot their average bitrate, especially
            // for short clips. Correct using measured size, then verify again.
            budget = (budget * (limit as f64 / encoded_size as f64) * 0.90).floor();
        }
    }
    let info = run.probe(ffprobe, &output)?;
    let out_video = video(&info)?;
    let (ow, oh) = dimensions(out_video)?;
    let size = fs::metadata(&output).map_err(|e| e.to_string())?.len();
    let checks = [
        (!compatible(&info), format!("stream requirements: video {} / {}, {ow}x{oh}, average FPS {}, nominal FPS {}; audio {}",
            out_video["codec_name"], out_video["pix_fmt"], out_video["avg_frame_rate"], out_video["r_frame_rate"],
            audio(&info).map(|a| format!("{} / {} Hz / {} channels / {} bps", a["codec_name"], a["sample_rate"], a["channels"], a["bit_rate"])).unwrap_or_else(|| "none".into()))),
        (size > limit, format!("file is {size} bytes, limit is {limit}")),
        (!fast_start(&output)?, "fast-start metadata is missing".into()),
        (audio(&input).is_some() != audio(&info).is_some(), "audio was not preserved".into()),
        (ow > iw + 0.01 || oh > ih + 0.01, "video was upscaled".into()),
        (((ow / oh) / (iw / ih) - 1.0).abs() > 2.0 / w as f64 + 2.0 / h as f64, "aspect ratio changed".into()),
        (number(&info["format"]["duration"]).is_none_or(|out| (out - duration).abs() > 0.5f64.max(duration * 0.01)), "duration changed".into()),
    ];
    let failures: Vec<_> = checks.into_iter().filter_map(|(failed, reason)| failed.then_some(reason)).collect();
    if !failures.is_empty() {
        return Err(format!("The converted video did not pass XRBAZAAR verification: {}. The source was kept.", failures.join("; ")));
    }
    run.run(
        Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-xerror",
                "-i",
            ])
            .arg(&output)
            .args(["-map", "0:v:0", "-map", "0:a?", "-f", "null", "-"]),
    )?;
    if cancel.load(Ordering::SeqCst) {
        return Err("Canceled".into());
    }
    // create_new preserves existing exports, including concurrent conversions.
    let stem = source.file_stem().unwrap().to_string_lossy();
    for n in 0..1000 {
        let suffix = if n == 0 {
            String::new()
        } else {
            format!(" ({n})")
        };
        let path = source.with_file_name(format!("{stem}-compatible{suffix}.mp4"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut dest) => {
                let result = (|| {
                    std::io::copy(&mut File::open(&output)?, &mut dest)?;
                    dest.sync_all()
                })();
                if let Err(e) = result {
                    let _ = fs::remove_file(&path);
                    return Err(e.to_string());
                }
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Too many compatible copies already exist.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_preserves_portrait_landscape_and_small_sources() {
        assert_eq!(target_dimensions(1920.0, 1080.0), (1280, 720));
        assert_eq!(target_dimensions(1080.0, 1920.0), (720, 1280));
        assert_eq!(target_dimensions(394.0, 854.0), (394, 854));
    }
    #[test]
    fn reject_wrong_codecs_and_audio_limits() {
        let mut info = serde_json::json!({"streams":[{"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p","width":640,"height":360,"avg_frame_rate":"30/1","r_frame_rate":"30/1"}]});
        assert!(compatible(&info));
        info["streams"][0]["pix_fmt"] = "yuv444p".into();
        assert!(!compatible(&info));
        info["streams"][0]["pix_fmt"] = "yuv420p".into();
        info["streams"].as_array_mut().unwrap().push(serde_json::json!({"codec_type":"audio","codec_name":"aac","sample_rate":"44100","channels":2,"bit_rate":"96000"}));
        assert!(!compatible(&info));
    }
    #[test]
    #[ignore = "Requires ffmpeg and ffprobe; exercises real conversion and verification"]
    fn real_conversion_preserves_sources_and_handles_limits() {
        let dir = TempDir(std::env::temp_dir().join(format!("xr-test-{}", uuid::Uuid::new_v4())));
        fs::create_dir(&dir.0).unwrap();
        let source = dir.0.join("Test ' & video.mp4");
        assert!(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1920x1080:rate=60",
                "-f",
                "lavfi",
                "-i",
                "sine=sample_rate=44100",
                "-t",
                "2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac"
            ])
            .arg(&source)
            .status()
            .unwrap()
            .success());
        let original = fs::read(&source).unwrap();
        let cancel = AtomicBool::new(false);
        let out = prepare_with_limit(
            Path::new("ffmpeg"),
            Path::new("ffprobe"),
            &source,
            &cancel,
            &mut |_| Ok(()),
            &mut |_| Ok(()),
            150_000,
        )
        .unwrap();
        assert!(fs::metadata(&out).unwrap().len() <= 150_000);
        assert_eq!(fs::read(&source).unwrap(), original);
        assert!(fast_start(&out).unwrap());
        let preserved = fs::read(&out).unwrap();
        let second = prepare_video(
            Path::new("ffmpeg"),
            Path::new("ffprobe"),
            &source,
            &cancel,
            &mut |_| Ok(()),
            &mut |_| Ok(()),
        )
        .unwrap();
        assert_ne!(out, second);
        assert_eq!(fs::read(&out).unwrap(), preserved);
        cancel.store(true, Ordering::SeqCst);
        assert!(prepare_video(
            Path::new("ffmpeg"),
            Path::new("ffprobe"),
            &source,
            &cancel,
            &mut |_| Ok(()),
            &mut |_| Ok(())
        )
        .is_err());
    }
    #[test]
    #[ignore = "Requires ffmpeg and ffprobe; tests portrait and silent source preservation"]
    fn real_conversion_preserves_small_silent_portrait_and_cancels_cleanly() {
        let dir =
            TempDir(std::env::temp_dir().join(format!("xr-portrait-{}", uuid::Uuid::new_v4())));
        fs::create_dir(&dir.0).unwrap();
        let source = dir.0.join("portrait.mp4");
        assert!(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=394x854:rate=24",
                "-t",
                "2",
                "-c:v",
                "libx264"
            ])
            .arg(&source)
            .status()
            .unwrap()
            .success());
        let cancel = AtomicBool::new(false);
        let out = prepare_video(
            Path::new("ffmpeg"),
            Path::new("ffprobe"),
            &source,
            &cancel,
            &mut |_| Ok(()),
            &mut |_| Ok(()),
        )
        .unwrap();
        let info: Value = serde_json::from_slice(
            &Command::new("ffprobe")
                .args(["-v", "error", "-show_streams", "-of", "json"])
                .arg(&out)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        assert_eq!(dimensions(video(&info).unwrap()).unwrap(), (394.0, 854.0));
        assert!(audio(&info).is_none());
        let result = prepare_video(
            Path::new("ffmpeg"),
            Path::new("ffprobe"),
            &source,
            &cancel,
            &mut |_| Ok(()),
            &mut |_| {
                cancel.store(true, Ordering::SeqCst);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(source.is_file());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    }
}
