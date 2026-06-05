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

//! Video capture runtime — delegates to the `hdmicap` warm-stream daemon.
//!
//! Ported from the Python `_video.py`, with one model change: the capture
//! device comes from the lab's `video` channel (per target), not a separate
//! `video.toml`. The daemon gets `PANIOLO_VISIONOCR` (for `/ocr`) and
//! `PANIOLO_TARGET` (so the dashboard power-cycle button can call back into
//! `paniolo power-cycle <target>`).

use std::process::{Command, Stdio};

use anyhow::{anyhow, Result};

use crate::daemons;

pub const DAEMON: &str = "hdmicap";

/// Default daemon port: 0 = OS-assigned (discovery carries the real port;
/// fixed defaults collide with stale dashboard tunnels).
pub const DEFAULT_PORT: u16 = 0;

pub fn daemon_url() -> Option<String> {
    daemons::daemon_url(DAEMON)
}

/// OCR the daemon's current frame via `GET /ocr` (optionally waiting for a
/// stable signal first), returning the recognized text.
pub fn ocr(stable: bool, timeout_ms: u64) -> Result<String> {
    let url = daemon_url()
        .ok_or_else(|| anyhow!("no video daemon running — start one with `paniolo video watch`"))?;
    if stable {
        // The snapshot blocks until the signal settles (or times out); the
        // body is discarded — only the wait matters.
        let _ = ureq::get(&format!("{url}/snapshot?wait=stable&timeout={timeout_ms}"))
            .timeout(std::time::Duration::from_millis(timeout_ms + 5_000))
            .call()
            .map_err(|e| anyhow!("waiting for a stable frame failed: {e}"))?;
    }
    ureq::get(&format!("{url}/ocr"))
        .timeout(std::time::Duration::from_secs(30))
        .call()
        .map_err(|e| anyhow!("OCR failed: {e}"))?
        .into_string()
        .map_err(|e| anyhow!("reading the OCR response failed: {e}"))
}

/// Start the hdmicap daemon for `device`, detached; caller polls discovery.
pub fn start_daemon(device: &str, port: u16, target_name: Option<&str>) -> Result<()> {
    let binary = daemons::find_binary(DAEMON)
        .ok_or_else(|| anyhow!("hdmicap not found (libexec or PATH) — run `paniolo setup`"))?;
    let mut cmd = Command::new(binary);
    cmd.arg("daemon")
        .arg("--device")
        .arg(device)
        .arg("--port")
        .arg(port.to_string());
    // visionocr on macOS, linuxocr (same interface) on Linux.
    if let Some(ocr) =
        daemons::find_binary("visionocr").or_else(|| daemons::find_binary("linuxocr"))
    {
        cmd.env("PANIOLO_VISIONOCR", ocr);
    }
    if let Some(name) = target_name {
        cmd.env("PANIOLO_TARGET", name);
    }
    // Capture stderr (tracing output) so a startup failure is diagnosable;
    // daemons::start_failure() reads the tail on timeout.
    let log = std::fs::File::create(daemons::ensure_runtime_dir(DAEMON)?.join("daemon.log"))?;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd.spawn()?;
    Ok(())
}

/// Stop the running daemon via `hdmicap stop`.
pub fn stop_daemon() -> Result<i32> {
    let binary = daemons::find_binary(DAEMON).ok_or_else(|| anyhow!("hdmicap not found"))?;
    let status = Command::new(binary).arg("stop").status()?;
    Ok(status.code().unwrap_or(1))
}

/// Run an `hdmicap` client subcommand (shot/devices/…) with stdio passed
/// through; returns the exit code.
pub fn passthrough(args: &[String]) -> Result<i32> {
    let binary = daemons::find_binary(DAEMON).ok_or_else(|| anyhow!("hdmicap not found"))?;
    let status = Command::new(binary).args(args).status()?;
    Ok(status.code().unwrap_or(1))
}
