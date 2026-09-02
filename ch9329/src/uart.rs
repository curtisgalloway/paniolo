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
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{info, warn};

use crate::proto::execute_line;
use crate::session::Session;

const REQ_CAP: usize = 256;
const TRANSCRIPT_CAP: usize = 256;
/// Ceiling on one client's wait for its reply. The owner thread services one
/// request at a time, so a request it never answers would otherwise hang
/// every later client queued behind it (and their WebSocket loops with them).
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// One item on the owner's queue.
enum Request {
    /// A command line, answered on `reply` with the `OK` data or the
    /// `ERR`/transport message.
    Line {
        line: String,
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Shutdown: release every held key, modifier and button, so the target
    /// is not left with one down after the daemon — the only thing that
    /// remembers what it pressed — exits. Answered on `done` once the report
    /// is written or there was no open link to write it to. Never touches the
    /// USB mux.
    Release { done: oneshot::Sender<()> },
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
    /// `ERR`/transport message). The line must not contain a newline. The
    /// wait is bounded by [`SEND_TIMEOUT`].
    pub async fn send(&self, line: String) -> Result<String, String> {
        self.send_within(line, SEND_TIMEOUT).await
    }

    /// [`send`](Self::send) with an explicit bound on the wait.
    async fn send_within(&self, line: String, limit: Duration) -> Result<String, String> {
        if line.contains('\n') || line.contains('\r') {
            return Err(format!("command contains a newline: {line:?}"));
        }
        let (tx, rx) = oneshot::channel();
        let round_trip = async {
            self.req_tx
                .send(Request::Line { line, reply: tx })
                .await
                .map_err(|_| "hid daemon stopped".to_string())?;
            rx.await
                .map_err(|_| "hid daemon dropped the request".to_string())?
        };
        match tokio::time::timeout(limit, round_trip).await {
            Ok(result) => result,
            Err(_) => Err(format!(
                "hid daemon did not answer within {} s",
                limit.as_secs()
            )),
        }
    }

    /// Shutdown hook: release every held key, modifier and button. Bounded by
    /// `limit`; the error is informational — the daemon exits either way.
    pub async fn release_for_shutdown(&self, limit: Duration) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        let round_trip = async {
            self.req_tx
                .send(Request::Release { done: tx })
                .await
                .map_err(|_| "hid control link owner is gone".to_string())?;
            rx.await
                .map_err(|_| "hid control link owner dropped the release".to_string())
        };
        match tokio::time::timeout(limit, round_trip).await {
            Ok(result) => result,
            Err(_) => Err(format!(
                "hid control link did not release keys within {} ms",
                limit.as_millis()
            )),
        }
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

/// True for the specific "no reply within the timeout" shape, the subset of
/// [`is_transport_error`] worth one retry before it is believed.
fn is_timeout(msg: &str) -> bool {
    msg.starts_with("timed out")
}

/// Try `attempt`, and if it fails with what looks like a timeout, try it once
/// more before accepting the failure. A slow target or a momentarily busy
/// host can lose a single round trip without the link itself being gone —
/// reopening for that is worse than one retry: each reopen briefly toggles
/// DTR/RTS, which on a KVM-Go is a hardware reset of its MCU.
fn retry_once_on_timeout<F: FnMut() -> Result<String, String>>(
    mut attempt: F,
) -> Result<String, String> {
    let first = attempt();
    match &first {
        Err(msg) if is_timeout(msg) => attempt(),
        _ => first,
    }
}

/// The owner loop (blocking thread): drain requests, execute each against the
/// one persistent [`Session`], broadcast the outcome.
fn run(device: String, mut req_rx: mpsc::Receiver<Request>, transcript: broadcast::Sender<Event>) {
    let mut session: Option<Session> = None;
    // The rate the last session ran at. After a transport error (adapter
    // replug, target power cycle) the chip is almost always still there, so a
    // reopen probes it first instead of paying the default candidates' failed
    // probes — and a `baud` command that moved the chip is remembered too.
    let mut last_baud: Option<u32> = None;
    info!("ch9329 UART owner started for {device}");

    while let Some(req) = req_rx.blocking_recv() {
        let (line, reply) = match req {
            Request::Line { line, reply } => (line, reply),
            Request::Release { done } => {
                if let Some(s) = session.as_mut() {
                    if let Err(e) = s.release_everything() {
                        warn!("ch9329 shutdown release failed: {e}");
                    }
                }
                let _ = done.send(());
                continue;
            }
        };

        if session.is_none() {
            match Session::open_preferring(&device, last_baud) {
                Ok(s) => {
                    info!("ch9329 UART open at {} baud for {device}", s.baud());
                    session = Some(s);
                }
                Err(e) => {
                    let msg = e.to_string();
                    broadcast_event(&transcript, &line, &Err(msg.clone()));
                    let _ = reply.send(Err(msg));
                    continue;
                }
            }
        }

        let s = session.as_mut().unwrap();
        let result = retry_once_on_timeout(|| execute_line(s, &line).map_err(|e| e.to_string()));
        // Recorded after every command: a `baud` command moves the chip, and
        // the next reopen must probe where it went.
        last_baud = Some(s.baud());
        if let Err(ref msg) = result {
            if is_transport_error(msg) {
                warn!("ch9329 UART transport error, will reopen: {msg}");
                session = None;
            }
        }
        broadcast_event(&transcript, &line, &result);
        let _ = reply.send(result);
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

    /// A request the owner never answers must come back as an error, not park
    /// the caller — and every client behind it — forever.
    #[tokio::test]
    async fn send_gives_up_when_the_owner_never_answers() {
        let (req_tx, _req_rx) = mpsc::channel(REQ_CAP);
        let (transcript, _) = broadcast::channel(TRANSCRIPT_CAP);
        let hid = HidHandle {
            req_tx,
            transcript,
            device: "none".into(),
        };
        let err = hid
            .send_within("ping".into(), Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(err.contains("did not answer"), "{err}");
    }

    #[test]
    fn transport_error_classification() {
        assert!(is_transport_error("cannot open /dev/x: busy"));
        assert!(is_transport_error("serial write failed: x"));
        assert!(is_transport_error("timed out waiting for CH9329 reply"));
        assert!(is_transport_error(
            "CH9329 did not respond on /dev/x at 115200/57600/9600 baud"
        ));
        assert!(!is_transport_error(
            "CH9329 rejected cmd 0x02: bad parameter (0xe5)"
        ));
        assert!(!is_transport_error("unknown command: foo"));
    }

    /// A lost reply gets one retry before the caller sees a failure at all.
    #[test]
    fn a_timeout_gets_one_retry_then_the_result_stands() {
        let mut calls = 0;
        let result = retry_once_on_timeout(|| {
            calls += 1;
            if calls == 1 {
                Err("timed out waiting for CH9329 reply".to_string())
            } else {
                Ok("ok".to_string())
            }
        });
        assert_eq!(result, Ok("ok".to_string()));
        assert_eq!(calls, 2);
    }

    /// A second consecutive timeout is accepted as failure, not retried again
    /// — one retry, not an unbounded loop.
    #[test]
    fn a_second_timeout_is_not_retried_again() {
        let mut calls = 0;
        let result = retry_once_on_timeout(|| {
            calls += 1;
            Err("timed out waiting for CH9329 reply".to_string())
        });
        assert_eq!(calls, 2);
        assert!(result.is_err());
    }

    /// A non-timeout failure (a genuine NAK, say) is not retried at all —
    /// retrying would just repeat a board-level rejection.
    #[test]
    fn a_non_timeout_error_is_not_retried() {
        let mut calls = 0;
        let result = retry_once_on_timeout(|| {
            calls += 1;
            Err("CH9329 rejected cmd 0x02: bad parameter (0xe5)".to_string())
        });
        assert_eq!(calls, 1);
        assert!(result.is_err());
    }

    #[test]
    fn timeout_classification() {
        assert!(is_timeout("timed out waiting for CH9329 reply"));
        assert!(!is_timeout(
            "CH9329 rejected cmd 0x02: bad parameter (0xe5)"
        ));
        assert!(!is_timeout("cannot open /dev/x: busy"));
    }

    /// The shutdown release is answered even when the board is absent (the
    /// port cannot open): "nothing held on a closed link", not a request left
    /// queued behind the reopen loop until the daemon's grace runs out.
    #[tokio::test]
    async fn release_for_shutdown_is_answered_without_a_board() {
        let hid = HidHandle::spawn("/nonexistent/ch9329-release-test".into());
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            hid.release_for_shutdown(Duration::from_secs(3)),
        )
        .await
        .expect("bounded by its own limit");
        assert_eq!(result, Ok(()));
    }
}
