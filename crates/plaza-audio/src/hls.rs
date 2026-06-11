//! HLS (Apple HTTP Live Streaming) audio source.
//!
//! Plaza's `/hls` endpoint is a master playlist pointing at three AAC bitrate
//! variants, each a sliding-window media playlist of MPEG-TS segments. This source
//! polls the chosen variant's media playlist, fetches new segments, demuxes the
//! AAC elementary stream out of the TS container ([`crate::ts`]), and
//! decodes it to PCM with symphonia.
//!
//! ## Threading and latency
//!
//! Fetching and decoding run on a **background thread** that feeds a bounded
//! channel; [`next_chunk`](HlsAacPcmSource::next_chunk) only does a non-blocking
//! `try_recv`. This is deliberate: doing the network/decode work inline on the
//! player's audio thread starves the sink during each refill, causing audible
//! drop-outs.
//!
//! Latency is bounded by `select_window`: we start near the live edge and, if we
//! ever fall more than `BUFFER_SEGMENTS` behind it (e.g. after a network stall),
//! we **skip forward** instead of playing every buffered segment in order. Without
//! this, audio drifts further and further behind the socket "now playing" metadata
//! (which is always at the live edge) — the drift never recovers.

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::Duration;

use m3u8_rs::{parse_playlist_res, Playlist};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::pcm::{PcmChunk, PcmError, PcmSource};
use crate::sources::first_audio_track;
use crate::ts::TsDemux;

/// How far behind the live edge we keep buffered, in segments. Plaza segments are
/// ~4s, so 2 ≈ 8s of latency — enough to ride out jitter without the large drift
/// of playing the whole sliding window from its oldest segment.
const BUFFER_SEGMENTS: u64 = 2;

/// Decoded-chunk channel capacity. The fetcher can never run ahead of the live
/// playlist, so this only caps memory if the consumer stalls (e.g. paused).
const CHANNEL_CAP: usize = 512;

/// Give up (and let the player reconnect) after this many consecutive poll failures.
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// A [`PcmSource`] for Plaza's HLS endpoint. A background thread fetches and
/// decodes AAC segments into a bounded buffer; [`next_chunk`](HlsAacPcmSource::next_chunk)
/// drains it without blocking on the network.
pub struct HlsAacPcmSource {
    rx: Receiver<PcmChunk>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl HlsAacPcmSource {
    /// Open the HLS stream at `master_url`, resolving it to the highest-bitrate
    /// variant and starting the background fetcher.
    ///
    /// # Errors
    /// [`PcmError::Permanent`] if the client can't be built or the master playlist
    /// has no variants; [`PcmError::Ended`] for a transient fetch failure.
    pub fn open(master_url: String) -> Result<Self, PcmError> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("plaza-tui/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| PcmError::Permanent(format!("HTTP client init failed: {e}")))?;

        // Resolve master -> media playlist up front so open() fails fast on a bad URL.
        let media_url = resolve_media_url(&client, &master_url)?;
        tracing::info!("HLS media playlist: {media_url}");

        let (tx, rx) = sync_channel::<PcmChunk>(CHANNEL_CAP);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("plaza-hls-fetch".to_string())
            .spawn(move || fetch_loop(client, media_url, tx, stop_for_thread))
            .map_err(|e| PcmError::Permanent(format!("HLS fetch thread spawn failed: {e}")))?;

        Ok(HlsAacPcmSource {
            rx,
            stop,
            handle: Some(handle),
        })
    }
}

impl PcmSource for HlsAacPcmSource {
    fn next_chunk(&mut self) -> Result<Option<PcmChunk>, PcmError> {
        use std::sync::mpsc::TryRecvError;
        match self.rx.try_recv() {
            Ok(chunk) => Ok(Some(chunk)),
            // Buffer momentarily empty (live edge / brief jitter): not an error.
            Err(TryRecvError::Empty) => Ok(None),
            // Fetcher exited (repeated failures): transient end -> player reconnects.
            Err(TryRecvError::Disconnected) => Err(PcmError::Ended),
        }
    }
}

impl Drop for HlsAacPcmSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Detach the fetch thread; dropping the receiver unblocks its in-flight send
        // so it exits promptly. We don't join — shutdown shouldn't wait on a fetch.
        self.handle.take();
    }
}

/// Result of one poll cycle in the fetch loop.
enum Poll {
    /// Made progress (or simply nothing new); sleep this long, then poll again.
    Ok(Duration),
    /// A transient failure (network/parse); retry shortly.
    Retry,
    /// The consumer dropped the receiver — exit the thread.
    Closed,
}

