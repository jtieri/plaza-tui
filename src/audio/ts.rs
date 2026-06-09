//! Minimal audio-only MPEG-TS demuxer.
//!
//! HLS segments are MPEG-TS: a sequence of 188-byte packets. Plaza's `/hls`
//! variants carry one AAC audio elementary stream (plus a timed-ID3 data PID we
//! ignore). We extract that audio stream's PES payload, which for AAC-in-TS is a
//! run of ADTS frames — exactly what symphonia's AAC reader consumes.
//!
//! This intentionally implements only what an audio-only HLS segment needs:
//! PAT → PMT → audio PID discovery, then PES reassembly. No video, no PCR, no
//! descriptors beyond what's required to find the audio PID.

const TS_PACKET_LEN: usize = 188;
const TS_SYNC: u8 = 0x47;

/// AAC in ADTS (the format Plaza's HLS uses).
const STREAM_TYPE_AAC_ADTS: u8 = 0x0F;
/// AAC in LATM (handled identically — payload still fed to the AAC reader).
const STREAM_TYPE_AAC_LATM: u8 = 0x11;

/// Extract the AAC elementary stream (ADTS bytes) from one or more MPEG-TS
/// segments. Returns the concatenated ADTS frames, ready to hand to symphonia.
///
/// Returns an empty vec if no audio stream is found (e.g. a malformed segment).
pub fn extract_adts(ts: &[u8]) -> Vec<u8> {
    let mut demux = TsDemux::new();
    demux.push(ts);
    demux.finish()
}

/// Streaming demuxer: feed segments with `push`, collect ADTS with `take`/`finish`.
/// State (audio PID, in-flight PES) persists across `push` calls so a continuous
/// HLS stream can be fed segment by segment.
pub struct TsDemux {
    pmt_pid: Option<u16>,
    audio_pid: Option<u16>,
    out: Vec<u8>,
    /// True once we've started collecting the current PES payload for the audio PID.
    in_audio_pes: bool,
}

impl TsDemux {
    pub fn new() -> Self {
        TsDemux {
            pmt_pid: None,
            audio_pid: None,
            out: Vec::new(),
            in_audio_pes: false,
        }
    }

    /// Feed a buffer of whole TS packets (typically one segment).
    pub fn push(&mut self, ts: &[u8]) {
        let mut off = 0;
        // Tolerate a leading partial/garbage byte run by seeking to the first sync.
        while off + TS_PACKET_LEN <= ts.len() {
            if ts[off] != TS_SYNC {
                // Resync: advance to the next 0x47 that looks packet-aligned.
                off += 1;
                continue;
            }
            self.handle_packet(&ts[off..off + TS_PACKET_LEN]);
            off += TS_PACKET_LEN;
        }
    }

