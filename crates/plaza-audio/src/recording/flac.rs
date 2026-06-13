//! FLAC encoding with Vorbis-comment tags and an optional embedded cover image.
//!
//! [`flacenc`] encodes the audio but doesn't expose the `VORBIS_COMMENT` or
//! `PICTURE` metadata blocks, so we splice those into the stream ourselves: both
//! are simple, well-specified blocks that sit right after `STREAMINFO`.

use flacenc::component::BitRepr;
use flacenc::error::Verify;

use super::{Error, Result};

/// An embedded cover image.
pub struct Picture {
    /// MIME type, e.g. `"image/jpeg"`.
    pub mime: String,
    /// Raw image bytes.
    pub data: Vec<u8>,
}

/// FLAC `STREAMINFO` is metadata block type 0, `VORBIS_COMMENT` is 4, `PICTURE` is 6.
const BLOCK_VORBIS_COMMENT: u8 = 4;
const BLOCK_PICTURE: u8 = 6;

/// Encode interleaved f32 PCM to a complete FLAC byte stream, tagged with `comments`
/// (e.g. `("ARTIST", "…")`) and an optional embedded `cover`.
///
/// Samples are quantized to 16-bit, which matches Plaza's source material.
///
/// # Errors
/// Returns [`Error::Config`] or [`Error::Encode`] if the encoder rejects the input.
pub fn encode_flac(
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    comments: &[(&str, &str)],
    cover: Option<&Picture>,
) -> Result<Vec<u8>> {
    let pcm: Vec<i32> = samples
        .iter()
        .map(|&x| (x.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i32)
        .collect();

    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| Error::Config(e.to_string()))?;
    let source =
        flacenc::source::MemSource::from_samples(&pcm, channels as usize, 16, sample_rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, 4096)
        .map_err(|e| Error::Encode(e.to_string()))?;

    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| Error::Encode(e.to_string()))?;

    splice_metadata(sink.into_inner(), comments, cover)
}

/// Insert `VORBIS_COMMENT` (and an optional `PICTURE`) after the encoder's existing
/// metadata blocks, fixing up the "last metadata block" flags. Robust to whatever
/// blocks (STREAMINFO, padding, …) the encoder already emitted.
fn splice_metadata(
    flac: Vec<u8>,
    comments: &[(&str, &str)],
    cover: Option<&Picture>,
) -> Result<Vec<u8>> {
    if flac.len() < 8 || &flac[0..4] != b"fLaC" {
        return Err(Error::Encode(
            "encoder did not produce a FLAC stream".into(),
        ));
    }

    // Walk the metadata blocks to find where they end (start of the audio frames)
    // and the header offset of the current last block.
    let mut pos = 4;
    let mut last_header;
    loop {
        if pos + 4 > flac.len() {
            return Err(Error::Encode("truncated metadata block header".into()));
        }
        let is_last = flac[pos] & 0x80 != 0;
        let len = u32::from_be_bytes([0, flac[pos + 1], flac[pos + 2], flac[pos + 3]]) as usize;
        last_header = pos;
        pos += 4 + len;
        if pos > flac.len() {
            return Err(Error::Encode("truncated metadata block".into()));
        }
        if is_last {
            break;
        }
    }
    let frames_start = pos;

    let mut blocks: Vec<(u8, Vec<u8>)> =
        vec![(BLOCK_VORBIS_COMMENT, vorbis_comment_block(comments))];
    if let Some(pic) = cover {
        blocks.push((BLOCK_PICTURE, picture_block(pic)));
    }

    let extra: usize = blocks.iter().map(|(_, d)| d.len() + 4).sum();
    let mut out = Vec::with_capacity(flac.len() + extra);
    out.extend_from_slice(&flac[..frames_start]);
    // The block that used to be last is no longer last.
    out[last_header] &= 0x7F;

    // flacenc emits fixed-blocking frames but writes a STREAMINFO that declares
    // variable blocking (min_block_size != max_block_size, the former being the short
    // final block). Strict readers (symphonia) then expect sample-numbered frame
    // headers and desync. The frames are genuinely fixed, so make STREAMINFO agree by
    // setting min_block_size = max_block_size. STREAMINFO is the first block; its data
    // begins at offset 8, with min/max block size in the first four bytes.
    if flac[4] & 0x7F == 0 {
        let (min, max) = out.split_at_mut(10);
        min[8..10].copy_from_slice(&max[0..2]);
    }
    for (i, (block_type, data)) in blocks.iter().enumerate() {
        let last = i == blocks.len() - 1;
        out.push(if last { 0x80 } else { 0 } | (block_type & 0x7F));
        out.extend_from_slice(&u24_be(data.len()));
        out.extend_from_slice(data);
    }
    out.extend_from_slice(&flac[frames_start..]);
    Ok(out)
}

