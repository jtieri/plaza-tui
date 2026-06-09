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
            &FormatOptions {
                enable_gapless: false,
                ..Default::default()
            },
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
        let Ok(decoded) = decoder.decode(&packet) else {
            continue;
        };
        if sample_buf.is_none() {
            sample_buf = Some(SampleBuffer::<f32>::new(
                decoded.capacity() as u64,
                *decoded.spec(),
            ));
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

// --- Integrated PcmSource tests: exercise the actual source implementations the
//     player uses, against the live endpoints. These are the real end-to-end proof
//     that each codec path works. ---

use plaza_tui::audio::pcm::PcmSource;

/// Pull up to `max_chunks` decoded chunks from a live source, tolerating the
/// `Ok(None)` (live-edge wait) that HLS returns. Returns (total, nonzero) samples.
fn drain_source(mut source: Box<dyn PcmSource>, max_chunks: usize) -> (usize, usize) {
    let mut total = 0;
    let mut nonzero = 0;
    let mut chunks = 0;
    let mut idle = 0;
    while chunks < max_chunks && idle < 200 {
        match source.next_chunk() {
            Ok(Some(chunk)) => {
                chunks += 1;
                idle = 0;
                for &s in &chunk.samples {
                    total += 1;
                    if s.abs() > 1e-6 {
                        nonzero += 1;
                    }
                }
            }
            Ok(None) => {
                idle += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => panic!("source errored before producing audio: {e}"),
        }
    }
    (total, nonzero)
}

fn assert_audible(label: &str, total: usize, nonzero: usize) {
    eprintln!("{label}: {total} samples, {nonzero} non-zero");
    assert!(
        total > 10_000,
        "{label}: too little audio ({total} samples)"
    );
    assert!(
        nonzero as f64 / total as f64 > 0.5,
        "{label}: looks silent ({nonzero}/{total})"
    );
}

#[test]
#[ignore = "network: exercises SymphoniaPcmSource against live /mp3"]
fn test_mp3_source_decodes() {
    use plaza_tui::audio::sources::SymphoniaPcmSource;
    let url = plaza_tui::config::StreamQuality::Mp3
        .stream_url()
        .to_string();
    let source = SymphoniaPcmSource::open(url, "audio/mpeg").expect("open mp3 source");
    let (t, nz) = drain_source(Box::new(source), 400);
    assert_audible("MP3 source", t, nz);
}

#[test]
#[ignore = "network: exercises OpusPcmSource against live /ogg"]
fn test_opus_source_decodes() {
    use plaza_tui::audio::sources::OpusPcmSource;
    let url = plaza_tui::config::StreamQuality::Ogg
        .stream_url()
        .to_string();
    let source = OpusPcmSource::open(url).expect("open opus source");
    let (t, nz) = drain_source(Box::new(source), 400);
    assert_audible("Opus source", t, nz);
}

#[test]
#[ignore = "network: exercises HlsAacPcmSource against live /hls"]
fn test_hls_source_decodes() {
    use plaza_tui::audio::hls::HlsAacPcmSource;
    let url = plaza_tui::config::StreamQuality::Hls
        .stream_url()
        .to_string();
    let source = HlsAacPcmSource::open(url).expect("open hls source");
    let (t, nz) = drain_source(Box::new(source), 400);
    assert_audible("HLS source", t, nz);
}

#[test]
#[ignore = "network: hits live radio.plaza.one"]
fn test_mp3_stream_decodes_to_audio() {
    let url = plaza_tui::config::StreamQuality::Mp3.stream_url();
    let bytes = fetch_bytes(url, 256 * 1024);
    assert!(
        bytes.len() > 32 * 1024,
        "fetched too little: {} bytes",
        bytes.len()
    );
    let (total, nonzero) = decode_pcm_stats(bytes, "audio/mpeg");
    eprintln!("MP3: decoded {total} samples, {nonzero} non-zero");
    assert!(
        total > 10_000,
        "expected substantial decoded audio, got {total} samples"
    );
    assert!(
        nonzero as f64 / total as f64 > 0.5,
        "stream looks silent: {nonzero}/{total} non-zero"
    );
}

#[test]
#[ignore = "network: proves symphonia ogg-demux + opus crate decode the live Opus stream"]
fn test_ogg_opus_decodes_with_libopus() {
    // Validates the Phase 1 approach: symphonia demuxes the Ogg/Opus container into
    // raw Opus packets, and the `opus` crate (libopus) decodes them to PCM.
    use opus::{Channels, Decoder as OpusDecoder};

    let url = plaza_tui::config::StreamQuality::Ogg.stream_url();
    let bytes = fetch_bytes(url, 256 * 1024);
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    hint.mime_type("audio/ogg");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .expect("ogg probe");
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .expect("opus track");
    let track_id = track.id;
    let channels = match track.codec_params.channels.map(|c| c.count()).unwrap_or(2) {
        1 => Channels::Mono,
        _ => Channels::Stereo,
    };
    let nch = if matches!(channels, Channels::Mono) {
        1
    } else {
        2
    };
    let mut decoder = OpusDecoder::new(48_000, channels).expect("opus decoder");

    let mut total = 0usize;
    let mut nonzero = 0usize;
    let mut out = vec![0f32; 5760 * nch]; // max Opus frame: 120ms @ 48kHz
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode_float(&packet.data, &mut out, false) {
            Ok(per_ch) => {
                for &s in &out[..per_ch * nch] {
                    total += 1;
                    if s.abs() > 1e-6 {
                        nonzero += 1;
                    }
                }
            }
            Err(_) => continue,
        }
    }
    eprintln!("OPUS: decoded {total} samples, {nonzero} non-zero ({nch}ch)");
    assert!(
        total > 10_000,
        "expected substantial decoded opus audio, got {total}"
    );
    assert!(
        nonzero as f64 / total as f64 > 0.5,
        "opus stream looks silent: {nonzero}/{total}"
    );
}

#[test]
#[ignore = "network: confirms /ogg is Opus (undecodable by symphonia alone) — documents the gap"]
fn test_ogg_stream_is_opus_and_currently_undecodable() {
    // Documents WHY audio broke: /ogg is now Opus, which symphonia can't decode.
    // When Phase 1 adds Opus support this test should be replaced with a decode check.
    let url = plaza_tui::config::StreamQuality::Ogg.stream_url();
    let bytes = fetch_bytes(url, 128 * 1024);
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    hint.mime_type("audio/ogg");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .expect("ogg container should still probe");
    let track = probed
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .cloned();
    // The container parses, but making a decoder fails (no Opus support).
    if let Some(track) = track {
        let made =
            symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default());
        assert!(
            made.is_err(),
            "expected Opus to be undecodable by symphonia today"
        );
    }
}