/// Background fetch/decode loop: keep the channel fed with PCM near the live edge.
fn fetch_loop(
    client: reqwest::blocking::Client,
    media_url: String,
    tx: SyncSender<PcmChunk>,
    stop: Arc<AtomicBool>,
) {
    let mut demux = TsDemux::new();
    let mut next_seq: Option<u64> = None;
    let mut failures: u32 = 0;

    while !stop.load(Ordering::SeqCst) {
        match poll_once(&client, &media_url, &mut demux, &mut next_seq, &tx, &stop) {
            Poll::Ok(interval) => {
                failures = 0;
                sleep_interruptible(interval, &stop);
            }
            Poll::Retry => {
                failures += 1;
                if failures >= MAX_CONSECUTIVE_FAILURES {
                    tracing::error!("HLS: giving up after {failures} consecutive failures");
                    return;
                }
                sleep_interruptible(Duration::from_secs(1), &stop);
            }
            Poll::Closed => return,
        }
    }
}

fn poll_once(
    client: &reqwest::blocking::Client,
    media_url: &str,
    demux: &mut TsDemux,
    next_seq: &mut Option<u64>,
    tx: &SyncSender<PcmChunk>,
    stop: &Arc<AtomicBool>,
) -> Poll {
    let body = match client
        .get(media_url)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.bytes())
    {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("HLS: media playlist fetch failed: {e}");
            return Poll::Retry;
        }
    };
    let media = match parse_playlist_res(&body) {
        Ok(Playlist::MediaPlaylist(m)) => m,
        Ok(Playlist::MasterPlaylist(_)) => {
            tracing::warn!("HLS: expected media playlist, got master");
            return Poll::Retry;
        }
        Err(_) => {
            tracing::warn!("HLS: media playlist parse failed");
            return Poll::Retry;
        }
    };

    let interval = Duration::from_secs((media.target_duration.max(1) / 2).max(1));
    let base_seq = media.media_sequence;
    let count = media.segments.len();

    let prev_next = *next_seq;
    let window = select_window(base_seq, count, prev_next, BUFFER_SEGMENTS);
    if let Some(prev) = prev_next {
        if window.start > prev {
            tracing::warn!(
                "HLS: fell behind live, skipping {} segment(s) to re-sync",
                window.start - prev
            );
        }
    }
    *next_seq = Some(window.end);

    for seq in window {
        if stop.load(Ordering::SeqCst) {
            return Poll::Closed;
        }
        let idx = (seq - base_seq) as usize;
        let Some(seg) = media.segments.get(idx) else {
            continue;
        };
        let seg_url = resolve_url(media_url, &seg.uri);
        match fetch_and_decode(client, &seg_url, demux) {
            Ok(chunks) => {
                for chunk in chunks {
                    match tx.try_send(chunk) {
                        Ok(()) => {}
                        Err(TrySendError::Full(chunk)) => {
                            // Consumer is slower than fetch (e.g. paused): block until
                            // there's room, bailing out if the consumer goes away.
                            if blocking_send(tx, chunk, stop).is_err() {
                                return Poll::Closed;
                            }
                        }
                        Err(TrySendError::Disconnected(_)) => return Poll::Closed,
                    }
                }
            }
            Err(e) => tracing::warn!("HLS: segment {seq} failed: {e}"),
        }
    }
    Poll::Ok(interval)
}

/// Send a chunk, waiting (in short interruptible steps) for channel space.
/// Returns Err if the receiver is gone or a stop was requested.
fn blocking_send(
    tx: &SyncSender<PcmChunk>,
    mut chunk: PcmChunk,
    stop: &Arc<AtomicBool>,
) -> Result<(), ()> {
    loop {
        if stop.load(Ordering::SeqCst) {
            return Err(());
        }
        match tx.try_send(chunk) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Full(c)) => {
                chunk = c;
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(TrySendError::Disconnected(_)) => return Err(()),
        }
    }
}

fn sleep_interruptible(dur: Duration, stop: &Arc<AtomicBool>) {
    let step = Duration::from_millis(50);
    let mut left = dur;
    while left > Duration::ZERO {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let s = step.min(left);
        std::thread::sleep(s);
        left = left.saturating_sub(s);
    }
}

fn fetch_and_decode(
    client: &reqwest::blocking::Client,
    url: &str,
    demux: &mut TsDemux,
) -> Result<Vec<PcmChunk>, String> {
    let bytes = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.bytes())
        .map_err(|e| format!("segment fetch: {e}"))?;
    demux.push(&bytes);
    let adts = demux.take();
    Ok(decode_adts_to_chunks(adts))
}

/// Decide which segment sequence numbers to fetch this poll, bounding how far
/// behind the live edge we play.
///
/// `media_sequence` is the sequence of the first segment in the playlist;
/// `segment_count` how many it has; `next_seq` the next sequence we intend to play
/// (None on the first poll). We keep at most `buffer_segments` behind the live edge
/// — on the first poll we *start* there, and on later polls we skip forward to it
/// if a stall left us further behind. Returns the half-open sequence range to fetch
/// (its `end` is the new `next_seq`); an empty range means "nothing new yet".
fn select_window(
    media_sequence: u64,
    segment_count: usize,
    next_seq: Option<u64>,
    buffer_segments: u64,
) -> std::ops::Range<u64> {
    let live_edge = media_sequence + segment_count as u64;
    let floor = live_edge.saturating_sub(buffer_segments);
    let start = match next_seq {
        None => floor,             // first poll: begin buffer_segments behind live
        Some(ns) => ns.max(floor), // never drift further than buffer_segments behind
    };
    // Clamp into what the playlist actually offers.
    let start = start.clamp(media_sequence, live_edge);
    start..live_edge
}

