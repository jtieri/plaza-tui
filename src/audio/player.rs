use rodio::{buffer::SamplesBuffer, OutputStream, OutputStreamHandle, Sink};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use crate::error::{AudioError, PlazaError, Result};

pub struct Player {
    sink: Arc<Mutex<Option<Sink>>>,
    _stream: Option<OutputStream>,
    stream_handle: Option<OutputStreamHandle>,
    task_handle: Option<std::thread::JoinHandle<()>>,
    cmd_tx: Option<std::sync::mpsc::SyncSender<PlayerCommand>>,
    volume: f32,
    is_playing: bool,
}

#[derive(Debug)]
enum PlayerCommand {
    Pause,
    Resume,
    SetVolume(f32),
    Stop,
}

impl Player {
    pub fn new() -> Result<Self> {
        let (stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| PlazaError::Audio(AudioError::OutputInit(e.to_string())))?;

        Ok(Player {
            sink: Arc::new(Mutex::new(None)),
            _stream: Some(stream),
            stream_handle: Some(stream_handle),
            task_handle: None,
            cmd_tx: None,
            volume: 0.8,
            is_playing: false,
        })
    }

    pub fn start_stream(&mut self, url: String) -> Result<()> {
        // Stop any existing playback first
        self.stop_inner();

        let stream_handle = self.stream_handle.as_ref()
            .ok_or_else(|| PlazaError::Audio(AudioError::OutputInit("No stream handle".to_string())))?;

        let sink = Sink::try_new(stream_handle)
            .map_err(|e| PlazaError::Audio(AudioError::OutputInit(e.to_string())))?;
        sink.set_volume(self.volume);

        let sink_arc = Arc::clone(&self.sink);
        *sink_arc.lock().unwrap() = Some(sink);

        let sink_for_thread = Arc::clone(&self.sink);
        // Bounded channel so Stop is delivered promptly
        let (cmd_tx, cmd_rx) = std::sync::mpsc::sync_channel::<PlayerCommand>(8);
        self.cmd_tx = Some(cmd_tx);

        let handle = std::thread::Builder::new()
            .name("plaza-audio".to_string())
            .spawn(move || {
                stream_thread(url, sink_for_thread, cmd_rx);
            })
            .map_err(|e| PlazaError::Audio(AudioError::OutputInit(e.to_string())))?;

        self.task_handle = Some(handle);
        self.is_playing = true;
        Ok(())
    }

    pub fn pause(&mut self) {
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.try_send(PlayerCommand::Pause);
        }
        self.is_playing = false;
    }

    pub fn resume(&mut self) {
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.try_send(PlayerCommand::Resume);
        }
        self.is_playing = true;
    }

    pub fn set_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.volume = volume;
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.try_send(PlayerCommand::SetVolume(volume));
        }
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn stop(&mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        if let Some(tx) = self.cmd_tx.take() {
            let _ = tx.try_send(PlayerCommand::Stop);
        }
        if let Ok(mut guard) = self.sink.lock() {
            if let Some(sink) = guard.as_mut() {
                sink.stop();
            }
            *guard = None;
        }
        self.is_playing = false;
        // Don't join — the audio thread may be blocked on I/O and will exit when the connection drops
        self.task_handle.take();
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing
    }
}

/// Wraps a blocking HTTP response stream as a symphonia MediaSource.
/// Since HTTP streams are not seekable, Seek always returns an error
/// and is_seekable() returns false so symphonia avoids seeking during probe.
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
        Err(io::Error::new(io::ErrorKind::Unsupported, "stream is not seekable"))
    }
}

// Safety: HttpStreamSource is only accessed from the single audio decode thread.
unsafe impl Sync for HttpStreamSource {}

