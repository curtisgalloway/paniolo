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

/// The target's running hdmicap daemon — port and token — or None if it
/// isn't running.
pub fn daemon(target: &str) -> Option<daemons::Endpoint> {
    daemons::daemon_endpoint(DAEMON, Some(target))
}

/// The dashboard URL for a human to open: the daemon's `GET /` with the token
/// a browser needs carried as `?token=`. None if the daemon isn't running.
pub fn preview_url(target: &str) -> Option<String> {
    daemon(target).map(|d| d.http_url("/"))
}

/// OCR the target daemon's current frame via `GET /ocr` (optionally waiting for
/// a stable signal first), returning the raw v1 envelope (see docs/dev/ocr.md).
pub fn ocr(target: &str, stable: bool, timeout_ms: u64) -> Result<String> {
    let daemon = daemon(target)
        .ok_or_else(|| anyhow!("no video daemon running — start one with `paniolo video watch`"))?;
    if stable {
        // The snapshot blocks until the signal settles (or times out); the
        // body is discarded — only the wait matters.
        let _ = daemon
            .get(&format!("/snapshot?wait=stable&timeout={timeout_ms}"))
            .timeout(std::time::Duration::from_millis(timeout_ms + 5_000))
            .call()
            .map_err(|e| anyhow!("waiting for a stable frame failed: {e}"))?;
    }
    match daemon
        .get("/ocr")
        .timeout(std::time::Duration::from_secs(30))
        .call()
    {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| anyhow!("reading the OCR response failed: {e}")),
        // Surface the daemon's own explanation ("no video signal", "no capture
        // device") instead of a bare status code — an agent must be able to
        // tell "display is off" apart from "screen is blank".
        Err(ureq::Error::Status(code, resp)) => {
            let msg = resp.into_string().unwrap_or_default();
            let msg = msg.trim();
            if msg.is_empty() {
                Err(anyhow!("OCR failed: daemon returned status {code}"))
            } else {
                Err(anyhow!("OCR failed: {msg}"))
            }
        }
        Err(e) => Err(anyhow!("OCR failed: {e}")),
    }
}

/// The recognized text from an OCR envelope.
///
/// `/ocr` returns the whole envelope so callers can reach confidences and
/// boxes, but `paniolo video read` prints text by default — that is what a
/// human or an agent grepping the screen wants, and it is what the command
/// printed before the envelope existed. A body that is not an envelope is
/// passed through unchanged rather than rejected, so a daemon older than this
/// CLI still reads screens.
pub fn text_of(body: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) if v.get("version").is_some() => v
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string(),
        _ => body.to_string(),
    }
}

/// The OCR helper for a target's screens.
///
/// The platform default is right everywhere except one case: a **Linux** host
/// looking at **GUI** screens, where Tesseract loses whole rows of anti-aliased
/// UI text that PP-OCRv6 reads cleanly (0.312 vs 0.083 token-recall error,
/// measured on a Pi 5 — see evals/ocr). `ocr_mode = "gui"` selects `rapidocr`
/// there.
///
/// Nothing is selected by mode on macOS or Windows: Apple Vision and
/// `Windows.Media.Ocr` win both screen types on their own platforms, so a mode
/// field there would only add a way to choose wrongly.
fn ocr_helper(ocr_mode: Option<&str>) -> Option<std::path::PathBuf> {
    if cfg!(target_os = "linux") && ocr_mode == Some("gui") {
        if let Some(p) = daemons::find_binary("rapidocr") {
            return Some(p);
        }
        eprintln!(
            "note: video ocr_mode = \"gui\" but the rapidocr helper is not installed; \
             falling back to the default engine, which loses rows of GUI text. \
             Install it with `paniolo setup`."
        );
    }
    daemons::find_binary("visionocr")
        .or_else(|| daemons::find_binary("winocr"))
        .or_else(|| daemons::find_binary("linuxocr"))
}

