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

//! Owns the serial port. A supervisor task opens the port, reconnects on
//! loss/hot-unplug, fans received bytes out to all WebSocket clients via a
//! `broadcast` channel, and drains a single `mpsc` of client input back to the
//! port. A small ring buffer keeps recent output so a client that connects
//! mid-stream sees scrollback (e.g. boot log already in progress).
//!
//! Every received chunk is also tee'd to a dedicated OS thread that owns the
//! [`capture::LineLog`], which assembles timestamped lines and persists them to
//! disk. Keeping that work (and its file I/O) off the supervisor's select loop
//! means a slow disk can never stall the live WebSocket fan-out.

use std::collections::VecDeque;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc as stdmpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use serialport::SerialPort;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_serial::SerialPortBuilderExt;
use tracing::{info, warn};

use crate::capture::LineLog;

const RING_BYTES: usize = 64 * 1024;
const BROADCAST_CAP: usize = 1024;
const WRITE_CAP: usize = 256;
const REOPEN_DELAY: Duration = Duration::from_millis(750);
/// Bytes handed to the port per pass of the supervisor loop (see [`Outbox`]).
const WRITE_CHUNK: usize = 256;
/// Bytes queued in the supervisor before it stops draining `write_tx`, so a
/// flood of input backs up to the HTTP/WebSocket senders (which wait on the
/// channel) instead of growing the queue without bound.
const OUTBOX_HIGH_WATER: usize = 256 * 1024;
/// After the first "open failed" warning, repeat it at most this often.
const OPEN_FAIL_LOG_EVERY: Duration = Duration::from_secs(60);
/// Symlink hops followed when deciding whether a device path is a pty.
const MAX_LINK_HOPS: usize = 16;

#[derive(Clone)]
pub struct Status {
    pub device: String,
    pub baud: u32,
    pub connected: bool,
    /// Current power state as read from the configured modem-control sense line.
    /// `None` when no sense signal is configured for this interface.
    pub power_on: Option<bool>,
}

/// One serial interface the daemon owns: a stable `name` (used by the CLI, the
/// dashboard selector, and the capture sub-directory) bound to a `device`/`baud`.
#[derive(Clone)]
pub struct InterfaceSpec {
    pub name: String,
    pub device: String,
    pub baud: u32,
    /// Optional modem-control input pin wired to the target's 3.3 V rail so the
    /// host can detect whether the target is powered on.  Values: "cts", "dsr",
    /// "dcd", "ri".  None when not wired up.
    pub power_sense_signal: Option<String>,
}

/// One running interface: its name paired with the live handle.
#[derive(Clone)]
pub struct NamedSerial {
    pub name: String,
    pub handle: SerialHandle,
}

/// All interfaces the daemon is serving, in the order they were configured. The
/// first is the default for endpoints/commands that don't name one. Cheap to
/// clone (everything inside is channels or `Arc`s).
#[derive(Clone)]
pub struct Serials {
    inner: Arc<Vec<NamedSerial>>,
}

impl Serials {
    /// Spawn a supervisor + capture thread per interface and collect the handles.
    /// Each interface captures into `<capture_base>/<name>/`.
    pub fn spawn_all(specs: &[InterfaceSpec], capture_base: &Path, buffer_lines: u64) -> Self {
        let inner = specs
            .iter()
            .map(|spec| {
                let dir = crate::capture::interface_dir(capture_base, &spec.name);
                let handle = spawn_interface(spec.clone(), dir, buffer_lines);
                NamedSerial {
                    name: spec.name.clone(),
                    handle,
                }
            })
            .collect();
        Serials {
            inner: Arc::new(inner),
        }
    }

    /// Look up an interface by name.
    pub fn get(&self, name: &str) -> Option<&SerialHandle> {
        self.inner
            .iter()
            .find(|ns| ns.name == name)
            .map(|ns| &ns.handle)
    }

    /// The default interface (the first configured), if any.
    pub fn default(&self) -> Option<&NamedSerial> {
        self.inner.first()
    }

    /// All interfaces, in configuration order.
    pub fn all(&self) -> &[NamedSerial] {
        &self.inner
    }
}

/// Handle shared with the HTTP layer. Cheap to clone (all fields are channels
/// or `Arc`s).
#[derive(Clone)]
pub struct SerialHandle {
    /// Serial bytes flowing out to every connected client.
    to_clients: broadcast::Sender<Bytes>,
    /// Client keystrokes flowing back to the port.
    pub write_tx: mpsc::Sender<Bytes>,
    ring: Arc<Mutex<VecDeque<u8>>>,
    status: Arc<Mutex<Status>>,
    /// Button-press requests: caller sends (duration_ms, responder); the
    /// supervisor asserts DTR for that many milliseconds then replies.
    dtr_tx: mpsc::Sender<(u64, oneshot::Sender<()>)>,
}

impl SerialHandle {
    /// The scrollback snapshot and a live subscription for a new client, taken
    /// together under the ring lock so the two tile exactly: every chunk the
    /// supervisor publishes lands either in the snapshot or on the
    /// subscription — never both, never neither. (`publish` appends to the
    /// ring and broadcasts under the same lock.) Taking them separately left a
    /// window in which a chunk was delivered twice, or not at all.
    pub fn attach(&self) -> (Vec<u8>, broadcast::Receiver<Bytes>) {
        let ring = self.ring.lock().unwrap();
        let rx = self.to_clients.subscribe();
        (ring.iter().copied().collect(), rx)
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// Assert DTR for `duration_ms` milliseconds then release.
    ///
    /// Models pressing the J2 power button on a Raspberry Pi (or equivalent).
    /// The caller decides what the press means for the target hardware:
    /// - short press (≤500 ms): OS receives power-button event → graceful reboot/halt
    /// - long press (≥3 s): PMIC hard power-off
    ///
    /// Blocks until the press completes. Concurrent calls queue and execute serially.
    pub async fn dtr_press(&self, duration_ms: u64) -> anyhow::Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.dtr_tx
            .send((duration_ms, resp_tx))
            .await
            .map_err(|_| anyhow::anyhow!("supervisor not running"))?;
        resp_rx
            .await
            .map_err(|_| anyhow::anyhow!("supervisor dropped response"))
    }