impl MediaSource for HttpStreamSource {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

fn stream_thread(
    url: String,
    sink_arc: Arc<Mutex<Option<Sink>>>,
    cmd_rx: std::sync::mpsc::Receiver<PlayerCommand>,
) {
    tracing::info!("Audio thread: connecting to {}", url);

    let response = match reqwest::blocking::Client::builder()
        .user_agent("plaza-tui/0.1.0")
        .build()
        .and_then(|c| c.get(&url).send())
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Audio thread: HTTP connect failed: {}", e);
            return;
        }
    };

    tracing::info!("Audio thread: HTTP connected, probing format");

    let source = HttpStreamSource { inner: response };
    let mss = MediaSourceStream::new(Box::new(source), Default::default());

    let mut hint = Hint::new();
    hint.mime_type("audio/ogg");

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
        Err(e) => {
            tracing::error!("Audio thread: format probe failed: {}", e);
            return;
        }
    };

    let mut format = probed.format;

    let track = match format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
    {
        Some(t) => t,
        None => {
            tracing::error!("Audio thread: no audio track found");
            return;
        }
    };

    let mut track_id = track.id;
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let mut channels = track.codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);

    let mut decoder = match symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
    {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Audio thread: decoder init failed: {}", e);
            return;
        }
    };

    tracing::info!("Audio thread: decoding {}Hz {}ch stream", sample_rate, channels);

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut pending_samples: Vec<f32> = Vec::with_capacity(16384);
    // Flush to sink every ~0.1s at 44100Hz stereo
    const FLUSH_SAMPLES: usize = 44100 / 10 * 2;

    loop {
        // Non-blocking command check
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                PlayerCommand::Stop => {
                    tracing::info!("Audio thread: received Stop");
                    return;
                }
                PlayerCommand::Pause => {
                    if let Ok(g) = sink_arc.lock() {
                        if let Some(s) = g.as_ref() {
                            s.pause();
                        }
                    }
                }
                PlayerCommand::Resume => {
                    if let Ok(g) = sink_arc.lock() {
                        if let Some(s) = g.as_ref() {
                            s.play();
                        }
                    }
                }
                PlayerCommand::SetVolume(v) => {
                    if let Ok(g) = sink_arc.lock() {
                        if let Some(s) = g.as_ref() {
                            s.set_volume(v);
                        }
                    }
                }
            }
        }

        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::ResetRequired) => {
                // Chained Ogg stream: a new song/bitstream has started.
                // The format reader already loaded the new stream headers;
                // we just need a fresh decoder for the new track.
                tracing::info!("Audio thread: chained stream boundary, resetting decoder");
                let track = match format
                    .tracks()
                    .iter()
                    .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
                {
                    Some(t) => t,
                    None => {
                        tracing::warn!("Audio thread: no track after reset, stopping");
                        break;
                    }
                };
                track_id = track.id;
                sample_rate = track.codec_params.sample_rate.unwrap_or(sample_rate);
                channels = track.codec_params.channels.map(|c| c.count() as u16).unwrap_or(channels);
                decoder = match symphonia::default::get_codecs()
                    .make(&track.codec_params, &DecoderOptions::default())
                {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::error!("Audio thread: decoder reset failed: {}", e);
                        break;
                    }
                };
                sample_buf = None;
                tracing::info!("Audio thread: decoder reset, now decoding {}Hz {}ch", sample_rate, channels);
                continue;
            }
            Err(e) => {
                tracing::warn!("Audio thread: stream ended or read error: {}", e);
                break;
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!("Audio thread: decode error (skipping packet): {}", e);
                continue;
            }
        };

        if sample_buf.is_none() {
            let spec = *decoded.spec();
            let duration = decoded.capacity() as u64;
            sample_buf = Some(SampleBuffer::<f32>::new(duration, spec));
        }

        if let Some(ref mut buf) = sample_buf {
            buf.copy_interleaved_ref(decoded);
            pending_samples.extend_from_slice(buf.samples());
        }

        // Flush accumulated samples to the sink to keep it fed.
        // Limit queue depth to keep volume changes responsive: each queued SamplesBuffer
        // source has its own periodic_access volume controller that only activates when
        // that source is playing. With too many queued sources, volume changes feel laggy.
        if pending_samples.len() >= FLUSH_SAMPLES {
            let to_push = std::mem::take(&mut pending_samples);
            if let Ok(g) = sink_arc.lock() {
                if let Some(sink) = g.as_ref() {
                    if sink.len() < 4 {
                        sink.append(SamplesBuffer::new(channels, sample_rate, to_push));
                    } else {
                        // Sink is full; put samples back and let playback catch up.
                        drop(g);
                        pending_samples = to_push;
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                }
            }
        }
    }

    // Flush any remaining samples
    if !pending_samples.is_empty() {
        if let Ok(g) = sink_arc.lock() {
            if let Some(sink) = g.as_ref() {
                sink.append(SamplesBuffer::new(channels, sample_rate, pending_samples));
            }
        }
    }

    tracing::info!("Audio thread: exiting");
}
