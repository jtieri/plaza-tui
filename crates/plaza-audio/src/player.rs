//! Audio playback engine.
//!
//! [`Player`] owns the rodio output and a dedicated decode thread. The thread runs
//! a codec-agnostic loop that pulls PCM from a [`PcmSource`] (MP3, Opus, or
//! HLS/AAC — see [`crate::sources`]) and feeds it to the sink with batching and
//! backpressure. Reconnect policy lives here; decoding lives in the sources.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamHandle, Sink};

use crate::error::{Error, Result};
use crate::pcm::{PcmChunk, PcmError, PcmSource};
use crate::quality::StreamQuality;
use crate::recording::{RecordMode, Recorder, RecordingConfig};
use crate::sources::{build_live_source, build_preview_source};

/// A callback the audio thread invokes with a human-readable message when playback
/// fails in a way it cannot recover from. The binary adapts this to its own event
/// channel, keeping the audio engine free of any async-runtime dependency.
pub type ErrorReporter = Arc<dyn Fn(String) + Send + Sync>;

/// Whether a playback session is finite (a downloaded preview) or infinite
/// (a live radio stream that should be reconnected on a transient end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamMode {
    OneShot,
    Live,
}

/// Plays Plaza streams through the system's default audio output.
///
/// Playback runs on a dedicated thread; the `Player` itself is a lightweight handle
/// for issuing commands (play, pause, volume, stop). Stopping or dropping it tears
/// the playback thread down.
pub struct Player {
    sink: Arc<Mutex<Option<Sink>>>,
    _stream: Option<OutputStream>,
    stream_handle: Option<OutputStreamHandle>,
    task_handle: Option<std::thread::JoinHandle<()>>,
    cmd_tx: Option<std::sync::mpsc::SyncSender<PlayerCommand>>,
    error_report: Option<ErrorReporter>,
    recorder: Option<Recorder>,
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
    /// Open the default audio output device.
    ///
    /// # Errors
    /// Returns [`Error::Output`] if no output device is available or the stream
    /// cannot be created.
    pub fn new() -> Result<Self> {
        let (stream, stream_handle) =
            OutputStream::try_default().map_err(|e| Error::Output(e.to_string()))?;

        Ok(Player {
            sink: Arc::new(Mutex::new(None)),
            _stream: Some(stream),
            stream_handle: Some(stream_handle),
            task_handle: None,
            cmd_tx: None,
            error_report: None,
            recorder: None,
            volume: 0.8,
            is_playing: false,
        })
    }

    /// Start the recorder, which captures completed songs from supported (Opus)
    /// live streams. Spawns a background recorder thread; safe to call with
    /// [`RecordMode::Off`] so recording can be toggled on later.
    pub fn configure_recording(&mut self, config: RecordingConfig) {
        self.recorder = Some(Recorder::spawn(config));
    }

    /// The current recording mode ([`RecordMode::Off`] if recording isn't configured).
    pub fn recording_mode(&self) -> RecordMode {
        self.recorder
            .as_ref()
            .map_or(RecordMode::Off, Recorder::mode)
    }

    /// Advance recording to its next mode (off → cache → session → off), returning it.
    pub fn cycle_recording_mode(&mut self) -> RecordMode {
        match &mut self.recorder {
            Some(r) => {
                let mode = r.mode().next();
                r.set_mode(mode);
                mode
            }
            None => RecordMode::Off,
        }
    }

    /// Promote the most recently cached song into the permanent library.
    pub fn keep_recording(&self) {
        if let Some(r) = &self.recorder {
            r.keep();
        }
    }

    /// Update the now-playing artwork URL used to tag songs as they're recorded.
    pub fn set_now_playing_artwork(&self, url: Option<String>) {
        if let Some(r) = &self.recorder {
            r.set_artwork(url);
        }
    }

