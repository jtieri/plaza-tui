//! Concrete [`PcmSource`] implementations and the live-source builder.
//!
//! - [`SymphoniaPcmSource`]: MP3 (and Vorbis), decoded entirely by symphonia.
//! - [`OpusPcmSource`]: Ogg/Opus — symphonia demuxes the Ogg container into raw
//!   Opus packets which libopus (the `opus` crate) decodes. symphonia has no Opus
//!   decoder, but its Ogg reader still maps Opus logical streams.
//!
//! HLS/AAC lives in [`crate::hls`] because it needs playlist + TS demuxing.

use std::io::{self, Read, Seek, SeekFrom};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::hls::HlsAacPcmSource;
use crate::pcm::{PcmChunk, PcmError, PcmSource};
use crate::quality::StreamQuality;
use crate::recording::{RecordHandle, SongTags};

/// Build the live [`PcmSource`] for a stream quality, opening the network
/// connection. A failure to open is returned as [`PcmError`] so the player can
/// decide whether to retry (transient) or stop (permanent).
///
/// `rec` is wired into the Opus source only — it's the one format with exact
/// in-band song boundaries, so it's the only one that can record correctly.
pub(crate) fn build_live_source(
    quality: &StreamQuality,
    rec: Option<RecordHandle>,
) -> Result<Box<dyn PcmSource>, PcmError> {
    let url = quality.stream_url().to_string();
    match quality {
        StreamQuality::Mp3 | StreamQuality::Mp3Low => {
            Ok(Box::new(SymphoniaPcmSource::open(url, "audio/mpeg")?))
        }
        StreamQuality::Ogg | StreamQuality::OggLow => Ok(Box::new(OpusPcmSource::open(url, rec)?)),
        StreamQuality::Hls => Ok(Box::new(HlsAacPcmSource::open(url)?)),
    }
}

/// Build a one-shot source for a song preview (a static MP3 on plaza.one).
pub fn build_preview_source(url: String) -> Result<Box<dyn PcmSource>, PcmError> {
    Ok(Box::new(SymphoniaPcmSource::open(url, "audio/mpeg")?))
}

// ---------------------------------------------------------------------------
// HTTP MediaSource (shared by the symphonia-based sources)
// ---------------------------------------------------------------------------

/// Wraps a blocking HTTP response as a symphonia [`MediaSource`]. HTTP streams
/// aren't seekable, so `seek` errors and `is_seekable()` is false.
struct HttpStreamSource {
    inner: reqwest::blocking::Response,
}

impl Read for HttpStreamSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for HttpStreamSource {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "stream is not seekable",
        ))
    }
}

// Safety: only ever touched from the single audio decode thread.
unsafe impl Sync for HttpStreamSource {}

impl MediaSource for HttpStreamSource {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// Open a blocking HTTP GET and wrap it as a symphonia media source.
fn open_http_media(url: &str) -> Result<MediaSourceStream, PcmError> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("plaza-tui/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| PcmError::Permanent(format!("HTTP client init failed: {e}")))?;
    let response = match client.get(url).send() {
        Ok(r) => r,
        Err(e) => {
            // A failed connection is transient — the player reconnects.
            tracing::warn!("Audio: HTTP connect failed: {e}");
            return Err(PcmError::Ended);
        }
    };
    if !response.status().is_success() {
        return Err(PcmError::Permanent(format!(
            "stream returned HTTP {}",
            response.status()
        )));
    }
    let source: Box<dyn MediaSource> = Box::new(HttpStreamSource { inner: response });
    Ok(MediaSourceStream::new(source, Default::default()))
}

/// Find the first decodable audio track in a format reader.
pub(crate) fn first_audio_track(
    format: &dyn FormatReader,
) -> Option<(u32, symphonia::core::codecs::CodecParameters)> {
    format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .map(|t| (t.id, t.codec_params.clone()))
}

// ---------------------------------------------------------------------------
// SymphoniaPcmSource — MP3 / Vorbis
// ---------------------------------------------------------------------------

/// A [`PcmSource`] backed entirely by symphonia: MP3 and Ogg/Vorbis.
pub struct SymphoniaPcmSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
}

