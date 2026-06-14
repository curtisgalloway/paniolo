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

//! The control-link owner: a single dedicated thread that owns the control
//! board's CDC link and multiplexes three streams over it — HID/control commands
//! (from the CLI and the web console), their inbound replies, and the DUT serial
//! console (`0x03` frames bridged to/from a PTY).
//!
//! It uses the blocking `serialport` path (async tty reads are unreliable on a
//! macOS tty), so the multiplexing is a **poll loop**, not a select: each
//! iteration drains queued commands, pumps PTY input downstream, then reads
//! inbound bytes (a short read timeout paces the idle loop) and demuxes complete
//! frames — `0x02` replies fulfil the in-flight control request, `0x03` payloads
//! go to the PTY master. HID frames are fire-and-forget; only control frames
//! (ping/version/power) await a reply, tracked by deadline across iterations
//! because the reply now arrives interleaved with console output rather than via
//! a dedicated blocking read.
//!
//! The port is opened/reopened lazily, so the daemon starts and recovers across
//! adapter replug and target power cycles without a restart — and the console
//! PTY (handed in at spawn) keeps its stable identity across those reopens.

use std::fs::File;
use std::io::{Read, Write};
use std::thread;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, info, warn};

use crate::compose::{Composer, F_CTRL};
use crate::proto::open_port;

const REQ_CAP: usize = 256;
const TRANSCRIPT_CAP: usize = 256;
/// Idle pacing: the serial read blocks at most this long, bounding the latency
/// added to a queued command (and the console round trip) while keeping idle CPU
/// low. Data still returns as soon as it arrives.
const POLL_TIMEOUT: Duration = Duration::from_millis(5);
/// How long a control frame (ping/version/power) waits for its reply.
const REPLY_TIMEOUT: Duration = Duration::from_millis(1_500);
/// Backoff between port-open attempts while the board is absent (DUT off).
const REOPEN_BACKOFF: Duration = Duration::from_millis(200);
/// Console-frame selector byte for `0x03` (one DUT UART today).
const CONSOLE_PORT: u8 = 0;
/// Console frame type tag.
const F_CONSOLE: u8 = 0x03;

/// One queued command awaiting its reply.
struct Request {
    line: String,
    reply: oneshot::Sender<Result<String, String>>,
}

