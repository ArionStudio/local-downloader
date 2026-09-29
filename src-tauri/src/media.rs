use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use uuid::Uuid;

// WebKitGTK cannot stream video through WebKit's custom URI schemes. Serve only
// explicitly registered media, on loopback, with opaque per-session URLs.
pub struct MediaServer {
    server: Arc<Server>,
    address: String,
    files: Arc<Mutex<HashMap<String, PathBuf>>>,
}

impl MediaServer {
    pub fn start() -> Result<Self, String> {
        let server = Arc::new(Server::http("127.0.0.1:0").map_err(|e| e.to_string())?);
        let address = server.server_addr().to_string();
        let files = Arc::new(Mutex::new(HashMap::new()));
        // A bounded worker pool allows metadata requests and seeking while a
        // different video is streaming, without a thread per incoming request.
        for _ in 0..4 {
            let server = server.clone();
            let files = files.clone();
            std::thread::spawn(move || {
                for request in server.incoming_requests() {
                    serve(request, &files);
                }
            });
        }
        Ok(Self {
            server,
            address,
            files,
        })
    }

    pub fn register(&self, path: PathBuf) -> Result<String, String> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("Cannot read video: {e}"))?;
        if !path.is_file() || mime_type(&path).is_none() {
            return Err("This file is not a supported video.".into());
        }
        let mut files = self.files.lock().map_err(|e| e.to_string())?;
        let token = files
            .iter()
            .find_map(|(token, registered)| (registered == &path).then(|| token.clone()))
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        files.insert(token.clone(), path);
        Ok(format!("http://{}/{token}", self.address))
    }
}

impl Drop for MediaServer {
    fn drop(&mut self) {
        for _ in 0..4 {
            self.server.unblock();
        }
    }
}

fn mime_type(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => Some("video/mp4"),
        "webm" => Some("video/webm"),
        "mov" => Some("video/quicktime"),
        "mkv" => Some("video/x-matroska"),
        _ => None,
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).expect("static HTTP header or numeric value")
}

fn serve(request: Request, files: &Mutex<HashMap<String, PathBuf>>) {
    if !matches!(request.method(), Method::Get | Method::Head) {
        let _ = request.respond(Response::empty(405));
        return;
    }
    let path = files
        .lock()
        .ok()
        .and_then(|files| files.get(request.url().trim_start_matches('/')).cloned());
    let Some(path) = path else {
        let _ = request.respond(Response::empty(404));
        return;
    };
    let Ok(mut file) = File::open(&path) else {
        let _ = request.respond(Response::empty(404));
        return;
    };
    let Ok(metadata) = file.metadata() else {
        let _ = request.respond(Response::empty(500));
        return;
    };
    let size = metadata.len();
    let range = request.headers().iter().find(|h| h.field.equiv("Range"));
    let (start, length) = match byte_range(range.map(|h| h.value.as_str()), size) {
        Ok(value) => value,
        Err(()) => {
            let _ = request.respond(
                Response::empty(416)
                    .with_header(header("Content-Range", &format!("bytes */{size}"))),
            );
            return;
        }
    };
    if file.seek(SeekFrom::Start(start)).is_err() {
        let _ = request.respond(Response::empty(500));
        return;
    }
    let mut headers = vec![
        header(
            "Content-Type",
            mime_type(&path).unwrap_or("application/octet-stream"),
        ),
        header("Accept-Ranges", "bytes"),
        header("Cache-Control", "no-store"),
        header("X-Content-Type-Options", "nosniff"),
    ];
    if range.is_some() {
        headers.push(header(
            "Content-Range",
            &format!("bytes {start}-{}/{size}", start + length - 1),
        ));
    }
    let Ok(length_usize) = usize::try_from(length) else {
        let _ = request.respond(Response::empty(500));
        return;
    };
    let response = Response::new(
        StatusCode(if range.is_some() { 206 } else { 200 }),
        headers,
        file.take(length),
        Some(length_usize),
        None,
    )
    .with_chunked_threshold(usize::MAX);
    // tiny_http suppresses the body for HEAD while preserving Content-Length.
    let _ = request.respond(response);
}

fn byte_range(range: Option<&str>, size: u64) -> Result<(u64, u64), ()> {
    let Some(range) = range else {
        return Ok((0, size));
    };
    let (start, end) = range
        .strip_prefix("bytes=")
        .ok_or(())?
        .split_once('-')
        .ok_or(())?;
    if size == 0 {
        return Err(());
    }
    if start.is_empty() {
        let length = end.parse::<u64>().map_err(|_| ())?.min(size);
        return if length == 0 {
            Err(())
        } else {
            Ok((size - length, length))
        };
    }
    let start = start.parse::<u64>().map_err(|_| ())?;
    let end = if end.is_empty() {
        size - 1
    } else {
        end.parse::<u64>().map_err(|_| ())?.min(size - 1)
    };
    if start >= size || end < start {
        return Err(());
    }
    Ok((start, end - start + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_ranges_cover_metadata_seeking_and_invalid_requests() {
        assert_eq!(byte_range(None, 100), Ok((0, 100)));
        assert_eq!(byte_range(Some("bytes=0-"), 100), Ok((0, 100)));
        assert_eq!(byte_range(Some("bytes=20-29"), 100), Ok((20, 10)));
        assert_eq!(byte_range(Some("bytes=-10"), 100), Ok((90, 10)));
        assert_eq!(byte_range(Some("bytes=90-200"), 100), Ok((90, 10)));
        for range in [
            "bytes=100-",
            "bytes=9-2",
            "bytes=-0",
            "bytes=0-1,4-5",
            "bad",
        ] {
            assert_eq!(byte_range(Some(range), 100), Err(()));
        }
        assert_eq!(byte_range(Some("bytes=0-"), 0), Err(()));
    }

    #[test]
    fn http_stream_preserves_bytes_and_hides_unregistered_files() {
        let path = std::env::temp_dir().join(format!("video ' & {}.mp4", Uuid::new_v4()));
        std::fs::write(&path, b"0123456789").unwrap();
        let server = MediaServer::start().unwrap();
        let url = server.register(path.clone()).unwrap();
        let mut response = ureq::get(&url).header("Range", "bytes=3-6").call().unwrap();
        assert_eq!(response.status(), 206);
        assert_eq!(response.headers()["content-range"], "bytes 3-6/10");
        assert_eq!(response.body_mut().read_to_vec().unwrap(), b"3456");
        let response = ureq::head(&url).call().unwrap();
        assert_eq!(response.headers()["content-length"], "10");
        assert!(ureq::get(&format!("http://{}/etc/passwd", server.address))
            .call()
            .is_err());
        std::fs::remove_file(path).unwrap();
    }
}