    /// Write `data` to the port through the supervisor's normal write path.
    ///
    /// When `pace` is non-zero, the bytes are dripped one at a time with `pace`
    /// between each, throttling input for a slow polled console that has no
    /// hardware flow control (each byte is consumed before the next arrives, so
    /// the receiver's RX FIFO can't overflow). When `pace` is zero the whole
    /// buffer is sent in one message (full line-rate, same as interactive input).
    ///
    /// The supervisor's select loop is unchanged: it just sees one or many write
    /// messages. The interactive WebSocket path shares `write_tx` but never paces,
    /// so live typing stays immediate.
    pub async fn write_paced(&self, data: Bytes, pace: Duration) -> anyhow::Result<()> {
        let dead = |_| anyhow::anyhow!("supervisor not running");
        if pace.is_zero() {
            self.write_tx.send(data).await.map_err(dead)?;
            return Ok(());
        }
        for i in 0..data.len() {
            self.write_tx
                .send(data.slice(i..i + 1))
                .await
                .map_err(dead)?;
            tokio::time::sleep(pace).await;
        }
        Ok(())
    }
}

/// Spawn the supervisor for one interface on the current tokio runtime and return
/// its handle. `capture_dir` / `buffer_lines` configure that interface's on-disk
/// line log; its capture thread is started here and fed via a non-blocking channel.
pub fn spawn_interface(
    spec: InterfaceSpec,
    capture_dir: PathBuf,
    buffer_lines: u64,
) -> SerialHandle {
    let (to_clients, _) = broadcast::channel(BROADCAST_CAP);
    let (write_tx, write_rx) = mpsc::channel(WRITE_CAP);
    let (dtr_tx, dtr_rx) = mpsc::channel::<(u64, oneshot::Sender<()>)>(1);
    let ring = Arc::new(Mutex::new(VecDeque::with_capacity(RING_BYTES)));
    let status = Arc::new(Mutex::new(Status {
        device: spec.device.clone(),
        baud: spec.baud,
        connected: false,
        power_on: None,
    }));

    let line_tx = spawn_capture(capture_dir, buffer_lines);

    tokio::spawn(supervisor(
        spec,
        to_clients.clone(),
        write_rx,
        dtr_rx,
        ring.clone(),
        status.clone(),
        line_tx,
    ));

    SerialHandle {
        to_clients,
        write_tx,
        ring,
        status,
        dtr_tx,
    }
}

/// Start the OS thread that owns the line log and return a non-blocking sender
/// for raw byte chunks. An unbounded channel means the supervisor never blocks
/// on capture; serial throughput is tiny, so the queue can't grow meaningfully.
///
/// The log defers rewrites of its pending-line sidecar (see
/// `LineLog::sidecar_due`); when one is outstanding the thread waits with a
/// deadline so an idle port still gets its last partial line mirrored.
fn spawn_capture(capture_dir: PathBuf, buffer_lines: u64) -> stdmpsc::Sender<Bytes> {
    let (tx, rx) = stdmpsc::channel::<Bytes>();
    std::thread::Builder::new()
        .name("serialcap-capture".into())
        .spawn(move || {
            use stdmpsc::RecvTimeoutError;
            let mut log = LineLog::open(capture_dir, buffer_lines);
            loop {
                let next = match log.sidecar_due(Instant::now()) {
                    None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                    Some(wait) => rx.recv_timeout(wait),
                };
                match next {
                    Ok(chunk) => log.ingest(&chunk),
                    Err(RecvTimeoutError::Timeout) => log.flush_sidecar(),
                    // Every sender has dropped: the daemon is shutting down.
                    Err(RecvTimeoutError::Disconnected) => {
                        log.flush_sidecar();
                        break;
                    }
                }
            }
        })
        .expect("spawn capture thread");
    tx
}

/// `device` with symlinks followed, so a link to a pty (the hidrig console
/// bridge publishes one) is judged by where it points rather than by its own
/// name. Resolved link by link with `read_link` rather than `canonicalize`,
/// which needs the target to exist — this is decided before the open, and a
/// VM's console may not be there yet. `..` and `.` components are folded
/// lexically so a relative link target still yields a `/dev/...` path.
fn resolve_links(device: &str) -> String {
    let mut path = PathBuf::from(device);
    for _ in 0..MAX_LINK_HOPS {
        let Ok(target) = std::fs::read_link(&path) else {
            break;
        };
        path = if target.is_absolute() {
            target
        } else {
            let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
            base.join(target)
        };
    }
    let mut folded = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                folded.pop();
            }
            Component::CurDir => {}
            other => folded.push(other),
        }
    }
    folded.to_string_lossy().into_owned()
}

/// True when `device` names (or links to) a pseudo-terminal slave rather than
/// a real serial port: `/dev/ttysNNN` on macOS, `/dev/pts/N` on Linux. That
/// is what `utmctl attach` prints for a UTM guest's console and what qemu
/// reports for `-serial pty`, so it is the shape a VM's console arrives in.
///
/// A path test rather than an fd test on purpose — the decision has to be made
/// *before* the open, because on macOS it changes how the open is performed.
fn is_pty(device: &str) -> bool {
    is_pty_path(&resolve_links(device))
}

/// The pure prefix test behind [`is_pty`], on an already-resolved path.
fn is_pty_path(device: &str) -> bool {
    let numbered = |rest: &str| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit());
    device.strip_prefix("/dev/ttys").is_some_and(numbered)
        || device.strip_prefix("/dev/pts/").is_some_and(numbered)
}

