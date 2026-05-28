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

//! Localhost HTTP API. The daemon can own several named serial interfaces; every
//! per-interface endpoint takes `?interface=NAME` and falls back to the default
//! (first-configured) interface when it's omitted, so single-interface clients
//! (and the existing dashboard) keep working unchanged.
//!
//! `/stream` is a bidirectional WebSocket: the daemon sends serial output (binary
//! frames) and accepts client keystrokes (binary or text) to write back to the
//! port. The hdmicap preview page connects here cross-port, so responses carry a
//! permissive CORS header.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tracing::debug;

use crate::serial_io::{NamedSerial, SerialHandle, Serials};

#[derive(Clone)]
pub struct AppState {
    pub serials: Serials,
}

#[derive(Deserialize)]
pub struct IfaceParam {
    interface: Option<String>,
}

#[derive(Deserialize)]
pub struct ButtonParam {
    interface: Option<String>,
    ms: u64,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/stream", get(stream))
        .route("/status", get(status))
        .route("/interfaces", get(interfaces))
        .route("/devices", get(devices))
        .route("/button", post(button))
        .with_state(state)
}

const CORS: (header::HeaderName, &str) = (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");

/// Resolve the requested interface, or the default (first) when none is named.
fn resolve<'a>(serials: &'a Serials, name: &Option<String>) -> Option<&'a SerialHandle> {
    match name {
        Some(n) => serials.get(n),
        None => serials.default().map(|ns| &ns.handle),
    }
}

fn status_json(ns: &NamedSerial) -> serde_json::Value {
    let st = ns.handle.status();
    serde_json::json!({
        "name": ns.name,
        "device": st.device,
        "baud": st.baud,
        "connected": st.connected,
        "power_on": st.power_on,   // null when no sense signal is configured
    })
}

/// Status of one interface (`?interface=NAME`) or, by default, all of them.
async fn status(State(s): State<AppState>, Query(q): Query<IfaceParam>) -> Response {
    match &q.interface {
        Some(name) => match s.serials.all().iter().find(|ns| &ns.name == name) {
            Some(ns) => ([CORS], Json(status_json(ns))).into_response(),
            None => (
                StatusCode::NOT_FOUND,
                [CORS],
                format!("no interface '{name}'"),
            )
                .into_response(),
        },
        None => {
            let all: Vec<_> = s.serials.all().iter().map(status_json).collect();
            ([CORS], Json(all)).into_response()
        }
    }
}

/// All interfaces this daemon owns (name, device, baud, connected).
async fn interfaces(State(s): State<AppState>) -> Response {
    let all: Vec<_> = s.serials.all().iter().map(status_json).collect();
    ([CORS], Json(all)).into_response()
}

async fn devices() -> Response {
    match crate::serial_io::list_ports() {
        Ok(list) => (
            [CORS],
            Json(
                list.into_iter()
                    .map(|(path, desc)| serde_json::json!({"path": path, "misc": desc}))
                    .collect::<Vec<_>>(),
            ),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("{e:#}"),
        )
            .into_response(),
    }
}

/// Press the J2 power button on the attached target for `ms` milliseconds.
///
/// `POST /button?ms=200[&interface=NAME]`
///
/// Short presses (≤500 ms) deliver a power-button event to the OS (graceful
/// reboot/halt, target-OS-defined).  Long presses (≥3000 ms) trigger a PMIC
/// hard power-off.  The call blocks until the press completes.
/// Returns 200 on success, 503 if the supervisor is not running.
async fn button(State(s): State<AppState>, Query(q): Query<ButtonParam>) -> Response {
    let handle = match resolve(&s.serials, &q.interface) {
        Some(h) => h.clone(),
        None => {
            let what = q.interface.as_deref().unwrap_or("(default)");
            return (
                StatusCode::NOT_FOUND,
                [CORS],
                format!("no interface '{what}'"),
            )
                .into_response();
        }
    };
    match handle.dtr_press(q.ms).await {
        Ok(()) => ([CORS], format!("button pressed for {} ms\n", q.ms)).into_response(),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, [CORS], format!("{e:#}\n")).into_response(),
    }
}

async fn stream(
    ws: WebSocketUpgrade,
    State(s): State<AppState>,
    Query(q): Query<IfaceParam>,
) -> Response {
    let handle = match resolve(&s.serials, &q.interface) {
        Some(h) => h.clone(),
        None => {
            let what = q.interface.as_deref().unwrap_or("(default)");
            return (
                StatusCode::NOT_FOUND,
                [CORS],
                format!("no interface '{what}'"),
            )
                .into_response();
        }
    };
    ws.on_upgrade(move |socket| handle_ws(socket, handle))
}

async fn handle_ws(socket: WebSocket, serial: SerialHandle) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = serial.subscribe();

    // Send recent scrollback so a mid-stream connection isn't blank.
    let snapshot = serial.scrollback();
    if !snapshot.is_empty() && sender.send(Message::Binary(snapshot)).await.is_err() {
        return;
    }

    // serial -> client
    let mut send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(bytes) => {
                    if sender.send(Message::Binary(bytes.to_vec())).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    debug!("ws client lagged, dropped {n} messages");
                }
                Err(RecvError::Closed) => break,
            }
        }
    });

    // client -> serial
    let write_tx = serial.write_tx.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Binary(b) => {
                    let _ = write_tx.try_send(Bytes::from(b));
                }
                Message::Text(t) => {
                    let _ = write_tx.try_send(Bytes::from(t.into_bytes()));
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}
