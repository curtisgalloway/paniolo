// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Localhost HTTP/WebSocket API for the hid daemon (identical to hidrig's, so
//! `paniolo console` drives a CH9329 daemon the same way).
//!
//! `GET /hid` is a bidirectional WebSocket carrying the HID serial protocol:
//! each client text frame is one command line; the daemon replies one text
//! frame per command and also pushes a transcript of commands injected by
//! *other* clients (and the CLI), so the web console sees the full intermixed
//! stream. `POST /send` is the one-shot equivalent used by the CLI when a
//! daemon is already running. Responses carry a permissive CORS header because
//! the hdmicap dashboard connects cross-port.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::header,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;
use tracing::debug;

use crate::uart::HidHandle;

#[derive(Clone)]
pub struct AppState {
    pub hid: HidHandle,
}

const CORS: (header::HeaderName, &str) = (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/hid", get(hid_ws))
        .route("/send", post(send))
        .route("/status", get(status))
        .route("/version", get(version))
        .with_state(state)
}

/// `GET /status` — daemon liveness + the device it owns.
async fn status(State(s): State<AppState>) -> Response {
    (
        [CORS],
        Json(serde_json::json!({
            "device": s.hid.device,
            "pid": std::process::id(),
        })),
    )
        .into_response()
}

/// `GET /version` — forwards a `version` command to the injector.
async fn version(State(s): State<AppState>) -> Response {
    match s.hid.send("version".to_string()).await {
        Ok(data) => ([CORS], data).into_response(),
        Err(e) => (axum::http::StatusCode::SERVICE_UNAVAILABLE, [CORS], e).into_response(),
    }
}

/// `POST /send`, body = one command line. Returns the `OK <data>` payload, or
/// 503 with the `ERR`/transport message. Used by the CLI one-shot path.
async fn send(State(s): State<AppState>, body: String) -> Response {
    let line = body.trim_end_matches(['\r', '\n']).to_string();
    match s.hid.send(line).await {
        Ok(data) => ([CORS], data).into_response(),
        Err(e) => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            [CORS],
            format!("{e}\n"),
        )
            .into_response(),
    }
}

async fn hid_ws(ws: WebSocketUpgrade, State(s): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, s.hid))
}

/// Per-client WebSocket loop. Inbound text frames are command lines executed
/// against the shared UART owner; each gets a one-frame reply. Concurrently we
/// push transcript events for commands injected by everyone else.
async fn handle_ws(socket: WebSocket, hid: HidHandle) {
    let (mut sender, mut receiver) = socket.split();
    let mut events = hid.subscribe();

    // transcript -> client (commands run by other clients / the CLI)
    let mut feed_task = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(ev) => {
                    let tag = if ev.ok { "evt ok" } else { "evt err" };
                    let frame = format!("{tag} {} :: {}", ev.line, ev.reply);
                    if sender.send(Message::Text(frame)).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(n)) => debug!("hid ws observer lagged {n}"),
                Err(RecvError::Closed) => break,
            }
        }
    });

    // client -> UART. Every command (from any client or the CLI) produces a
    // single broadcast `evt ok/err …` frame, so the issuer sees its own result
    // there too — no separate per-issuer reply channel is needed, and all
    // clients observe one consistent intermixed transcript.
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            let line = match msg {
                Message::Text(t) => t.trim().to_string(),
                Message::Close(_) => break,
                _ => continue,
            };
            if line.is_empty() {
                continue;
            }
            let _ = hid.send(line).await; // result is broadcast as an `evt` frame
        }
    });

    tokio::select! {
        _ = &mut feed_task => recv_task.abort(),
        _ = &mut recv_task => feed_task.abort(),
    }
}