impl SymphoniaPcmSource {
    /// Open `url`, probing the container with `mime` as a format hint.
    ///
    /// # Errors
    /// [`PcmError::Ended`] for a connection/probe failure (the player retries) and
    /// [`PcmError::Permanent`] if the codec is unsupported.
    pub fn open(url: String, mime: &str) -> Result<Self, PcmError> {
        let mss = open_http_media(&url)?;
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
            .map_err(|_| PcmError::Ended)?;
        let format = probed.format;
        let (track_id, params) = first_audio_track(format.as_ref()).ok_or(PcmError::Ended)?;
        let decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(|e| {
                PcmError::Permanent(format!(
                    "audio codec not supported ({e}). Try a different stream quality."
                ))
            })?;
        Ok(SymphoniaPcmSource {
            format,
            decoder,
            track_id,
        })
    }

    /// Re-init decoder at a chained-stream boundary (e.g. a new Vorbis logical stream).
    fn reset(&mut self) -> Result<(), PcmError> {
        let (track_id, params) = first_audio_track(self.format.as_ref()).ok_or(PcmError::Ended)?;
        self.track_id = track_id;
        self.decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(|e| {
                PcmError::Permanent(format!("unsupported codec after stream change ({e})"))
            })?;
        Ok(())
    }
}

impl PcmSource for SymphoniaPcmSource {
    fn next_chunk(&mut self) -> Result<Option<PcmChunk>, PcmError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymphoniaError::ResetRequired) => {
                    self.reset()?;
                    continue;
                }
                Err(_) => return Err(PcmError::Ended),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymphoniaError::DecodeError(_)) => continue, // recoverable, skip frame
                Err(_) => return Err(PcmError::Ended),
            };
            let spec = *decoded.spec();
            let mut sb = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            sb.copy_interleaved_ref(decoded);
            let samples = sb.samples().to_vec();
            if samples.is_empty() {
                continue;
            }
            return Ok(Some(PcmChunk::new(
                samples,
                spec.rate,
                spec.channels.count() as u16,
            )));
        }
    }
}

// ---------------------------------------------------------------------------
// OpusPcmSource — Ogg/Opus via symphonia demux + libopus decode
// ---------------------------------------------------------------------------

/// Opus always decodes at 48 kHz; max frame is 120 ms.
const OPUS_SAMPLE_RATE: u32 = 48_000;
const OPUS_MAX_FRAME: usize = 5760; // 120ms @ 48kHz, per channel

/// A [`PcmSource`] for Ogg/Opus: symphonia demuxes the container and libopus
/// decodes the packets.
///
/// Each Plaza song is its own chained-Ogg logical stream, so a stream reset marks
/// an exact song boundary. When recording, this source brackets each song with
/// begin/finish events and forwards the decoded samples — the boundary precision
/// that makes lossless splitting possible.
pub struct OpusPcmSource {
    format: Box<dyn FormatReader>,
    decoder: opus::Decoder,
    track_id: u32,
    channels: u16,
    out: Vec<f32>,
    rec: Option<RecordHandle>,
}

impl OpusPcmSource {
    /// Open an Ogg/Opus stream at `url`, optionally feeding `rec` with recording events.
    ///
    /// # Errors
    /// [`PcmError::Ended`] for a connection/probe failure (the player retries) and
    /// [`PcmError::Permanent`] if an Opus decoder cannot be created.
    pub fn open(url: String, rec: Option<RecordHandle>) -> Result<Self, PcmError> {
        let mss = open_http_media(&url)?;
        let mut hint = Hint::new();
        hint.mime_type("audio/ogg");
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|_| PcmError::Ended)?;
        let mut format = probed.format;
        let (track_id, params) = first_audio_track(format.as_ref()).ok_or(PcmError::Ended)?;
        let channels = params.channels.map(|c| c.count() as u16).unwrap_or(2);
        let decoder = make_opus_decoder(channels)?;

