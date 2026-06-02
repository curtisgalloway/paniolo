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
use std::path::{Path, PathBuf};
use std::sync::mpsc as stdmpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    pub fn subscribe(&self) -> broadcast::Receiver<Bytes> {
        self.to_clients.subscribe()
    }

    /// Snapshot of recent output for a newly-connected client.
    pub fn scrollback(&self) -> Vec<u8> {
        self.ring.lock().unwrap().iter().copied().collect()
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
fn spawn_capture(capture_dir: PathBuf, buffer_lines: u64) -> stdmpsc::Sender<Bytes> {
    let (tx, rx) = stdmpsc::channel::<Bytes>();
    std::thread::Builder::new()
        .name("serialcap-capture".into())
        .spawn(move || {
            let mut log = LineLog::open(capture_dir, buffer_lines);
            // Iteration ends when every sender has dropped (daemon shutting down).
            for chunk in rx {
                log.ingest(&chunk);
            }
        })
        .expect("spawn capture thread");
    tx
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

    loop {
        let port = match tokio_serial::new(&device, baud).open_native_async() {
            Ok(mut p) => {
                info!("serial port opened: {device} @ {baud}");
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
                warn!("open {device} failed: {e}");
                status.lock().unwrap().connected = false;
                tokio::time::sleep(REOPEN_DELAY).await;
                continue;
            }
        };

        let (mut rd, mut wr) = tokio::io::split(port);
        let mut buf = [0u8; 65536];

        enum InnerExit {
            Disconnect,
            DtrPress {
                duration_ms: u64,
                resp_tx: oneshot::Sender<()>,
            },
        }

        let exit = loop {
            tokio::select! {
                read = rd.read(&mut buf) => match read {
                    Ok(0) => {
                        // Serial ports don't have EOF; Ok(0) means the async
                        // read resolved without data. Yield to avoid a spin loop.
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    Ok(n) => {
                        let chunk = Bytes::copy_from_slice(&buf[..n]);
                        push_ring(&ring, &chunk);
                        if line_tx.send(chunk.clone()).is_err() {
                            warn!("capture thread dead — bytes lost");
                        }
                        // Err just means no subscribers; that's fine.
                        let _ = to_clients.send(chunk);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    Err(e) => { warn!("serial read error: {e}"); break InnerExit::Disconnect; }
                },
                Some(data) = write_rx.recv() => {
                    if let Err(e) = wr.write_all(&data).await {
                        warn!("serial write error: {e}");
                        break InnerExit::Disconnect;
                    }
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
                // Drop the port and re-enter the outer reconnect loop immediately.
                drop(port);
                continue;
            }
            InnerExit::Disconnect => {
                // We only reach here after a successful open, so this is a real
                // disconnect (link dropped / device unplugged), not a failed open.
                status.lock().unwrap().connected = false;
                emit_marker(&ring, &to_clients, &line_tx, "disconnected", 31); // red
                tokio::time::sleep(REOPEN_DELAY).await;
            }
        }
    }
}

/// Inject a styled, timestamped status line into the stream and scrollback so the
/// web terminal shows exactly when the serial link dropped or came back. ANSI
/// color `code` (31 red / 32 green / 36 cyan); only the WS terminal renders it —
/// `tio` uses a different path.
fn emit_marker(
    ring: &Arc<Mutex<VecDeque<u8>>>,
    to_clients: &broadcast::Sender<Bytes>,
    line_tx: &stdmpsc::Sender<Bytes>,
    label: &str,
    code: u8,
) {
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
    let bytes = Bytes::from(line.into_bytes());
    push_ring(ring, &bytes);
    let _ = line_tx.send(bytes.clone());
    let _ = to_clients.send(bytes);
}

fn push_ring(ring: &Arc<Mutex<VecDeque<u8>>>, chunk: &[u8]) {
    // try_lock: if scrollback() holds the lock, skip rather than blocking
    // the supervisor OS thread (which stalls the read loop and risks FIFO overflow).
    if let Ok(mut r) = ring.try_lock() {
        r.extend(chunk.iter().copied());
        let overflow = r.len().saturating_sub(RING_BYTES);
        if overflow > 0 {
            r.drain(0..overflow);
        }
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

    fn ring() -> Arc<Mutex<VecDeque<u8>>> {
        Arc::new(Mutex::new(VecDeque::new()))
    }

    fn snapshot(r: &Arc<Mutex<VecDeque<u8>>>) -> Vec<u8> {
        r.lock().unwrap().iter().copied().collect()
    }

    // ── push_ring: scrollback ring-buffer truncation ────────────────────────

    #[test]
    fn push_ring_accumulates_in_order() {
        let r = ring();
        push_ring(&r, b"hello ");
        push_ring(&r, b"world");
        assert_eq!(snapshot(&r), b"hello world");
    }

    #[test]
    fn push_ring_truncates_to_capacity_keeping_newest() {
        let r = ring();
        push_ring(&r, &vec![b'A'; RING_BYTES]); // fill exactly to capacity
        push_ring(&r, b"XYZ"); // 3 bytes over -> 3 oldest dropped
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
        let r = ring();
        let big: Vec<u8> = (0..(RING_BYTES as u32 + 100)).map(|i| i as u8).collect();
        push_ring(&r, &big);
        let snap = snapshot(&r);
        assert_eq!(snap.len(), RING_BYTES);
        assert_eq!(
            snap,
            big[big.len() - RING_BYTES..],
            "keeps the most-recent window"
        );
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
            ring: ring(),
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
}
