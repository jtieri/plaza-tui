use std::path::PathBuf;

use clap::Parser;
use plaza_audio::StreamQuality;
use plaza_tui::{app, config::Config};
use tracing_subscriber::{fmt, EnvFilter};

/// A terminal client for Nightwave Plaza, the vaporwave radio station.
#[derive(Parser, Debug)]
#[command(name = "plaza-tui", about, version)]
struct Cli {
    /// Forget the saved authentication token and exit.
    #[arg(long)]
    reset_auth: bool,

    /// Stream quality to play: hls, ogg, ogg-low, mp3, or mp3-low.
    #[arg(long, value_name = "QUALITY")]
    stream_quality: Option<String>,

    /// Log level: error, warn, info, debug, or trace.
    #[arg(long, default_value = "info")]
    log_level: String,
}

/// Parse a CLI quality string into a [`StreamQuality`], if recognized.
fn parse_quality(s: &str) -> Option<StreamQuality> {
    match s {
        "hls" => Some(StreamQuality::Hls),
        "ogg" => Some(StreamQuality::Ogg),
        "ogg-low" => Some(StreamQuality::OggLow),
        "mp3" => Some(StreamQuality::Mp3),
        "mp3-low" => Some(StreamQuality::Mp3Low),
        _ => None,
    }
}

/// Send tracing output to a log file, since stdout is the TUI's drawing surface.
fn setup_logging(log_level: &str) -> anyhow::Result<()> {
    let log_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("plaza-tui");
    std::fs::create_dir_all(&log_dir)?;

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("plaza-tui.log"))?;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));
    fmt()
        .with_env_filter(filter)
        .with_writer(std::sync::Mutex::new(file))
        .with_ansi(false)
        .init();
    Ok(())
}

/// Restore the terminal before a panic prints, so a crash doesn't leave the
/// terminal in raw mode on the alternate screen.
fn install_panic_hook() {
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableBracketedPaste,
        );
        default_panic(info);
    }));
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    setup_logging(&cli.log_level)?;
    tracing::info!("Plaza TUI starting up");

    if cli.reset_auth {
        plaza_api::auth::delete_token();
        println!("Authentication token cleared.");
        return Ok(());
    }

    install_panic_hook();

    let mut config = Config::load().unwrap_or_default();
    if let Some(quality) = cli.stream_quality.as_deref().and_then(parse_quality) {
        config.stream_quality = quality;
    }

    let token = plaza_api::auth::load_token();
    let api = plaza_api::ApiClient::new(token);

    if let Err(e) = app::run(config, api).await {
        tracing::error!("App error: {e}");
        eprintln!("Error: {e}");
    }

    tracing::info!("Plaza TUI shutting down");
    Ok(())
}
