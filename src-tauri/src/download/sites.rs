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
        SiteKind::X => vec!["X article/media extraction may require cookies.".to_string()],
        SiteKind::Youtube => {
            vec![
                "YouTube will try without cookies first, then retry with saved auth if needed."
                    .to_string(),
            ]
        }
        _ => Vec::new(),
    }
}
