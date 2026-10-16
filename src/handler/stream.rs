use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::{broadcast::error::RecvError, watch};

use crate::state::AppState;

pub async fn stream_ws_handler(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    tracing::info!("🔥 WS upgrade received");
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

struct LiveClientGuard(Arc<watch::Sender<usize>>);

impl LiveClientGuard {
    fn new(clients: Arc<watch::Sender<usize>>) -> Self {
        clients.send_modify(|n| *n += 1);
        Self(clients)
    }
}

impl Drop for LiveClientGuard {
    fn drop(&mut self) {
        self.0.send_modify(|n| *n = n.saturating_sub(1));
    }
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let _guard = LiveClientGuard::new(state.live_clients.clone());
    tracing::info!(
        clients = *state.live_clients.borrow(),
        "🔥 Live WebSocket client connected"
    );
    let mut rx = state.tx_broadcast.subscribe();
    let (mut sender, mut receiver) = socket.split();

    let mut recv_task =
        tokio::spawn(async move { while let Some(Ok(_)) = receiver.next().await {} });

    let mut send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(payload) => {
                    if sender.send(Message::Text(payload)).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "WS client lagged, dropping backlog");
                    continue;
                }
                Err(RecvError::Closed) => break,
            }
        }
    });

    tokio::select! {
        _ = &mut recv_task => send_task.abort(),
        _ = &mut send_task => recv_task.abort(),
    }
}