/// Build a `VORBIS_COMMENT` block body (little-endian lengths, no framing bit).
fn vorbis_comment_block(comments: &[(&str, &str)]) -> Vec<u8> {
    const VENDOR: &[u8] = b"plaza-tui";
    let mut d = Vec::new();
    d.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    d.extend_from_slice(VENDOR);
    d.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for (key, value) in comments {
        let entry = format!("{key}={value}");
        d.extend_from_slice(&(entry.len() as u32).to_le_bytes());
        d.extend_from_slice(entry.as_bytes());
    }
    d
}

/// Build a `PICTURE` block body (big-endian; type 3 = front cover).
fn picture_block(pic: &Picture) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&3u32.to_be_bytes()); // picture type: front cover
    d.extend_from_slice(&(pic.mime.len() as u32).to_be_bytes());
    d.extend_from_slice(pic.mime.as_bytes());
    d.extend_from_slice(&0u32.to_be_bytes()); // empty description
                                              // width, height, color depth, colors used — 0 = unspecified.
    d.extend_from_slice(&[0u8; 16]);
    d.extend_from_slice(&(pic.data.len() as u32).to_be_bytes());
    d.extend_from_slice(&pic.data);
    d
}

fn u24_be(n: usize) -> [u8; 3] {
    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    /// One second of a 440 Hz stereo tone.
    fn tone() -> (Vec<f32>, u32, u16) {
        let rate = 48_000u32;
        let mut s = Vec::with_capacity(rate as usize * 2);
        for i in 0..rate as usize {
            let v = (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5;
            s.push(v);
            s.push(v);
        }
        (s, rate, 2)
    }

    /// Decode FLAC bytes with symphonia and return the per-channel frame count.
    fn decode_frames(flac: &[u8]) -> u64 {
        let mss = MediaSourceStream::new(
            Box::new(std::io::Cursor::new(flac.to_vec())),
            Default::default(),
        );
        let mut hint = Hint::new();
        hint.with_extension("flac");
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .expect("our FLAC output must be decodable by symphonia");
        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .expect("audio track");
        let track_id = track.id;
        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .expect("flac decoder");
        let mut frames = 0u64;
        while let Ok(packet) = format.next_packet() {
            if packet.track_id() != track_id {
                continue;
            }
            if let Ok(decoded) = decoder.decode(&packet) {
                let mut sb = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
                sb.copy_interleaved_ref(decoded);
                frames += (sb.samples().len() / 2) as u64;
            }
        }
        frames
    }

    #[test]
    fn round_trips_to_lossless_audio_of_the_same_length() {
        let (samples, rate, ch) = tone();
        let flac = encode_flac(&samples, ch, rate, &[("ARTIST", "Test")], None).unwrap();
        assert_eq!(&flac[0..4], b"fLaC");
        // Lossless: exactly one second of stereo frames back out.
        assert_eq!(decode_frames(&flac), rate as u64);
    }

    #[test]
    fn embeds_vorbis_comments_and_a_picture() {
        let (samples, rate, ch) = tone();
        let cover = Picture {
            mime: "image/jpeg".into(),
            data: vec![0xFF, 0xD8, 0xFF, 0xD9],
        };
        let flac = encode_flac(
            &samples,
            ch,
            rate,
            &[("ARTIST", "la trace"), ("TITLE", "Man Enough")],
            Some(&cover),
        )
        .unwrap();

        // The comment strings and picture MIME are present verbatim in the blocks.
        assert!(contains(&flac, b"ARTIST=la trace"));
        assert!(contains(&flac, b"TITLE=Man Enough"));
        assert!(contains(&flac, b"image/jpeg"));
        // Audio still decodes after the metadata splice.
        assert_eq!(decode_frames(&flac), rate as u64);
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }
}
