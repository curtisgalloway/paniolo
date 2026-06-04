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

//! The UART owner: a single async task that owns the injector's control UART
//! and serializes every command — CLI-injected and WebSocket-injected alike —
//! onto the one wire, one in flight, request/reply. That single queue is what
//! makes events from the web console and the CLI intermix correctly.
//!
//! The port is opened lazily and dropped on I/O error, so the daemon recovers
//! across adapter replug and target power cycles without a restart.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_serial::{SerialPortBuilderExt, SerialStream};
use tracing::{info, warn};

use crate::proto::BAUD;

/// Per-command reply timeout. A long `type`/`move` executes a HID report per
/// step before the board answers, so allow generous slack.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
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

/// Cloneable handle to the UART owner task.
#[derive(Clone)]
pub struct HidHandle {
    req_tx: mpsc::Sender<Request>,
    transcript: broadcast::Sender<Event>,
    pub device: String,
}

impl HidHandle {
    /// Spawn the owner task for `device` and return a handle. The port itself
    /// is opened lazily on the first command (so the daemon starts even with
    /// the target — and therefore the board — currently powered off).
    pub fn spawn(device: String) -> HidHandle {
        let (req_tx, req_rx) = mpsc::channel(REQ_CAP);
        let (transcript, _) = broadcast::channel(TRANSCRIPT_CAP);
        let handle = HidHandle {
            req_tx,
            transcript: transcript.clone(),
            device: device.clone(),
        };
        tokio::spawn(run(device, req_rx, transcript));
        handle
    }

    /// Submit one command line and await the board's reply (the `OK` data, or
    /// the `ERR`/transport message). The line must not contain a newline.
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

/// Open the control UART as an async stream.
async fn open(device: &str) -> Result<SerialStream, String> {
    tokio_serial::new(device, BAUD)
        .data_bits(tokio_serial::DataBits::Eight)
        .parity(tokio_serial::Parity::None)
        .stop_bits(tokio_serial::StopBits::One)
        .open_native_async()
        .map_err(|e| format!("cannot open {device}: {e}"))
}

/// Parse a reply line into the `OK <data>` payload or an `Err` message.
fn parse_reply(line: &str) -> Result<String, String> {
    let reply = line.trim_end_matches(['\r', '\n']);
    if let Some(rest) = reply.strip_prefix("OK") {
        Ok(rest.trim().to_string())
    } else if reply.starts_with("ERR") {
        Err(reply.to_string())
    } else {
        Err(format!("unexpected reply: {reply:?}"))
    }
}

/// The owner loop: drain requests, write each, read its reply, broadcast the
/// outcome. The port is reopened on the next request after any I/O error.
async fn run(
    device: String,
    mut req_rx: mpsc::Receiver<Request>,
    transcript: broadcast::Sender<Event>,
) {
    let mut stream: Option<BufReader<SerialStream>> = None;
    info!("hid UART owner started for {device}");

    while let Some(req) = req_rx.recv().await {
        // Ensure the port is open.
        if stream.is_none() {
            match open(&device).await {
                Ok(s) => stream = Some(BufReader::new(s)),
                Err(e) => {
                    let _ = req.reply.send(Err(e.clone()));
                    broadcast_event(&transcript, &req.line, &Err(e));
                    continue;
                }
            }
        }
        let port = stream.as_mut().unwrap();

        let result = exchange(port, &req.line).await;
        if result.is_err() && is_io_failure(result.as_ref().err().unwrap()) {
            // Drop the port so the next request reopens it (adapter replug etc).
            warn!(
                "hid UART I/O error, will reopen: {:?}",
                result.as_ref().err()
            );
            stream = None;
        }
        broadcast_event(&transcript, &req.line, &result);
        let _ = req.reply.send(result);
    }
    info!("hid UART owner stopped for {device}");
}

/// True for errors that mean the port itself is gone (vs. a board-level `ERR`).
fn is_io_failure(msg: &str) -> bool {
    msg.starts_with("write error")
        || msg.starts_with("read error")
        || msg.starts_with("device closed")
}

/// Write one command line and read exactly one reply line (with a timeout).
async fn exchange(port: &mut BufReader<SerialStream>, line: &str) -> Result<String, String> {
    let msg = format!("{line}\n");
    port.get_mut()
        .write_all(msg.as_bytes())
        .await
        .map_err(|e| format!("write error: {e}"))?;
    port.get_mut()
        .flush()
        .await
        .map_err(|e| format!("write error: {e}"))?;

    let mut buf = String::new();
    match tokio::time::timeout(REPLY_TIMEOUT, port.read_line(&mut buf)).await {
        Ok(Ok(0)) => Err("device closed".to_string()),
        Ok(Ok(_)) => parse_reply(&buf),
        Ok(Err(e)) => Err(format!("read error: {e}")),
        Err(_) => Err(format!(
            "timed out waiting for a reply to {line:?} — is the injector powered \
             (target on)?"
        )),
    }
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
    fn parse_reply_ok_and_err() {
        assert_eq!(parse_reply("OK\n").unwrap(), "");
        assert_eq!(
            parse_reply("OK 1 impl moveabs\r\n").unwrap(),
            "1 impl moveabs"
        );
        assert!(parse_reply("ERR no such key").is_err());
        assert!(parse_reply("garbage").is_err());
    }

    #[test]
    fn io_failure_classification() {
        assert!(is_io_failure("write error: x"));
        assert!(is_io_failure("device closed"));
        assert!(!is_io_failure("ERR unknown command"));
        assert!(!is_io_failure("timed out waiting"));
    }
}
