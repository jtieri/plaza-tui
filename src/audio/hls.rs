//! HLS (Apple HTTP Live Streaming) audio source.
//!
//! Plaza's `/hls` endpoint is a master playlist pointing at three AAC bitrate
//! variants, each a sliding-window media playlist of MPEG-TS segments. This source
//! polls the chosen variant's media playlist, fetches new segments, demuxes the
//! AAC elementary stream out of the TS container ([`crate::audio::ts`]), and
//! decodes it to PCM with symphonia.
//!
//! It is fully synchronous (blocking reqwest) so it slots into the player's audio
//! thread alongside the other [`PcmSource`]s.

use std::collections::VecDeque;
use std::io::Cursor;
use std::time::{Duration, Instant};

use m3u8_rs::{Playlist, parse_playlist_res};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::audio::pcm::{PcmChunk, PcmError, PcmSource};
use crate::audio::sources::first_audio_track;
use crate::audio::ts::TsDemux;

pub struct HlsAacPcmSource {
    client: reqwest::blocking::Client,
    /// Resolved media-playlist URL for the chosen bitrate variant.
    media_url: String,
    /// Sequence number of the next segment we want (media_sequence + index).
    next_seq: Option<u64>,
    /// Persistent TS demuxer so the discovered audio PID carries across segments.
    demux: TsDemux,
    /// Decoded PCM ready to hand out.
    pending: VecDeque<PcmChunk>,
    /// Throttle media-playlist polling.
    last_poll: Option<Instant>,
    poll_interval: Duration,
}

impl HlsAacPcmSource {
    pub fn open(master_url: String) -> Result<Self, PcmError> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("plaza-tui/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| PcmError::Permanent(format!("HTTP client init failed: {e}")))?;

        let media_url = resolve_media_url(&client, &master_url)?;
        tracing::info!("HLS media playlist: {media_url}");

        Ok(HlsAacPcmSource {
            client,
            media_url,
            next_seq: None,
            demux: TsDemux::new(),
            pending: VecDeque::new(),
            last_poll: None,
            // Default; replaced by the playlist's EXT-X-TARGETDURATION.
            poll_interval: Duration::from_secs(2),
        })
    }

    /// Fetch the media playlist and decode any not-yet-seen segments into `pending`.
    fn refill(&mut self) -> Result<(), PcmError> {
        let body = self
            .client
            .get(&self.media_url)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.bytes())
            .map_err(|e| {
                tracing::warn!("HLS: media playlist fetch failed: {e}");
                PcmError::Ended
            })?;

        let media = match parse_playlist_res(&body) {
            Ok(Playlist::MediaPlaylist(m)) => m,
            Ok(Playlist::MasterPlaylist(_)) => {
                return Err(PcmError::Permanent(
                    "HLS: expected a media playlist, got a master playlist".into(),
                ))
            }
            Err(_) => {
                tracing::warn!("HLS: media playlist parse failed");
                return Err(PcmError::Ended);
            }
        };

        // Pace future polls to ~half the target duration (min 1s).
        let target = media.target_duration.max(1);
        self.poll_interval = Duration::from_secs((target / 2).max(1));

        let base_seq = media.media_sequence;
        let want_from = self.next_seq.unwrap_or(base_seq);
        for (i, seg) in media.segments.iter().enumerate() {
            let seq = base_seq + i as u64;
            if seq < want_from {
                continue;
            }
            let seg_url = resolve_url(&self.media_url, &seg.uri);
            match self.fetch_and_decode(&seg_url) {
                Ok(chunks) => self.pending.extend(chunks),
                Err(e) => {
                    tracing::warn!("HLS: segment {seq} failed: {e}");
                    // Skip this segment but keep going; advance past it.
                }
            }
            self.next_seq = Some(seq + 1);
        }
        Ok(())
    }

    fn fetch_and_decode(&mut self, url: &str) -> Result<Vec<PcmChunk>, PcmError> {
        let bytes = self
            .client
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.bytes())
            .map_err(|e| transient(format!("segment fetch: {e}")))?;
        self.demux.push(&bytes);
        let adts = self.demux.take();
        Ok(decode_adts_to_chunks(adts))
    }
}

impl PcmSource for HlsAacPcmSource {
    fn next_chunk(&mut self) -> Result<Option<PcmChunk>, PcmError> {
        // Top up the buffer when it's empty or the poll interval has elapsed, so we
        // stay ahead of playback without hammering the server.
        let due = self
            .last_poll
            .map(|t| t.elapsed() >= self.poll_interval)
            .unwrap_or(true);
        if self.pending.is_empty() || due {
            self.refill()?;
            self.last_poll = Some(Instant::now());
        }
        // Ok(None) when caught up at the live edge — the player backs off briefly.
        Ok(self.pending.pop_front())
    }
}

