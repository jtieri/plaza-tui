//! Network smoke tests against the live Plaza endpoints.
//!
//! These are `#[ignore]`d so they never run in offline/CI builds. Run them on
//! demand to verify the real stream still decodes with our codec set:
//!
//! ```sh
//! cargo test --test stream_smoke_test -- --ignored --nocapture
//! ```
//!
//! `test_mp3_stream_decodes_to_audio` is the end-to-end proof for the Phase 0 fix:
//! the default `/mp3` stream is reachable, has the correct URL, and decodes to
//! non-silent PCM via symphonia (exactly the path `audio::player` uses).

use std::io::Read;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Fetch up to `max_bytes` from a URL into memory (blocking).
fn fetch_bytes(url: &str, max_bytes: usize) -> Vec<u8> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("plaza-tui-test/0.1")
        .build()
        .expect("build client");
    let mut resp = client.get(url).send().expect("send request");
    assert!(resp.status().is_success(), "GET {url} -> {}", resp.status());
    let mut buf = vec![0u8; max_bytes];
    let mut filled = 0;
    while filled < max_bytes {
        match resp.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => break,
        }
    }
    buf.truncate(filled);
    buf
}

/// Decode an in-memory audio blob and return (decoded_sample_count, nonzero_sample_count).
fn decode_pcm_stats(bytes: Vec<u8>, mime: &str) -> (usize, usize) {
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    hint.mime_type(mime);
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions { enable_gapless: false, ..Default::default() },
            &MetadataOptions::default(),
        )
        .expect("probe format");
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .expect("an audio track");
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .expect("make decoder");

    let mut total = 0usize;
    let mut nonzero = 0usize;
    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    // Decode whatever packets we have buffered; stop at end of the blob.
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let Ok(decoded) = decoder.decode(&packet) else { continue };
        if sample_buf.is_none() {
            sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec()));
        }
        let sb = sample_buf.as_mut().unwrap();
        sb.copy_interleaved_ref(decoded);
        for &s in sb.samples() {
            total += 1;
            if s.abs() > 1e-6 {
                nonzero += 1;
            }
        }
    }
    (total, nonzero)
}

#[test]
#[ignore = "network: hits live radio.plaza.one"]
fn test_mp3_stream_decodes_to_audio() {
    let url = plaza_tui::config::StreamQuality::Mp3.stream_url();
    let bytes = fetch_bytes(url, 256 * 1024);
    assert!(bytes.len() > 32 * 1024, "fetched too little: {} bytes", bytes.len());
    let (total, nonzero) = decode_pcm_stats(bytes, "audio/mpeg");
    eprintln!("MP3: decoded {total} samples, {nonzero} non-zero");
    assert!(total > 10_000, "expected substantial decoded audio, got {total} samples");
    assert!(
        nonzero as f64 / total as f64 > 0.5,
        "stream looks silent: {nonzero}/{total} non-zero"
    );
}

#[test]
#[ignore = "network: confirms /ogg is Opus (currently undecodable) — documents the gap"]
fn test_ogg_stream_is_opus_and_currently_undecodable() {
    // Documents WHY audio broke: /ogg is now Opus, which symphonia can't decode.
    // When Phase 1 adds Opus support this test should be replaced with a decode check.
    let url = plaza_tui::config::StreamQuality::Ogg.stream_url();
    let bytes = fetch_bytes(url, 128 * 1024);
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    hint.mime_type("audio/ogg");
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .expect("ogg container should still probe");
    let track = probed
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .cloned();
    // The container parses, but making a decoder fails (no Opus support).
    if let Some(track) = track {
        let made = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default());
        assert!(made.is_err(), "expected Opus to be undecodable by symphonia today");
    }
}
