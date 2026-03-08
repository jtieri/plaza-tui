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
        StreamQuality::Ogg
    }
}

impl StreamQuality {
    pub fn stream_url(&self) -> &'static str {
        match self {
            StreamQuality::Hls => "https://radio.plaza.one/hls",
            StreamQuality::Ogg => "https://radio.plaza.one/ogg",
            StreamQuality::OggLow => "https://radio.plaza.one/ogg/low",
            StreamQuality::Mp3 => "https://radio.plaza.one/mp3",
            StreamQuality::Mp3Low => "https://radio.plaza.one/mp3/low",
        }
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
        let mut config: Config = toml::from_str(&content)
            .map_err(|e| PlazaError::Config(format!("Failed to parse config: {}", e)))?;
        // HLS segments use MPEG-TS which symphonia cannot decode — migrate to OGG
        if config.stream_quality == StreamQuality::Hls {
            config.stream_quality = StreamQuality::Ogg;
            let _ = config.save();
        }
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
