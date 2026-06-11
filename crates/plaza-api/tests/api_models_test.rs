use plaza_api::models::*;

#[test]
fn test_song_deserialize() {
    let json = r#"{
        "id": 42,
        "artist": "Macintosh Plus",
        "album": "Floral Shoppe",
        "title": "リサフランク420",
        "length": 214,
        "artwork_src": "https://plaza.one/art/42.jpg",
        "artwork_sm_src": "https://plaza.one/art/42_sm.jpg",
        "preview_src": null
    }"#;
    let song: Song = serde_json::from_str(json).unwrap();
    assert_eq!(song.artist, "Macintosh Plus");
    assert_eq!(song.title, "リサフランク420");
    assert_eq!(song.album.as_deref(), Some("Floral Shoppe"));
    assert_eq!(song.length, Some(214));
    assert_eq!(song.id_str(), "42");
    assert_eq!(song.duration_display(), "3:34");
}

#[test]
fn test_song_string_id() {
    let json = r#"{"id": "abc-123", "artist": "Artist", "title": "Title"}"#;
    let song: Song = serde_json::from_str(json).unwrap();
    assert_eq!(song.id_str(), "abc-123");
}

#[test]
fn test_status_resource_deserialize() {
    // reactions and position live inside song, matching the actual Plaza API format
    let json = r#"{
        "song": {
            "id": 1,
            "artist": "HOME",
            "title": "Resonance",
            "length": 213,
            "reactions": 42,
            "position": 101.5
        },
        "listeners": 1337,
        "updated_at": 1700000000
    }"#;
    let status: StatusResource = serde_json::from_str(json).unwrap();
    assert_eq!(status.song.artist, "HOME");
    assert_eq!(status.listeners, 1337);
    assert_eq!(status.song.reactions, 42);
    assert!((status.song.position.unwrap() - 101.5).abs() < 0.01);
}

#[test]
fn test_paginated_history_deserialize() {
    let json = r#"{
        "meta": {
            "current_page": 1,
            "last_page": 5,
            "per_page": 25,
            "total": 123
        },
        "data": [
            {
                "song": {"id": 10, "artist": "Boards of Canada", "title": "Roygbiv"},
                "played_at": 1700000000
            }
        ]
    }"#;
    let page: Paginated<HistoryEntry> = serde_json::from_str(json).unwrap();
    assert_eq!(page.meta.current_page, 1);
    assert_eq!(page.meta.last_page, 5);
    assert_eq!(page.meta.total, 123);
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].song.artist, "Boards of Canada");
    assert_eq!(page.data[0].played_at, Some(1700000000));
}

#[test]
fn test_favorite_entry_deserialize() {
    let json = r#"{
        "id": 99,
        "song": {"id": 5, "artist": "Daft Punk", "title": "Digital Love"},
        "created_at": 1699000000
    }"#;
    let fav: FavoriteEntry = serde_json::from_str(json).unwrap();
    assert_eq!(fav.id, 99);
    assert_eq!(fav.song.title, "Digital Love");
    assert_eq!(fav.created_at, Some(1699000000));
}

#[test]
fn test_rating_entry_deserialize() {
    let json = r#"{
        "song": {"id": 7, "artist": "Kavinsky", "title": "Nightcall"},
        "likes": 4521,
        "rank": 1
    }"#;
    let rating: RatingEntry = serde_json::from_str(json).unwrap();
    assert_eq!(rating.likes, 4521);
    assert_eq!(rating.rank, Some(1));
    assert_eq!(rating.song.artist, "Kavinsky");
}

#[test]
fn test_news_item_deserialize() {
    let json = r#"{
        "id": 11,
        "text": "Welcome to the new plaza!",
        "author": "Admin",
        "created_at": 1698000000
    }"#;
    let news: NewsItem = serde_json::from_str(json).unwrap();
    assert_eq!(news.id, 11);
    assert_eq!(news.text, "Welcome to the new plaza!");
    assert_eq!(news.author.as_deref(), Some("Admin"));
}

#[test]
fn test_user_deserialize() {
    let json = r#"{
        "id": 1000,
        "username": "vaporwave_fan",
        "email": "test@example.com",
        "created_at": 1500000000
    }"#;
    let user: User = serde_json::from_str(json).unwrap();
    assert_eq!(user.id, 1000);
    assert_eq!(user.username, "vaporwave_fan");
    assert_eq!(user.email.as_deref(), Some("test@example.com"));
}

#[test]
fn test_user_stats_deserialize() {
    let json = r#"{"reactions": 500, "favorites": 42}"#;
    let stats: UserStats = serde_json::from_str(json).unwrap();
    assert_eq!(stats.reactions, 500);
    assert_eq!(stats.favorites, 42);
}

#[test]
fn test_user_stats_defaults() {
    // Missing fields should default to 0
    let json = r#"{}"#;
    let stats: UserStats = serde_json::from_str(json).unwrap();
    assert_eq!(stats.reactions, 0);
    assert_eq!(stats.favorites, 0);
}

#[test]
fn test_song_duration_display_no_length() {
    let json = r#"{"id": 1, "artist": "A", "title": "B"}"#;
    let song: Song = serde_json::from_str(json).unwrap();
    assert_eq!(song.duration_display(), "--:--");
}

#[test]
fn test_rating_range_as_str() {
    assert_eq!(RatingRange::Overtime.as_str(), "overtime");
    assert_eq!(RatingRange::Weekly.as_str(), "weekly");
    assert_eq!(RatingRange::Monthly.as_str(), "monthly");
}