/// A transient end with a logged context message (not shown to the user).
fn transient(ctx: String) -> PcmError {
    tracing::warn!("HLS transient: {ctx}");
    PcmError::Ended
}

/// Resolve the master playlist into a concrete media-playlist URL, choosing the
/// highest-bitrate variant for best quality. If `master_url` is already a media
/// playlist, it is returned unchanged.
fn resolve_media_url(client: &reqwest::blocking::Client, master_url: &str) -> Result<String, PcmError> {
    let body = client
        .get(master_url)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.bytes())
        .map_err(|e| transient(format!("master playlist: {e}")))?;
    match parse_playlist_res(&body) {
        Ok(Playlist::MasterPlaylist(master)) => {
            let variant = master
                .variants
                .iter()
                .max_by_key(|v| v.bandwidth)
                .ok_or_else(|| PcmError::Permanent("HLS master playlist has no variants".into()))?;
            Ok(resolve_url(master_url, &variant.uri))
        }
        // Some servers hand back a media playlist directly.
        Ok(Playlist::MediaPlaylist(_)) => Ok(master_url.to_string()),
        Err(_) => Err(PcmError::Permanent("HLS: could not parse master playlist".into())),
    }
}

/// Resolve a possibly-relative playlist/segment URI against a base URL.
fn resolve_url(base_url: &str, uri: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }
    match base_url.rfind('/') {
        Some(idx) => format!("{}{}", &base_url[..=idx], uri),
        None => format!("{base_url}/{uri}"),
    }
}

/// Decode a buffer of ADTS (AAC) frames to PCM chunks via symphonia.
fn decode_adts_to_chunks(adts: Vec<u8>) -> Vec<PcmChunk> {
    if adts.is_empty() {
        return Vec::new();
    }
    let mss = MediaSourceStream::new(Box::new(Cursor::new(adts)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("aac");
    hint.mime_type("audio/aac");
    let probed = match symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions { enable_gapless: false, ..Default::default() },
        &MetadataOptions::default(),
    ) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let mut format = probed.format;
    let Some((track_id, params)) = first_audio_track(format.as_ref()) else {
        return Vec::new();
    };
    let mut decoder = match symphonia::default::get_codecs().make(&params, &DecoderOptions::default()) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut chunks = Vec::new();
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        if let Ok(decoded) = decoder.decode(&packet) {
            let spec = *decoded.spec();
            let mut sb = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            sb.copy_interleaved_ref(decoded);
            if !sb.samples().is_empty() {
                chunks.push(PcmChunk::new(
                    sb.samples().to_vec(),
                    spec.rate,
                    spec.channels.count() as u16,
                ));
            }
        }
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_url_handles_relative_and_absolute() {
        assert_eq!(
            resolve_url("https://radio.plaza.one/hls", "aac_hifi.m3u8"),
            "https://radio.plaza.one/aac_hifi.m3u8"
        );
        assert_eq!(
            resolve_url("https://radio.plaza.one/aac_hifi.m3u8", "seg_1.ts"),
            "https://radio.plaza.one/seg_1.ts"
        );
        assert_eq!(
            resolve_url("https://x/hls", "https://cdn/abs.ts"),
            "https://cdn/abs.ts"
        );
    }

    /// End-to-end (offline): the real TS fixture demuxes to ADTS and symphonia
    /// decodes it to non-silent PCM. This proves the full HLS decode chain minus
    /// the network polling.
    #[test]
    fn decodes_real_segment_fixture_to_audio() {
        let seg = include_bytes!("../../tests/fixtures/hls_aac_segment.ts");
        let mut demux = TsDemux::new();
        demux.push(seg);
        let adts = demux.take();
        let chunks = decode_adts_to_chunks(adts);
        assert!(!chunks.is_empty(), "should decode AAC chunks from the segment");
        let total: usize = chunks.iter().map(|c| c.samples.len()).sum();
        let nonzero: usize = chunks
            .iter()
            .flat_map(|c| c.samples.iter())
            .filter(|s| s.abs() > 1e-6)
            .count();
        assert!(total > 10_000, "expected substantial audio, got {total} samples");
        assert!(
            nonzero as f64 / total as f64 > 0.5,
            "segment looks silent: {nonzero}/{total}"
        );
        // Plaza HLS is 44.1kHz stereo AAC-LC.
        assert_eq!(chunks[0].channels, 2);
    }
}
