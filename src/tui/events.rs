use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::time::interval;
use crate::api::models::StatusResource;
use crate::socket::SocketEvent;

#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    Resize(u16, u16),
    Tick,
    Paste(String),
    StatusUpdate(StatusResource),
    ListenersUpdate(u32),
    ReactionsUpdate(u32),
    AudioError(String),
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
                    Some(Ok(Event::Key(key))) => {
                        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                            AppEvent::Quit
                        } else {
                            AppEvent::Key(key)
                        }
                    }
                    Some(Ok(Event::Paste(text))) => AppEvent::Paste(text),
                    Some(Ok(Event::Resize(w, h))) => AppEvent::Resize(w, h),
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
