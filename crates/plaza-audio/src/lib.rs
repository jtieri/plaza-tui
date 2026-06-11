//! Streaming audio engine for Nightwave Plaza.
//!
//! [`Player`] owns an audio output and a decode thread; calling
//! [`Player::start_live`] connects to a [`StreamQuality`] endpoint and plays it,
//! reconnecting on transient drops. Every format Plaza offers is supported:
//! MP3 and HLS/AAC decode in pure Rust via [symphonia](https://docs.rs/symphonia),
//! and Opus via libopus.
//!
//! Decoding is expressed through the [`PcmSource`] trait, which yields decoded PCM
//! independently of how it is played. The player's buffering, backpressure, and
//! reconnect logic are written once against that trait, so a new codec is a new
//! source rather than a change to playback.
//!
//! # Examples
//!
//! ```no_run
//! use plaza_audio::{Player, StreamQuality};
//!
//! let mut player = Player::new()?;
//! player.start_live(StreamQuality::Mp3)?;
//! # Ok::<(), plaza_audio::Error>(())
//! ```

#![warn(missing_docs)]

pub mod error;
pub mod hls;
pub mod pcm;
pub mod player;
pub mod quality;
pub mod sources;
pub mod ts;

pub use error::{Error, Result};
pub use pcm::{PcmChunk, PcmError, PcmSource};
pub use player::Player;
pub use quality::StreamQuality;