        // The song already playing when we connect is captured mid-way, so it is not
        // savable; only songs we see from their start (after a reset) are.
        if rec.as_ref().is_some_and(RecordHandle::is_active) {
            let tags = read_tags(format.as_mut());
            if let Some(r) = &rec {
                r.begin(tags, false);
            }
        }

        Ok(OpusPcmSource {
            format,
            decoder,
            track_id,
            channels,
            out: vec![0.0; OPUS_MAX_FRAME * channels.max(1) as usize],
            rec,
        })
    }

    /// Whether recording is on; gates the per-chunk sample clone.
    fn recording(&self) -> bool {
        self.rec.as_ref().is_some_and(RecordHandle::is_active)
    }
}

/// Read artist/album/title from the format reader's current metadata revision
/// (the Ogg/Opus `OpusTags`).
fn read_tags(format: &mut dyn FormatReader) -> SongTags {
    use symphonia::core::meta::StandardTagKey;
    let mut tags = SongTags::default();
    let metadata = format.metadata();
    if let Some(rev) = metadata.current() {
        for tag in rev.tags() {
            match tag.std_key {
                Some(StandardTagKey::Artist) => tags.artist = Some(tag.value.to_string()),
                Some(StandardTagKey::Album) => tags.album = Some(tag.value.to_string()),
                Some(StandardTagKey::TrackTitle) => tags.title = Some(tag.value.to_string()),
                _ => {}
            }
        }
    }
    tags
}

fn make_opus_decoder(channels: u16) -> Result<opus::Decoder, PcmError> {
    let ch = match channels {
        1 => opus::Channels::Mono,
        _ => opus::Channels::Stereo,
    };
    opus::Decoder::new(OPUS_SAMPLE_RATE, ch)
        .map_err(|e| PcmError::Permanent(format!("Opus decoder init failed: {e}")))
}

impl PcmSource for OpusPcmSource {
    fn next_chunk(&mut self) -> Result<Option<PcmChunk>, PcmError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymphoniaError::ResetRequired) => {
                    // A new logical stream is an exact song boundary: the previous
                    // song ended cleanly here.
                    if let Some(r) = &self.rec {
                        if r.is_active() {
                            r.finish();
                        }
                    }
                    let (track_id, params) = match first_audio_track(self.format.as_ref()) {
                        Some(t) => t,
                        None => {
                            if let Some(r) = &self.rec {
                                if r.is_active() {
                                    r.abort();
                                }
                            }
                            return Err(PcmError::Ended);
                        }
                    };
                    self.track_id = track_id;
                    let channels = params.channels.map(|c| c.count() as u16).unwrap_or(2);
                    if channels != self.channels {
                        self.channels = channels;
                        self.decoder = make_opus_decoder(channels)?;
                        self.out = vec![0.0; OPUS_MAX_FRAME * channels.max(1) as usize];
                    }
                    // The next song is captured from its start, so it is savable.
                    if self.recording() {
                        let tags = read_tags(self.format.as_mut());
                        if let Some(r) = &self.rec {
                            r.begin(tags, true);
                        }
                    }
                    continue;
                }
                Err(_) => {
                    // The stream dropped mid-song; that capture is incomplete.
                    if let Some(r) = &self.rec {
                        if r.is_active() {
                            r.abort();
                        }
                    }
                    return Err(PcmError::Ended);
                }
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let nch = self.channels.max(1) as usize;
            match self
                .decoder
                .decode_float(&packet.data, &mut self.out, false)
            {
                Ok(per_ch) => {
                    let n = per_ch * nch;
                    if n == 0 {
                        continue;
                    }
                    let chunk =
                        PcmChunk::new(self.out[..n].to_vec(), OPUS_SAMPLE_RATE, self.channels);
                    if self.recording() {
                        if let Some(r) = &self.rec {
                            r.samples(chunk.clone());
                        }
                    }
                    return Ok(Some(chunk));
                }
                // A corrupt packet shouldn't kill the stream; skip it.
                Err(_) => continue,
            }
        }
    }
}
