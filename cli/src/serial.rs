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
//! `<runtime-base>/serialcap/<target>/daemon.json` (per-target, so multiple
//! targets' daemons coexist on one host — see daemons.rs), holding
//! `{pid, port, token, …}`; every request carries the token (see
//! `daemons::Endpoint`). An interface is passed to the daemon as
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

/// The target's running serialcap daemon — port and token — or None if it
/// isn't running.
pub fn daemon(target: &str) -> Option<daemons::Endpoint> {
    daemons::daemon_endpoint(DAEMON, Some(target))
}

/// Base URL of the target's running serialcap daemon, or None if it isn't
/// running. For printing; API calls go through [`daemon`].
pub fn daemon_url(target: &str) -> Option<String> {
    daemons::daemon_url(DAEMON, Some(target))
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

/// Start the target's serialcap daemon (owning every given interface),
/// detached. The caller polls [`daemon_url`] for readiness.
pub fn start_daemon(ifaces: &[SerialChannel], port: u16, target: &str) -> Result<()> {
    let binary = daemons::find_binary(DAEMON)
        .ok_or_else(|| anyhow!("serialcap not found (libexec or PATH) — run `paniolo setup`"))?;
    // Record which binary this daemon runs, so a later upgrade/rebuild can be
    // detected as stale (see daemons::binary_is_stale).
    daemons::record_binmeta(&binary, DAEMON, Some(target));
    let mut cmd = Command::new(binary);
    cmd.arg("daemon").arg("--port").arg(port.to_string());
    cmd.envs(daemons::helper_env(DAEMON, Some(target)));
    for ch in ifaces {
        cmd.arg("--interface").arg(interface_arg(ch));
    }
    // Capture stderr (tracing output) so a startup failure is diagnosable;
    // daemons::start_failure() reads the tail on timeout.
    let log = daemons::create_log(DAEMON, Some(target))?;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
    // Detach into its own process group so it survives this CLI exiting.
    crate::platform::detach(&mut cmd);
    cmd.spawn()?;
    Ok(())
}

/// Stop the target's running daemon via `serialcap stop` (it owns the clean
/// shutdown). The per-target `helper_env` points `serialcap stop` at the right
/// instance's discovery file.
pub fn stop_daemon(target: &str) -> Result<i32> {
    let binary = daemons::find_binary(DAEMON).ok_or_else(|| anyhow!("serialcap not found"))?;
    let status = Command::new(binary)
        .arg("stop")
        .envs(daemons::helper_env(DAEMON, Some(target)))
        .status()?;
    Ok(status.code().unwrap_or(1))
}

// ── input ───────────────────────────────────────────────────────────────────

/// POST raw bytes to the serial port the daemon owns; input coexists with
/// capture. `pace_ms > 0` drips bytes one at a time (the substitute for flow
/// control on slow polled consoles), so the timeout is scaled to match.
pub fn send_input(
    daemon: &daemons::Endpoint,
    interface: &str,
    data: &[u8],
    pace_ms: u32,
) -> Result<()> {
    let mut path = format!("/input?interface={interface}");
    if pace_ms > 0 {
        path.push_str(&format!("&pace_ms={pace_ms}"));
    }
    let timeout_ms = std::cmp::max(15_000, data.len() as u64 * pace_ms as u64 + 10_000);
    daemon
        .post(&path)
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
    let err = crate::platform::exec_replace(
        Command::new(tio)
            .arg("--baudrate")
            .arg(baud.to_string())
            .arg(device),
    );
    bail!("exec tio failed: {err}")
}

// ── device listing ──────────────────────────────────────────────────────────

/// Available serial device paths on this platform. On Linux, one entry per
/// physical port, named by its stable /dev/serial symlink — by-id preferred
/// (names the adapter; what lab files typically use), by-path as the fallback
/// (port-derived; the only stable name for adapters without a serial number).
/// On Windows the OS assigns `COM<n>` names and there is no stable by-path
/// analogue, so the enumeration comes from the `serialport` crate as-is.
pub fn list_devices() -> Vec<String> {
    let mut out = enumerate_devices();
    out.sort();
    out
}

/// Windows enumeration: whatever the OS reports as a serial port.
///
/// COM numbers are assigned by the OS and can move when an adapter is
/// re-enumerated, so a lab file pinned to `COM7` is not as stable as a Linux
/// by-id path. There is no better handle to offer.
#[cfg(windows)]
fn enumerate_devices() -> Vec<String> {
    serialport::available_ports()
        .map(|ports| ports.into_iter().map(|p| p.port_name).collect())
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn enumerate_devices() -> Vec<String> {
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
        // Group the /dev/serial symlinks by the tty they resolve to, so each
        // physical port lists once even though udev usually gives it several
        // aliases (a by-id name, plus by-path `usb`/`usbv2` twins).
        let mut by_tty: std::collections::BTreeMap<std::path::PathBuf, Vec<String>> =
            Default::default();
        for dir in ["/dev/serial/by-id", "/dev/serial/by-path"] {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let path = e.path();
                    let tty = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                    by_tty
                        .entry(tty)
                        .or_default()
                        .push(path.display().to_string());
                }
            }
        }
        out.extend(by_tty.values().filter_map(|a| preferred_alias(a).cloned()));
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
    out
}

/// The display name for one physical port's symlink aliases: the by-id name
/// when the adapter has one, else the first (sorted) alias — which keeps the
/// plain `usb` by-path variant ahead of its `usbv2` twin.
#[cfg(not(windows))]
fn preferred_alias(aliases: &[String]) -> Option<&String> {
    aliases
        .iter()
        .find(|a| a.contains("/by-id/"))
        .or_else(|| aliases.iter().min())
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
            power_button: false,
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

    // `preferred_alias` serves the /dev/serial enumeration, which does not
    // exist on Windows.
    #[cfg(not(windows))]
    #[test]
    fn preferred_alias_picks_by_id_over_by_path() {
        let aliases = vec![
            "/dev/serial/by-path/platform-xhci-hcd.1-usb-0:1.2:1.0-port0".to_string(),
            "/dev/serial/by-id/usb-1a86_USB2.0-Serial-if00-port0".to_string(),
            "/dev/serial/by-path/platform-xhci-hcd.1-usbv2-0:1.2:1.0-port0".to_string(),
        ];
        assert_eq!(
            preferred_alias(&aliases).unwrap(),
            "/dev/serial/by-id/usb-1a86_USB2.0-Serial-if00-port0"
        );
    }

    // `preferred_alias` serves the /dev/serial enumeration, which does not
    // exist on Windows.
    #[cfg(not(windows))]
    #[test]
    fn preferred_alias_falls_back_to_first_sorted_by_path() {
        // No by-id (adapter without a serial number): the plain `usb` by-path
        // variant sorts ahead of its `usbv2` twin and wins deterministically.
        let aliases = vec![
            "/dev/serial/by-path/platform-xhci-hcd.1-usbv2-0:1.2:1.0-port0".to_string(),
            "/dev/serial/by-path/platform-xhci-hcd.1-usb-0:1.2:1.0-port0".to_string(),
        ];
        assert_eq!(
            preferred_alias(&aliases).unwrap(),
            "/dev/serial/by-path/platform-xhci-hcd.1-usb-0:1.2:1.0-port0"
        );
        assert_eq!(preferred_alias(&[]), None);
    }
}
