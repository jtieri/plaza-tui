use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    pub id: serde_json::Value, // could be int or string
    pub artist: String,
    pub album: Option<String>,
    pub title: String,
    pub length: Option<u32>,
    #[serde(default)]
    pub artwork_src: Option<String>,
    #[serde(default)]
    pub artwork_sm_src: Option<String>,
    #[serde(default)]
    pub preview_src: Option<String>,
    /// Reactions count — API returns this inside `song` object
    #[serde(default)]
    pub reactions: u32,
    /// Current playback position in seconds — API returns this inside `song` object
    #[serde(default)]
    pub position: Option<f64>,
}

impl Song {
    pub fn id_str(&self) -> String {
        match &self.id {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            v => v.to_string(),
        }
    }

    pub fn display_name(&self) -> String {
        format!("{} \u{2014} {}", self.artist, self.title)
    }

    pub fn duration_display(&self) -> String {
        match self.length {
            Some(secs) => format!("{}:{:02}", secs / 60, secs % 60),
            None => "--:--".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub email: Option<String>,
    pub created_at: Option<i64>,
}

impl User {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResource {
    pub song: Song,
    #[serde(default)]
    pub listeners: u32,
    #[serde(default)]
    pub updated_at: Option<i64>,
}

/// Generic wrapper for single-resource responses: `{"data": T}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataWrapper<T> {
    pub data: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
    pub remember: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub data: User,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    pub current_page: u32,
    pub last_page: u32,
    pub per_page: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paginated<T> {
    pub meta: PaginationMeta,
    pub data: Vec<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FavoriteEntry {
    pub id: u64,
    pub song: Song,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub song: Song,
    #[serde(default)]
    pub played_at: Option<i64>,
}

impl HistoryEntry {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RatingEntry {
    pub song: Song,
    #[serde(default)]
    pub likes: u32,
    #[serde(default)]
    pub rank: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub id: u64,
    pub text: String,
    #[serde(default)]
    pub author: Option<String>,
    pub created_at: Option<i64>,
}

impl NewsItem {
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserStats {
    #[serde(default)]
    pub reactions: u64,
    #[serde(default)]
    pub favorites: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongResource {
    pub song: Song,
    #[serde(default)]
    pub is_favorited: bool,
    #[serde(default)]
    pub favorite_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatingRange {
    Overtime,
    Weekly,
    Monthly,
}

impl RatingRange {
    pub fn as_str(&self) -> &'static str {
        match self {
            RatingRange::Overtime => "overtime",
            RatingRange::Weekly => "weekly",
            RatingRange::Monthly => "monthly",
        }
    }

    pub fn display(&self) -> &'static str {
        match self {
            RatingRange::Overtime => "All Time",
            RatingRange::Weekly => "Weekly",
            RatingRange::Monthly => "Monthly",
        }
    }
}
