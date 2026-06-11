//! Client for the Nightwave Plaza backend.
//!
//! Three concerns live here:
//!
//! - [`ApiClient`] — the REST API ([`api.plaza.one`](https://api.plaza.one)): now
//!   playing, history, favourites, ratings, news, reactions, and the user profile.
//! - [`SocketClient`] — the real-time Socket.IO feed of status, listener, and
//!   reaction updates, with automatic reconnection.
//! - [`auth`] — login and bearer-token persistence (system keyring, with a file
//!   fallback).
//!
//! All response shapes are in [`models`].
//!
//! # Examples
//!
//! ```no_run
//! # async fn run() -> plaza_api::Result<()> {
//! let client = plaza_api::ApiClient::new(None);
//! let status = client.get_status().await?;
//! println!("{} — {}", status.song.artist, status.song.title);
//! # Ok(())
//! # }
//! ```

use std::time::Duration;

use reqwest::Client;

pub mod auth;
pub mod client;
pub mod error;
pub mod models;
pub mod socket;

pub use error::{Error, Result};
pub use socket::{SocketClient, SocketEvent};

/// Base URL of the Plaza REST API.
pub const BASE_URL: &str = "https://api.plaza.one";

/// A handle to the Plaza REST API.
///
/// Cloning is cheap — the underlying [`reqwest::Client`] shares a connection pool —
/// so clone it freely to issue concurrent requests.
#[derive(Clone)]
pub struct ApiClient {
    pub(crate) client: Client,
    pub(crate) token: Option<String>,
    pub(crate) base_url: String,
}

impl ApiClient {
    /// Create a client, optionally authenticated with a bearer `token`.
    pub fn new(token: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("plaza-tui/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("default reqwest client configuration is always valid");

        ApiClient {
            client,
            token,
            base_url: BASE_URL.to_string(),
        }
    }

    /// Override the API base URL. Intended for tests against a mock server.
    pub fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.to_string();
        self
    }

    /// Set (or clear) the bearer token used for authenticated requests.
    pub fn set_token(&mut self, token: Option<String>) {
        self.token = token;
    }

    /// Whether a bearer token is currently set.
    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    pub(crate) fn auth_request(
        &self,
        method: reqwest::Method,
        url: &str,
    ) -> reqwest::RequestBuilder {
        let builder = self.client.request(method, url);
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }
}