/// Resolve the master playlist into a concrete media-playlist URL, choosing the
/// highest-bitrate variant for best quality. If `master_url` is already a media
/// playlist, it is returned unchanged.
fn resolve_media_url(
    client: &reqwest::blocking::Client,
    master_url: &str,
) -> Result<String, PcmError> {
    let body = client
        .get(master_url)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.bytes())
        .map_err(|e| {
            tracing::warn!("HLS: master playlist fetch failed: {e}");
            PcmError::Ended
        })?;
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
        Err(_) => Err(PcmError::Permanent(
            "HLS: could not parse master playlist".into(),
        )),
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
        &FormatOptions {
            enable_gapless: false,
            ..Default::default()
        },
        &MetadataOptions::default(),
    ) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let mut format = probed.format;
    let Some((track_id, params)) = first_audio_track(format.as_ref()) else {
        return Vec::new();
    };
    let mut decoder =
        match symphonia::default::get_codecs().make(&params, &DecoderOptions::default()) {
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

    // --- select_window: the latency/drift-bounding logic (regression tests for the
    //     "audio runs 20s behind the Now Playing tab" bug). ---

    #[test]
    fn first_poll_starts_near_live_edge_not_oldest_segment() {
        // Playlist seq 100..103 (3 segments). With BUFFER_SEGMENTS=2 we must start at
        // 101 (2 behind live), NOT 100 — starting at the oldest is the latency bug.
        let w = select_window(100, 3, None, 2);
        assert_eq!(w, 101..103, "should start 2 segments behind the live edge");
    }

    #[test]
    fn steady_state_fetches_only_new_segments() {
        // Window slid by one (now 101..104); we already consumed through 103.
        let w = select_window(101, 3, Some(103), 2);
        assert_eq!(w, 103..104, "should fetch exactly the one new segment");
    }

    #[test]
    fn nothing_new_yields_empty_range() {
        let w = select_window(101, 3, Some(104), 2);
        assert!(w.is_empty(), "no new segments -> empty range, got {w:?}");
    }

    #[test]
    fn skips_forward_when_fallen_behind() {
        // We're at 103 but the playlist jumped far ahead (live edge now 113) after a
        // stall. We must skip to within BUFFER_SEGMENTS of live (111), not replay the
        // gap 103..111 — that is what caused the unbounded ~20s drift.
        let w = select_window(110, 3, Some(103), 2);
        assert_eq!(w, 111..113, "should skip forward to re-sync near live");
        assert!(w.start > 103, "must drop the stale gap rather than play it");
    }

    #[test]
    fn never_buffers_more_than_buffer_segments_behind_live() {
        // For any state, the start is at most BUFFER_SEGMENTS behind the live edge.
        for (ms, count, next) in [(100u64, 5usize, None), (100, 5, Some(0)), (50, 3, Some(40))] {
            let w = select_window(ms, count, next, 2);
            let live = ms + count as u64;
            assert!(
                live - w.start <= 2,
                "latency exceeded buffer for ms={ms} next={next:?}"
            );
        }
    }

    #[test]
    fn handles_empty_playlist_without_panicking() {
        let w = select_window(100, 0, None, 2);
        assert!(w.is_empty());
        let w = select_window(100, 0, Some(50), 2);
        assert!(w.is_empty());
    }

    /// End-to-end (offline): the real TS fixture demuxes to ADTS and symphonia
    /// decodes it to non-silent PCM — the full HLS decode chain minus networking.
    #[test]
    fn decodes_real_segment_fixture_to_audio() {
        let seg = include_bytes!("../tests/fixtures/hls_aac_segment.ts");
        let mut demux = TsDemux::new();
        demux.push(seg);
        let adts = demux.take();
        let chunks = decode_adts_to_chunks(adts);
        assert!(
            !chunks.is_empty(),
            "should decode AAC chunks from the segment"
        );
        let total: usize = chunks.iter().map(|c| c.samples.len()).sum();
        let nonzero: usize = chunks
            .iter()
            .flat_map(|c| c.samples.iter())
            .filter(|s| s.abs() > 1e-6)
            .count();
        assert!(
            total > 10_000,
            "expected substantial audio, got {total} samples"
        );
        assert!(
            nonzero as f64 / total as f64 > 0.5,
            "segment looks silent: {nonzero}/{total}"
        );
        assert_eq!(chunks[0].channels, 2);
    }
}
