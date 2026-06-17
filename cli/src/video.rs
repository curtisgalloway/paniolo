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

pub fn daemon_url(target: &str) -> Option<String> {
    daemons::daemon_url(DAEMON, Some(target))
}

/// OCR the target daemon's current frame via `GET /ocr` (optionally waiting for
/// a stable signal first), returning the recognized text.
pub fn ocr(target: &str, stable: bool, timeout_ms: u64) -> Result<String> {
    let url = daemon_url(target)
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

/// Start the `target`'s hdmicap daemon for `device`, detached; caller polls
/// discovery. The target also names the per-target runtime dir (so multiple
/// targets' daemons coexist) and rides along as `PANIOLO_TARGET` for the
/// dashboard's power-cycle button.
pub fn start_daemon(device: &str, port: u16, target: &str) -> Result<()> {
    let binary = daemons::find_binary(DAEMON)
        .ok_or_else(|| anyhow!("hdmicap not found (libexec or PATH) — run `paniolo setup`"))?;
    let mut cmd = Command::new(binary);
    cmd.arg("daemon")
        .arg("--device")
        .arg(device)
        .arg("--port")
        .arg(port.to_string());
    cmd.envs(daemons::helper_env(DAEMON, Some(target)));
    // visionocr on macOS, linuxocr (same interface) on Linux.
    if let Some(ocr) =
        daemons::find_binary("visionocr").or_else(|| daemons::find_binary("linuxocr"))
    {
        cmd.env("PANIOLO_VISIONOCR", ocr);
    }
    cmd.env("PANIOLO_TARGET", target);
    // Capture stderr (tracing output) so a startup failure is diagnosable;
    // daemons::start_failure() reads the tail on timeout.
    let log = std::fs::File::create(
        daemons::ensure_runtime_dir(DAEMON, Some(target))?.join("daemon.log"),
    )?;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd.spawn()?;
    Ok(())
}

/// Stop the target's running daemon via `hdmicap stop`. The per-target
/// `helper_env` points `hdmicap stop` at the right instance's discovery file.
pub fn stop_daemon(target: &str) -> Result<i32> {
    let binary = daemons::find_binary(DAEMON).ok_or_else(|| anyhow!("hdmicap not found"))?;
    let status = Command::new(binary)
        .arg("stop")
        .envs(daemons::helper_env(DAEMON, Some(target)))
        .status()?;
    Ok(status.code().unwrap_or(1))
}

/// Run an `hdmicap` client subcommand (shot/devices/…) with stdio passed
/// through; returns the exit code. `instance` is the target whose daemon to
/// reach (`None` for daemon-less subcommands like `devices`).
pub fn passthrough(args: &[String], instance: Option<&str>) -> Result<i32> {
    let binary = daemons::find_binary(DAEMON).ok_or_else(|| anyhow!("hdmicap not found"))?;
    let status = Command::new(binary)
        .args(args)
        .envs(daemons::helper_env(DAEMON, instance))
        .status()?;
    Ok(status.code().unwrap_or(1))
}
