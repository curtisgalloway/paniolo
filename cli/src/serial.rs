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

//! Serial console runtime: `tio` for an interactive terminal, and the
//! `serialcap` daemon (which owns the port, captures a timestamped log, and
//! accepts input over localhost HTTP so input coexists with capture).
//!
//! Ported from the Python `_serial.py`. serialcap's discovery file is
//! `/tmp/paniolo-<uid>/serialcap/daemon.json` (see daemons.rs), holding
//! `{pid, port, …}`; an interface is passed to the daemon as
//! `NAME=DEVICE@BAUD[:SENSE]`.

use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};

use crate::daemons;
use crate::model::SerialChannel;

pub const DAEMON: &str = "serialcap";

/// Default daemon port: 0 = OS-assigned. The discovery file carries the actual
/// port and every consumer reads it, so a fixed default buys nothing and
/// collides with stale `ssh -L` dashboard tunnels squatting the old 8724.
pub const DEFAULT_PORT: u16 = 0;

/// Base URL of the running serialcap daemon, or None if it isn't running.
pub fn daemon_url() -> Option<String> {
    daemons::daemon_url(DAEMON)
}

// ── daemon control ──────────────────────────────────────────────────────────

/// Format one interface for the daemon's repeatable `--interface` flag:
/// `NAME=DEVICE@BAUD[:SENSE]`.
pub fn interface_arg(ch: &SerialChannel) -> String {
    let mut arg = format!("{}={}@{}", ch.name, ch.device, ch.baud);
    if let Some(sense) = &ch.power_sense_signal {
        arg.push(':');
        arg.push_str(sense);
    }
    arg
}

/// Start the serialcap daemon (owning every given interface), detached.
/// The caller polls [`daemon_url`] for readiness.
pub fn start_daemon(ifaces: &[SerialChannel], port: u16) -> Result<()> {
    let binary = daemons::find_binary(DAEMON)
        .ok_or_else(|| anyhow!("serialcap not found (libexec or PATH) — run `paniolo setup`"))?;
    let mut cmd = Command::new(binary);
    cmd.arg("daemon").arg("--port").arg(port.to_string());
    for ch in ifaces {
        cmd.arg("--interface").arg(interface_arg(ch));
    }
    // Capture stderr (tracing output) so a startup failure is diagnosable;
    // daemons::start_failure() reads the tail on timeout.
    let log = std::fs::File::create(daemons::ensure_runtime_dir(DAEMON)?.join("daemon.log"))?;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
    // Detach into its own process group so it survives this CLI exiting.
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd.spawn()?;
    Ok(())
}

/// Stop the running daemon via `serialcap stop` (it owns the clean shutdown).
pub fn stop_daemon() -> Result<i32> {
    let binary = daemons::find_binary(DAEMON).ok_or_else(|| anyhow!("serialcap not found"))?;
    let status = Command::new(binary).arg("stop").status()?;
    Ok(status.code().unwrap_or(1))
}

// ── input ───────────────────────────────────────────────────────────────────

/// POST raw bytes to the serial port the daemon owns; input coexists with
/// capture. `pace_ms > 0` drips bytes one at a time (the substitute for flow
/// control on slow polled consoles), so the timeout is scaled to match.
pub fn send_input(base_url: &str, interface: &str, data: &[u8], pace_ms: u32) -> Result<()> {
    let mut url = format!("{base_url}/input?interface={interface}");
    if pace_ms > 0 {
        url.push_str(&format!("&pace_ms={pace_ms}"));
    }
    let timeout_ms = std::cmp::max(15_000, data.len() as u64 * pace_ms as u64 + 10_000);
    ureq::post(&url)
        .timeout(Duration::from_millis(timeout_ms))
        .send_bytes(data)
        .map(|_| ())
        .map_err(|e| anyhow!("serialcap /input failed: {e}"))
}

// ── interactive console ─────────────────────────────────────────────────────

/// Replace this process with `tio` on the given device (never returns on
/// success).
pub fn exec_tio(device: &str, baud: i64) -> Result<()> {
    let tio = daemons::find_binary("tio")
        .ok_or_else(|| anyhow!("tio not found in PATH — install it (e.g. brew install tio)"))?;
    let err = std::os::unix::process::CommandExt::exec(
        Command::new(tio)
            .arg("--baudrate")
            .arg(baud.to_string())
            .arg(device),
    );
    bail!("exec tio failed: {err}")
}

// ── device listing ──────────────────────────────────────────────────────────

/// Available serial device paths on this platform. On Linux, prefers the
/// stable /dev/serial/by-path symlinks.
pub fn list_devices() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if cfg!(target_os = "macos") {
        if let Ok(rd) = std::fs::read_dir("/dev") {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.starts_with("tty.usbserial-") || n.starts_with("tty.usbmodem") {
                    out.push(format!("/dev/{n}"));
                }
            }
        }
    } else {
        if let Ok(rd) = std::fs::read_dir("/dev/serial/by-path") {
            for e in rd.flatten() {
                out.push(e.path().display().to_string());
            }
        }
        if out.is_empty() {
            if let Ok(rd) = std::fs::read_dir("/dev") {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().into_owned();
                    if n.starts_with("ttyUSB") || n.starts_with("ttyACM") {
                        out.push(format!("/dev/{n}"));
                    }
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(name: &str, sense: Option<&str>) -> SerialChannel {
        SerialChannel {
            name: name.into(),
            device: "/dev/ttyUSB0".into(),
            baud: 115200,
            power_sense_signal: sense.map(String::from),
            host: None,
        }
    }

    #[test]
    fn interface_arg_formats_name_device_baud() {
        assert_eq!(
            interface_arg(&ch("console", None)),
            "console=/dev/ttyUSB0@115200"
        );
    }

    #[test]
    fn interface_arg_appends_sense() {
        assert_eq!(
            interface_arg(&ch("console", Some("cts"))),
            "console=/dev/ttyUSB0@115200:cts"
        );
    }
}
