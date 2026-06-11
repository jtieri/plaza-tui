//! Capturing the live stream into a local, tagged FLAC library.
//!
//! Songs arrive as a continuous PCM stream; the recorder relies on exact, in-band
//! song boundaries (each Plaza Opus song is its own chained-Ogg logical stream) to
//! split them losslessly. A captured song is only ever persisted in full — see the
//! correctness guarantees in `tasks/recording-design.md`.

pub mod flac;

/// An error while encoding or writing a recording.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The FLAC encoder rejected its configuration.
    #[error("flac configuration error: {0}")]
    Config(String),
    /// Encoding the audio to FLAC failed.
    #[error("flac encode error: {0}")]
    Encode(String),
    /// Writing the file to disk failed.
    #[error("i/o error")]
    Io(#[from] std::io::Error),
}

/// Result alias for recording operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;
