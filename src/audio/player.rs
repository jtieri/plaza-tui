use rodio::{buffer::SamplesBuffer, OutputStream, OutputStreamHandle, Sink};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use crate::error::{AudioError, PlazaError, Result};

/// Whether a playback session is finite (a downloaded preview) or
/// infinite (an icecast-style live stream that should be reconnected on EOF).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMode {
    OneShot,
    Live,
}

pub struct Player {
    sink: Arc<Mutex<Option<Sink>>>,
    _stream: Option<OutputStream>,
    stream_handle: Option<OutputStreamHandle>,
    task_handle: Option<std::thread::JoinHandle<()>>,
    cmd_tx: Option<std::sync::mpsc::SyncSender<PlayerCommand>>,
    /// Reports unrecoverable audio failures (e.g. an undecodable codec) to the UI.
    error_tx: Option<tokio::sync::mpsc::Sender<String>>,
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
            error_tx: None,
            volume: 0.8,
            is_playing: false,
        })
    }

    /// Provide a channel on which the audio thread reports unrecoverable failures
    /// (e.g. an undecodable codec) so the UI can show them instead of silently retrying.
    pub fn set_error_sender(&mut self, tx: tokio::sync::mpsc::Sender<String>) {
        self.error_tx = Some(tx);
    }

    /// Plays a finite source (e.g. a song preview). Stops at EOF.
    pub fn start_stream(&mut self, url: String) -> Result<()> {
        self.start_with_factory(http_factory(url), StreamMode::OneShot)
    }

    /// Plays an infinite live stream (icecast). Reconnects on EOF or read errors.
    pub fn start_live_stream(&mut self, url: String) -> Result<()> {
        self.start_with_factory(http_factory(url), StreamMode::Live)
    }

    fn start_with_factory<F>(&mut self, factory: F, mode: StreamMode) -> Result<()>
    where
        F: FnMut() -> io::Result<Box<dyn MediaSource>> + Send + 'static,
    {
        // Stop any existing playback first
        self.stop_inner();

        let stream_handle = self.stream_handle.as_ref().ok_or_else(|| {
            PlazaError::Audio(AudioError::OutputInit("No stream handle".to_string()))
        })?;

        let sink = Sink::try_new(stream_handle)
            .map_err(|e| PlazaError::Audio(AudioError::OutputInit(e.to_string())))?;
        sink.set_volume(self.volume);

        let sink_arc = Arc::clone(&self.sink);
        *sink_arc.lock().unwrap() = Some(sink);

        let sink_for_thread = Arc::clone(&self.sink);
        // Bounded channel so Stop is delivered promptly
        let (cmd_tx, cmd_rx) = std::sync::mpsc::sync_channel::<PlayerCommand>(8);
        self.cmd_tx = Some(cmd_tx);
        let error_tx = self.error_tx.clone();

        let handle = std::thread::Builder::new()
            .name("plaza-audio".to_string())
            .spawn(move || {
                run_audio_loop(factory, sink_for_thread, cmd_rx, mode, error_tx);
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
/// HTTP streams are not seekable, so Seek always errors and is_seekable() returns false.
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

fn http_factory(url: String) -> impl FnMut() -> io::Result<Box<dyn MediaSource>> + Send + 'static {
    move || {
        let client = reqwest::blocking::Client::builder()
            .user_agent("plaza-tui/0.1.0")
            .build()
            .map_err(io::Error::other)?;
        let response = client
            .get(&url)
            .send()
            .map_err(io::Error::other)?;
        Ok(Box::new(HttpStreamSource { inner: response }) as Box<dyn MediaSource>)
    }
}

const INITIAL_BACKOFF: Duration = Duration::from_millis(200);
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Why a single decode session ended.
#[derive(Debug, PartialEq, Eq)]
enum SessionOutcome {
    /// Stop command received — the whole audio thread should exit.
    Stopped,
    /// The source ended or returned a *transient* error (dropped connection, read
    /// error, empty body). In Live mode the outer loop reconnects with backoff.
    Ended,
    /// A *permanent* failure: the stream opened and was understood as a container,
    /// but its codec can't be decoded by this build. Reconnecting would reopen the
    /// exact same undecodable stream forever, so the loop must stop and report it.
    Permanent(String),
}

fn run_audio_loop<F>(
    factory: F,
    sink_arc: Arc<Mutex<Option<Sink>>>,
    cmd_rx: std::sync::mpsc::Receiver<PlayerCommand>,
    mode: StreamMode,
    error_tx: Option<tokio::sync::mpsc::Sender<String>>,
) where
    F: FnMut() -> io::Result<Box<dyn MediaSource>>,
{
    audio_loop_core(factory, sink_arc, cmd_rx, mode, error_tx, decode_session);
}

/// Core reconnect/decode loop, parameterised over the per-session decode step so the
/// retry policy can be unit-tested without a live audio stream.
fn audio_loop_core<F, D>(
    mut factory: F,
    sink_arc: Arc<Mutex<Option<Sink>>>,
    cmd_rx: std::sync::mpsc::Receiver<PlayerCommand>,
    mode: StreamMode,
    error_tx: Option<tokio::sync::mpsc::Sender<String>>,
    mut decode: D,
) where
    F: FnMut() -> io::Result<Box<dyn MediaSource>>,
    D: FnMut(
        Box<dyn MediaSource>,
        &Arc<Mutex<Option<Sink>>>,
        &std::sync::mpsc::Receiver<PlayerCommand>,
    ) -> SessionOutcome,
{
    let mut backoff = INITIAL_BACKOFF;
    loop {
        // Honour any pending Stop before opening a new connection.
        if drain_commands(&cmd_rx, &sink_arc) {
            tracing::info!("Audio thread: received Stop");
            return;
        }

        match factory() {
            Ok(source) => {
                tracing::info!("Audio thread: source opened, decoding");
                backoff = INITIAL_BACKOFF;
                match decode(source, &sink_arc, &cmd_rx) {
                    SessionOutcome::Stopped => {
                        tracing::info!("Audio thread: exiting (stopped)");
                        return;
                    }
                    SessionOutcome::Permanent(msg) => {
                        // Do NOT reconnect — that caused the runaway loop. Report once.
                        tracing::error!("Audio thread: permanent failure: {}", msg);
                        if let Some(tx) = &error_tx {
                            let _ = tx.try_send(msg);
                        }
                        return;
                    }
                    SessionOutcome::Ended => {
                        if mode == StreamMode::OneShot {
                            tracing::info!("Audio thread: exiting (one-shot ended)");
                            return;
                        }
                        tracing::warn!("Audio thread: live stream ended, will reconnect");
                    }
                }
            }
            Err(e) => {
                if mode == StreamMode::OneShot {
                    tracing::error!("Audio thread: connect failed: {}", e);
                    return;
                }
                tracing::warn!(
                    "Audio thread: connect failed ({}), retrying in {:?}",
                    e,
                    backoff
                );
            }
        }

        // Wait before reconnecting, but stay responsive to Stop.
        if wait_for_stop(&cmd_rx, backoff) {
            tracing::info!("Audio thread: exiting (stopped during backoff)");
            return;
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Sleep up to `dur`; return true if Stop was received (so the caller should exit).
fn wait_for_stop(
    cmd_rx: &std::sync::mpsc::Receiver<PlayerCommand>,
    dur: Duration,
) -> bool {
    use std::sync::mpsc::RecvTimeoutError;
    let deadline = std::time::Instant::now() + dur;
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return false;
        }
        match cmd_rx.recv_timeout(deadline - now) {
            Ok(PlayerCommand::Stop) => return true,
            Ok(_) => continue, // ignore Pause/Resume/SetVolume while disconnected
            Err(RecvTimeoutError::Timeout) => return false,
            Err(RecvTimeoutError::Disconnected) => return true,
        }
    }
}

/// Drain any queued commands. Returns true if Stop was seen.
fn drain_commands(
    cmd_rx: &std::sync::mpsc::Receiver<PlayerCommand>,
    sink_arc: &Arc<Mutex<Option<Sink>>>,
) -> bool {
    use std::sync::mpsc::TryRecvError;
    loop {
        match cmd_rx.try_recv() {
            Ok(PlayerCommand::Stop) => return true,
            Ok(PlayerCommand::Pause) => {
                if let Ok(g) = sink_arc.lock() {
                    if let Some(s) = g.as_ref() {
                        s.pause();
                    }
                }
            }
            Ok(PlayerCommand::Resume) => {
                if let Ok(g) = sink_arc.lock() {
                    if let Some(s) = g.as_ref() {
                        s.play();
                    }
                }
            }
            Ok(PlayerCommand::SetVolume(v)) => {
                if let Ok(g) = sink_arc.lock() {
                    if let Some(s) = g.as_ref() {
                        s.set_volume(v);
                    }
                }
            }
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => return true,
        }
    }
}

fn decode_session(
    source: Box<dyn MediaSource>,
    sink_arc: &Arc<Mutex<Option<Sink>>>,
    cmd_rx: &std::sync::mpsc::Receiver<PlayerCommand>,
) -> SessionOutcome {
    let mss = MediaSourceStream::new(source, Default::default());

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
            tracing::warn!("Audio thread: format probe failed: {}", e);
            return SessionOutcome::Ended;
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
            tracing::warn!("Audio thread: no audio track found");
            return SessionOutcome::Ended;
        }
    };

    let mut track_id = track.id;
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let mut channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(2);

    let mut decoder = match symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
    {
        Ok(d) => d,
        Err(e) => {
            // Permanent: the container parsed but the codec is unsupported (e.g. Opus).
            // Reconnecting reopens the same undecodable stream, so stop and report.
            tracing::warn!("Audio thread: decoder init failed: {}", e);
            return SessionOutcome::Permanent(format!(
                "This stream's audio codec isn't supported yet ({e}). Try a different stream quality."
            ));
        }
    };

    tracing::info!(
        "Audio thread: decoding {}Hz {}ch stream",
        sample_rate,
        channels
    );

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut pending_samples: Vec<f32> = Vec::with_capacity(16384);
    // Flush to sink every ~0.1s at 44100Hz stereo
    const FLUSH_SAMPLES: usize = 44100 / 10 * 2;

    loop {
        if drain_commands(cmd_rx, sink_arc) {
            return SessionOutcome::Stopped;
        }

        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::ResetRequired) => {
                // Chained Ogg stream: a new logical bitstream has started.
                // Flush any leftover samples in the OLD format before we change sample_rate/channels.
                flush_pending(&mut pending_samples, sample_rate, channels, sink_arc);
                tracing::info!("Audio thread: chained stream boundary, resetting decoder");
                let track = match format
                    .tracks()
                    .iter()
                    .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
                {
                    Some(t) => t,
                    None => {
                        tracing::warn!("Audio thread: no track after reset, ending session");
                        return SessionOutcome::Ended;
                    }
                };
                track_id = track.id;
                sample_rate = track.codec_params.sample_rate.unwrap_or(sample_rate);
                channels = track
                    .codec_params
                    .channels
                    .map(|c| c.count() as u16)
                    .unwrap_or(channels);
                decoder = match symphonia::default::get_codecs()
                    .make(&track.codec_params, &DecoderOptions::default())
                {
                    Ok(d) => d,
                    Err(e) => {
                        // A new logical stream uses an unsupported codec. Permanent.
                        tracing::warn!("Audio thread: decoder reset failed: {}", e);
                        return SessionOutcome::Permanent(format!(
                            "A track in this stream uses an unsupported codec ({e}). Try a different stream quality."
                        ));
                    }
                };
                sample_buf = None;
                tracing::info!(
                    "Audio thread: decoder reset, now decoding {}Hz {}ch",
                    sample_rate,
                    channels
                );
                continue;
            }
            Err(e) => {
                tracing::warn!("Audio thread: stream ended or read error: {}", e);
                flush_pending(&mut pending_samples, sample_rate, channels, sink_arc);
                return SessionOutcome::Ended;
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

        // Flush accumulated samples to the sink. Limit queue depth to keep volume
        // changes responsive: each queued SamplesBuffer source has its own
        // periodic_access volume controller that only activates when playing.
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
                        std::thread::sleep(Duration::from_millis(20));
                    }
                } else {
                    // Sink has been torn down (stop_inner called). Bail out.
                    return SessionOutcome::Stopped;
                }
            }
        }
    }
}

