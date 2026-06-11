//! Codec-agnostic decoded-audio interface.
//!
//! Every stream type (MP3, Opus, HLS/AAC) is decoded behind a single [`PcmSource`]
//! that yields interleaved f32 PCM. The player's playback loop is written once
//! against this trait, so adding a codec means adding a source — not touching the
//! sink-feeding, backpressure, or reconnect logic.

/// A block of decoded, interleaved f32 PCM and its format.
#[derive(Debug, Clone, PartialEq)]
pub struct PcmChunk {
    /// Interleaved samples (L,R,L,R,… for stereo).
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl PcmChunk {
    pub fn new(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
        PcmChunk {
            samples,
            sample_rate,
            channels,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// Why a [`PcmSource`] stopped producing audio.
#[derive(Debug)]
pub enum PcmError {
    /// Transient: the stream/connection ended or hit a read error. For a live
    /// stream the player reconnects (with backoff); for a one-shot it stops.
    Ended,
    /// Permanent: unrecoverable for this stream (e.g. an unsupported codec, or a
    /// malformed playlist). Reconnecting would fail identically, so the player
    /// stops and surfaces the message to the user.
    Permanent(String),
}

impl std::fmt::Display for PcmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PcmError::Ended => write!(f, "stream ended"),
            PcmError::Permanent(m) => write!(f, "{m}"),
        }
    }
}

/// A pull-based source of decoded PCM. Implementations own their network
/// connection and decoder; the player calls [`next_chunk`](PcmSource::next_chunk)
/// repeatedly on a dedicated audio thread.
pub trait PcmSource: Send {
    /// Produce the next chunk of audio.
    ///
    /// - `Ok(Some(chunk))` — decoded audio.
    /// - `Ok(None)` — no data available right now but the stream is healthy (e.g.
    ///   an HLS source waiting at the live edge for the next segment). The caller
    ///   should briefly back off and try again, staying responsive to commands.
    /// - `Err(_)` — the stream ended ([`PcmError::Ended`]) or failed permanently.
    fn next_chunk(&mut self) -> Result<Option<PcmChunk>, PcmError>;
}
