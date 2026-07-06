use super::{AuthRequirement, OutputKind, Pipeline, Preset, SiteKind};
use url::Url;

pub fn all_presets() -> Vec<Preset> {
    vec![
        preset(
            "generic-page-video-highest",
            &[SiteKind::Generic],
            "Generic Page Video",
            "Find the highest quality video available on a standard page.",
            OutputKind::Video,
            Pipeline::YtDlp,
            AuthRequirement::Optional,
        ),
        preset(
            "generic-direct-stream-highest",
            &[SiteKind::Generic, SiteKind::DirectHls, SiteKind::Sawhorse],
            "Direct Stream Video",
            "Save the highest quality HLS or DASH stream as mp4.",
            OutputKind::Video,
            Pipeline::HttpResolveThenDownload,
            AuthRequirement::Optional,
        ),
        preset(
            "reddit-post-video-highest",
            &[SiteKind::Reddit],
            "Reddit Post Video",
            "Download the highest quality video from a single Reddit post.",
            OutputKind::Video,
            Pipeline::YtDlp,
            AuthRequirement::Optional,
        ),
        preset(
            "reddit-multiple-media-highest",
            &[SiteKind::Reddit],
            "Reddit Multiple Media",
            "Download the highest quality videos from a multi-media post.",
            OutputKind::Video,
            Pipeline::YtDlp,
            AuthRequirement::Optional,
        ),
        preset(
            "youtube-video-highest",
            &[SiteKind::Youtube],
            "YouTube Video",
            "Download the highest quality YouTube video and retry with saved auth only if needed.",
            OutputKind::Video,
            Pipeline::YtDlp,
            AuthRequirement::Recommended,
        ),
        preset(
            "linkedin-post-video-highest",
            &[SiteKind::Linkedin],
            "LinkedIn Post Video",
            "Use local cookies to save the highest quality post video.",
            OutputKind::Video,
            Pipeline::YtDlp,
            AuthRequirement::Required,
        ),
        preset(
            "linkedin-article-video-highest",
            &[SiteKind::Linkedin],
            "LinkedIn Article Video",
            "Use local cookies to resolve video embedded in an article.",
            OutputKind::Video,
            Pipeline::HttpResolveThenDownload,
            AuthRequirement::Required,
        ),
        preset(
            "linkedin-feed-update-video-highest",
            &[SiteKind::Linkedin],
            "LinkedIn Feed Update",
            "Use local cookies to resolve the DASH stream from feed update metadata.",
            OutputKind::Video,
            Pipeline::HttpResolveThenDownload,
            AuthRequirement::Required,
        ),
        preset(
            "crunchyroll-video-highest",
            &[SiteKind::Crunchyroll],
            "Crunchyroll Video",
            "Download the highest quality stream available to yt-dlp with your account cookies.",
            OutputKind::Video,
            Pipeline::YtDlp,
            AuthRequirement::Required,
        ),
        preset(
            "x-article-video-highest",
            &[SiteKind::X],
            "X Article Video",
            "Resolve every video embedded in an X article and download the highest MP4 variant.",
            OutputKind::Video,
            Pipeline::HttpResolveThenDownload,
            AuthRequirement::Required,
        ),
        preset(
            "vimeo-video-highest",
            &[SiteKind::Vimeo],
            "Vimeo Video",
            "Download the highest quality Vimeo video.",
            OutputKind::Video,
            Pipeline::YtDlp,
            AuthRequirement::Optional,
        ),
        preset(
            "sawhorse-portfolio-video-highest",
            &[SiteKind::Sawhorse],
            "Sawhorse Portfolio Video",
            "Resolve embedded portfolio video at the highest quality.",
            OutputKind::Video,
            Pipeline::HttpResolveThenDownload,
            AuthRequirement::Optional,
        ),
    ]
}

pub fn matching_presets(site: &SiteKind) -> Vec<Preset> {
    let mut matched: Vec<Preset> = all_presets()
        .into_iter()
        .filter(|preset| preset.site_kinds.iter().any(|kind| kind == site))
        .collect();

    if matched.is_empty() {
        matched = all_presets()
            .into_iter()
            .filter(|preset| {
                preset
                    .site_kinds
                    .iter()
                    .any(|kind| kind == &SiteKind::Generic)
            })
            .collect();
    }

    matched
}

pub fn matching_presets_for_url(site: &SiteKind, input_url: &str) -> Vec<Preset> {
    let mut matched = matching_presets(site);

    if matches!(site, SiteKind::Linkedin) {
        if let Ok(url) = Url::parse(input_url) {
            let path = url.path().to_ascii_lowercase();
            if path.starts_with("/feed/update/") {
                promote_preset(&mut matched, "linkedin-feed-update-video-highest");
            } else if path.starts_with("/pulse/") || path.contains("/article/") {
                promote_preset(&mut matched, "linkedin-article-video-highest");
            } else if path.starts_with("/posts/") {
                promote_preset(&mut matched, "linkedin-post-video-highest");
            }
        }
    }

    if matches!(site, SiteKind::X) {
        if let Ok(url) = Url::parse(input_url) {
            if url.path().to_ascii_lowercase().contains("/article/") {
                promote_preset(&mut matched, "x-article-video-highest");
            }
        }
    }

    matched
}

pub fn find_preset(id: &str) -> Option<Preset> {
    all_presets().into_iter().find(|preset| preset.id == id)
}

fn promote_preset(presets: &mut Vec<Preset>, preset_id: &str) {
    if let Some(index) = presets.iter().position(|preset| preset.id == preset_id) {
        let preset = presets.remove(index);
        presets.insert(0, preset);
    }
}

fn preset(
    id: &str,
    site_kinds: &[SiteKind],
    label: &str,
    description: &str,
    output_kind: OutputKind,
    pipeline: Pipeline,
    auth: AuthRequirement,
) -> Preset {
    Preset {
        id: id.to_string(),
        site_kinds: site_kinds.to_vec(),
        label: label.to_string(),
        description: description.to_string(),
        output_kind,
        pipeline,
        auth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_crunchyroll_preset() {
        let presets = matching_presets(&SiteKind::Crunchyroll);

        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].id, "crunchyroll-video-highest");
        assert_eq!(presets[0].auth, AuthRequirement::Required);
    }
}
