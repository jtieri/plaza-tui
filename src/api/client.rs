use crate::api::{ApiClient, models::{*, DataWrapper}};
use crate::error::{ApiError, AuthError, PlazaError, Result};
use reqwest::Method;

impl ApiClient {
    pub async fn get_status(&self) -> Result<StatusResource> {
        let url = self.url("status");
        let resp = self.client.get(&url).send().await?;
        self.handle_response(resp).await
    }

    pub async fn get_history(&self, page: u32) -> Result<Paginated<HistoryEntry>> {
        let url = format!("{}/v2/history?page={}", self.base_url, page);
        let resp = self.auth_request(Method::GET, &url).send().await?;
        self.handle_response(resp).await
    }

    pub async fn get_song(&self, id: &str) -> Result<SongResource> {
        let url = self.url(&format!("v2/songs/{}", id));
        let resp = self.auth_request(Method::GET, &url).send().await?;
        let wrapper: DataWrapper<SongResource> = self.handle_response(resp).await?;
        Ok(wrapper.data)
    }

    pub async fn get_favorites(&self, page: u32) -> Result<Paginated<FavoriteEntry>> {
        let url = format!("{}/v2/users/me/favorites?page={}", self.base_url, page);
        let resp = self.auth_request(Method::GET, &url).send().await?;
        self.handle_response(resp).await
    }

    pub async fn add_favorite(&self, song_id: &str) -> Result<FavoriteEntry> {
        let url = self.url("v2/users/me/favorites");
        let body = serde_json::json!({ "song_id": song_id });
        let resp = self.auth_request(Method::POST, &url)
            .json(&body)
            .send()
            .await?;
        let wrapper: DataWrapper<FavoriteEntry> = self.handle_response(resp).await?;
        Ok(wrapper.data)
    }

    pub async fn remove_favorite(&self, favorite_id: u64) -> Result<()> {
        let url = self.url(&format!("v2/users/me/favorites/{}", favorite_id));
        let resp: reqwest::Response = self.auth_request(Method::DELETE, &url).send().await?;
        match resp.status() {
            s if s.is_success() => Ok(()),
            s if s == 401 => Err(PlazaError::Auth(AuthError::Unauthorized)),
            s if s == 404 => Err(PlazaError::Api(ApiError::NotFound)),
            s => Err(PlazaError::Api(ApiError::ServerError { status: s.as_u16() })),
        }
    }

    pub async fn send_reaction(&self, reaction: u8) -> Result<u32> {
        let url = self.url("v2/reactions");
        let body = serde_json::json!({ "reaction": reaction });
        let resp = self.auth_request(Method::POST, &url)
            .json(&body)
            .send()
            .await?;
        let value: serde_json::Value = self.handle_response(resp).await?;
        value.get("reactions")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .ok_or_else(|| PlazaError::Api(ApiError::UnexpectedResponse(
                "Missing reactions field".to_string()
            )))
    }

    pub async fn get_ratings(&self, range: RatingRange, page: u32) -> Result<Paginated<RatingEntry>> {
        let url = format!("{}/v2/ratings/{}?page={}", self.base_url, range.as_str(), page);
        let resp = self.client.get(&url).send().await?;
        self.handle_response(resp).await
    }

    pub async fn get_me(&self) -> Result<User> {
        let url = self.url("v2/users/me");
        let resp = self.auth_request(Method::GET, &url).send().await?;
        let wrapper: DataWrapper<User> = self.handle_response(resp).await?;
        Ok(wrapper.data)
    }

    pub async fn get_my_stats(&self) -> Result<UserStats> {
        let url = self.url("v2/users/me/stats");
        let resp = self.auth_request(Method::GET, &url).send().await?;
        let wrapper: DataWrapper<UserStats> = self.handle_response(resp).await?;
        Ok(wrapper.data)
    }

    pub async fn get_news(&self, page: u32) -> Result<Paginated<NewsItem>> {
        let url = format!("{}/v2/news?page={}", self.base_url, page);
        let resp = self.client.get(&url).send().await?;
        self.handle_response(resp).await
    }

    async fn handle_response<T: serde::de::DeserializeOwned>(&self, resp: reqwest::Response) -> Result<T> {
        match resp.status() {
            s if s.is_success() => {
                resp.json::<T>().await.map_err(PlazaError::Http)
            }
            s if s == 401 => Err(PlazaError::Auth(AuthError::Unauthorized)),
            s if s == 429 => Err(PlazaError::Api(ApiError::RateLimited)),
            s if s == 404 => Err(PlazaError::Api(ApiError::NotFound)),
            s => Err(PlazaError::Api(ApiError::ServerError { status: s.as_u16() })),
        }
    }
}
