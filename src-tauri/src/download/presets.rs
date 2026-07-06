use super::{AuthRequirement, OutputKind, Pipeline, Preset, SiteKind};

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
            "Use local cookies for feed update activity URLs.",
            OutputKind::Video,
            Pipeline::YtDlp,
            AuthRequirement::Required,
        ),
        preset(
            "x-article-video-highest",
            &[SiteKind::X],
            "X Article Video",
            "Download the highest quality video from an X article.",
            OutputKind::Video,
            Pipeline::YtDlp,
            AuthRequirement::Recommended,
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

pub fn find_preset(id: &str) -> Option<Preset> {
    all_presets().into_iter().find(|preset| preset.id == id)
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
