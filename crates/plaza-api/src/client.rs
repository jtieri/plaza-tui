//! REST methods on [`ApiClient`].
//!
//! Every method returns [`Result`]; failures cover transport errors, non-success
//! HTTP statuses (surfaced as [`Error::Unauthorized`], [`Error::NotFound`],
//! [`Error::RateLimited`], or [`Error::Server`]), and bodies that don't match the
//! expected shape ([`Error::Unexpected`]).

use reqwest::Method;

use crate::error::{Error, Result};
use crate::models::{DataWrapper, *};
use crate::ApiClient;

impl ApiClient {
    /// Fetch the current broadcast status (now playing and listener count).
    pub async fn get_status(&self) -> Result<StatusResource> {
        let url = self.url("status");
        let resp = self.client.get(&url).send().await?;
        self.handle_response(resp).await
    }

    /// Fetch a page of the recently played history.
    pub async fn get_history(&self, page: u32) -> Result<Paginated<HistoryEntry>> {
        let url = format!("{}/v2/history?page={}", self.base_url, page);
        let resp = self.auth_request(Method::GET, &url).send().await?;
        self.handle_response(resp).await
    }

    /// Fetch full detail for a song, including the signed-in user's favourite state.
    pub async fn get_song(&self, id: &str) -> Result<SongResource> {
        let url = self.url(&format!("v2/songs/{}", id));
        let resp = self.auth_request(Method::GET, &url).send().await?;
        let wrapper: DataWrapper<SongResource> = self.handle_response(resp).await?;
        Ok(wrapper.data)
    }

    /// Fetch a page of the signed-in user's favourites.
    pub async fn get_favorites(&self, page: u32) -> Result<Paginated<FavoriteEntry>> {
        let url = format!("{}/v2/users/me/favorites?page={}", self.base_url, page);
        let resp = self.auth_request(Method::GET, &url).send().await?;
        self.handle_response(resp).await
    }

    /// Favourite a song, returning the created entry.
    pub async fn add_favorite(&self, song_id: &str) -> Result<FavoriteEntry> {
        let url = self.url("v2/users/me/favorites");
        let body = serde_json::json!({ "song_id": song_id });
        let resp = self
            .auth_request(Method::POST, &url)
            .json(&body)
            .send()
            .await?;
        let wrapper: DataWrapper<FavoriteEntry> = self.handle_response(resp).await?;
        Ok(wrapper.data)
    }

    /// Remove a favourite by its [`FavoriteEntry::id`].
    pub async fn remove_favorite(&self, favorite_id: u64) -> Result<()> {
        let url = self.url(&format!("v2/users/me/favorites/{}", favorite_id));
        let resp: reqwest::Response = self.auth_request(Method::DELETE, &url).send().await?;
        match resp.status() {
            s if s.is_success() => Ok(()),
            s if s == 401 => Err(Error::Unauthorized),
            s if s == 404 => Err(Error::NotFound),
            s => Err(Error::Server { status: s.as_u16() }),
        }
    }

    /// Send a reaction for the current song, returning the new reaction total.
    pub async fn send_reaction(&self, reaction: u8) -> Result<u32> {
        let url = self.url("v2/reactions");
        let body = serde_json::json!({ "reaction": reaction });
        let resp = self
            .auth_request(Method::POST, &url)
            .json(&body)
            .send()
            .await?;
        let value: serde_json::Value = self.handle_response(resp).await?;
        value
            .get("reactions")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .ok_or_else(|| Error::Unexpected("missing reactions field".to_string()))
    }

    /// Fetch a page of the ratings chart for the given time range.
    pub async fn get_ratings(
        &self,
        range: RatingRange,
        page: u32,
    ) -> Result<Paginated<RatingEntry>> {
        let url = format!(
            "{}/v2/ratings/{}?page={}",
            self.base_url,
            range.as_str(),
            page
        );
        let resp = self.client.get(&url).send().await?;
        self.handle_response(resp).await
    }

    /// Fetch the signed-in user's profile.
    pub async fn get_me(&self) -> Result<User> {
        let url = self.url("v2/users/me");
        let resp = self.auth_request(Method::GET, &url).send().await?;
        let wrapper: DataWrapper<User> = self.handle_response(resp).await?;
        Ok(wrapper.data)
    }

    /// Fetch the signed-in user's aggregate stats (reaction and favourite counts).
    pub async fn get_my_stats(&self) -> Result<UserStats> {
        let url = self.url("v2/users/me/stats");
        let resp = self.auth_request(Method::GET, &url).send().await?;
        let wrapper: DataWrapper<UserStats> = self.handle_response(resp).await?;
        Ok(wrapper.data)
    }

    /// Fetch a page of news posts.
    pub async fn get_news(&self, page: u32) -> Result<Paginated<NewsItem>> {
        let url = format!("{}/v2/news?page={}", self.base_url, page);
        let resp = self.client.get(&url).send().await?;
        self.handle_response(resp).await
    }

    /// Deserialize a successful response, or map its status to an [`Error`].
    async fn handle_response<T: serde::de::DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T> {
        match resp.status() {
            s if s.is_success() => resp.json::<T>().await.map_err(Error::Http),
            s if s == 401 => Err(Error::Unauthorized),
            s if s == 429 => Err(Error::RateLimited),
            s if s == 404 => Err(Error::NotFound),
            s => Err(Error::Server { status: s.as_u16() }),
        }
    }
}
