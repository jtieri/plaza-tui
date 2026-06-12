//! User configuration, persisted as TOML under the platform config directory.

use std::path::PathBuf;

use anyhow::Context;
use plaza_audio::{RecordMode, RecordingConfig, StreamQuality};
use serde::{Deserialize, Serialize};

/// Persistent user settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Which stream endpoint to play.
    #[serde(default)]
    pub stream_quality: StreamQuality,
    /// Output volume in `0.0..=1.0`.
    #[serde(default = "default_volume")]
    pub volume: f32,
    /// Force a specific terminal image protocol (e.g. `"kitty"`, `"sixel"`);
    /// `None` auto-detects.
    #[serde(default)]
    pub image_protocol: Option<String>,
    /// Song-recording settings.
    #[serde(default)]
    pub recording: RecordingSettings,
}

fn default_volume() -> f32 {
    0.8
}

impl Default for Config {
    fn default() -> Self {
        Config {
            stream_quality: StreamQuality::default(),
            volume: default_volume(),
            image_protocol: None,
            recording: RecordingSettings::default(),
        }
    }
}

/// Settings for recording songs from the live stream to a local FLAC library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingSettings {
    /// What to do with completed songs: `off`, `cache`, or `session`.
    #[serde(default)]
    pub mode: RecordMode,
    /// Library directory; empty means `<audio dir>/Plaza`.
    #[serde(default)]
    pub directory: Option<String>,
    /// How many songs the rolling cache keeps.
    #[serde(default = "default_cache_size")]
    pub cache_size: usize,
    /// Download and embed cover art.
    #[serde(default = "default_true")]
    pub embed_artwork: bool,
    /// Skip re-saving a song already in the library.
    #[serde(default = "default_true")]
    pub deduplicate: bool,
}

fn default_cache_size() -> usize {
    20
}

fn default_true() -> bool {
    true
}

impl Default for RecordingSettings {
    fn default() -> Self {
        RecordingSettings {
            mode: RecordMode::Off,
            directory: None,
            cache_size: default_cache_size(),
            embed_artwork: true,
            deduplicate: true,
        }
    }
}

impl RecordingSettings {
    /// Resolve these settings into a [`RecordingConfig`] for the audio engine.
    pub fn to_config(&self) -> RecordingConfig {
        let root = self
            .directory
            .as_ref()
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_recording_root);
        RecordingConfig {
            mode: self.mode,
            root,
            cache_size: self.cache_size,
            embed_artwork: self.embed_artwork,
            deduplicate: self.deduplicate,
        }
    }
}

/// Default recording library: the platform music directory's `Plaza` folder.
fn default_recording_root() -> PathBuf {
    dirs::audio_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Plaza")
}

impl Config {
    /// Path to the config file (`$XDG_CONFIG_HOME/plaza-tui/config.toml` or the
    /// platform equivalent).
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("plaza-tui")
            .join("config.toml")
    }

    /// Load the config, creating it with defaults if it does not yet exist.
    ///
    /// # Errors
    /// Returns an error if the file exists but cannot be read or parsed, or if a
    /// freshly created default config cannot be written.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            let config = Config::default();
            config.save()?;
            return Ok(config);
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("reading config from {}", path.display()))?;
        toml::from_str(&content).with_context(|| format!("parsing config at {}", path.display()))
    }

    /// Write the config back to [`config_path`](Self::config_path), creating the
    /// parent directory if needed.
    ///
    /// # Errors
    /// Returns an error if the directory or file cannot be written.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating config directory {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(self).context("serializing config")?;
        std::fs::write(&path, content)
            .with_context(|| format!("writing config to {}", path.display()))
    }
}
