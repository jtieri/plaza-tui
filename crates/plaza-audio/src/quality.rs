//! Stream format selection.

use serde::{Deserialize, Serialize};

/// A Nightwave Plaza stream endpoint, identified by codec and bitrate.
///
/// Each variant maps to a live URL ([`stream_url`](StreamQuality::stream_url)) and
/// a decoder: MP3 and HLS/AAC decode in pure Rust via symphonia, Opus via libopus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamQuality {
    /// Adaptive AAC over HLS (the highest-bandwidth variant is selected).
    Hls,
    /// 64 kbps Opus.
    Ogg,
    /// 96 kbps Opus.
    OggLow,
    /// 128 kbps MP3. The default: it plays everywhere with no extra setup, whereas
    /// lower-bandwidth Opus and adaptive HLS are opt-in.
    #[default]
    Mp3,
    /// 96 kbps MP3.
    Mp3Low,
}

impl StreamQuality {
    /// The live stream URL for this quality.
    pub fn stream_url(self) -> &'static str {
        match self {
            StreamQuality::Hls => "https://radio.plaza.one/hls",
            StreamQuality::Ogg => "https://radio.plaza.one/ogg",
            // The low-quality endpoints use an underscore; the slash forms
            // (`/ogg/low`, `/mp3/low`) return 404 on the live server.
            StreamQuality::OggLow => "https://radio.plaza.one/ogg_low",
            StreamQuality::Mp3 => "https://radio.plaza.one/mp3",
            StreamQuality::Mp3Low => "https://radio.plaza.one/mp3_low",
        }
    }

    /// A short human-readable label, e.g. for status messages.
    pub fn label(self) -> &'static str {
        match self {
            StreamQuality::Hls => "HLS/AAC",
            StreamQuality::Ogg => "Opus 64k",
            StreamQuality::OggLow => "Opus 96k",
            StreamQuality::Mp3 => "MP3 128k",
            StreamQuality::Mp3Low => "MP3 96k",
        }
    }
}