/// The baud rate to *open* `device` with, which is not always the configured one.
///
/// macOS applies a serial port's line rate with the `IOSSIOSPEED` ioctl, which
/// pseudo-terminals do not implement: the open fails outright with `ENOTTY`
/// ("Not a typewriter"), so before this paniolo could not attach to a VM
/// console at all. The `serialport` crate skips that ioctl when the requested
/// rate is 0, which is its documented way to open a pty — and a pty has no line
/// rate to set in the first place, so nothing is lost by not setting one.
///
/// Linux needs no such thing (its ptys accept the ordinary termios path) and
/// must not be given it: there a rate of 0 is written into `c_ospeed` as `B0`,
/// which means *hang up the line*.
fn open_baud(device: &str, baud: u32) -> u32 {
    if cfg!(target_os = "macos") && is_pty(device) {
        0
    } else {
        baud
    }
}

/// Read one modem-control sense signal and translate it to `power_on`.
///
/// The target's 3.3 V rail is wired (with a pull-down) to the chosen FTDI input
/// pin.  The pin is HIGH when the rail is up (power on) and LOW when off.  FTDI
/// signal sense is active-low in RS-232 convention, so `read_*()` returns
/// `true` when the pin is LOW — meaning powered off.  We invert to get a
/// natural `power_on = true` when the board is running.
fn read_power_sense(port: &mut impl SerialPort, signal: &str) -> Option<bool> {
    match signal {
        "cts" => port.read_clear_to_send().ok().map(|v| !v),
        "dsr" => port.read_data_set_ready().ok().map(|v| !v),
        "dcd" => port.read_carrier_detect().ok().map(|v| !v),
        "ri" => port.read_ring_indicator().ok().map(|v| !v),
        _ => None,
    }
}

/// Bytes waiting to go out to the port, handed over at most [`WRITE_CHUNK`]
/// at a time.
///
/// The supervisor used to `write_all` each input message to completion before
/// it polled the port again: a 64 KiB paste at 115200 baud is ~6 s of line
/// time during which nothing was read — the kernel's tty buffer overflowed and
/// bytes vanished from the stream, the scrollback and the capture, and a
/// `/button` press queued behind it. Queueing input here and writing one
/// bounded slice per pass of the loop keeps the read arm live throughout.
#[derive(Default)]
struct Outbox {
    queue: VecDeque<Bytes>,
    /// Bytes of `queue.front()` already written.
    cursor: usize,
    /// Bytes not yet written, across the whole queue.
    pending: usize,
}

impl Outbox {
    fn push(&mut self, data: Bytes) {
        if data.is_empty() {
            return;
        }
        self.pending += data.len();
        self.queue.push_back(data);
    }

    fn is_empty(&self) -> bool {
        self.pending == 0
    }

    /// Bytes still to be written.
    fn pending(&self) -> usize {
        self.pending
    }

    /// The next slice to write: up to [`WRITE_CHUNK`] bytes, empty when
    /// nothing is waiting.
    fn front(&self) -> &[u8] {
        match self.queue.front() {
            Some(head) => &head[self.cursor..(self.cursor + WRITE_CHUNK).min(head.len())],
            None => &[],
        }
    }

    /// Account for `n` bytes of `front()` having been written. A short write
    /// leaves the cursor mid-message; the rest goes out next pass.
    fn consume(&mut self, n: usize) {
        let Some(head) = self.queue.front() else {
            return;
        };
        let n = n.min(head.len() - self.cursor);
        self.cursor += n;
        self.pending -= n;
        if self.cursor >= head.len() {
            self.queue.pop_front();
            self.cursor = 0;
        }
    }
}

/// Rate limit for the "open failed" warning. A missing device is retried every
/// [`REOPEN_DELAY`], and warning on every attempt wrote 10-15 MB a day to
/// daemon.log for one unplugged adapter. Log the first failure, then at most
/// one a minute, then once on recovery.
struct OpenFailures {
    attempts: u64,
    last_logged: Option<Instant>,
}

impl OpenFailures {
    fn new() -> Self {
        OpenFailures {
            attempts: 0,
            last_logged: None,
        }
    }

    /// Record a failed attempt; `Some(attempts so far)` when this one should
    /// be logged.
    fn failed(&mut self, now: Instant) -> Option<u64> {
        self.attempts += 1;
        let due = match self.last_logged {
            None => true,
            Some(last) => now.duration_since(last) >= OPEN_FAIL_LOG_EVERY,
        };
        if due {
            self.last_logged = Some(now);
            Some(self.attempts)
        } else {
            None
        }
    }

    /// Record a successful open; `Some(failed attempts)` when failures
    /// preceded it, so the recovery can be logged once.
    fn recovered(&mut self) -> Option<u64> {
        self.last_logged = None;
        let n = std::mem::take(&mut self.attempts);
        (n > 0).then_some(n)
    }
}

/// Why the supervisor's read/write loop stopped.
enum InnerExit {
    Disconnect,
    DtrPress {
        duration_ms: u64,
        resp_tx: oneshot::Sender<()>,
    },
}

