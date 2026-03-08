use rust_socketio::{
    asynchronous::{Client, ClientBuilder},
    Payload,
};
use std::time::Duration;
use tokio::sync::broadcast;

use crate::api::models::StatusResource;

#[derive(Debug, Clone)]
pub enum SocketEvent {
    Status(StatusResource),
    Listeners(u32),
    Reactions(u32),
    Disconnected,
    Reconnected,
}

pub struct SocketClient {
    sender: broadcast::Sender<SocketEvent>,
}

impl SocketClient {
    pub async fn connect() -> crate::error::Result<Self> {
        let (sender, _) = broadcast::channel::<SocketEvent>(64);
        let sender_for_task = sender.clone();

        tokio::spawn(async move {
            let mut backoff_secs: u64 = 1;
            loop {
                tracing::info!("Connecting to Plaza socket.io server...");

                // Clone the broadcast sender for each callback before building.
                // broadcast::Sender<T> is Clone + Send, so this is fine.
                let s_status = sender_for_task.clone();
                let s_listeners = sender_for_task.clone();
                let s_reactions = sender_for_task.clone();
                let s_disconnect = sender_for_task.clone();
                let s_connect = sender_for_task.clone();

                // The Plaza socket.io server uses a custom engine.io path: /ws/socket.io/
                // rust_socketio only replaces the path with /socket.io/ when the URL path is /;
                // for a custom path we must include /socket.io/ explicitly in the URL.
                let result = ClientBuilder::new("https://plaza.one/ws/socket.io/")
                    .on("status", move |payload: Payload, _socket: Client| {
                        let s = s_status.clone();
                        Box::pin(async move {
                            if let Payload::Text(values) = payload {
                                if let Some(first) = values.into_iter().next() {
                                    match serde_json::from_value::<StatusResource>(first) {
                                        Ok(status) => {
                                            let _ = s.send(SocketEvent::Status(status));
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "Failed to parse status event: {}",
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                        })
                    })
                    .on(
                        "listeners",
                        move |payload: Payload, _socket: Client| {
                            let s = s_listeners.clone();
                            Box::pin(async move {
                                if let Payload::Text(values) = payload {
                                    if let Some(first) = values.into_iter().next() {
                                        if let Some(n) = first.as_u64() {
                                            let _ = s.send(SocketEvent::Listeners(n as u32));
                                        }
                                    }
                                }
                            })
                        },
                    )
                    .on(
                        "reactions",
                        move |payload: Payload, _socket: Client| {
                            let s = s_reactions.clone();
                            Box::pin(async move {
                                if let Payload::Text(values) = payload {
                                    if let Some(first) = values.into_iter().next() {
                                        if let Some(n) = first.as_u64() {
                                            let _ = s.send(SocketEvent::Reactions(n as u32));
                                        }
                                    }
                                }
                            })
                        },
                    )
                    .on("disconnect", move |_payload: Payload, _socket: Client| {
                        let s = s_disconnect.clone();
                        Box::pin(async move {
                            tracing::warn!("Socket.io disconnected");
                            let _ = s.send(SocketEvent::Disconnected);
                        })
                    })
                    .on("connect", move |_payload: Payload, _socket: Client| {
                        let s = s_connect.clone();
                        Box::pin(async move {
                            tracing::info!("Socket.io connected");
                            let _ = s.send(SocketEvent::Reconnected);
                        })
                    })
                    .connect()
                    .await;

                match result {
                    Ok(_client) => {
                        backoff_secs = 1;
                        tracing::info!("Socket.io connection established");
                        // Keep the spawned task (and therefore the client) alive.
                        // The client runs its event loop internally; parking here
                        // prevents the task from completing and dropping the client.
                        loop {
                            tokio::time::sleep(Duration::from_secs(30)).await;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Socket.io connection failed: {}", e);
                        let _ = sender_for_task.send(SocketEvent::Disconnected);
                        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(30);
                    }
                }
            }
        });

        Ok(SocketClient { sender })
    }

    /// Subscribe to socket events.
    pub fn subscribe(&self) -> broadcast::Receiver<SocketEvent> {
        self.sender.subscribe()
    }

    /// Clone the underlying sender (for forwarding events elsewhere).
    pub fn sender(&self) -> broadcast::Sender<SocketEvent> {
        self.sender.clone()
    }
}