    /// Register a callback for unrecoverable playback failures (e.g. an undecodable
    /// codec). It is invoked from the audio thread, so it must be `Send + Sync`.
    pub fn on_error<F>(&mut self, report: F)
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        self.error_report = Some(Arc::new(report));
    }

    /// Play a live radio stream of the given quality, replacing any current
    /// playback. Reconnects automatically on transient drops.
    ///
    /// # Errors
    /// Returns [`Error::Output`] if the playback sink or thread cannot be created.
    /// A bad stream surfaces later through the [`on_error`](Self::on_error) callback.
    pub fn start_live(&mut self, quality: StreamQuality) -> Result<()> {
        let rec = self.recorder.as_ref().map(Recorder::handle);
        let factory = move || build_live_source(&quality, rec.clone());
        self.start_with_factory(factory, StreamMode::Live)
    }

    /// Play a one-shot source (a song preview MP3), replacing any current playback.
    /// Stops at end of stream rather than reconnecting.
    ///
    /// # Errors
    /// Returns [`Error::Output`] if the playback sink or thread cannot be created.
    pub fn start_preview(&mut self, url: String) -> Result<()> {
        let factory = move || build_preview_source(url.clone());
        self.start_with_factory(factory, StreamMode::OneShot)
    }

    fn start_with_factory<F>(&mut self, factory: F, mode: StreamMode) -> Result<()>
    where
        F: FnMut() -> std::result::Result<Box<dyn PcmSource>, PcmError> + Send + 'static,
    {
        // Stop any existing playback first.
        self.stop_inner();

        let stream_handle = self
            .stream_handle
            .as_ref()
            .ok_or_else(|| Error::Output("No stream handle".to_string()))?;

        let sink = Sink::try_new(stream_handle).map_err(|e| Error::Output(e.to_string()))?;
        sink.set_volume(self.volume);

        let sink_arc = Arc::clone(&self.sink);
        *sink_arc.lock().unwrap() = Some(sink);

        let sink_for_thread = Arc::clone(&self.sink);
        // Bounded channel so Stop is delivered promptly.
        let (cmd_tx, cmd_rx) = std::sync::mpsc::sync_channel::<PlayerCommand>(8);
        self.cmd_tx = Some(cmd_tx);
        let error_report = self.error_report.clone();

        let handle = std::thread::Builder::new()
            .name("plaza-audio".to_string())
            .spawn(move || {
                run_audio_loop(factory, sink_for_thread, cmd_rx, mode, error_report);
            })
            .map_err(|e| Error::Output(e.to_string()))?;

        self.task_handle = Some(handle);
        self.is_playing = true;
        Ok(())
    }

    /// Pause playback, keeping the connection and buffered audio.
    pub fn pause(&mut self) {
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.try_send(PlayerCommand::Pause);
        }
        self.is_playing = false;
    }

    /// Resume playback after a [`pause`](Self::pause).
    pub fn resume(&mut self) {
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.try_send(PlayerCommand::Resume);
        }
        self.is_playing = true;
    }

    /// Set the output volume, clamped to `0.0..=1.0`.
    pub fn set_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.volume = volume;
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.try_send(PlayerCommand::SetVolume(volume));
        }
    }

    /// The current output volume in `0.0..=1.0`.
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Stop playback and tear down the playback thread.
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
        // Don't join — the audio thread may be blocked on I/O and exits when it
        // next checks for commands or finds the sink gone.
        self.task_handle.take();
    }

    /// Whether playback is currently active (not paused or stopped).
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }
}

const INITIAL_BACKOFF: Duration = Duration::from_millis(200);
const MAX_BACKOFF: Duration = Duration::from_secs(5);
/// Cap on queued sink buffers. Each carries its own volume controller, so too many
/// makes volume changes feel laggy; too few risks underruns. ~0.8s at 0.1s/buffer.
const MAX_QUEUED_BUFFERS: usize = 8;

/// Outcome of pumping a single source session.
#[derive(Debug, PartialEq, Eq)]
enum Pump {
    /// Stop command received — the whole audio thread should exit.
    Stopped,
    /// Source ended transiently (connection/read). Live mode reconnects.
    Ended,
    /// Permanent failure (unsupported codec, bad playlist). Stop and report.
    Permanent(String),
}

