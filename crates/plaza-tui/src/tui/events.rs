use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use tokio::sync::{broadcast, mpsc};
use tokio::time::interval;

use plaza_api::models::StatusResource;
use plaza_api::SocketEvent;

/// A single thing for the run loop to react to, merged from all input sources.
//
// `StatusUpdate` is larger than the rest, but events are handled one at a time
// rather than stored in bulk, so boxing it would only add an allocation per song
// change for no real saving.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// A key was pressed.
    Key(KeyEvent),
    /// The terminal was resized; the next draw re-fits automatically.
    Resize,
    /// The periodic timer fired (drives redraws and position tracking).
    Tick,
    /// Text was pasted into the terminal.
    Paste(String),
    /// The now-playing song changed.
    StatusUpdate(StatusResource),
    /// The live listener count changed.
    ListenersUpdate(u32),
    /// The reaction total changed.
    ReactionsUpdate(u32),
    /// The audio engine reported an unrecoverable error.
    AudioError(String),
    /// The user asked to quit.
    Quit,
}

pub struct EventHandler {
    event_stream: EventStream,
    socket_rx: broadcast::Receiver<SocketEvent>,
    audio_error_rx: mpsc::Receiver<String>,
    tick: tokio::time::Interval,
}

impl EventHandler {
    pub fn new(
        socket_rx: broadcast::Receiver<SocketEvent>,
        audio_error_rx: mpsc::Receiver<String>,
    ) -> Self {
        EventHandler {
            event_stream: EventStream::new(),
            socket_rx,
            audio_error_rx,
            tick: interval(Duration::from_millis(250)),
        }
    }

    pub async fn next(&mut self) -> AppEvent {
        tokio::select! {
            // Terminal events
            event = self.event_stream.next() => {
                match event {
                    // Ignore key-release events: terminals with the kitty keyboard
                    // protocol (or Windows) emit Press AND Release for every key, which
                    // would double-fire actions and make navigation feel broken. We act
                    // on Press/Repeat only.
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Release => AppEvent::Tick,
                    Some(Ok(Event::Key(key))) => {
                        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                            AppEvent::Quit
                        } else {
                            AppEvent::Key(key)
                        }
                    }
                    Some(Ok(Event::Paste(text))) => AppEvent::Paste(text),
                    Some(Ok(Event::Resize(_, _))) => AppEvent::Resize,
                    _ => AppEvent::Tick,
                }
            }
            // Tick
            _ = self.tick.tick() => AppEvent::Tick,
            // Socket events
            event = self.socket_rx.recv() => {
                match event {
                    Ok(SocketEvent::Status(s)) => AppEvent::StatusUpdate(s),
                    Ok(SocketEvent::Listeners(n)) => AppEvent::ListenersUpdate(n),
                    Ok(SocketEvent::Reactions(n)) => AppEvent::ReactionsUpdate(n),
                    _ => AppEvent::Tick,
                }
            }
            // Audio errors
            err = self.audio_error_rx.recv() => {
                match err {
                    Some(e) => AppEvent::AudioError(e),
                    None => AppEvent::Tick,
                }
            }
        }
    }
}