async fn supervisor(
    spec: InterfaceSpec,
    to_clients: broadcast::Sender<Bytes>,
    mut write_rx: mpsc::Receiver<Bytes>,
    mut dtr_rx: mpsc::Receiver<(u64, oneshot::Sender<()>)>,
    ring: Arc<Mutex<VecDeque<u8>>>,
    status: Arc<Mutex<Status>>,
    line_tx: stdmpsc::Sender<Bytes>,
) {
    let InterfaceSpec {
        device,
        baud,
        power_sense_signal,
        ..
    } = spec;

    // Track whether we've ever connected so the first open shows "connected"
    // and later opens show "reconnected".
    let mut ever_connected = false;
    let mut open_failures = OpenFailures::new();

    // A pty (a VM's console) is both opened and torn down differently from a
    // real port; see `open_baud` and the `Ok(0)` arm of the read loop.
    let pty = is_pty(&device);
    let baud = open_baud(&device, baud);
    if pty {
        info!("{device} is a pty — opening without a line rate");
    }

    loop {
        // DTR may be the target's power button (see `dtr_press`), so the port
        // is opened with it de-asserted rather than left at the OS default:
        // Linux raises DTR on open and drops it on close, which made every
        // open — daemon start, every reconnect — a driver-timed press. Linux
        // still pulses it for the duration of the open itself; that is in the
        // tty core and out of reach from here.
        let port = match tokio_serial::new(&device, baud)
            .dtr_on_open(false)
            .open_native_async()
        {
            Ok(mut p) => {
                match open_failures.recovered() {
                    Some(n) => {
                        info!("serial port opened: {device} @ {baud} (after {n} failed attempts)")
                    }
                    None => info!("serial port opened: {device} @ {baud}"),
                }
                {
                    let mut st = status.lock().unwrap();
                    st.connected = true;
                    if let Some(sig) = &power_sense_signal {
                        st.power_on = read_power_sense(&mut p, sig);
                    }
                }
                if ever_connected {
                    emit_marker(&ring, &to_clients, &line_tx, "reconnected", 32);
                // green
                } else {
                    emit_marker(&ring, &to_clients, &line_tx, "connected", 36); // cyan
                    ever_connected = true;
                }
                p
            }
            Err(e) => {
                match open_failures.failed(Instant::now()) {
                    Some(1) => warn!(
                        "open {device} failed: {e} (retrying every {REOPEN_DELAY:?}; \
                         further failures are logged once a minute)"
                    ),
                    Some(n) => warn!("open {device} still failing after {n} attempts: {e}"),
                    None => {}
                }
                status.lock().unwrap().connected = false;
                tokio::time::sleep(REOPEN_DELAY).await;
                continue;
            }
        };

        let (mut rd, mut wr) = tokio::io::split(port);
        let mut buf = [0u8; 65536];
        let mut outbox = Outbox::default();

        // One open port. A DTR press pauses this loop but does not leave it;
        // only a disconnect does.
        loop {
            let exit = loop {
                tokio::select! {
                    read = rd.read(&mut buf) => match read {
                        Ok(0) if pty => {
                            // A pty *does* have an EOF: `Ok(0)` means the far end
                            // closed — the VM powered off, or its hypervisor let go
                            // of the master. Treat it as the disconnect it is rather
                            // than spinning, so the marker is emitted and the
                            // reconnect loop can pick the console back up.
                            break InnerExit::Disconnect;
                        }
                        Ok(0) => {
                            // Real serial ports don't have EOF; Ok(0) means the async
                            // read resolved without data. Yield to avoid a spin loop.
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }
                        Ok(n) => {
                            publish(&ring, &to_clients, &line_tx, Bytes::copy_from_slice(&buf[..n]));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }
                        Err(e) => { warn!("serial read error: {e}"); break InnerExit::Disconnect; }
                    },
                    // One bounded slice per pass, so the read arm above keeps
                    // its turn while a large paste drains (see `Outbox`).
                    written = wr.write(outbox.front()), if !outbox.is_empty() => match written {
                        Ok(0) => tokio::time::sleep(Duration::from_millis(1)).await,
                        Ok(n) => outbox.consume(n),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }
                        Err(e) => { warn!("serial write error: {e}"); break InnerExit::Disconnect; }
                    },
                    // Past the high-water mark the queue is left in the
                    // channel, where senders wait on it.
                    Some(data) = write_rx.recv(), if outbox.pending() < OUTBOX_HIGH_WATER => {
                        outbox.push(data);
                    },
                    Some((duration_ms, resp_tx)) = dtr_rx.recv() => {
                        break InnerExit::DtrPress { duration_ms, resp_tx };
                    }
                }
            };

            match exit {
                InnerExit::DtrPress {
                    duration_ms,
                    resp_tx,
                } => {
                    // Rejoin the split halves to regain the SerialPort trait methods.
                    let mut port = rd.unsplit(wr);
                    emit_marker(&ring, &to_clients, &line_tx, "button press", 35); // magenta
                    port.write_data_terminal_ready(true).ok();
                    tokio::time::sleep(Duration::from_millis(duration_ms)).await;
                    port.write_data_terminal_ready(false).ok();
                    // Read power state immediately after releasing the button — the
                    // 3.3 V rail may have dropped (long press → power-off).
                    if let Some(sig) = &power_sense_signal {
                        status.lock().unwrap().power_on = read_power_sense(&mut port, sig);
                    }
                    resp_tx.send(()).ok();
                    // Keep the port open. Closing and reopening it here let the
                    // OS drop and re-raise DTR on the way back in — a second,
                    // driver-timed button press after every deliberate one.
                    let (r, w) = tokio::io::split(port);
                    rd = r;
                    wr = w;
                }
                InnerExit::Disconnect => {
                    // We only reach here after a successful open, so this is a real
                    // disconnect (link dropped / device unplugged), not a failed open.
                    status.lock().unwrap().connected = false;
                    emit_marker(&ring, &to_clients, &line_tx, "disconnected", 31); // red
                    tokio::time::sleep(REOPEN_DELAY).await;
                    break;
                }
            }
        }
    }
}

/// A styled, timestamped status line for the stream, so the web terminal
/// shows exactly when the serial link dropped or came back. ANSI color `code`
/// (31 red / 32 green / 33 yellow / 36 cyan); only the WS terminal renders
/// it — `tio` uses a different path.
pub(crate) fn marker_line(label: &str, code: u8) -> Bytes {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let sod = secs % 86_400;
    let line = format!(
        "\r\n\x1b[1;{code}m── serial {label} [{:02}:{:02}:{:02} UTC] ──\x1b[0m\r\n",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60,
    );
    Bytes::from(line.into_bytes())
}

/// Inject a [`marker_line`] into the stream and scrollback.
fn emit_marker(
    ring: &Arc<Mutex<VecDeque<u8>>>,
    to_clients: &broadcast::Sender<Bytes>,
    line_tx: &stdmpsc::Sender<Bytes>,
    label: &str,
    code: u8,
) {
    publish(ring, to_clients, line_tx, marker_line(label, code));
}

