mod api;
mod app;
mod audio;
mod auth;
mod config;
mod error;
mod socket;
mod theme;
mod tui;

use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Parser, Debug)]
#[command(
    name = "plaza-tui",
    about = "Nightwave Plaza TUI - Vaporwave radio in your terminal",
    version
)]
pub struct Cli {
    /// Reset saved authentication token
    #[arg(long)]
    pub reset_auth: bool,

    /// Stream quality to use (hls, ogg, ogg-low, mp3, mp3-low)
    #[arg(long, value_name = "QUALITY")]
    pub stream_quality: Option<String>,

    /// Log level (error, warn, info, debug, trace)
    #[arg(long, default_value = "info")]
    pub log_level: String,
}

fn setup_logging(log_level: &str) -> anyhow::Result<()> {
    let log_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("plaza-tui");

    std::fs::create_dir_all(&log_dir)?;

    let log_file = log_dir.join("plaza-tui.log");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)?;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    fmt()
        .with_env_filter(filter)
        .with_writer(std::sync::Mutex::new(file))
        .with_ansi(false)
        .init();

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    setup_logging(&cli.log_level)?;
    tracing::info!("Plaza TUI starting up");

    if cli.reset_auth {
        auth::delete_token();
        println!("Authentication token cleared.");
        return Ok(());
    }

    // Install panic hook to restore terminal before printing panic
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

    // Load config
    let mut config = config::Config::load().unwrap_or_default();

    // Override stream quality from CLI
    if let Some(ref quality) = cli.stream_quality {
        config.stream_quality = match quality.as_str() {
            "ogg" => config::StreamQuality::Ogg,
            "ogg-low" => config::StreamQuality::OggLow,
            "mp3" => config::StreamQuality::Mp3,
            "mp3-low" => config::StreamQuality::Mp3Low,
            _ => config::StreamQuality::Hls,
        };
    }

    // Load saved auth token
    let token = auth::load_token();
    let api = api::ApiClient::new(token);

    // Run the app
    if let Err(e) = app::run(config, api).await {
        // Make sure terminal is restored before printing error
        tracing::error!("App error: {}", e);
        eprintln!("Error: {}", e);
    }

    tracing::info!("Plaza TUI shutting down");
    Ok(())
}
