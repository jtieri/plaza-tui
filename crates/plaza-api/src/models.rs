//! Response and request types for the Plaza REST and Socket.IO APIs.
//!
//! These mirror the JSON shapes the backend returns. Fields the server may omit
//! are `#[serde(default)]` so partial payloads still deserialize.

use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// A track: the unit of "now playing", history, favourites, and charts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    /// Server id. Encoded as either a JSON number or string depending on the
    /// endpoint; use [`Song::id_str`] for a stable string form.
    pub id: serde_json::Value,
    /// Performing artist.
    pub artist: String,
    /// Album title, if known.
    pub album: Option<String>,
    /// Track title.
    pub title: String,
    /// Track length in seconds, if known.
    pub length: Option<u32>,
    /// Full-size artwork URL.
    #[serde(default)]
    pub artwork_src: Option<String>,
    /// Thumbnail artwork URL.
    #[serde(default)]
    pub artwork_sm_src: Option<String>,
    /// URL of a short preview clip, if one is available.
    #[serde(default)]
    pub preview_src: Option<String>,
    /// Total reactions this track has received.
    #[serde(default)]
    pub reactions: u32,
    /// Playback position within the track, in seconds, when nested in a status update.
    #[serde(default)]
    pub position: Option<f64>,
}

impl Song {
    /// The id as a string, regardless of how the server encoded it.
    pub fn id_str(&self) -> String {
        match &self.id {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            v => v.to_string(),
        }
    }

    /// `"Artist — Title"`, for single-line display.
    pub fn display_name(&self) -> String {
        format!("{} \u{2014} {}", self.artist, self.title)
    }

    /// Length formatted as `M:SS`, or `--:--` when unknown.
    pub fn duration_display(&self) -> String {
        match self.length {
            Some(secs) => format!("{}:{:02}", secs / 60, secs % 60),
            None => "--:--".to_string(),
        }
    }
}

/// An authenticated user account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Server id.
    pub id: u64,
    /// Display name.
    pub username: String,
    /// Account email, if the server returns it.
    pub email: Option<String>,
    /// Account creation time as a Unix timestamp.
    pub created_at: Option<i64>,
}

impl User {
    /// The "member since" month and year (e.g. `"March 2024"`), or `"Unknown"`.
    pub fn member_since(&self) -> String {
        match self.created_at {
            Some(ts) => {
                let dt = Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now);
                dt.format("%B %Y").to_string()
            }
            None => "Unknown".to_string(),
        }
    }
}

/// The live broadcast state: the current song and listener count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResource {
    /// The currently playing song.
    pub song: Song,
    /// Number of listeners currently tuned in.
    #[serde(default)]
    pub listeners: u32,
    /// When this status was produced, as a Unix timestamp.
    #[serde(default)]
    pub updated_at: Option<i64>,
}

/// Generic envelope for single-resource responses shaped like `{"data": T}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataWrapper<T> {
    /// The wrapped resource.
    pub data: T,
}

/// Credentials submitted to the login endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginForm {
    /// Account username.
    pub username: String,
    /// Account password.
    pub password: String,
    /// Whether to request a long-lived session.
    pub remember: bool,
}

/// A successful login: the user record plus a bearer token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    /// The authenticated user.
    pub data: User,
    /// Bearer token for subsequent authenticated requests.
    pub token: String,
}

/// Pagination metadata accompanying a [`Paginated`] response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    /// 1-based index of the returned page.
    pub current_page: u32,
    /// Index of the last available page.
    pub last_page: u32,
    /// Items per page.
    pub per_page: u32,
    /// Total items across all pages.
    pub total: u32,
}

/// A page of results plus its [`PaginationMeta`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paginated<T> {
    /// Pagination metadata.
    pub meta: PaginationMeta,
    /// The items on this page.
    pub data: Vec<T>,
}

/// One entry in the user's favourites library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FavoriteEntry {
    /// Favourite id, used to remove it later.
    pub id: u64,
    /// The favourited song.
    pub song: Song,
    /// When it was favourited, as a Unix timestamp.
    pub created_at: Option<i64>,
}

/// One entry in the recently played history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// The song that played.
    pub song: Song,
    /// When it played, as a Unix timestamp.
    #[serde(default)]
    pub played_at: Option<i64>,
}

impl HistoryEntry {
    /// The play time formatted as `MM/DD HH:MM`, or a placeholder when unknown.
    pub fn played_at_display(&self) -> String {
        match self.played_at {
            Some(ts) => {
                let dt = Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now);
                dt.format("%m/%d %H:%M").to_string()
            }
            None => "--/-- --:--".to_string(),
        }
    }
}

/// One ranked entry in a ratings chart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RatingEntry {
    /// The ranked song.
    pub song: Song,
    /// Number of likes over the chart's range.
    #[serde(default)]
    pub likes: u32,
    /// 1-based rank within the chart, if provided.
    #[serde(default)]
    pub rank: Option<u32>,
}

/// A news / blog post.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    /// Server id.
    pub id: u64,
    /// Post body (may contain HTML).
    pub text: String,
    /// Author name, if attributed.
    #[serde(default)]
    pub author: Option<String>,
    /// Publication time as a Unix timestamp.
    pub created_at: Option<i64>,
}

impl NewsItem {
    /// The publication time formatted as `YYYY-MM-DD HH:MM`, or `"Unknown"`.
    pub fn created_at_display(&self) -> String {
        match self.created_at {
            Some(ts) => {
                let dt = Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now);
                dt.format("%Y-%m-%d %H:%M").to_string()
            }
            None => "Unknown".to_string(),
        }
    }
}

/// Aggregate counts for the signed-in user's profile.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserStats {
    /// Total reactions the user has sent.
    #[serde(default)]
    pub reactions: u64,
    /// Total songs the user has favourited.
    #[serde(default)]
    pub favorites: u64,
}

/// A song detail response, including the viewer's relationship to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongResource {
    /// The song.
    pub song: Song,
    /// Whether the signed-in user has favourited it.
    #[serde(default)]
    pub is_favorited: bool,
    /// The favourite id, if favourited (used to remove it).
    #[serde(default)]
    pub favorite_id: Option<u64>,
}

/// The time window for a ratings chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatingRange {
    /// All-time ratings.
    Overtime,
    /// The past week.
    Weekly,
    /// The past month.
    Monthly,
}

impl RatingRange {
    /// The value used in the API path segment (e.g. `"weekly"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            RatingRange::Overtime => "overtime",
            RatingRange::Weekly => "weekly",
            RatingRange::Monthly => "monthly",
        }
    }

    /// A human-readable label for the UI (e.g. `"All Time"`).
    pub fn display(&self) -> &'static str {
        match self {
            RatingRange::Overtime => "All Time",
            RatingRange::Weekly => "Weekly",
            RatingRange::Monthly => "Monthly",
        }
    }
}
