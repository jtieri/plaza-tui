//! Errors surfaced by the audio engine's public API.

/// An error from setting up or driving audio playback.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No usable audio output device, or the output stream could not be created.
    #[error("audio output unavailable: {0}")]
    Output(String),
}

/// Result alias for the audio engine.
pub type Result<T, E = Error> = std::result::Result<T, E>;
