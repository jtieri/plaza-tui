use crate::api::ApiClient;
use crate::api::models::{LoginForm, LoginResponse};
use crate::error::{AuthError, PlazaError, Result};
use keyring::Entry;

const KEYRING_SERVICE: &str = "plaza-tui";
const KEYRING_ACCOUNT: &str = "auth-token";

pub async fn login(client: &ApiClient, username: &str, password: &str) -> Result<String> {
    let form = LoginForm {
        username: username.to_string(),
        password: password.to_string(),
        remember: true,
    };

    let url = client.url("v2/auth/token");
    let response: reqwest::Response = client
        .client
        .post(&url)
        .json(&form)
        .send()
        .await?;

    match response.status() {
        s if s == 200 || s == 201 => {
            let login_resp: LoginResponse = response.json().await?;
            Ok(login_resp.token)
        }
        s if s == 401 => Err(PlazaError::Auth(AuthError::Unauthorized)),
        s => {
            let _body: String = response.text().await.unwrap_or_default();
            Err(PlazaError::Api(crate::error::ApiError::ServerError {
                status: s.as_u16(),
            }))
        }
    }
}

pub async fn logout(client: &ApiClient) -> Result<()> {
    let url = client.url("v2/auth/logout");
    let _ = client
        .auth_request(reqwest::Method::POST, &url)
        .send()
        .await;
    delete_token();
    Ok(())
}

pub fn save_token(token: &str) {
    match Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
        Ok(entry) => {
            if let Err(e) = entry.set_password(token) {
                tracing::warn!("Failed to save token to keyring: {}", e);
            }
        }
        Err(e) => {
            tracing::warn!("Failed to access keyring: {}", e);
        }
    }
}

pub fn load_token() -> Option<String> {
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).ok()?;
    entry.get_password().ok()
}

pub fn delete_token() {
    match Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
        Ok(entry) => {
            if let Err(e) = entry.delete_credential() {
                tracing::debug!("Failed to delete token from keyring (may not exist): {}", e);
            }
        }
        Err(e) => {
            tracing::warn!("Failed to access keyring for deletion: {}", e);
        }
    }
}
