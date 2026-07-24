use super::SiteKind;
use url::Url;

pub fn normalize_url(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    let url = Url::parse(trimmed).map_err(|_| "Enter a valid URL.".to_string())?;

    match url.scheme() {
        "http" | "https" => Ok(url.to_string()),
        _ => Err("Only http and https URLs are supported.".to_string()),
    }
}

pub fn youtube_channel_videos_url(input: &str) -> Result<String, String> {
    youtube_channel_tab_url(input, "videos")
}

pub fn youtube_channel_shorts_url(input: &str) -> Result<String, String> {
    youtube_channel_tab_url(input, "shorts")
}

fn youtube_channel_tab_url(input: &str, tab: &str) -> Result<String, String> {
    let mut url = Url::parse(input.trim()).map_err(|_| "Enter a valid URL.".to_string())?;
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .trim_start_matches("m.")
        .to_ascii_lowercase();
    if host != "youtube.com" {
        return Err("Enter a YouTube channel link.".to_string());
    }

    let segments = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let identity_len = match segments.as_slice() {
        [handle, ..] if handle.starts_with('@') && handle.len() > 1 => 1,
        [kind, value, ..] if matches!(*kind, "channel" | "c" | "user") && !value.is_empty() => 2,
        _ => {
            return Err(
                "Enter a YouTube channel handle, /channel/, /c/, or /user/ link.".to_string(),
            )
        }
    };

    let mut path = segments[..identity_len].join("/");
    path.insert(0, '/');
    path.push('/');
    path.push_str(tab);
    url.set_scheme("https")
        .map_err(|_| "Invalid YouTube URL.".to_string())?;
    url.set_host(Some("www.youtube.com"))
        .map_err(|_| "Invalid YouTube URL.".to_string())?;
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

pub fn detect_site(input: &str) -> SiteKind {
    let Ok(url) = Url::parse(input) else {
        return SiteKind::Generic;
    };

    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let path = url.path().to_ascii_lowercase();

    if path.ends_with(".m3u8") || path.ends_with(".mpd") {
        return SiteKind::DirectHls;
    }

    if path.ends_with(".mp4")
        || path.ends_with(".mov")
        || path.ends_with(".m4v")
        || path.ends_with(".webm")
        || path.ends_with(".mkv")
    {
        return SiteKind::DirectFile;
    }

    if host == "redd.it" || host.ends_with("reddit.com") {
        return SiteKind::Reddit;
    }

    if host.ends_with("linkedin.com") {
        return SiteKind::Linkedin;
    }

    if host.ends_with("crunchyroll.com") {
        return SiteKind::Crunchyroll;
    }

    if host == "youtu.be" || host.ends_with("youtube.com") {
        return SiteKind::Youtube;
    }

    if host == "x.com" || host == "twitter.com" {
        return SiteKind::X;
    }

    if host.ends_with("vimeo.com") {
        return SiteKind::Vimeo;
    }

    if host.ends_with("sawhorsela.com") {
        return SiteKind::Sawhorse;
    }

    SiteKind::Generic
}

pub fn warnings_for_site(site: &SiteKind) -> Vec<String> {
    match site {
        SiteKind::Linkedin => {
            vec!["LinkedIn usually needs browser cookies from the same local machine.".to_string()]
        }
        SiteKind::Crunchyroll => {
            vec![
                "Crunchyroll needs your own account cookies; DRM-protected streams are not bypassed."
                    .to_string(),
            ]
        }
        SiteKind::X => vec!["X article/media extraction may require cookies.".to_string()],
        SiteKind::Reddit => {
            vec![
                "Reddit may require logged-in cookies and yt-dlp browser impersonation support."
                    .to_string(),
            ]
        }
        SiteKind::Youtube => {
            vec![
                "YouTube will try without cookies first, then retry with saved auth if needed."
                    .to_string(),
            ]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_crunchyroll_urls() {
        assert_eq!(
            detect_site("https://www.crunchyroll.com/watch/G31UXDEJY/example"),
            SiteKind::Crunchyroll
        );
        assert_eq!(
            detect_site("https://www.crunchyroll.com/series/G4PH0WXVJ/spy-x-family"),
            SiteKind::Crunchyroll
        );
    }

    #[test]
    fn keeps_direct_stream_detection_before_site_detection() {
        assert_eq!(
            detect_site("https://static.crunchyroll.com/example/master.m3u8"),
            SiteKind::DirectHls
        );
    }

    #[test]
    fn normalizes_supported_youtube_channel_links_to_videos_tab() {
        let cases = [
            (
                "https://youtube.com/@anthropic-ai",
                "https://www.youtube.com/@anthropic-ai/videos",
            ),
            (
                "https://m.youtube.com/@anthropic-ai/shorts?view=0",
                "https://www.youtube.com/@anthropic-ai/videos",
            ),
            (
                "https://youtube.com/channel/UC123/videos",
                "https://www.youtube.com/channel/UC123/videos",
            ),
            (
                "https://www.youtube.com/c/HuggingFace/playlists",
                "https://www.youtube.com/c/HuggingFace/videos",
            ),
            (
                "https://youtube.com/user/example/about",
                "https://www.youtube.com/user/example/videos",
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(youtube_channel_videos_url(input).unwrap(), expected);
        }
    }

    #[test]
    fn normalizes_supported_youtube_channel_links_to_shorts_tab() {
        assert_eq!(
            youtube_channel_shorts_url("https://youtube.com/@anthropic-ai/videos").unwrap(),
            "https://www.youtube.com/@anthropic-ai/shorts"
        );
        assert_eq!(
            youtube_channel_shorts_url("https://youtube.com/channel/UC123/about").unwrap(),
            "https://www.youtube.com/channel/UC123/shorts"
        );
    }

    #[test]
    fn rejects_youtube_video_and_playlist_links_as_channels() {
        assert!(youtube_channel_videos_url("https://youtube.com/watch?v=abc").is_err());
        assert!(youtube_channel_videos_url("https://youtube.com/playlist?list=abc").is_err());
        assert!(youtube_channel_videos_url("https://youtu.be/abc").is_err());
    }
}
