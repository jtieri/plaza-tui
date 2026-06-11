//! Black-box tests for [`plaza_api::ApiClient`] against a mock HTTP server.
//!
//! These cover request shape (path, query, auth header), response parsing, and the
//! mapping from HTTP status codes to [`plaza_api::Error`] variants — without
//! touching the live backend.

use plaza_api::{ApiClient, Error};
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A minimal but complete `song` object for embedding in responses.
fn song_json() -> serde_json::Value {
    json!({
        "id": 42,
        "artist": "Macintosh Plus",
        "album": "Floral Shoppe",
        "title": "リサフランク420 / 現代のコンピュー",
        "length": 432,
    })
}

#[tokio::test]
async fn get_history_parses_pagination_and_items() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/history"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": { "current_page": 1, "last_page": 3, "per_page": 20, "total": 50 },
            "data": [ { "song": song_json(), "played_at": 1_700_000_000 } ],
        })))
        .mount(&server)
        .await;

    let client = ApiClient::new(None).with_base_url(&server.uri());
    let page = client.get_history(1).await.expect("history should parse");

    assert_eq!(page.meta.last_page, 3);
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].song.artist, "Macintosh Plus");
}

#[tokio::test]
async fn authenticated_requests_send_bearer_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/users/me"))
        .and(header("authorization", "Bearer secret-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "id": 7, "username": "vapor", "email": null, "created_at": null }
        })))
        .mount(&server)
        .await;

    let client = ApiClient::new(Some("secret-token".to_string())).with_base_url(&server.uri());
    let user = client.get_me().await.expect("profile should parse");

    assert_eq!(user.id, 7);
    assert_eq!(user.username, "vapor");
}

#[tokio::test]
async fn add_favorite_posts_song_id_and_returns_entry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/users/me/favorites"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "data": { "id": 99, "song": song_json(), "created_at": 1_700_000_000 }
        })))
        .mount(&server)
        .await;

    let client = ApiClient::new(Some("t".to_string())).with_base_url(&server.uri());
    let entry = client
        .add_favorite("42")
        .await
        .expect("favorite should parse");

    assert_eq!(entry.id, 99);
}

#[tokio::test]
async fn send_reaction_returns_updated_total() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/reactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "reactions": 1234 })))
        .mount(&server)
        .await;

    let client = ApiClient::new(Some("t".to_string())).with_base_url(&server.uri());
    assert_eq!(client.send_reaction(2).await.unwrap(), 1234);
}

#[tokio::test]
async fn send_reaction_without_count_field_is_unexpected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/reactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
        .mount(&server)
        .await;

    let client = ApiClient::new(Some("t".to_string())).with_base_url(&server.uri());
    assert!(matches!(
        client.send_reaction(2).await,
        Err(Error::Unexpected(_))
    ));
}

#[tokio::test]
async fn status_codes_map_to_error_variants() {
    let cases = [
        (401, "unauthorized"),
        (404, "not found"),
        (429, "rate limited"),
        (503, "server"),
    ];
    for (code, _label) in cases {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/users/me"))
            .respond_with(ResponseTemplate::new(code))
            .mount(&server)
            .await;

        let client = ApiClient::new(Some("t".to_string())).with_base_url(&server.uri());
        let err = client.get_me().await.expect_err("should be an error");
        match (code, err) {
            (401, Error::Unauthorized) => {}
            (404, Error::NotFound) => {}
            (429, Error::RateLimited) => {}
            (503, Error::Server { status: 503 }) => {}
            (c, other) => panic!("status {c} mapped to unexpected error {other:?}"),
        }
    }
}
