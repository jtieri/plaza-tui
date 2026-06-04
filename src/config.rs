use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::error::{PlazaError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum StreamQuality {
    Hls,
    Ogg,
    OggLow,
    Mp3,
    Mp3Low,
}

impl Default for StreamQuality {
    fn default() -> Self {
        // MP3 is the only format decodable today (Opus/HLS arrive in Phase 1) and
        // is the most broadly compatible, so it is the safe default.
        StreamQuality::Mp3
    }
}

impl StreamQuality {
    pub fn stream_url(&self) -> &'static str {
        match self {
            StreamQuality::Hls => "https://radio.plaza.one/hls",
            StreamQuality::Ogg => "https://radio.plaza.one/ogg",
            // NOTE: the low-quality paths use an underscore, not a slash.
            // `/ogg/low` and `/mp3/low` return 404 on the live server.
            StreamQuality::OggLow => "https://radio.plaza.one/ogg_low",
            StreamQuality::Mp3 => "https://radio.plaza.one/mp3",
            StreamQuality::Mp3Low => "https://radio.plaza.one/mp3_low",
        }
    }

    /// Human-readable label for notifications / UI.
    pub fn label(&self) -> &'static str {
        match self {
            StreamQuality::Hls => "HLS/AAC",
            StreamQuality::Ogg => "Opus 64k",
            StreamQuality::OggLow => "Opus 96k",
            StreamQuality::Mp3 => "MP3 128k",
            StreamQuality::Mp3Low => "MP3 96k",
        }
    }

    /// Whether this client can currently decode this stream.
    ///
    /// Phase 0 only ships the symphonia-native MP3 path. Opus (`/ogg*`) needs a
    /// libopus decoder and HLS needs an MPEG-TS demuxer; both land in Phase 1, at
    /// which point this returns true for every variant.
    pub fn is_supported(&self) -> bool {
        matches!(self, StreamQuality::Mp3 | StreamQuality::Mp3Low)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub stream_quality: StreamQuality,
    #[serde(default = "default_volume")]
    pub volume: f32,
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
            volume: 0.8,
            image_protocol: None,
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("plaza-tui")
            .join("config.toml")
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            let config = Config::default();
            config.save()?;
            return Ok(config);
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| PlazaError::Config(format!("Failed to read config: {}", e)))?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| PlazaError::Config(format!("Failed to parse config: {}", e)))?;
        // The stored preference is kept verbatim — if the selected quality isn't
        // decodable yet (Opus/HLS until Phase 1), the run loop falls back to MP3 at
        // runtime via StreamQuality::is_supported() without overwriting the file, so
        // the preference is honoured automatically once that codec is supported.
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| PlazaError::Config(format!("Failed to create config dir: {}", e)))?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| PlazaError::Config(format!("Failed to serialize config: {}", e)))?;
        std::fs::write(&path, content)
            .map_err(|e| PlazaError::Config(format!("Failed to write config: {}", e)))?;
        Ok(())
    }
}
