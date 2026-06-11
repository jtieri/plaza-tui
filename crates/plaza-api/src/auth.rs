//! Login and bearer-token persistence.
//!
//! The token is stored in the OS keyring when available, and mirrored to a
//! permission-restricted file so a session survives even where the keyring does
//! not persist between runs.

use std::path::PathBuf;

use keyring::Entry;

use crate::error::{Error, Result};
use crate::models::{LoginForm, LoginResponse};
use crate::ApiClient;

const KEYRING_SERVICE: &str = "plaza-tui";
const KEYRING_ACCOUNT: &str = "auth-token";

/// Authenticate with a username and password, returning a bearer token.
///
/// # Errors
/// Returns [`Error::Unauthorized`] for bad credentials, [`Error::Server`] for
/// other non-success statuses, or [`Error::Http`] on a transport failure.
pub async fn login(client: &ApiClient, username: &str, password: &str) -> Result<String> {
    let form = LoginForm {
        username: username.to_string(),
        password: password.to_string(),
        remember: true,
    };

    let url = client.url("v2/auth/token");
    let response: reqwest::Response = client.client.post(&url).json(&form).send().await?;

    match response.status() {
        s if s == 200 || s == 201 => {
            let login_resp: LoginResponse = response.json().await?;
            Ok(login_resp.token)
        }
        s if s == 401 => Err(Error::Unauthorized),
        s => Err(Error::Server { status: s.as_u16() }),
    }
}

/// Invalidate the session server-side and clear the locally stored token.
///
/// # Errors
/// This always clears the local token; the returned `Result` is currently `Ok`
/// even if the server request fails, since logout should never strand a session.
pub async fn logout(client: &ApiClient) -> Result<()> {
    let url = client.url("v2/auth/logout");
    let _ = client
        .auth_request(reqwest::Method::POST, &url)
        .send()
        .await;
    delete_token();
    Ok(())
}

/// Persist the bearer token to the OS keyring and a file fallback.
pub fn save_token(token: &str) {
    // Save to keyring
    match Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
        Ok(entry) => match entry.set_password(token) {
            Ok(()) => tracing::debug!("Token saved to keyring"),
            Err(e) => tracing::warn!("Failed to save token to keyring: {}", e),
        },
        Err(e) => tracing::warn!("Failed to access keyring: {}", e),
    }

    // Always save to file as well, since keyring may not persist
    save_token_file(token);
}

/// Load the saved bearer token, preferring the keyring and falling back to the file.
pub fn load_token() -> Option<String> {
    // Try keyring first
    match Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT) {
        Ok(entry) => match entry.get_password() {
            Ok(token) => {
                tracing::debug!("Token loaded from keyring");
                return Some(token);
            }
            Err(e) => tracing::debug!("Keyring load failed: {}", e),
        },
        Err(e) => tracing::debug!("Keyring access failed: {}", e),
    }

    // Fall back to file
    load_token_file()
}

/// Remove the saved bearer token from both the keyring and the file fallback.
pub fn delete_token() {
    // Delete from keyring
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

    // Also delete file fallback
    let path = token_file_path();
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            tracing::warn!("Failed to delete token file: {}", e);
        }
    }
}

// --- File-based fallback ---

fn token_file_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("plaza-tui")
        .join(".token")
}

fn save_token_file(token: &str) {
    let path = token_file_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("Failed to create token dir: {}", e);
            return;
        }
    }

    match std::fs::write(&path, token) {
        Ok(()) => {
            // Restrict file permissions on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            tracing::debug!("Token saved to file fallback");
        }
        Err(e) => tracing::warn!("Failed to save token file: {}", e),
    }
}

fn load_token_file() -> Option<String> {
    let path = token_file_path();
    match std::fs::read_to_string(&path) {
        Ok(token) => {
            let token = token.trim().to_string();
            if token.is_empty() {
                None
            } else {
                tracing::debug!("Token loaded from file fallback");
                Some(token)
            }
        }
        Err(_) => None,
    }
}