    /// Take everything decoded so far, leaving the demuxer ready for more input.
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    /// Consume the demuxer and return all collected ADTS bytes.
    pub fn finish(mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    fn handle_packet(&mut self, pkt: &[u8]) {
        let pusi = (pkt[1] & 0x40) != 0;
        let pid = (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16;
        let afc = (pkt[3] & 0x30) >> 4;
        let has_payload = afc == 1 || afc == 3;
        if !has_payload {
            return;
        }
        // Payload offset: skip the 4-byte header and any adaptation field.
        let mut payload_off = 4;
        if afc == 3 {
            let af_len = pkt[4] as usize;
            payload_off = 5 + af_len;
        }
        if payload_off >= pkt.len() {
            return;
        }
        let payload = &pkt[payload_off..];

        match pid {
            0x0000 => self.parse_pat(payload, pusi),
            _ if Some(pid) == self.pmt_pid => self.parse_pmt(payload, pusi),
            _ if Some(pid) == self.audio_pid => self.collect_audio(payload, pusi),
            _ => {}
        }
    }

    /// PAT (PID 0): find the first program's PMT PID.
    fn parse_pat(&mut self, payload: &[u8], pusi: bool) {
        if self.pmt_pid.is_some() {
            return;
        }
        let Some(section) = psi_section(payload, pusi) else {
            return;
        };
        // table_id 0x00 = PAT. Section: table_id(1) sec_len(2) tsid(2) ver(1) sec#(1)
        // last#(1) then program loop, minus 4-byte CRC at the end.
        if section.len() < 8 || section[0] != 0x00 {
            return;
        }
        let section_length = (((section[1] & 0x0F) as usize) << 8) | section[2] as usize;
        let end = (3 + section_length).min(section.len());
        let mut i = 8;
        while i + 4 <= end.saturating_sub(4) {
            let program_number = ((section[i] as u16) << 8) | section[i + 1] as u16;
            let pid = (((section[i + 2] & 0x1F) as u16) << 8) | section[i + 3] as u16;
            if program_number != 0 {
                self.pmt_pid = Some(pid);
                return;
            }
            i += 4;
        }
    }

    /// PMT: find the AAC elementary stream PID.
    fn parse_pmt(&mut self, payload: &[u8], pusi: bool) {
        if self.audio_pid.is_some() {
            return;
        }
        let Some(section) = psi_section(payload, pusi) else {
            return;
        };
        // table_id 0x02 = PMT.
        if section.len() < 12 || section[0] != 0x02 {
            return;
        }
        let section_length = (((section[1] & 0x0F) as usize) << 8) | section[2] as usize;
        let end = (3 + section_length).min(section.len());
        let program_info_length = (((section[10] & 0x0F) as usize) << 8) | section[11] as usize;
        let mut i = 12 + program_info_length;
        // ES loop, minus the trailing 4-byte CRC.
        while i + 5 <= end.saturating_sub(4) {
            let stream_type = section[i];
            let elementary_pid = (((section[i + 1] & 0x1F) as u16) << 8) | section[i + 2] as u16;
            let es_info_length =
                (((section[i + 3] & 0x0F) as usize) << 8) | section[i + 4] as usize;
            if stream_type == STREAM_TYPE_AAC_ADTS || stream_type == STREAM_TYPE_AAC_LATM {
                self.audio_pid = Some(elementary_pid);
                return;
            }
            i += 5 + es_info_length;
        }
    }

    /// Collect the audio PID's PES payload (= ADTS frames).
    fn collect_audio(&mut self, payload: &[u8], pusi: bool) {
        if pusi {
            // Start of a new PES packet: strip the PES header.
            if let Some(adts) = pes_payload(payload) {
                self.in_audio_pes = true;
                self.out.extend_from_slice(adts);
            }
        } else if self.in_audio_pes {
            // Continuation of the current PES packet.
            self.out.extend_from_slice(payload);
        }
    }
}

impl Default for TsDemux {
    fn default() -> Self {
        Self::new()
    }
}

/// For a PSI payload with PUSI set, the first byte is a pointer_field giving the
/// offset to the table start. Without PUSI we don't handle continuation here
/// (PAT/PMT fit in one packet for these streams).
fn psi_section(payload: &[u8], pusi: bool) -> Option<&[u8]> {
    if !pusi || payload.is_empty() {
        return None;
    }
    let pointer = payload[0] as usize;
    let start = 1 + pointer;
    payload.get(start..)
}

/// Strip the PES header from a PES-start payload, returning the elementary payload.
fn pes_payload(payload: &[u8]) -> Option<&[u8]> {
    // PES starts with the 24-bit start code 0x000001.
    if payload.len() < 9 || payload[0] != 0x00 || payload[1] != 0x00 || payload[2] != 0x01 {
        return None;
    }
    // payload[3] = stream_id, payload[4..6] = PES_packet_length.
    // For audio: '10' marker bits at payload[6], PES_header_data_length at payload[8].
    let pes_header_data_length = payload[8] as usize;
    let start = 9 + pes_header_data_length;
    payload.get(start..)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real Plaza HLS segment (AAC on PID 0x100). extract_adts must return ADTS
    /// frames — the first byte of an ADTS frame is the 0xFF sync word.
    #[test]
    fn extracts_adts_from_real_segment() {
        let seg = include_bytes!("../../tests/fixtures/hls_aac_segment.ts");
        assert_eq!(
            seg.len() % TS_PACKET_LEN,
            0,
            "fixture should be whole TS packets"
        );
        let adts = extract_adts(seg);
        assert!(!adts.is_empty(), "should extract ADTS bytes");
        // ADTS syncword: 12 bits set (0xFFF) -> first byte 0xFF, next byte high nibble 0xF.
        assert_eq!(adts[0], 0xFF, "ADTS must start with sync byte 0xFF");
        assert_eq!(
            adts[1] & 0xF0,
            0xF0,
            "ADTS second byte high nibble must be 0xF"
        );
        // Sanity: a 4s AAC segment is a substantial amount of ADTS data.
        assert!(
            adts.len() > 10_000,
            "unexpectedly little ADTS: {} bytes",
            adts.len()
        );
    }

    /// The audio PID must be discovered via PAT/PMT (0x100 for this fixture).
    #[test]
    fn discovers_audio_pid_from_psi() {
        let seg = include_bytes!("../../tests/fixtures/hls_aac_segment.ts");
        let mut demux = TsDemux::new();
        demux.push(seg);
        assert_eq!(demux.audio_pid, Some(0x100), "should find AAC PID 0x100");
    }

    /// Garbage / non-sync input must not panic and yields nothing.
    #[test]
    fn handles_garbage_without_panicking() {
        assert!(extract_adts(&[0u8; 50]).is_empty());
        assert!(extract_adts(&[0xAB; 188 * 3]).is_empty());
        assert!(extract_adts(&[]).is_empty());
    }
}