/// Start the `target`'s hdmicap daemon for `device`, detached; caller polls
/// discovery. The target also names the per-target runtime dir (so multiple
/// targets' daemons coexist) and rides along as `PANIOLO_TARGET` for the
/// dashboard's power-cycle button. `ocr_mode` picks the OCR helper the daemon
/// will run — see [`ocr_helper`].
pub fn start_daemon(device: &str, port: u16, target: &str, ocr_mode: Option<&str>) -> Result<()> {
    let binary = daemons::find_binary(DAEMON)
        .ok_or_else(|| anyhow!("hdmicap not found (libexec or PATH) — run `paniolo setup`"))?;
    // Record which binary this daemon runs, so a later upgrade/rebuild can be
    // detected as stale (see daemons::binary_is_stale).
    daemons::record_binmeta(&binary, DAEMON, Some(target));
    let mut cmd = Command::new(binary);
    cmd.arg("daemon")
        .arg("--device")
        .arg(device)
        .arg("--port")
        .arg(port.to_string());
    cmd.envs(daemons::helper_env(DAEMON, Some(target)));
    if let Some(ocr) = ocr_helper(ocr_mode) {
        cmd.env("PANIOLO_VISIONOCR", ocr);
    }
    cmd.env("PANIOLO_TARGET", target);
    // Capture stderr (tracing output) so a startup failure is diagnosable;
    // daemons::start_failure() reads the tail on timeout.
    let log = daemons::create_log(DAEMON, Some(target))?;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
    crate::platform::detach(&mut cmd);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `ocr_mode = "gui"` must reach `rapidocr` on Linux and must NOT change
    /// anything anywhere else — Apple Vision and Windows.Media.Ocr win both
    /// screen types on their own platforms, so honouring the field there would
    /// only be a way to pick the wrong engine.
    #[test]
    fn gui_mode_selects_rapidocr_only_on_linux() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("rapidocr");
        std::fs::write(&fake, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&fake).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&fake, p).unwrap();
        }

        let prev = std::env::var_os("PATH");
        // Only our temp dir: otherwise a real visionocr/linuxocr on this
        // machine decides the outcome and the test proves nothing.
        // Safe: single-threaded test, restored below.
        unsafe { std::env::set_var("PATH", dir.path()) };
        let picked = ocr_helper(Some("gui"));
        let default = ocr_helper(None);
        match prev {
            Some(p) => unsafe { std::env::set_var("PATH", p) },
            None => unsafe { std::env::remove_var("PATH") },
        }

        if cfg!(target_os = "linux") {
            assert_eq!(
                picked.as_deref(),
                Some(fake.as_path()),
                "gui mode must select rapidocr on Linux"
            );
        } else {
            assert!(
                picked.as_deref() != Some(fake.as_path()),
                "gui mode must not select rapidocr off Linux"
            );
        }
        // With no mode set, rapidocr is never the answer on any platform.
        assert!(default.as_deref() != Some(fake.as_path()));
    }

    #[test]
    fn text_of_extracts_the_envelope_text() {
        let body = r#"{"version":1,"engine":"visionocr","width":1280,"height":720,
                       "text":"login:\nPassword:","lines":[]}"#;
        assert_eq!(text_of(body), "login:\nPassword:");
    }

    /// A daemon older than this CLI still returns bare text. Reading a screen
    /// must keep working against it rather than printing a parse error, which
    /// would look like a capture fault rather than a version skew.
    #[test]
    fn text_of_passes_through_pre_envelope_output() {
        assert_eq!(text_of("login:\nPassword:"), "login:\nPassword:");
    }

    /// JSON that is not an envelope is not an envelope. Without the version
    /// check, a screen showing JSON would be silently mined for a "text" key.
    #[test]
    fn text_of_ignores_json_that_is_not_an_envelope() {
        let body = r#"{"text":"not from us"}"#;
        assert_eq!(text_of(body), body);
    }
}
