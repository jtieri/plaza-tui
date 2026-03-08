use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlazaError {
    #[error("Authentication error: {0}")]
    Auth(#[from] AuthError),

    #[error("API error: {0}")]
    Api(#[from] ApiError),

    #[error("Audio error: {0}")]
    Audio(#[from] AudioError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Config error: {0}")]
    Config(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Unauthorized - invalid credentials or token expired")]
    Unauthorized,

    #[error("No token available - please log in")]
    NoToken,

    #[error("Keyring error: {0}")]
    Keyring(String),
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("Rate limited - please wait before retrying")]
    RateLimited,

    #[error("Not found")]
    NotFound,

    #[error("Server error: {status}")]
    ServerError { status: u16 },

    #[error("Unexpected response: {0}")]
    UnexpectedResponse(String),
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("Failed to initialize audio output: {0}")]
    OutputInit(String),

    #[error("HLS fetch failed after retries: {0}")]
    HlsFailed(String),

    #[error("Decode error: {0}")]
    Decode(String),

    #[error("Stream error: {0}")]
    Stream(String),
}

pub type Result<T> = std::result::Result<T, PlazaError>;
