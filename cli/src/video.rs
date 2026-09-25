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

/// The hdmicap daemon holding `device` that no discovery file accounts for,
/// if there is one — an orphan left behind when the runtime file that named it
/// was deleted out from under it (see the "untracked daemons" note in
/// daemons.rs). It still owns the capture device, so it has to be reaped
/// before a replacement can start.
pub fn untracked(device: &str) -> Option<daemons::Untracked> {
    daemons::untracked_on_devices(DAEMON, &[device.to_string()])
        .into_iter()
        .next()
}

/// The daemon's address with no token: `http://127.0.0.1:<port>`. This is what
/// paniolo prints, everywhere: its output lands in agent transcripts, CI logs
/// and pasted terminal output, which is not where a bearer credential belongs
/// (#196). The token-bearing form is built inline in the one command whose job
/// is to produce it, `video preview`, and nowhere else — there is deliberately
/// no helper for it, so it cannot be reached for by accident. None if the
/// daemon isn't running.
///
/// Mirrors `serial::daemon_url`, the same accessor on the sibling channel.
pub fn daemon_url(target: &str) -> Option<String> {
    daemons::daemon_url(DAEMON, Some(target))
}

/// The local request timeout (ms) for a stable-frame wait of `timeout_ms`:
/// the daemon's own `wait=stable&timeout=` plus 5s of slack, so the local
/// timeout never fires first. `saturating_add` rather than `+`: `timeout_ms`
/// is a caller-supplied `--timeout`, and a huge value must clamp to
/// `u64::MAX` rather than wrap into a too-short local timeout, or panic
/// outright in a debug build (Review low #8).
fn stable_wait_timeout_ms(timeout_ms: u64) -> u64 {
    timeout_ms.saturating_add(5_000)
}

