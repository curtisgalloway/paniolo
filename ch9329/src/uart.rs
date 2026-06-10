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

//! The UART owner: a single dedicated thread that owns the CH9329 control UART
//! (one long-lived [`Session`]) and serializes every command — CLI-injected and
//! WebSocket-injected alike — onto the one wire, one in flight. That single
//! queue is what makes events from the web console and the CLI intermix
//! correctly, and it is what makes held state (`down`/`mdown`/drag) work: the
//! one persistent `Session` carries the modifier/button report across commands,
//! which a one-shot CLI invocation cannot.
//!
//! It uses the **blocking** `serialport` path (the same one the one-shot CLI
//! uses), not async I/O: tokio-serial's async reads do not get reliable
//! read-readiness on a macOS tty. The thread bridges to the async axum server
//! via tokio channels — `blocking_recv` for requests, `oneshot`/`broadcast`
//! sends for replies.
//!
//! The `Session` is opened lazily and dropped on transport error, so the daemon
//! recovers across adapter replug and target power cycles without a restart.

use std::thread;

use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{info, warn};

use crate::proto::execute_line;
use crate::session::Session;

const REQ_CAP: usize = 256;
const TRANSCRIPT_CAP: usize = 256;

/// One queued command awaiting its reply.
struct Request {
    line: String,
    reply: oneshot::Sender<Result<String, String>>,
}

/// A transcript event broadcast to every WebSocket observer: the command that
/// ran and its one-line outcome. Lets a passive viewer see what the CLI (or
/// another browser) just injected.
#[derive(Clone, Debug)]
pub struct Event {
    pub line: String,
    pub ok: bool,
    pub reply: String,
}

/// Cloneable handle to the UART owner thread.
#[derive(Clone)]
pub struct HidHandle {
    req_tx: mpsc::Sender<Request>,
    transcript: broadcast::Sender<Event>,
    pub device: String,
}

impl HidHandle {
    /// Spawn the owner thread for `device` and return a handle. The port itself
    /// is opened lazily on the first command (so the daemon starts even with
    /// the target — and therefore the CH9329 — currently powered off).
    pub fn spawn(device: String) -> HidHandle {
        let (req_tx, req_rx) = mpsc::channel(REQ_CAP);
        let (transcript, _) = broadcast::channel(TRANSCRIPT_CAP);
        let handle = HidHandle {
            req_tx,
            transcript: transcript.clone(),
            device: device.clone(),
        };
        thread::spawn(move || run(device, req_rx, transcript));
        handle
    }

    /// Submit one command line and await the reply (the `OK` data, or the
    /// `ERR`/transport message). The line must not contain a newline.
    pub async fn send(&self, line: String) -> Result<String, String> {
        if line.contains('\n') || line.contains('\r') {
            return Err(format!("command contains a newline: {line:?}"));
        }
        let (tx, rx) = oneshot::channel();
        self.req_tx
            .send(Request { line, reply: tx })
            .await
            .map_err(|_| "hid daemon stopped".to_string())?;
        rx.await
            .map_err(|_| "hid daemon dropped the request".to_string())?
    }

    /// Subscribe to the command transcript (for WebSocket observers).
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.transcript.subscribe()
    }
}

/// True for errors that mean the port/session itself is gone (vs. a board-level
/// `ERR`), so the next request reopens it (adapter replug, target power cycle).
fn is_transport_error(msg: &str) -> bool {
    msg.starts_with("cannot open")
        || msg.starts_with("serial write failed")
        || msg.starts_with("serial read failed")
        || msg.starts_with("timed out")
        || msg.starts_with("serial port closed")
        || msg.starts_with("CH9329 did not respond")
}

/// The owner loop (blocking thread): drain requests, execute each against the
/// one persistent [`Session`], broadcast the outcome.
fn run(device: String, mut req_rx: mpsc::Receiver<Request>, transcript: broadcast::Sender<Event>) {
    let mut session: Option<Session> = None;
    info!("ch9329 UART owner started for {device}");

    while let Some(req) = req_rx.blocking_recv() {
        if session.is_none() {
            match Session::open(&device, None) {
                Ok(s) => {
                    info!("ch9329 UART open at {} baud for {device}", s.baud());
                    session = Some(s);
                }
                Err(e) => {
                    let msg = e.to_string();
                    broadcast_event(&transcript, &req.line, &Err(msg.clone()));
                    let _ = req.reply.send(Err(msg));
                    continue;
                }
            }
        }

        let result = execute_line(session.as_mut().unwrap(), &req.line).map_err(|e| e.to_string());
        if let Err(ref msg) = result {
            if is_transport_error(msg) {
                warn!("ch9329 UART transport error, will reopen: {msg}");
                session = None;
            }
        }
        broadcast_event(&transcript, &req.line, &result);
        let _ = req.reply.send(result);
    }
    info!("ch9329 UART owner stopped for {device}");
}

fn broadcast_event(tx: &broadcast::Sender<Event>, line: &str, result: &Result<String, String>) {
    let ev = match result {
        Ok(data) => Event {
            line: line.to_string(),
            ok: true,
            reply: if data.is_empty() {
                "OK".to_string()
            } else {
                format!("OK {data}")
            },
        },
        Err(e) => Event {
            line: line.to_string(),
            ok: false,
            reply: e.clone(),
        },
    };
    let _ = tx.send(ev);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_error_classification() {
        assert!(is_transport_error("cannot open /dev/x: busy"));
        assert!(is_transport_error("serial write failed: x"));
        assert!(is_transport_error("timed out waiting for CH9329 reply"));
        assert!(is_transport_error(
            "CH9329 did not respond on /dev/x at 115200/9600 baud"
        ));
        assert!(!is_transport_error(
            "CH9329 rejected cmd 0x02: bad parameter (0xe5)"
        ));
        assert!(!is_transport_error("unknown command: foo"));
    }
}
