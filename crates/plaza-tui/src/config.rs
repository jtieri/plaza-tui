//! User configuration, persisted as TOML under the platform config directory.

use std::path::PathBuf;

use anyhow::Context;
use plaza_audio::StreamQuality;
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
        }
    }
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