/// OCR the target daemon's current frame via `GET /ocr` (optionally waiting for
/// a stable signal first), returning the raw v1 envelope (see docs/dev/ocr.md).
pub fn ocr(target: &str, stable: bool, timeout_ms: u64) -> Result<String> {
    let daemon = daemon(target).ok_or_else(|| {
        crate::error::daemon_down(
            DAEMON,
            "no video daemon running — start one with `paniolo video watch`",
        )
        .target(target)
    })?;
    if stable {
        // The snapshot blocks until the signal settles (or times out); the
        // body is discarded — only the wait matters.
        let _ = daemon
            .get(&format!("/snapshot?wait=stable&timeout={timeout_ms}"))
            .timeout(std::time::Duration::from_millis(stable_wait_timeout_ms(
                timeout_ms,
            )))
            .call()
            .map_err(|e| {
                crate::error::daemon_request_failed(DAEMON, "waiting for a stable frame", e)
                    .target(target)
            })?;
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
        Err(e) => Err(crate::error::daemon_request_failed(DAEMON, "OCR", e)
            .target(target)
            .into()),
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
    ocr_helper_with(ocr_mode, cfg!(target_os = "linux"), daemons::find_binary)
}

/// [`ocr_helper`] with its two host dependencies passed in: which platform this
/// is, and how a helper name resolves.
///
/// Both are parameters so the whole matrix is testable from one host. The
/// lookup, because the real `find_binary` searches the libexec dirs before
/// `$PATH`, so a packaged `rapidocr` in `/usr/libexec/paniolo/bin` beat any
/// `$PATH` a test set and the test read the machine instead of the code
/// (GitHub #206). The platform, because a `cfg!` folded in here would leave
/// each CI platform exercising only its own half, and the claim this function
/// makes is about the *difference* between them.
fn ocr_helper_with(
    ocr_mode: Option<&str>,
    is_linux: bool,
    find: impl Fn(&str) -> Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    if is_linux && ocr_mode == Some("gui") {
        if let Some(p) = find("rapidocr") {
            return Some(p);
        }
        eprintln!(
            "note: video ocr_mode = \"gui\" but the rapidocr helper is not installed; \
             falling back to the default engine, which loses rows of GUI text. \
             Install it with `paniolo setup`."
        );
    }
    find("visionocr")
        .or_else(|| find("winocr"))
        .or_else(|| find("linuxocr"))
}

/// Start the `target`'s hdmicap daemon for `device`, detached; caller polls
/// discovery. The target also names the per-target runtime dir (so multiple
/// targets' daemons coexist) and rides along as `PANIOLO_TARGET` for the
/// dashboard's power-cycle button. `ocr_mode` picks the OCR helper the daemon
/// will run — see [`ocr_helper`].
pub fn start_daemon(
    device: &str,
    port: u16,
    target: &str,
    ocr_mode: Option<&str>,
) -> Result<std::process::Child> {
    let binary = daemons::find_binary(DAEMON).ok_or_else(|| {
        crate::error::PanioloError::not_configured(
            "hdmicap not found (libexec or PATH) — run `paniolo setup`".to_string(),
        )
    })?;
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
    Ok(cmd.spawn()?)
}

/// Stop the target's running daemon via `hdmicap stop`. The per-target
/// `helper_env` points `hdmicap stop` at the right instance's discovery file.
pub fn stop_daemon(target: &str) -> Result<std::process::ExitStatus> {
    let binary =
        daemons::find_binary(DAEMON).ok_or_else(|| crate::error::helper_missing("hdmicap"))?;
    let status = Command::new(binary)
        .arg("stop")
        .envs(daemons::helper_env(DAEMON, Some(target)))
        .status()?;
    Ok(status)
}

/// Run an `hdmicap` client subcommand (shot/devices/…) with stdio passed
/// through; returns the exit code. `instance` is the target whose daemon to
/// reach (`None` for daemon-less subcommands like `devices`).
pub fn passthrough(args: &[String], instance: Option<&str>) -> Result<i32> {
    let binary =
        daemons::find_binary(DAEMON).ok_or_else(|| crate::error::helper_missing("hdmicap"))?;
    let status = Command::new(binary)
        .args(args)
        .envs(daemons::helper_env(DAEMON, instance))
        .status()?;
    Ok(crate::error::shell_code(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The old `timeout_ms + 5_000` panics on overflow in a debug build (and
    /// wraps to a too-short timeout in release) for a `--timeout` near
    /// `u64::MAX`; the saturating version clamps instead (Review low #8).
    #[test]
    fn stable_wait_timeout_saturates_instead_of_overflowing() {
        assert_eq!(stable_wait_timeout_ms(2_000), 7_000);
        assert_eq!(stable_wait_timeout_ms(u64::MAX), u64::MAX);
    }

    /// `ocr_mode = "gui"` must reach `rapidocr` on Linux and must NOT change
    /// anything anywhere else — Apple Vision and Windows.Media.Ocr win both
    /// screen types on their own platforms, so honoring the field there would
    /// only be a way to pick the wrong engine.
    ///
    /// Both platforms are asserted from whichever host runs the test: the
    /// `cfg!` lives in `ocr_helper`, not in the function under test, so the
    /// Linux and non-Linux arms are both reachable here. With the `cfg!`
    /// inside, each CI platform only ever ran its own half and the
    /// "only on Linux" in the name was never actually checked.
    #[test]
    fn gui_mode_selects_rapidocr_only_on_linux() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["rapidocr", "visionocr", "linuxocr"] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        let find = |name: &str| {
            let p = dir.path().join(name);
            p.is_file().then_some(p)
        };
        let rapidocr = dir.path().join("rapidocr");
        let visionocr = dir.path().join("visionocr");

        // Linux + gui: the one case that selects rapidocr.
        assert_eq!(
            ocr_helper_with(Some("gui"), true, find).as_deref(),
            Some(rapidocr.as_path()),
            "gui mode must select rapidocr on Linux"
        );
        // Same mode off Linux: rapidocr is never the answer.
        assert_eq!(
            ocr_helper_with(Some("gui"), false, find).as_deref(),
            Some(visionocr.as_path()),
            "gui mode must not select rapidocr off Linux"
        );
        // No mode set: rapidocr is never the answer on any platform.
        for is_linux in [true, false] {
            assert_eq!(
                ocr_helper_with(None, is_linux, find).as_deref(),
                Some(visionocr.as_path()),
                "the default engine wins with no mode set (is_linux={is_linux})"
            );
            assert_eq!(
                ocr_helper_with(Some("text"), is_linux, find).as_deref(),
                Some(visionocr.as_path()),
                "text mode never reaches rapidocr (is_linux={is_linux})"
            );
        }
    }

    /// The fallback chain, in order: `visionocr`, then `winocr`, then
    /// `linuxocr`. Dropping a link used to leave the suite green while
    /// `ocr_helper` returned `None` on every host of that platform, which
    /// silently disables `paniolo video read` and the dashboard's OCR button.
    #[test]
    fn the_default_engine_falls_back_through_every_helper() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let only = |present: &'static str| {
            let root = root.clone();
            move |name: &str| (name == present).then(|| root.join(name))
        };
        for name in ["visionocr", "winocr", "linuxocr"] {
            assert_eq!(
                ocr_helper_with(None, false, only(name)).as_deref(),
                Some(root.join(name).as_path()),
                "{name} alone must be found"
            );
        }
        // Nothing installed: no engine, rather than a wrong one.
        assert_eq!(ocr_helper_with(None, true, |_| None), None);
        // gui mode with no rapidocr falls through to the default chain.
        assert_eq!(
            ocr_helper_with(Some("gui"), true, only("linuxocr")).as_deref(),
            Some(root.join("linuxocr").as_path()),
        );
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
