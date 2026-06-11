//! Errors returned by the Plaza API client.

/// A failure talking to the Nightwave Plaza backend.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The request was rejected for missing or invalid credentials (HTTP 401).
    #[error("unauthorized")]
    Unauthorized,
    /// The server is rate limiting the client (HTTP 429).
    #[error("rate limited")]
    RateLimited,
    /// The requested resource does not exist (HTTP 404).
    #[error("not found")]
    NotFound,
    /// The server returned an unexpected status code.
    #[error("server returned status {status}")]
    Server {
        /// The HTTP status code returned.
        status: u16,
    },
    /// The response was successful but its body was not what we expected.
    #[error("unexpected response: {0}")]
    Unexpected(String),
    /// A transport-level failure (connection, TLS, timeout, decoding).
    #[error("http transport error")]
    Http(#[from] reqwest::Error),
}

/// Result alias for the Plaza API client.
pub type Result<T, E = Error> = std::result::Result<T, E>;