fn flush_pending(
    pending: &mut Vec<f32>,
    sample_rate: u32,
    channels: u16,
    sink_arc: &Arc<Mutex<Option<Sink>>>,
) {
    if pending.is_empty() {
        return;
    }
    let samples = std::mem::take(pending);
    if let Ok(g) = sink_arc.lock() {
        if let Some(sink) = g.as_ref() {
            sink.append(SamplesBuffer::new(channels, sample_rate, samples));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::sync_channel;

    /// Empty MediaSource — Read returns Ok(0) immediately, so symphonia probe fails
    /// and decode_session returns Ended. Used to simulate a dropped HTTP stream.
    struct EmptySource;
    impl Read for EmptySource {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }
    impl Seek for EmptySource {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "no seek"))
        }
    }
    impl MediaSource for EmptySource {
        fn is_seekable(&self) -> bool {
            false
        }
        fn byte_len(&self) -> Option<u64> {
            None
        }
    }

    /// Live mode must reconnect when the source factory keeps failing.
    /// Regression test for the bug where the audio thread silently exited
    /// the first time the icecast HTTP stream returned EOF.
    #[test]
    fn live_mode_retries_after_factory_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let factory = move || -> io::Result<Box<dyn MediaSource>> {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Err(io::Error::new(io::ErrorKind::ConnectionReset, "boom"))
        };

        let sink: Arc<Mutex<Option<Sink>>> = Arc::new(Mutex::new(None));
        let (tx, rx) = sync_channel::<PlayerCommand>(8);
        let sink_clone = Arc::clone(&sink);
        let handle = std::thread::spawn(move || {
            run_audio_loop(factory, sink_clone, rx, StreamMode::Live, None);
        });

        // Give the loop time to attempt several reconnects under backoff.
        std::thread::sleep(Duration::from_millis(700));
        let attempts = calls.load(Ordering::SeqCst);
        assert!(
            attempts >= 2,
            "Live mode should retry after EOF; only got {} attempt(s)",
            attempts
        );

        // Stop must terminate the loop cleanly.
        tx.send(PlayerCommand::Stop).unwrap();
        drop(tx);
        handle.join().expect("audio loop thread should exit");
    }

    /// OneShot mode must NOT reconnect after a failure — used for finite previews.
    #[test]
    fn oneshot_mode_does_not_retry_after_factory_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let factory = move || -> io::Result<Box<dyn MediaSource>> {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Err(io::Error::new(io::ErrorKind::ConnectionReset, "boom"))
        };

        let sink: Arc<Mutex<Option<Sink>>> = Arc::new(Mutex::new(None));
        let (_tx, rx) = sync_channel::<PlayerCommand>(8);
        let sink_clone = Arc::clone(&sink);
        let handle = std::thread::spawn(move || {
            run_audio_loop(factory, sink_clone, rx, StreamMode::OneShot, None);
        });

        handle.join().expect("oneshot loop should exit on its own");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "OneShot mode must not retry after failure"
        );
    }

    /// Live mode must also reconnect when the source opens successfully but produces
    /// no decodable data (the actual production failure mode: HTTP read returns Ok(0),
    /// symphonia surfaces it as "end of stream"). decode_session returns Ended and the
    /// outer loop reopens the source.
    #[test]
    fn live_mode_reopens_source_after_empty_stream() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let factory = move || -> io::Result<Box<dyn MediaSource>> {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(EmptySource) as Box<dyn MediaSource>)
        };

        let sink: Arc<Mutex<Option<Sink>>> = Arc::new(Mutex::new(None));
        let (tx, rx) = sync_channel::<PlayerCommand>(8);
        let sink_clone = Arc::clone(&sink);
        let handle = std::thread::spawn(move || {
            run_audio_loop(factory, sink_clone, rx, StreamMode::Live, None);
        });

        std::thread::sleep(Duration::from_millis(700));
        let attempts = calls.load(Ordering::SeqCst);
        assert!(
            attempts >= 2,
            "Live mode should reopen the source after EOF; only got {} attempt(s)",
            attempts
        );

        tx.send(PlayerCommand::Stop).unwrap();
        drop(tx);
        handle.join().expect("audio loop thread should exit");
    }

    /// Stop received between reconnect attempts must terminate the loop promptly.
    #[test]
    fn stop_terminates_live_loop_during_backoff() {
        let factory = || -> io::Result<Box<dyn MediaSource>> {
            Err(io::Error::new(io::ErrorKind::ConnectionReset, "boom"))
        };

        let sink: Arc<Mutex<Option<Sink>>> = Arc::new(Mutex::new(None));
        let (tx, rx) = sync_channel::<PlayerCommand>(8);
        let sink_clone = Arc::clone(&sink);
        let handle = std::thread::spawn(move || {
            run_audio_loop(factory, sink_clone, rx, StreamMode::Live, None);
        });

        std::thread::sleep(Duration::from_millis(50));
        let start = std::time::Instant::now();
        tx.send(PlayerCommand::Stop).unwrap();
        drop(tx);
        handle.join().expect("audio loop should exit on Stop");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "Stop should terminate live loop quickly, took {:?}",
            start.elapsed()
        );
    }

    /// decode_session must return Ended (not panic) when the source contains nothing
    /// decodable. This is the unit-level guarantee that makes the reconnect loop work.
    #[test]
    fn decode_session_returns_ended_on_empty_source() {
        let sink: Arc<Mutex<Option<Sink>>> = Arc::new(Mutex::new(None));
        let (_tx, rx) = sync_channel::<PlayerCommand>(8);
        let outcome = decode_session(Box::new(EmptySource), &sink, &rx);
        assert_eq!(outcome, SessionOutcome::Ended);
    }

    /// Regression test for the Opus reconnect storm: when a session ends with a
    /// PERMANENT failure (undecodable codec), Live mode must NOT reconnect — it must
    /// open the source exactly once, report the error, and exit. Previously this spun
    /// forever (~1 reopen/sec), which also degraded UI responsiveness.
    #[test]
    fn live_mode_does_not_retry_on_permanent_failure_and_reports_error() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let factory = move || -> io::Result<Box<dyn MediaSource>> {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(EmptySource) as Box<dyn MediaSource>)
        };
        // Fake decode that always reports an unsupported codec (what Opus does today).
        let decode = |_src: Box<dyn MediaSource>,
                      _sink: &Arc<Mutex<Option<Sink>>>,
                      _rx: &std::sync::mpsc::Receiver<PlayerCommand>| {
            SessionOutcome::Permanent("unsupported codec".to_string())
        };

        let sink: Arc<Mutex<Option<Sink>>> = Arc::new(Mutex::new(None));
        let (_tx, rx) = sync_channel::<PlayerCommand>(8);
        let (err_tx, mut err_rx) = tokio::sync::mpsc::channel::<String>(8);
        let sink_clone = Arc::clone(&sink);
        let handle = std::thread::spawn(move || {
            audio_loop_core(factory, sink_clone, rx, StreamMode::Live, Some(err_tx), decode);
        });

        // Must terminate on its own (no Stop sent) because it does not retry.
        handle.join().expect("loop must exit on permanent failure without retrying");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "Live mode must open the source exactly once on a permanent failure"
        );
        assert_eq!(
            err_rx.try_recv().ok().as_deref(),
            Some("unsupported codec"),
            "the permanent failure must be reported to the UI error channel"
        );
    }

    /// A transient end in Live mode (fake decode returns Ended) must still reconnect,
    /// proving the permanent/transient distinction is what gates retries.
    #[test]
    fn live_mode_retries_on_transient_session_end() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let factory = move || -> io::Result<Box<dyn MediaSource>> {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(EmptySource) as Box<dyn MediaSource>)
        };
        let decode = |_src: Box<dyn MediaSource>,
                      _sink: &Arc<Mutex<Option<Sink>>>,
                      _rx: &std::sync::mpsc::Receiver<PlayerCommand>| {
            SessionOutcome::Ended
        };

        let sink: Arc<Mutex<Option<Sink>>> = Arc::new(Mutex::new(None));
        let (tx, rx) = sync_channel::<PlayerCommand>(8);
        let sink_clone = Arc::clone(&sink);
        let handle = std::thread::spawn(move || {
            audio_loop_core(factory, sink_clone, rx, StreamMode::Live, None, decode);
        });

        std::thread::sleep(Duration::from_millis(700));
        assert!(
            calls.load(Ordering::SeqCst) >= 2,
            "Live mode should reconnect after a transient session end"
        );
        tx.send(PlayerCommand::Stop).unwrap();
        drop(tx);
        handle.join().expect("audio loop thread should exit");
    }
}