fn run_audio_loop<F>(
    factory: F,
    sink_arc: Arc<Mutex<Option<Sink>>>,
    cmd_rx: Receiver<PlayerCommand>,
    mode: StreamMode,
    error_report: Option<ErrorReporter>,
) where
    F: FnMut() -> std::result::Result<Box<dyn PcmSource>, PcmError>,
{
    audio_loop_core(factory, sink_arc, cmd_rx, mode, error_report);
}

/// The reconnect/playback loop, generic over the source factory so the retry
/// policy is unit-testable with fake sources (no network).
fn audio_loop_core<F>(
    mut factory: F,
    sink_arc: Arc<Mutex<Option<Sink>>>,
    cmd_rx: Receiver<PlayerCommand>,
    mode: StreamMode,
    error_report: Option<ErrorReporter>,
) where
    F: FnMut() -> std::result::Result<Box<dyn PcmSource>, PcmError>,
{
    let report = |msg: String| {
        tracing::error!("Audio: permanent failure: {msg}");
        if let Some(reporter) = &error_report {
            reporter(msg);
        }
    };

    let mut backoff = INITIAL_BACKOFF;
    loop {
        if drain_commands(&cmd_rx, &sink_arc) {
            tracing::info!("Audio thread: received Stop");
            return;
        }

        match factory() {
            Ok(mut source) => {
                tracing::info!("Audio thread: source opened");
                backoff = INITIAL_BACKOFF;
                match pump_source(source.as_mut(), &sink_arc, &cmd_rx) {
                    Pump::Stopped => return,
                    Pump::Permanent(msg) => {
                        report(msg);
                        return;
                    }
                    Pump::Ended => {
                        if mode == StreamMode::OneShot {
                            tracing::info!("Audio thread: one-shot ended");
                            return;
                        }
                        tracing::warn!("Audio thread: live stream ended, will reconnect");
                    }
                }
            }
            Err(PcmError::Permanent(msg)) => {
                report(msg);
                return;
            }
            Err(PcmError::Ended) => {
                if mode == StreamMode::OneShot {
                    tracing::error!("Audio thread: source open failed (one-shot)");
                    return;
                }
                tracing::warn!("Audio thread: source open failed, retrying in {backoff:?}");
            }
        }

        if wait_for_stop(&cmd_rx, backoff) {
            tracing::info!("Audio thread: exiting (stopped during backoff)");
            return;
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Pull chunks from one source and feed the sink until it ends or stops.
fn pump_source(
    source: &mut dyn PcmSource,
    sink_arc: &Arc<Mutex<Option<Sink>>>,
    cmd_rx: &Receiver<PlayerCommand>,
) -> Pump {
    let mut feeder = SinkFeeder::new();
    loop {
        if drain_commands(cmd_rx, sink_arc) {
            return Pump::Stopped;
        }
        match source.next_chunk() {
            Ok(Some(chunk)) => match feeder.push(chunk, sink_arc, cmd_rx) {
                Feed::Ok => {}
                Feed::Stopped => return Pump::Stopped,
                Feed::SinkGone => return Pump::Stopped,
            },
            Ok(None) => {
                // No data right now (e.g. HLS live edge). Back off briefly while
                // staying responsive to pause/volume/stop.
                if drain_commands(cmd_rx, sink_arc) {
                    return Pump::Stopped;
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            Err(PcmError::Ended) => {
                feeder.flush(sink_arc);
                return Pump::Ended;
            }
            Err(PcmError::Permanent(msg)) => {
                feeder.flush(sink_arc);
                return Pump::Permanent(msg);
            }
        }
    }
}

enum Feed {
    Ok,
    Stopped,
    SinkGone,
}

/// Accumulates PCM into ~0.1s batches and appends them to the sink, applying
/// backpressure (capping queued buffers) and flushing on format changes.
struct SinkFeeder {
    pending: Vec<f32>,
    rate: u32,
    channels: u16,
}

impl SinkFeeder {
    fn new() -> Self {
        SinkFeeder {
            pending: Vec::with_capacity(16384),
            rate: 0,
            channels: 0,
        }
    }

    fn push(
        &mut self,
        chunk: PcmChunk,
        sink_arc: &Arc<Mutex<Option<Sink>>>,
        cmd_rx: &Receiver<PlayerCommand>,
    ) -> Feed {
        // A format change must flush the old batch before the spec changes.
        if (self.rate != chunk.sample_rate || self.channels != chunk.channels)
            && !self.pending.is_empty()
            && !self.flush(sink_arc)
        {
            return Feed::SinkGone;
        }
        self.rate = chunk.sample_rate;
        self.channels = chunk.channels;
        self.pending.extend_from_slice(&chunk.samples);

        let flush_threshold = (self.rate as usize / 10) * self.channels.max(1) as usize;
        if self.pending.len() < flush_threshold.max(1) {
            return Feed::Ok;
        }

        // Append the batch, waiting for the sink to drain if it's full.
        loop {
            if drain_commands(cmd_rx, sink_arc) {
                return Feed::Stopped;
            }
            let guard = match sink_arc.lock() {
                Ok(g) => g,
                Err(_) => return Feed::SinkGone,
            };
            match guard.as_ref() {
                None => return Feed::SinkGone,
                Some(sink) if sink.len() < MAX_QUEUED_BUFFERS => {
                    let batch = std::mem::take(&mut self.pending);
                    sink.append(SamplesBuffer::new(self.channels, self.rate, batch));
                    return Feed::Ok;
                }
                Some(_) => {
                    drop(guard);
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }

    /// Flush remaining samples to the sink. Returns false if the sink is gone.
    fn flush(&mut self, sink_arc: &Arc<Mutex<Option<Sink>>>) -> bool {
        if self.pending.is_empty() {
            return true;
        }
        let batch = std::mem::take(&mut self.pending);
        match sink_arc.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(sink) => {
                    sink.append(SamplesBuffer::new(self.channels, self.rate, batch));
                    true
                }
                None => false,
            },
            Err(_) => false,
        }
    }
}

/// Sleep up to `dur`; return true if Stop was received (so the caller should exit).
fn wait_for_stop(cmd_rx: &Receiver<PlayerCommand>, dur: Duration) -> bool {
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

/// Drain queued commands, applying them to the sink. Returns true if Stop was seen.
fn drain_commands(cmd_rx: &Receiver<PlayerCommand>, sink_arc: &Arc<Mutex<Option<Sink>>>) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::sync_channel;

    /// A fake source whose first `next_chunk` returns a fixed terminal result.
    /// Used to drive the loop's retry/permanent/stop policy without a network.
    enum Behavior {
        End,
        Permanent(String),
    }
    struct FakeSource {
        behavior: Behavior,
    }
    impl PcmSource for FakeSource {
        fn next_chunk(&mut self) -> std::result::Result<Option<PcmChunk>, PcmError> {
            match &self.behavior {
                Behavior::End => Err(PcmError::Ended),
                Behavior::Permanent(m) => Err(PcmError::Permanent(m.clone())),
            }
        }
    }

    fn no_sink() -> Arc<Mutex<Option<Sink>>> {
        Arc::new(Mutex::new(None))
    }

    /// A reporter that records every reported message for later assertion.
    fn recording_reporter() -> (ErrorReporter, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        let reporter: ErrorReporter = Arc::new(move |msg| sink.lock().unwrap().push(msg));
        (reporter, log)
    }

    /// Live mode must reconnect when opening the source keeps failing transiently.
    #[test]
    fn live_mode_retries_after_open_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let factory = move || -> std::result::Result<Box<dyn PcmSource>, PcmError> {
            c.fetch_add(1, Ordering::SeqCst);
            Err(PcmError::Ended)
        };
        let (tx, rx) = sync_channel::<PlayerCommand>(8);
        let handle = std::thread::spawn(move || {
            run_audio_loop(factory, no_sink(), rx, StreamMode::Live, None);
        });
        std::thread::sleep(Duration::from_millis(700));
        assert!(
            calls.load(Ordering::SeqCst) >= 2,
            "live mode should retry open failures"
        );
        tx.send(PlayerCommand::Stop).unwrap();
        drop(tx);
        handle.join().unwrap();
    }

    /// One-shot mode must NOT retry after an open failure.
    #[test]
    fn oneshot_mode_does_not_retry_after_open_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let factory = move || -> std::result::Result<Box<dyn PcmSource>, PcmError> {
            c.fetch_add(1, Ordering::SeqCst);
            Err(PcmError::Ended)
        };
        let (_tx, rx) = sync_channel::<PlayerCommand>(8);
        let handle = std::thread::spawn(move || {
            run_audio_loop(factory, no_sink(), rx, StreamMode::OneShot, None);
        });
        handle.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one-shot must not retry");
    }

    /// Regression for the Opus reconnect storm: a permanent failure must open the
    /// source exactly once, report the error, and exit — never reconnect.
    #[test]
    fn live_mode_does_not_retry_on_permanent_failure_and_reports_error() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let factory = move || -> std::result::Result<Box<dyn PcmSource>, PcmError> {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeSource {
                behavior: Behavior::Permanent("unsupported codec".into()),
            }))
        };
        let (_tx, rx) = sync_channel::<PlayerCommand>(8);
        let (reporter, log) = recording_reporter();
        let handle = std::thread::spawn(move || {
            run_audio_loop(factory, no_sink(), rx, StreamMode::Live, Some(reporter));
        });
        handle.join().unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "permanent failure must not retry"
        );
        assert_eq!(log.lock().unwrap().as_slice(), ["unsupported codec"]);
    }

    /// A permanent failure surfaced while *opening* the source must also stop + report.
    #[test]
    fn permanent_open_failure_reports_and_stops() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let factory = move || -> std::result::Result<Box<dyn PcmSource>, PcmError> {
            c.fetch_add(1, Ordering::SeqCst);
            Err(PcmError::Permanent("bad stream".into()))
        };
        let (_tx, rx) = sync_channel::<PlayerCommand>(8);
        let (reporter, log) = recording_reporter();
        let handle = std::thread::spawn(move || {
            run_audio_loop(factory, no_sink(), rx, StreamMode::Live, Some(reporter));
        });
        handle.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(log.lock().unwrap().as_slice(), ["bad stream"]);
    }

    /// A transient session end in Live mode must reconnect (reopen the source).
    #[test]
    fn live_mode_retries_on_transient_session_end() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let factory = move || -> std::result::Result<Box<dyn PcmSource>, PcmError> {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeSource {
                behavior: Behavior::End,
            }))
        };
        let (tx, rx) = sync_channel::<PlayerCommand>(8);
        let handle = std::thread::spawn(move || {
            run_audio_loop(factory, no_sink(), rx, StreamMode::Live, None);
        });
        std::thread::sleep(Duration::from_millis(700));
        assert!(
            calls.load(Ordering::SeqCst) >= 2,
            "transient end should reconnect"
        );
        tx.send(PlayerCommand::Stop).unwrap();
        drop(tx);
        handle.join().unwrap();
    }

    /// Stop between reconnect attempts must terminate the loop promptly.
    #[test]
    fn stop_terminates_live_loop_during_backoff() {
        let factory =
            || -> std::result::Result<Box<dyn PcmSource>, PcmError> { Err(PcmError::Ended) };
        let (tx, rx) = sync_channel::<PlayerCommand>(8);
        let handle = std::thread::spawn(move || {
            run_audio_loop(factory, no_sink(), rx, StreamMode::Live, None);
        });
        std::thread::sleep(Duration::from_millis(50));
        let start = std::time::Instant::now();
        tx.send(PlayerCommand::Stop).unwrap();
        drop(tx);
        handle.join().unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "Stop should be prompt"
        );
    }
}