/// Fan one chunk out: the capture thread, the scrollback ring and every live
/// client. The ring is appended and the broadcast sent under the ring lock,
/// so a client attaching at the same moment (`SerialHandle::attach`, which
/// snapshots and subscribes under that lock) sees the chunk exactly once.
/// The critical section is a ≤64 KiB memcpy, so the lock is taken outright —
/// the old `try_lock` skipped the ring whenever a client was reading it,
/// leaving holes in the scrollback.
fn publish(
    ring: &Arc<Mutex<VecDeque<u8>>>,
    to_clients: &broadcast::Sender<Bytes>,
    line_tx: &stdmpsc::Sender<Bytes>,
    chunk: Bytes,
) {
    if line_tx.send(chunk.clone()).is_err() {
        warn!("capture thread dead — bytes lost");
    }
    let mut r = ring.lock().unwrap();
    push_ring(&mut r, &chunk);
    // Err just means no subscribers; that's fine.
    let _ = to_clients.send(chunk);
}

/// Append `chunk` to the scrollback ring, evicting the oldest bytes past
/// [`RING_BYTES`].
fn push_ring(ring: &mut VecDeque<u8>, chunk: &[u8]) {
    ring.extend(chunk.iter().copied());
    let overflow = ring.len().saturating_sub(RING_BYTES);
    if overflow > 0 {
        ring.drain(0..overflow);
    }
}

/// Enumerate serial ports on this host.
pub fn list_ports() -> anyhow::Result<Vec<(String, String)>> {
    let ports = tokio_serial::available_ports()?;
    Ok(ports
        .into_iter()
        .map(|p| (p.port_name, describe(&p.port_type)))
        .collect())
}

