pub mod models;
pub mod client;

use reqwest::Client;
use std::time::Duration;

pub const BASE_URL: &str = "https://api.plaza.one";

#[derive(Clone)]
pub struct ApiClient {
    pub(crate) client: Client,
    pub(crate) token: Option<String>,
    pub(crate) base_url: String,
}

impl ApiClient {
    pub fn new(token: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("plaza-tui/0.1.0")
            .build()
            .expect("Failed to build HTTP client");

        ApiClient {
            client,
            token,
            base_url: BASE_URL.to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.to_string();
        self
    }

    pub fn set_token(&mut self, token: Option<String>) {
        self.token = token;
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    pub(crate) fn auth_request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        let builder = self.client.request(method, url);
        if let Some(token) = &self.token {
            builder.bearer_auth(token)
        } else {
            builder
        }
    }
}