/// An in-flight control command awaiting its `0x02` reply.
struct Pending {
    line: String,
    reply: oneshot::Sender<Result<String, String>>,
    deadline: Instant,
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

/// Cloneable handle to the control-link owner thread.
#[derive(Clone)]
pub struct HidHandle {
    req_tx: mpsc::Sender<Request>,
    transcript: broadcast::Sender<Event>,
    pub device: String,
}

impl HidHandle {
    /// Spawn the owner thread for `device` and return a handle. `console` is the
    /// PTY master for the DUT serial-console bridge (`None` disables it). The
    /// port itself is opened lazily, so the daemon starts even with the target —
    /// and therefore the board — currently powered off.
    pub fn spawn(device: String, console: Option<File>) -> HidHandle {
        let (req_tx, req_rx) = mpsc::channel(REQ_CAP);
        let (transcript, _) = broadcast::channel(TRANSCRIPT_CAP);
        let handle = HidHandle {
            req_tx,
            transcript: transcript.clone(),
            device: device.clone(),
        };
        thread::spawn(move || run(device, req_rx, transcript, console));
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

/// Outcome of servicing one queued command.
enum ServiceOutcome {
    /// Fire-and-forget (HID) done; keep draining the queue.
    Continue,
    /// A control frame was sent; stop draining and wait for its reply.
    AwaitReply,
    /// The write failed; the port must be reopened.
    Transport(String),
}

/// The owner loop (blocking thread). See the module docs for the multiplexing
/// model.
fn run(
    device: String,
    mut req_rx: mpsc::Receiver<Request>,
    transcript: broadcast::Sender<Event>,
    mut console: Option<File>,
) {
    let mut composer = Composer::new();
    let mut inbuf: Vec<u8> = Vec::new();
    let mut pending: Option<Pending> = None;
    let mut port: Option<Box<dyn serialport::SerialPort>> = None;
    info!("hid control link owner started for {device}");

    loop {
        // (Re)open the port. Until it opens we can relay nothing: fail any
        // queued commands and back off so we don't busy-spin while the board is
        // unpowered (DUT off). The console PTY persists across reopens.
        let mut p = match port.take() {
            Some(p) => p,
            None => match open_port(&device) {
                Ok(mut p) => {
                    let _ = p.set_timeout(POLL_TIMEOUT);
                    inbuf.clear();
                    info!("hid control link open for {device}");
                    p
                }
                Err(e) => {
                    let msg = e.to_string();
                    if let Some(pd) = pending.take() {
                        finish(&transcript, &pd.line, Err(msg.clone()), pd.reply);
                    }
                    if !drain_failing(&mut req_rx, &transcript, &msg) {
                        break; // channel closed: daemon shutting down
                    }
                    thread::sleep(REOPEN_BACKOFF);
                    continue;
                }
            },
        };
        let mut lost = false;

        // 1. Drain queued commands. HID frames are fire-and-forget; a control
        //    frame sets `pending` and pauses draining until its reply or timeout.
        if pending.is_none() {
            loop {
                match req_rx.try_recv() {
                    Ok(req) => {
                        match service_request(&mut composer, &mut p, req, &transcript, &mut pending)
                        {
                            ServiceOutcome::Continue => continue,
                            ServiceOutcome::AwaitReply => break,
                            ServiceOutcome::Transport(msg) => {
                                warn!("hid control link write error, will reopen: {msg}");
                                lost = true;
                                break;
                            }
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        info!("hid control link owner stopped for {device}");
                        return;
                    }
                }
            }
        }

        // 2. Pump PTY input (host typing toward the DUT) -> 0x03 -> port.
        if !lost {
            if let Some(c) = console.as_mut() {
                if let Err(msg) = pump_console(c, &mut p) {
                    warn!("hid control link write error (console), will reopen: {msg}");
                    lost = true;
                }
            }
        }

        // 3. Read inbound bytes (paced by POLL_TIMEOUT) and demux frames.
        if !lost {
            match read_inbound(&mut p, &mut inbuf) {
                Ok(()) => demux(&mut inbuf, &mut pending, &transcript, &mut console),
                Err(msg) => {
                    warn!("hid control link transport error, will reopen: {msg}");
                    lost = true;
                }
            }
        }

        // 4. Time out an unanswered control reply.
        if !lost {
            let timed_out = pending
                .as_ref()
                .is_some_and(|pd| Instant::now() >= pd.deadline);
            if timed_out {
                let pd = pending.take().unwrap();
                finish(
                    &transcript,
                    &pd.line,
                    Err("timed out waiting for a control reply".to_string()),
                    pd.reply,
                );
                lost = true; // a missing reply means the link is suspect
            }
        }

        if lost {
            // Drop the port (don't restore it) so we reopen next iteration, and
            // fail any still-pending reply so its caller isn't left hanging.
            if let Some(pd) = pending.take() {
                finish(
                    &transcript,
                    &pd.line,
                    Err("hid control link reset".to_string()),
                    pd.reply,
                );
            }
        } else {
            port = Some(p);
        }
    }
}

/// Compose `req.line`, write its frames, and either reply immediately
/// (fire-and-forget HID) or arm `pending` for its control reply.
fn service_request(
    composer: &mut Composer,
    port: &mut Box<dyn serialport::SerialPort>,
    req: Request,
    transcript: &broadcast::Sender<Event>,
    pending: &mut Option<Pending>,
) -> ServiceOutcome {
    let frames = match composer.dispatch(&req.line) {
        Ok(f) => f,
        Err(e) => {
            // A composition error (unknown command) is not a transport failure.
            finish(transcript, &req.line, Err(e.to_string()), req.reply);
            return ServiceOutcome::Continue;
        }
    };
    let wants_reply = frames.iter().any(|f| f.first() == Some(&F_CTRL));
    for f in &frames {
        if let Err(e) = port.write_all(f) {
            let msg = format!("write error: {e}");
            finish(transcript, &req.line, Err(msg.clone()), req.reply);
            return ServiceOutcome::Transport(msg);
        }
    }
    if let Err(e) = port.flush() {
        let msg = format!("write error: {e}");
        finish(transcript, &req.line, Err(msg.clone()), req.reply);
        return ServiceOutcome::Transport(msg);
    }
    if wants_reply {
        *pending = Some(Pending {
            line: req.line,
            reply: req.reply,
            deadline: Instant::now() + REPLY_TIMEOUT,
        });
        ServiceOutcome::AwaitReply
    } else {
        finish(transcript, &req.line, Ok(String::new()), req.reply);
        ServiceOutcome::Continue
    }
}

/// Drain DUT-bound bytes from the PTY master and frame them to the port as
/// `0x03`. PTY-side errors (no slave reader yet, slave closed) are not serial
/// transport failures, so they stop this round without dropping the port; only a
/// serial write error is propagated.
fn pump_console(
    console: &mut File,
    port: &mut Box<dyn serialport::SerialPort>,
) -> Result<(), String> {
    let mut buf = [0u8; 1024];
    let mut wrote = false;
    loop {
        match console.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for chunk in buf[..n].chunks(255) {
                    let mut frame = Vec::with_capacity(3 + chunk.len());
                    frame.push(F_CONSOLE);
                    frame.push(CONSOLE_PORT);
                    frame.push(chunk.len() as u8);
                    frame.extend_from_slice(chunk);
                    port.write_all(&frame)
                        .map_err(|e| format!("write error: {e}"))?;
                    wrote = true;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                debug!("console pty read: {e}");
                break;
            }
        }
    }
    if wrote {
        port.flush().map_err(|e| format!("write error: {e}"))?;
    }
    Ok(())
}

/// Read whatever inbound bytes are available within the read timeout.
fn read_inbound(
    port: &mut Box<dyn serialport::SerialPort>,
    inbuf: &mut Vec<u8>,
) -> Result<(), String> {
    let mut buf = [0u8; 1024];
    match port.read(&mut buf) {
        Ok(0) => Ok(()),
        Ok(n) => {
            inbuf.extend_from_slice(&buf[..n]);
            Ok(())
        }
        Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(()),
        Err(e) => Err(format!("read error: {e}")),
    }
}

/// Route complete frames out of `inbuf`: `0x02` replies fulfil the in-flight
/// control request; `0x03` payloads go to the console PTY master.
fn demux(
    inbuf: &mut Vec<u8>,
    pending: &mut Option<Pending>,
    transcript: &broadcast::Sender<Event>,
    console: &mut Option<File>,
) {
    let (frames, consumed) = split_frames(inbuf);
    for (ftype, payload) in frames {
        if ftype == F_CTRL {
            if let Some(pd) = pending.take() {
                let text = String::from_utf8_lossy(&payload).into_owned();
                finish(transcript, &pd.line, Ok(text), pd.reply);
            }
            // else: a stray/late reply with nothing waiting — drop it.
        } else if let Some(c) = console.as_mut() {
            // Best-effort: a full PTY buffer (no reader) drops console output.
            if let Err(e) = c.write_all(&payload) {
                if e.kind() != std::io::ErrorKind::WouldBlock {
                    debug!("console pty write: {e}");
                }
            }
        }
    }
    if consumed > 0 {
        inbuf.drain(..consumed);
    }
}

/// Parse complete `[type][b1][len][payload]` frames from `buf`. Returns each
/// frame's type byte and payload, and how many bytes were consumed; a leading
/// unframed byte (type not `0x02`/`0x03`) is skipped to resync. The trailing
/// partial frame, if any, is left in `buf`.
fn split_frames(buf: &[u8]) -> (Vec<(u8, Vec<u8>)>, usize) {
    let mut out = Vec::new();
    let mut i = 0;
    let n = buf.len();
    while n - i >= 1 {
        let ftype = buf[i];
        if ftype == F_CTRL || ftype == F_CONSOLE {
            if n - i < 3 {
                break; // header incomplete
            }
            let need = 3 + buf[i + 2] as usize;
            if n - i < need {
                break; // payload incomplete
            }
            out.push((ftype, buf[i + 3..i + need].to_vec()));
            i += need;
        } else {
            i += 1; // unframed/unknown byte (HID isn't echoed upstream) — resync
        }
    }
    (out, i)
}

/// Fail every currently-queued request with `msg`. Returns false if the channel
/// has closed (daemon shutting down).
fn drain_failing(
    req_rx: &mut mpsc::Receiver<Request>,
    transcript: &broadcast::Sender<Event>,
    msg: &str,
) -> bool {
    loop {
        match req_rx.try_recv() {
            Ok(req) => finish(transcript, &req.line, Err(msg.to_string()), req.reply),
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
}

/// Broadcast the outcome to observers and answer the request's oneshot.
fn finish(
    transcript: &broadcast::Sender<Event>,
    line: &str,
    result: Result<String, String>,
    reply: oneshot::Sender<Result<String, String>>,
) {
    broadcast_event(transcript, line, &result);
    let _ = reply.send(result);
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
    fn split_frames_routes_control_and_console() {
        // 0x02 reply (cmd 2, payload "hi"), then a 0x03 console frame ("ab").
        let buf = [0x02, 0x02, 0x02, b'h', b'i', 0x03, 0x00, 0x02, b'a', b'b'];
        let (frames, consumed) = split_frames(&buf);
        assert_eq!(
            frames,
            vec![(F_CTRL, b"hi".to_vec()), (F_CONSOLE, b"ab".to_vec())]
        );
        assert_eq!(consumed, buf.len());
    }

    #[test]
    fn split_frames_leaves_trailing_partial() {
        // A complete 0x03 frame followed by a partial header.
        let buf = [0x03, 0x00, 0x01, b'x', 0x03, 0x00];
        let (frames, consumed) = split_frames(&buf);
        assert_eq!(frames, vec![(F_CONSOLE, b"x".to_vec())]);
        assert_eq!(consumed, 4); // the trailing [0x03, 0x00] is kept
    }

    #[test]
    fn split_frames_resyncs_past_unframed_byte() {
        let buf = [0x99, 0x03, 0x00, 0x01, b'x'];
        let (frames, consumed) = split_frames(&buf);
        assert_eq!(frames, vec![(F_CONSOLE, b"x".to_vec())]);
        assert_eq!(consumed, 5);
    }
}
