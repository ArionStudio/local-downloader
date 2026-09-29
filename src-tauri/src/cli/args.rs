use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "downloader-cli",
    version,
    about = "Headless media downloader for people and agents. Results are JSON on stdout."
)]
pub struct Cli {
    /// Private CLI state directory. Separate from desktop jobs.
    #[arg(long, global = true, env = "DOWNLOADER_DATA_DIR")]
    pub data_dir: Option<PathBuf>,
    /// Emit JSON (the default); accepted for explicit agent invocations.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    /// List every supported download preset.
    Presets,
    /// Normalize a URL and suggest presets.
    Analyze { url: String },
    /// Inspect available video and audio streams.
    Formats {
        url: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Download URLs, or submit the same JSON request as the desktop app.
    Download(DownloadArgs),
    /// Prepare an existing video to meet XRBAZAAR requirements.
    Prepare {
        file: PathBuf,
        #[arg(long)]
        events: bool,
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Describe the JSON request and result contract.
    Schema,
    #[command(subcommand)]
    Tools(Tools),
    #[command(subcommand)]
    Settings(Settings),
    #[command(subcommand)]
    Cookies(Cookies),
    #[command(subcommand)]
    Keys(Keys),
    #[command(subcommand)]
    History(History),
    /// List completed files, including files from canceled jobs.
    Files {
        #[arg(long)]
        job: Option<String>,
        #[arg(long)]
        hashes: bool,
    },
    /// Cancel a running CLI job in this data directory.
    Cancel { job_id: String },
}
#[derive(Args, Debug, Default)]
pub struct AuthArgs {
    /// Browser or browser:profile. Repeat to configure ordered fallbacks.
    #[arg(long, conflicts_with_all = ["cookie_file", "no_cookies"])]
    pub browser: Vec<String>,
    #[arg(long, conflicts_with = "no_cookies")]
    pub cookie_file: Option<PathBuf>,
    /// Disable explicit and saved cookie fallback for this request.
    #[arg(long)]
    pub no_cookies: bool,
}
#[derive(Args, Debug)]
pub struct DownloadArgs {
    #[arg(required_unless_present = "request", conflicts_with = "request")]
    pub urls: Vec<String>,
    /// Read a complete StartDownloadRequest from a file, or '-' for stdin.
    #[arg(long, conflicts_with_all = ["preset", "output", "profile", "filename_template", "browser", "cookie_file", "no_cookies", "format_id", "audio_only", "video_only", "start", "end", "export_name", "catalogue_content"])]
    pub request: Option<String>,
    #[arg(long)]
    pub preset: Option<String>,
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    #[arg(long, value_enum)]
    pub profile: Option<Profile>,
    #[arg(long)]
    pub filename_template: Option<String>,
    #[command(flatten)]
    pub auth: AuthArgs,
    #[arg(long, conflicts_with = "audio_only")]
    pub format_id: Option<String>,
    #[arg(long, conflicts_with = "video_only")]
    pub audio_only: bool,
    #[arg(long)]
    pub video_only: bool,
    /// Start time in seconds.
    #[arg(long)]
    pub start: Option<f64>,
    /// End time in seconds.
    #[arg(long)]
    pub end: Option<f64>,
    #[arg(long)]
    pub export_name: Option<String>,
    #[arg(long, value_enum)]
    pub catalogue_content: Option<Content>,
    /// Stream newline-delimited job events and a final result.
    #[arg(long)]
    pub events: bool,
    /// Cancel after this many seconds, retaining finished files (exit 124).
    #[arg(long)]
    pub timeout: Option<u64>,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Profile {
    Original,
    Xrbazaar,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Content {
    All,
    Videos,
    Shorts,
}
#[derive(Subcommand, Debug)]
pub enum Tools {
    Status,
    Platform,
    Install {
        #[arg(value_parser = ["yt-dlp", "ffmpeg", "all"])]
        tool: String,
    },
}
#[derive(Subcommand, Debug)]
pub enum Settings {
    Show,
    /// Replace settings with JSON read from a file, or '-' for stdin.
    Set {
        #[arg(long)]
        request: String,
    },
    /// Copy desktop settings and API-key IDs; desktop jobs stay untouched.
    ImportDesktop {
        #[arg(long)]
        from: Option<PathBuf>,
    },
}
#[derive(Subcommand, Debug)]
pub enum Cookies {
    Browsers,
    /// Export only cookies belonging to this URL's host, into a new private file.
    Export {
        url: String,
        #[arg(long)]
        browser: String,
        #[arg(short, long)]
        output: PathBuf,
    },
}
#[derive(Subcommand, Debug)]
pub enum Keys {
    List,
    /// Read a YouTube API key from stdin and store it in the OS credential vault.
    Add {
        #[arg(long, required = true)]
        stdin: bool,
    },
    Remove {
        id: String,
    },
}
#[derive(Subcommand, Debug)]
pub enum History {
    List,
    Show {
        job_id: String,
        #[arg(long)]
        logs: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supports_ordered_browser_profiles_and_trim() {
        let cli = Cli::try_parse_from([
            "downloader-cli",
            "download",
            "https://example.com",
            "--browser",
            "firefox:Profile A",
            "--browser",
            "chrome:Default",
            "--start",
            "2",
            "--end",
            "8",
            "--events",
        ])
        .unwrap();
        let Command::Download(args) = cli.command else {
            panic!()
        };
        assert_eq!(args.auth.browser, ["firefox:Profile A", "chrome:Default"]);
        assert_eq!(args.end, Some(8.0));
        assert!(args.events);
    }
    #[test]
    fn rejects_ambiguous_cookie_sources_and_request_overrides() {
        for flags in [
            vec!["--browser", "firefox", "--no-cookies"],
            vec!["--cookie-file", "a", "--browser", "chrome"],
            vec!["--request", "a", "--profile", "xrbazaar"],
        ] {
            let mut argv = vec!["downloader-cli", "download"];
            if flags[0] != "--request" {
                argv.push("https://example.com");
            }
            argv.extend(flags);
            assert!(Cli::try_parse_from(argv).is_err());
        }
    }
    #[test]
    fn requires_explicit_key_stdin_and_download_input() {
        assert!(Cli::try_parse_from(["downloader-cli", "download"]).is_err());
        assert!(Cli::try_parse_from(["downloader-cli", "keys", "add"]).is_err());
        assert!(Cli::try_parse_from(["downloader-cli", "keys", "add", "--stdin"]).is_ok());
    }
}
