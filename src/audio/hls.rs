use bytes::Bytes;
use m3u8_rs::{MasterPlaylist, MediaPlaylist, Playlist, parse_playlist_res};
use reqwest::Client;
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::mpsc;
use crate::error::{AudioError, PlazaError, Result};

pub async fn fetch_master_playlist(client: &Client, url: &str) -> Result<MasterPlaylist> {
    let content = client
        .get(url)
        .send()
        .await?
        .bytes()
        .await?;

    match parse_playlist_res(content.as_ref())
        .map_err(|_e| PlazaError::Audio(AudioError::HlsFailed(
            "Failed to parse master playlist".to_string()
        )))?
    {
        Playlist::MasterPlaylist(pl) => Ok(pl),
        Playlist::MediaPlaylist(_) => Err(PlazaError::Audio(AudioError::HlsFailed(
            "Expected master playlist but got media playlist".to_string()
        ))),
    }
}

pub fn select_stream_url(playlist: &MasterPlaylist) -> Option<String> {
    playlist.variants
        .iter()
        .max_by_key(|v| v.bandwidth)
        .map(|v| v.uri.clone())
}

pub async fn fetch_media_playlist(client: &Client, url: &str) -> Result<MediaPlaylist> {
    let content = client
        .get(url)
        .send()
        .await?
        .bytes()
        .await?;

    match parse_playlist_res(content.as_ref())
        .map_err(|_e| PlazaError::Audio(AudioError::HlsFailed(
            "Failed to parse media playlist".to_string()
        )))?
    {
        Playlist::MediaPlaylist(pl) => Ok(pl),
        Playlist::MasterPlaylist(_) => Err(PlazaError::Audio(AudioError::HlsFailed(
            "Expected media playlist but got master playlist".to_string()
        ))),
    }
}

pub fn resolve_segment_url(base_url: &str, segment_uri: &str) -> String {
    if segment_uri.starts_with("http://") || segment_uri.starts_with("https://") {
        return segment_uri.to_string();
    }

    // Get base directory
    let base = if base_url.ends_with('/') {
        base_url.to_string()
    } else {
        // Get everything up to the last /
        match base_url.rfind('/') {
            Some(idx) => base_url[..=idx].to_string(),
            None => format!("{}/", base_url),
        }
    };

    format!("{}{}", base, segment_uri)
}

pub async fn fetch_segment(client: &Client, url: &str) -> Result<Bytes> {
    let bytes = client
        .get(url)
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .bytes()
        .await?;
    Ok(bytes)
}

pub struct HlsSegmentStream {
    client: Client,
    media_playlist_url: String,
    seen_segments: HashSet<String>,
    target_duration: u64,
}

impl HlsSegmentStream {
    pub fn new(client: Client, media_playlist_url: String) -> Self {
        HlsSegmentStream {
            client,
            media_playlist_url,
            seen_segments: HashSet::new(),
            target_duration: 5,
        }
    }

    pub async fn next_segments(&mut self) -> Result<Vec<Bytes>> {
        let playlist = fetch_media_playlist(&self.client, &self.media_playlist_url).await?;

        self.target_duration = playlist.target_duration;

        let mut new_segments = Vec::new();

        for segment in &playlist.segments {
            let uri = &segment.uri;
            if self.seen_segments.contains(uri) {
                continue;
            }
            self.seen_segments.insert(uri.clone());

            let segment_url = resolve_segment_url(&self.media_playlist_url, uri);

            let mut retries = 0;
            loop {
                match fetch_segment(&self.client, &segment_url).await {
                    Ok(bytes) => {
                        new_segments.push(bytes);
                        break;
                    }
                    Err(e) if retries < 3 => {
                        retries += 1;
                        tracing::warn!("Segment fetch failed (retry {}): {}", retries, e);
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }
        }

        Ok(new_segments)
    }

    pub fn target_duration_secs(&self) -> u64 {
        self.target_duration
    }
}

/// Starts streaming HLS segments into a channel.
/// Returns a receiver that yields audio segment bytes.
pub async fn start_hls_stream(
    stream_url: &str,
    error_tx: mpsc::Sender<String>,
) -> Result<mpsc::Receiver<Bytes>> {
    let client = Client::builder()
        .user_agent("plaza-tui/0.1.0")
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| PlazaError::Audio(AudioError::HlsFailed(e.to_string())))?;

    // First, try to fetch a master playlist to find the actual media playlist URL
    let media_url = if stream_url.ends_with(".m3u8") && !stream_url.contains("stream") {
        // Might be a media playlist directly
        stream_url.to_string()
    } else {
        // Try as master playlist first
        match fetch_master_playlist(&client, stream_url).await {
            Ok(master) => {
                select_stream_url(&master)
                    .map(|uri| resolve_segment_url(stream_url, &uri))
                    .unwrap_or_else(|| stream_url.to_string())
            }
            Err(_) => {
                // Assume it's a media playlist directly
                stream_url.to_string()
            }
        }
    };

    tracing::info!("HLS media playlist URL: {}", media_url);

    let (tx, rx) = mpsc::channel::<Bytes>(16);

    tokio::spawn(async move {
        let mut stream = HlsSegmentStream::new(client, media_url);
        let mut consecutive_failures = 0;

        loop {
            match stream.next_segments().await {
                Ok(segments) => {
                    consecutive_failures = 0;
                    if segments.is_empty() {
                        // No new segments yet, wait
                        let wait = stream.target_duration_secs().max(1);
                        tokio::time::sleep(Duration::from_secs(wait)).await;
                    } else {
                        for segment in segments {
                            if tx.send(segment).await.is_err() {
                                tracing::info!("HLS stream receiver dropped, stopping");
                                return;
                            }
                        }
                        // Small wait to avoid hammering the server
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
                Err(e) => {
                    consecutive_failures += 1;
                    tracing::error!("HLS fetch failed: {}", e);

                    if consecutive_failures >= 3 {
                        let _ = error_tx.send(format!("HLS stream failed: {}", e)).await;
                        return;
                    }

                    let wait = (consecutive_failures * 2).min(10);
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                }
            }
        }
    });

    Ok(rx)
}