fn describe(t: &tokio_serial::SerialPortType) -> String {
    use tokio_serial::SerialPortType;
    match t {
        SerialPortType::UsbPort(info) => {
            let product = info.product.as_deref().unwrap_or("");
            let manuf = info.manufacturer.as_deref().unwrap_or("");
            format!("USB {:04x}:{:04x} {manuf} {product}", info.vid, info.pid)
                .trim()
                .to_string()
        }
        SerialPortType::PciPort => "PCI".into(),
        SerialPortType::BluetoothPort => "Bluetooth".into(),
        SerialPortType::Unknown => "unknown".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_serial::{SerialPortType, UsbPortInfo};

    fn snapshot(r: &VecDeque<u8>) -> Vec<u8> {
        r.iter().copied().collect()
    }

    // ── push_ring: scrollback ring-buffer truncation ────────────────────────

    #[test]
    fn push_ring_accumulates_in_order() {
        let mut r = VecDeque::new();
        push_ring(&mut r, b"hello ");
        push_ring(&mut r, b"world");
        assert_eq!(snapshot(&r), b"hello world");
    }

    #[test]
    fn push_ring_truncates_to_capacity_keeping_newest() {
        let mut r = VecDeque::new();
        push_ring(&mut r, &vec![b'A'; RING_BYTES]); // fill exactly to capacity
        push_ring(&mut r, b"XYZ"); // 3 bytes over -> 3 oldest dropped
        let snap = snapshot(&r);
        assert_eq!(snap.len(), RING_BYTES, "never grows past RING_BYTES");
        assert_eq!(&snap[snap.len() - 3..], b"XYZ", "newest bytes retained");
        assert!(
            snap[..RING_BYTES - 3].iter().all(|&b| b == b'A'),
            "exactly the 3 oldest bytes were evicted"
        );
    }

    #[test]
    fn push_ring_single_oversized_chunk_keeps_tail() {
        let mut r = VecDeque::new();
        let big: Vec<u8> = (0..(RING_BYTES as u32 + 100)).map(|i| i as u8).collect();
        push_ring(&mut r, &big);
        let snap = snapshot(&r);
        assert_eq!(snap.len(), RING_BYTES);
        assert_eq!(
            snap,
            big[big.len() - RING_BYTES..],
            "keeps the most-recent window"
        );
    }

    // ── publish + attach: a chunk reaches a client exactly once ─────────────

    /// A client attaching while output is flowing must see every chunk once:
    /// in its scrollback snapshot or on its subscription, never both, never
    /// neither. Taking the snapshot and the subscription separately (and
    /// skipping the ring under `try_lock`) left windows for both.
    ///
    /// The producer publishes consecutive numbers as fast as it can while the
    /// test attaches over and over; each attach checks that its snapshot is
    /// gap-free and that the first chunk on its subscription continues the
    /// snapshot's sequence.
    #[test]
    fn attach_tiles_scrollback_and_subscription_exactly() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let ring = Arc::new(Mutex::new(VecDeque::new()));
        let (to_clients, _) = broadcast::channel::<Bytes>(BROADCAST_CAP);
        let (line_tx, line_rx) = stdmpsc::channel::<Bytes>();
        let stop = Arc::new(AtomicBool::new(false));
        let producer = {
            let (ring, to_clients, stop) = (ring.clone(), to_clients.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut n: u32 = 0;
                while !stop.load(Ordering::Relaxed) {
                    publish(
                        &ring,
                        &to_clients,
                        &line_tx,
                        Bytes::copy_from_slice(&n.to_le_bytes()),
                    );
                    n += 1;
                    // Give the sampling thread a turn so it is not perpetually
                    // lagged behind the producer on a loaded machine.
                    std::thread::yield_now();
                }
                n
            })
        };
        // Drain the capture side so its queue does not grow unboundedly.
        let drain = std::thread::spawn(move || for _ in line_rx {});

        let handle = SerialHandle {
            to_clients: to_clients.clone(),
            write_tx: mpsc::channel(1).0,
            ring: ring.clone(),
            status: Arc::new(Mutex::new(Status {
                device: "test".into(),
                baud: 0,
                connected: true,
                power_on: None,
            })),
            dtr_tx: mpsc::channel(1).0,
        };

        // Sample until enough verdicts are in; a lagged sample yields none, and
        // how often that happens depends on machine load, not on correctness.
        let mut checked = 0;
        for _ in 0..50_000 {
            if checked >= 100 {
                break;
            }
            let (snap, mut rx) = handle.attach();
            let mut nums = snap
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes(*c));
            let Some(mut last) = nums.next() else {
                continue;
            };
            for n in nums {
                assert_eq!(n, last + 1, "hole in the scrollback: {last} then {n}");
                last = n;
            }
            // The first live chunk must be the one after the snapshot's last.
            let first = loop {
                match rx.try_recv() {
                    Ok(b) => break Some(u32::from_le_bytes(b[..].try_into().unwrap())),
                    Err(broadcast::error::TryRecvError::Empty) => std::thread::yield_now(),
                    // Fell behind the producer between attach and recv: no
                    // verdict from this sample.
                    Err(_) => break None,
                }
            };
            if let Some(first) = first {
                assert_eq!(
                    first,
                    last + 1,
                    "snapshot ended at {last}, subscription began at {first}"
                );
                checked += 1;
            }
        }
        stop.store(true, Ordering::Relaxed);
        let produced = producer.join().unwrap();
        drop(to_clients);
        drain.join().unwrap();
        assert!(produced > 100, "producer barely ran ({produced} chunks)");
        assert!(
            checked >= 100,
            "too few verdicts to mean anything ({checked})"
        );
    }

    // ── Outbox: bounded writes per loop pass ────────────────────────────────

    #[test]
    fn outbox_hands_out_at_most_one_chunk_per_pass_in_order() {
        let mut ob = Outbox::default();
        let data: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        ob.push(Bytes::from(data.clone()));
        assert_eq!(ob.pending(), 1000);
        let mut written = Vec::new();
        let mut passes = 0;
        while !ob.is_empty() {
            let slice = ob.front();
            assert!(
                slice.len() <= WRITE_CHUNK,
                "a pass wrote {} bytes",
                slice.len()
            );
            assert!(!slice.is_empty());
            written.extend_from_slice(slice);
            let n = slice.len();
            ob.consume(n);
            passes += 1;
        }
        assert_eq!(written, data, "bytes reach the port intact and in order");
        assert_eq!(passes, 1000_usize.div_ceil(WRITE_CHUNK));
        assert!(ob.front().is_empty());
    }

    #[test]
    fn outbox_short_write_resumes_mid_message_and_crosses_messages() {
        let mut ob = Outbox::default();
        ob.push(Bytes::from_static(b"abcdef"));
        ob.push(Bytes::from_static(b"gh"));
        assert_eq!(ob.front(), b"abcdef");
        ob.consume(2); // the port took only two bytes
        assert_eq!(ob.front(), b"cdef");
        assert_eq!(ob.pending(), 6);
        ob.consume(4);
        assert_eq!(ob.front(), b"gh", "moves on to the next message");
        ob.consume(2);
        assert!(ob.is_empty());
        ob.consume(1); // nothing to consume: harmless
        assert!(ob.is_empty());
    }

    #[test]
    fn outbox_ignores_empty_messages() {
        let mut ob = Outbox::default();
        ob.push(Bytes::new());
        assert!(ob.is_empty());
        assert!(ob.front().is_empty());
    }

    // ── OpenFailures: one warning, then one a minute, then the recovery ──────

    #[test]
    fn open_failures_log_first_then_once_a_minute_then_recovery() {
        let t0 = Instant::now();
        let mut f = OpenFailures::new();
        assert_eq!(f.failed(t0), Some(1), "the first failure is logged");
        assert_eq!(f.failed(t0 + Duration::from_millis(750)), None);
        assert_eq!(f.failed(t0 + Duration::from_secs(30)), None);
        assert_eq!(
            f.failed(t0 + OPEN_FAIL_LOG_EVERY),
            Some(4),
            "a minute later it is logged again, with the attempt count"
        );
        assert_eq!(
            f.failed(t0 + OPEN_FAIL_LOG_EVERY + Duration::from_secs(1)),
            None
        );
        assert_eq!(
            f.recovered(),
            Some(5),
            "recovery reports the failed attempts"
        );
        assert_eq!(f.recovered(), None, "a clean open has nothing to report");
        assert_eq!(f.failed(t0), Some(1), "the cycle restarts after recovery");
    }

    // ── describe: port-type formatting ──────────────────────────────────────

    #[test]
    fn describe_usb_with_full_info() {
        let info = UsbPortInfo {
            vid: 0x0403,
            pid: 0x6001,
            serial_number: Some("ABC123".into()),
            manufacturer: Some("FTDI".into()),
            product: Some("FT232R USB UART".into()),
        };
        assert_eq!(
            describe(&SerialPortType::UsbPort(info)),
            "USB 0403:6001 FTDI FT232R USB UART"
        );
    }

    #[test]
    fn describe_usb_trims_when_manufacturer_and_product_absent() {
        let info = UsbPortInfo {
            vid: 0x1234,
            pid: 0x5678,
            serial_number: None,
            manufacturer: None,
            product: None,
        };
        assert_eq!(describe(&SerialPortType::UsbPort(info)), "USB 1234:5678");
    }

    #[test]
    fn describe_non_usb_variants() {
        assert_eq!(describe(&SerialPortType::PciPort), "PCI");
        assert_eq!(describe(&SerialPortType::BluetoothPort), "Bluetooth");
        assert_eq!(describe(&SerialPortType::Unknown), "unknown");
    }

    // ── write_paced: the `serial send` pacing fan-out ───────────────────────

    fn test_handle() -> (SerialHandle, mpsc::Receiver<Bytes>) {
        let (to_clients, _) = broadcast::channel(16);
        let (write_tx, write_rx) = mpsc::channel(WRITE_CAP);
        let (dtr_tx, _dtr_rx) = mpsc::channel(1);
        let status = Arc::new(Mutex::new(Status {
            device: "test".into(),
            baud: 115_200,
            connected: false,
            power_on: None,
        }));
        let handle = SerialHandle {
            to_clients,
            write_tx,
            ring: Arc::new(Mutex::new(VecDeque::new())),
            status,
            dtr_tx,
        };
        // _dtr_rx is held by the caller's scope only long enough to build the
        // handle; write_paced never touches the DTR path.
        drop(_dtr_rx);
        (handle, write_rx)
    }

    #[tokio::test]
    async fn write_paced_zero_sends_whole_buffer_as_one_message() {
        let (h, mut rx) = test_handle();
        h.write_paced(Bytes::from_static(b"hello"), Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(rx.recv().await.unwrap(), Bytes::from_static(b"hello"));
        assert!(rx.try_recv().is_err(), "exactly one message at line rate");
    }

    #[tokio::test]
    async fn write_paced_nonzero_drips_one_byte_per_message_in_order() {
        let (h, mut rx) = test_handle();
        h.write_paced(Bytes::from_static(b"abc"), Duration::from_millis(1))
            .await
            .unwrap();
        let mut got = Vec::new();
        while let Ok(b) = rx.try_recv() {
            got.push(b);
        }
        assert_eq!(
            got,
            vec![
                Bytes::from_static(b"a"),
                Bytes::from_static(b"b"),
                Bytes::from_static(b"c"),
            ]
        );
    }

    #[tokio::test]
    async fn write_paced_empty_buffer_sends_nothing_when_paced() {
        let (h, mut rx) = test_handle();
        h.write_paced(Bytes::new(), Duration::from_millis(1))
            .await
            .unwrap();
        assert!(rx.try_recv().is_err());
    }

    // ── pty consoles (a VM's serial port) ───────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn is_pty_recognizes_platform_pty_paths() {
        assert!(is_pty("/dev/ttys006"), "macOS pty slave");
        assert!(is_pty("/dev/pts/3"), "Linux pty slave");
    }

    #[test]
    fn is_pty_rejects_real_serial_devices() {
        for dev in [
            "/dev/tty.usbserial-AA00BB11", // macOS FTDI
            "/dev/cu.usbmodem14201",       // macOS CDC-ACM
            "/dev/ttyS0",                  // Linux 16550
            "/dev/ttyUSB0",
            "/dev/ttyACM0",
            "/dev/serial/by-id/usb-FTDI_FT232R-if00-port0",
            "/dev/ttys", // prefix with no number
            "/dev/pts/", // ditto
        ] {
            assert!(!is_pty(dev), "{dev} is not a pty");
        }
    }

    #[cfg(unix)]
    fn tmp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("serialcap-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// A symlink to a pty is a pty: the hidrig console bridge publishes its
    /// console as a link with a stable name, and by that name alone the
    /// prefix test said "real port" — so on macOS the open applied a line
    /// rate and failed with ENOTTY. The link targets here do not exist, which
    /// is the point: the decision is made before the open, from the path.
    #[cfg(unix)]
    #[test]
    fn is_pty_follows_symlinks_to_the_device() {
        use std::os::unix::fs::symlink;
        let dir = tmp_dir("pty-links");

        let to_mac_pty = dir.join("console");
        symlink("/dev/ttys004", &to_mac_pty).unwrap();
        assert!(is_pty(to_mac_pty.to_str().unwrap()), "link to a macOS pty");

        let to_linux_pty = dir.join("guest");
        symlink("/dev/pts/7", &to_linux_pty).unwrap();
        assert!(
            is_pty(to_linux_pty.to_str().unwrap()),
            "link to a Linux pty"
        );

        let chained = dir.join("chained");
        symlink(&to_linux_pty, &chained).unwrap();
        assert!(is_pty(chained.to_str().unwrap()), "link to a link to a pty");

        let relative = dir.join("relative");
        symlink("./guest", &relative).unwrap();
        assert!(is_pty(relative.to_str().unwrap()), "relative link target");

        let to_real = dir.join("uart");
        symlink("/dev/tty.usbserial-0001", &to_real).unwrap();
        assert!(!is_pty(to_real.to_str().unwrap()), "link to a real port");

        let plain = dir.join("ttys004");
        std::fs::write(&plain, b"").unwrap();
        assert!(
            !is_pty(plain.to_str().unwrap()),
            "a plain file named like a pty"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn open_baud_drops_the_line_rate_for_a_pty_on_macos_only() {
        // Linux must keep the configured rate: there a 0 becomes B0, "hang up".
        let want = if cfg!(target_os = "macos") {
            0
        } else {
            115_200
        };
        assert_eq!(open_baud("/dev/ttys006", 115_200), want);
        // A real port always keeps its rate, on every platform.
        assert_eq!(open_baud("/dev/tty.usbserial-AA00BB11", 115_200), 115_200);
    }

    /// A pty pair, as a hypervisor hands one out: the returned fd is the end
    /// UTM/qemu keeps, and the path is what it prints for you to attach to.
    #[cfg(unix)]
    fn open_pty_master() -> (std::os::unix::io::RawFd, String) {
        use std::ffi::CStr;
        unsafe {
            let fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            assert!(fd >= 0, "posix_openpt: {}", std::io::Error::last_os_error());
            assert_eq!(libc::grantpt(fd), 0, "grantpt");
            assert_eq!(libc::unlockpt(fd), 0, "unlockpt");
            let name = libc::ptsname(fd);
            assert!(!name.is_null(), "ptsname");
            (fd, CStr::from_ptr(name).to_string_lossy().into_owned())
        }
    }

    /// The regression this whole path exists for: before `open_baud`, this open
    /// failed on macOS with ENOTTY ("Not a typewriter") and paniolo could not
    /// attach to a VM console at all.
    #[cfg(unix)]
    #[tokio::test]
    async fn opens_a_real_pty_and_carries_data_both_ways() {
        use std::os::unix::io::FromRawFd;

        let (master, device) = open_pty_master();
        assert!(is_pty(&device), "{device} should be recognised as a pty");
        let baud = open_baud(&device, 115_200);
        let mut port = tokio_serial::new(&device, baud)
            .open_native_async()
            .unwrap_or_else(|e| panic!("opening pty {device} at baud {baud}: {e}"));
        let mut far = unsafe { std::fs::File::from_raw_fd(master) };

        std::io::Write::write_all(&mut far, b"boot: hello\r\n").unwrap();
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(
            Duration::from_secs(5),
            AsyncReadExt::read(&mut port, &mut buf),
        )
        .await
        .expect("read from pty timed out")
        .expect("read from pty failed");
        assert_eq!(&buf[..n], b"boot: hello\r\n");

        AsyncWriteExt::write_all(&mut port, b"reply\r\n")
            .await
            .unwrap();
        let mut back = [0u8; 64];
        let n = std::io::Read::read(&mut far, &mut back).unwrap();
        assert_eq!(&back[..n], b"reply\r\n");
    }

    /// The same open through a symlink — the shape the hidrig console bridge
    /// publishes. On macOS this only succeeds if the link is recognised as a
    /// pty (baud 0); with a line rate the open fails with ENOTTY.
    #[cfg(unix)]
    #[tokio::test]
    async fn opens_a_pty_through_a_symlink() {
        use std::os::unix::fs::symlink;
        use std::os::unix::io::FromRawFd;

        let (master, device) = open_pty_master();
        let far = unsafe { std::fs::File::from_raw_fd(master) };
        let dir = tmp_dir("pty-symlink-open");
        let link = dir.join("console");
        symlink(&device, &link).unwrap();
        let link = link.to_str().unwrap().to_string();

        let baud = open_baud(&link, 115_200);
        let port = tokio_serial::new(&link, baud).open_native_async();
        std::fs::remove_dir_all(&dir).ok();
        port.unwrap_or_else(|e| panic!("opening pty via symlink {link} at baud {baud}: {e}"));
        drop(far);
    }

    /// The premise behind the supervisor's `Ok(0) if pty` arm: when the far end
    /// goes away the read must *resolve* — never hang, never yield data — so the
    /// loop can emit "disconnected" and start reconnecting. macOS reports this
    /// as EOF, Linux as EIO; both break the loop, a hang would not.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_closed_pty_far_end_resolves_the_read() {
        use std::os::unix::io::FromRawFd;

        let (master, device) = open_pty_master();
        let mut port = tokio_serial::new(&device, open_baud(&device, 115_200))
            .open_native_async()
            .expect("pty open");
        drop(unsafe { std::fs::File::from_raw_fd(master) }); // the VM went away

        let mut buf = [0u8; 32];
        let read = tokio::time::timeout(
            Duration::from_secs(5),
            AsyncReadExt::read(&mut port, &mut buf),
        )
        .await;
        match read.expect("read hung after the pty far end closed") {
            Ok(0) => {}
            Ok(n) => panic!("read {n} bytes from a pty whose far end is closed"),
            Err(e) => assert_ne!(
                e.kind(),
                std::io::ErrorKind::WouldBlock,
                "WouldBlock would spin the supervisor instead of reconnecting"
            ),
        }
    }

    // ── the supervisor on a real pty ────────────────────────────────────────

    /// A supervisor on the slave end of a fresh pty, with the master end (the
    /// "target") returned for the test to drive. The capture thread writes to
    /// a scratch directory.
    #[cfg(unix)]
    fn spawn_on_pty(tag: &str) -> (SerialHandle, std::fs::File, PathBuf) {
        use std::os::unix::io::FromRawFd;
        let (master, device) = open_pty_master();
        let far = unsafe { std::fs::File::from_raw_fd(master) };
        let dir = tmp_dir(tag);
        let handle = spawn_interface(
            InterfaceSpec {
                name: "console".into(),
                device,
                baud: 115_200,
                power_sense_signal: None,
            },
            dir.clone(),
            100,
        );
        (handle, far, dir)
    }

    /// Wait until the client-side stream has carried `needle`, returning
    /// everything received so far.
    #[cfg(unix)]
    async fn recv_until(rx: &mut broadcast::Receiver<Bytes>, needle: &[u8]) -> Vec<u8> {
        let mut got = Vec::new();
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "timed out waiting for {:?}; received so far: {:?}",
                        String::from_utf8_lossy(needle),
                        String::from_utf8_lossy(&got)
                    )
                })
                .expect("stream closed");
            got.extend_from_slice(&chunk);
            if got.windows(needle.len()).any(|w| w == needle) {
                return got;
            }
        }
    }

    /// Review M18. A large write must not stop the port being read. The
    /// target here never reads, so the pty's buffer fills after a few KiB and
    /// the rest of the 64 KiB paste cannot go out; the supervisor must still
    /// pick up and fan out what the target prints in the meantime. Before the
    /// outbox, `write_all` held the loop until the paste finished, so this
    /// timed out.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stuck_write_does_not_starve_reads() {
        let (handle, mut far, dir) = spawn_on_pty("stuck-write");
        let (_, mut rx) = handle.attach();
        recv_until(&mut rx, b"serial connected").await;

        handle
            .write_tx
            .send(Bytes::from(vec![b'x'; 64 * 1024]))
            .await
            .unwrap();
        // Give the supervisor time to start the write and wedge on the full
        // pty buffer before the target speaks.
        tokio::time::sleep(Duration::from_millis(200)).await;

        std::io::Write::write_all(&mut far, b"target says hi\r\n").unwrap();
        recv_until(&mut rx, b"target says hi").await;

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Review M19. A DTR press must not close and reopen the port: the OS
    /// drops and re-raises DTR across a reopen, which was a second, driver-
    /// timed button press after every deliberate one (and the read stream lost
    /// whatever arrived meanwhile). The tell is the marker: a reopen emits
    /// "reconnected". Data must keep flowing on the same port afterwards.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_dtr_press_keeps_the_port_open() {
        let (handle, mut far, dir) = spawn_on_pty("dtr-press");
        let (_, mut rx) = handle.attach();
        recv_until(&mut rx, b"serial connected").await;

        handle.dtr_press(20).await.unwrap();

        std::io::Write::write_all(&mut far, b"after the press\r\n").unwrap();
        let got = recv_until(&mut rx, b"after the press").await;
        let text = String::from_utf8_lossy(&got);
        assert!(text.contains("serial button press"), "{text}");
        assert!(
            !text.contains("reconnected") && !text.contains("disconnected"),
            "the press reopened the port: {text}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
