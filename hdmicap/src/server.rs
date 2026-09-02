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

//! Localhost HTTP API. Handlers never touch the device — they only read the
//! latest FrameState from their `watch::Receiver`. PNG encoding is lazy, here.
//!
//! Every route but the vendored xterm.js assets sits behind the auth layer
//! (`auth.rs`): loopback Host and Origin, and the daemon token. The dashboard
//! page reads the token from its own URL and appends it to every request it
//! makes back here.

use std::io::Cursor;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use image::{ImageBuffer, Rgb};
use serde::Deserialize;
use tokio::sync::{watch, Semaphore};

use crate::capture_thread::FrameRx;
use crate::frame::{FrameState, Signal, StatusDto};
use crate::pixel::{nv12_to_rgb, nv12_to_rgb_half, PixelData};

/// Concurrent permits for the genuinely expensive work a dashboard click can
/// trigger: PNG encode/decode and the OCR subprocess (Review M21). Small on
/// purpose — enough that one slow request doesn't serialize behind another
/// unrelated one, not so large that a burst of clicks piles up unbounded CPU
/// work or unbounded `visionocr` helper processes.
const EXPENSIVE_PERMITS: usize = 2;

#[derive(Clone)]
pub struct AppState {
    pub frames: FrameRx,
    /// Bounds concurrent PNG encode/decode and OCR subprocess work — see
    /// [`EXPENSIVE_PERMITS`].
    pub expensive: Arc<Semaphore>,
}

impl AppState {
    pub fn new(frames: FrameRx) -> Self {
        AppState {
            frames,
            expensive: Arc::new(Semaphore::new(EXPENSIVE_PERMITS)),
        }
    }
}

/// Paths served without the daemon token: the vendored xterm.js library files
/// the dashboard page loads by bare `<script>`/`<link>` path. They are public
/// code, not data, and a bare asset tag cannot carry a header.
pub const PUBLIC_ASSETS: &[&str] = &["/xterm.js", "/xterm.css", "/xterm-addon-fit.js"];

pub fn router(state: AppState, auth: crate::auth::Auth) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/status", get(status))
        .route("/snapshot", get(snapshot))
        .route("/preview", get(preview))
        .route("/ocr", get(ocr))
        .route("/power", get(power_state))
        .route("/power-on", post(power_on))
        .route("/power-off", post(power_off))
        .route("/power-cycle", post(power_cycle))
        .route("/devices", get(devices))
        // Vendored xterm.js assets for the serial terminal pane.
        .route("/xterm.js", get(xterm_js))
        .route("/xterm.css", get(xterm_css))
        .route("/xterm-addon-fit.js", get(xterm_fit_js))
        .layer(middleware::from_fn_with_state(auth, crate::auth::require))
        .with_state(state)
}

/// The dashboard. It must never render inside another page's frame: its power
/// buttons act on the target, and a framed page is how a click gets stolen.
async fn index() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_SECURITY_POLICY, "frame-ancestors 'none'"),
            (header::X_FRAME_OPTIONS, "DENY"),
        ],
        include_str!("../assets/index.html"),
    )
}

async fn xterm_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../assets/xterm.js"),
    )
}

async fn xterm_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../assets/xterm.css"),
    )
}

async fn xterm_fit_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../assets/xterm-addon-fit.js"),
    )
}

async fn status(State(s): State<AppState>) -> Json<StatusDto> {
    let f = s.frames.borrow().clone();
    Json(StatusDto::from(f.as_ref()))
}

#[derive(Deserialize)]
struct SnapReq {
    /// "stable" -> wait until signal == Stable.
    wait: Option<String>,
    /// Hex hash from a prior /status; wait until the published hash differs.
    changed_since: Option<String>,
    /// Milliseconds; default applied below.
    timeout: Option<u64>,
}

const DEFAULT_TIMEOUT_MS: u64 = 2000;

async fn snapshot(State(s): State<AppState>, Query(q): Query<SnapReq>) -> Response {
    let mut rx = s.frames.clone();
    let timeout_ms = q.timeout.unwrap_or(DEFAULT_TIMEOUT_MS);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms).min(Duration::from_secs(60));
    let want_stable = q.wait.as_deref() == Some("stable");
    let changed_since = q
        .changed_since
        .as_ref()
        .and_then(|h| u64::from_str_radix(h, 16).ok());

    loop {
        let ready = {
            let f = rx.borrow_and_update();
            match (want_stable, changed_since) {
                (true, _) => f.effective_signal() == Signal::Stable,
                (_, Some(h)) => f.hash != h,
                _ => true,
            }
        };

        if ready {
            let f = rx.borrow().clone();
            return png_response(&f, false, &s.expensive).await;
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let f = rx.borrow().clone();
            return png_response(&f, true, &s.expensive).await;
        }
        match tokio::time::timeout(remaining, rx.changed()).await {
            // Changed: loop back around and re-check readiness.
            Ok(Ok(())) => {}
            // Review M20. The capture thread's Sender dropped. Without this
            // arm, `rx.changed()` keeps resolving immediately with Err (there
            // is nothing left to wait for) and the outer `.is_err()` check —
            // which only sees the *outer* timeout, never this inner result —
            // let the loop spin at 100% CPU until the deadline.
            Ok(Err(_)) => {
                return (StatusCode::SERVICE_UNAVAILABLE, "capture thread gone").into_response();
            }
            Err(_) => {
                let f = rx.borrow().clone();
                return png_response(&f, true, &s.expensive).await;
            }
        }
    }
}

/// Decode the frame to a full-resolution RGB image. NV12 (macOS) converts
/// here, lazily; on the Linux MJPEG path we decode `jpeg` with turbojpeg.
fn decode_rgb(f: &FrameState) -> Option<ImageBuffer<Rgb<u8>, Vec<u8>>> {
    match &f.pixels {
        PixelData::Rgb(buf) => ImageBuffer::from_raw(f.width, f.height, buf.to_vec()),
        PixelData::Nv12 { y, cbcr } => Some(nv12_to_rgb(y, cbcr, f.width, f.height)),
        PixelData::Empty => {
            #[cfg(target_os = "linux")]
            if let Some(ref jpeg) = f.jpeg {
                return turbojpeg::decompress_image::<Rgb<u8>>(jpeg).ok();
            }
            None
        }
    }
}

/// Encode a preview JPEG from decoded pixels (the non-MJPEG fallback path).
/// Large NV12 frames are halved first — the human preview doesn't need 8 MP,
/// and 4:2:0 makes halving nearly free — then encoded with the fast
/// `jpeg-encoder` crate.
fn encode_preview_jpeg(f: &FrameState) -> Option<Vec<u8>> {
    const PREVIEW_MAX_WIDTH: u32 = 1920;
    let img = match &f.pixels {
        PixelData::Nv12 { y, cbcr } if f.width > PREVIEW_MAX_WIDTH => {
            nv12_to_rgb_half(y, cbcr, f.width, f.height)
        }
        PixelData::Nv12 { y, cbcr } => nv12_to_rgb(y, cbcr, f.width, f.height),
        PixelData::Rgb(buf) => ImageBuffer::from_raw(f.width, f.height, buf.to_vec())?,
        PixelData::Empty => return None,
    };
    let mut out = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut out, 80);
    encoder
        .encode(
            img.as_raw(),
            img.width() as u16,
            img.height() as u16,
            jpeg_encoder::ColorType::Rgb,
        )
        .ok()?;
    Some(out)
}

/// Encode the frame to PNG bytes. Shared by /snapshot and /ocr. Decode
/// (`decode_rgb`, which on Linux runs `turbojpeg::decompress_image`) and PNG
/// encoding both happen here, inline — callers reach it only through
/// [`encode_png_guarded`], which moves this off the async runtime.
fn encode_png(f: &FrameState) -> Option<Vec<u8>> {
    let img = decode_rgb(f)?;
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .ok()?;
    Some(bytes)
}

/// [`encode_png`], run on the blocking thread pool and gated by `expensive`
/// (Review M21). Decode + PNG encode is real CPU work — turbojpeg on Linux,
/// a full-resolution PNG encode everywhere — and running it inline on a
/// tokio worker blocks every other request that worker would otherwise
/// service. The semaphore bounds how many such encodes (plus OCR's own use
/// of it) can run at once, so repeated /snapshot or /ocr clicks queue rather
/// than piling up unbounded work.
async fn encode_png_guarded(f: Arc<FrameState>, expensive: &Semaphore) -> Option<Vec<u8>> {
    let _permit = expensive.acquire().await.ok()?;
    tokio::task::spawn_blocking(move || encode_png(&f))
        .await
        .unwrap_or(None)
}

/// Lazily encode the current RGB buffer to PNG. PNG for agent snapshots: text
/// edges matter for OCR and the dongle already adds MJPEG artifacts.
async fn png_response(f: &Arc<FrameState>, timed_out: bool, expensive: &Semaphore) -> Response {
    if f.effective_signal() == Signal::Stale {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::HeaderName::from_static("x-signal"), "stale")],
            "capture stalled; the last frame is too old to be the screen",
        )
            .into_response();
    }
    if f.signal == Signal::NoDevice || f.width == 0 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::HeaderName::from_static("x-signal"), "no_device")],
            "no capture device",
        )
            .into_response();
    }

    let bytes = match encode_png_guarded(Arc::clone(f), expensive).await {
        Some(b) => b,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "frame buffer size mismatch",
            )
                .into_response()
        }
    };

    let signal_str = signal_name(f.signal);

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png".to_string()),
            (
                header::HeaderName::from_static("x-signal"),
                signal_str.to_string(),
            ),
            (
                header::HeaderName::from_static("x-resolution-epoch"),
                f.resolution_epoch.to_string(),
            ),
            (
                header::HeaderName::from_static("x-frame-hash"),
                format!("{:016x}", f.hash),
            ),
            (
                header::HeaderName::from_static("x-timeout"),
                (timed_out as u8).to_string(),
            ),
        ],
        bytes,
    )
        .into_response()
}

/// multipart/x-mixed-replace MJPEG stream for the human browser preview.
/// Reads the same warm buffer as /snapshot — zero device contention.
/// When raw JPEG bytes are available (Linux MJPEG path), they are served
/// directly with zero server-side decode or re-encode. Otherwise we re-encode
/// from the decoded RGB buffer at quality 80.
async fn preview(State(s): State<AppState>) -> Response {
    let mut frames = s.frames.clone();

    let stream = async_stream::stream! {
        let mut interval = tokio::time::interval(Duration::from_millis(67));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Don't re-encode (or re-send) a frame the client already has; at
        // camera rates below the tick rate this halves the encode work.
        let mut last_served: Option<Instant> = None;

        loop {
            interval.tick().await;
            let f = frames.borrow_and_update().clone();

            if f.signal == Signal::NoDevice || f.width == 0 {
                continue;
            }
            if last_served == Some(f.captured_at) {
                continue;
            }

            // Fast path: raw JPEG bytes from the device — no decode/re-encode.
            let jpeg_bytes: Vec<u8> = if let Some(ref raw) = f.jpeg {
                raw.to_vec()
            } else {
                // Fallback: encode from native pixels (macOS NV12 / YUYV).
                // Review M21: real CPU work, so it runs off the async runtime
                // rather than blocking this stream's tokio worker (and every
                // other request sharing it) for the encode.
                let owned = f.clone();
                match tokio::task::spawn_blocking(move || encode_preview_jpeg(&owned)).await {
                    Ok(Some(b)) => b,
                    _ => continue,
                }
            };
            last_served = Some(f.captured_at);

            let part_header = format!(
                "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                jpeg_bytes.len()
            );
            let mut chunk = Vec::with_capacity(part_header.len() + jpeg_bytes.len() + 2);
            chunk.extend_from_slice(part_header.as_bytes());
            chunk.extend_from_slice(&jpeg_bytes);
            chunk.extend_from_slice(b"\r\n");

            yield Ok::<Bytes, std::io::Error>(Bytes::from(chunk));
        }
    };

    Response::builder()
        .header(
            header::CONTENT_TYPE,
            "multipart/x-mixed-replace;boundary=frame",
        )
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Locate the OCR tool: `PANIOLO_VISIONOCR` (paniolo sets this) wins, then a
/// `visionocr` installed next to our own executable (`paniolo setup` puts both
/// in the libexec dir), then a bare name resolved via PATH.
fn visionocr_bin() -> std::ffi::OsString {
    if let Some(bin) = std::env::var_os("PANIOLO_VISIONOCR") {
        return bin;
    }
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            for name in ["visionocr", "linuxocr", "winocr", "winocr.exe"] {
                let sibling = dir.join(name);
                if sibling.is_file() {
                    return sibling.into();
                }
            }
        }
    }
    "visionocr".into()
}

/// Wrap plain text from a pre-v1 helper in a v1 envelope.
///
/// A new daemon against an old installed helper should still read screens —
/// just without confidences — rather than failing in a way that looks like a
/// broken capture. The synthesized envelope names the binary so the cause is
/// visible, and carries no `lines`, because inventing boxes would be worse than
/// omitting them. See docs/ocr.md.
fn legacy_envelope(bin: &str, text: &str, width: u32, height: u32) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "engine": "unknown",
        "engine_detail": format!("pre-v1 helper ({bin}): plain text only, no confidences"),
        "width": width,
        "height": height,
        "text": text.trim_end_matches('\n'),
        "lines": [],
    })
}

/// Ceiling on one `visionocr`/`winocr`/`linuxocr` invocation. It wraps model
/// inference; a wedged model must not hold the request — and the `expensive`
/// permit it acquires below — forever. See [`wait_with_timeout`].
const OCR_TIMEOUT: Duration = Duration::from_secs(30);

/// Why [`wait_with_timeout`] did not return a completed `Output`.
enum WaitError {
    Io(std::io::Error),
    TimedOut,
}

/// Wait for `child` to finish, killing it if `timeout` elapses first (Review
/// M21). Relies on the caller having set `Command::kill_on_drop(true)`:
/// dropping `wait_with_output()`'s future on timeout drops the `Child`,
/// which tokio then kills.
async fn wait_with_timeout(
    child: tokio::process::Child,
    timeout: Duration,
) -> Result<std::process::Output, WaitError> {
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(WaitError::Io(e)),
        Err(_) => Err(WaitError::TimedOut),
    }
}

/// OCR the current warm frame by shelling out to the platform's OCR helper
/// (`visionocr` / `winocr` / `linuxocr`). The daemon links no OCR engine
/// itself — it pipes a PNG to the tool located by [`visionocr_bin`] and returns
/// the v1 envelope the helper emits under `--json` (see docs/ocr.md).
async fn ocr(State(s): State<AppState>) -> Response {
    let f = s.frames.borrow().clone();
    if f.effective_signal() == Signal::Stale {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "capture stalled; the last frame is too old to be the screen",
        )
            .into_response();
    }
    if f.signal == Signal::NoDevice || f.width == 0 {
        return (StatusCode::SERVICE_UNAVAILABLE, "no capture device").into_response();
    }
    // A dark/off display OCRs to empty text, which a caller can't tell apart
    // from a genuinely blank screen — report the missing signal instead.
    if f.signal == Signal::NoSignal {
        return (StatusCode::SERVICE_UNAVAILABLE, "no video signal").into_response();
    }
    let (fw, fh) = (f.width, f.height);
    let png = match encode_png_guarded(f, &s.expensive).await {
        Some(p) => p,
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "png encode failed").into_response(),
    };

    let bin = visionocr_bin();
    // Review M21: the same gate as PNG encoding, held across the subprocess
    // too — "visionocr" is the "unbounded helpers" a burst of /ocr clicks
    // used to be able to spawn.
    let _permit = match s.expensive.acquire().await {
        Ok(p) => p,
        Err(_) => return (StatusCode::SERVICE_UNAVAILABLE, "ocr gate closed").into_response(),
    };
    let mut child = match tokio::process::Command::new(&bin)
        .arg("--json")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::NOT_IMPLEMENTED,
                format!("visionocr unavailable ({}): {e}", bin.to_string_lossy()),
            )
                .into_response()
        }
    };

    // Write the PNG to stdin on a task while we collect stdout, so a large
    // frame can't deadlock the pipe.
    if let Some(mut stdin) = child.stdin.take() {
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(&png).await;
            // stdin dropped here -> EOF, so visionocr stops reading.
        });
    }

    let out = match wait_with_timeout(child, OCR_TIMEOUT).await {
        Ok(out) => out,
        Err(WaitError::TimedOut) => {
            return (
                StatusCode::GATEWAY_TIMEOUT,
                format!("visionocr timed out after {OCR_TIMEOUT:?}\n"),
            )
                .into_response()
        }
        Err(WaitError::Io(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("visionocr wait: {e}"),
            )
                .into_response()
        }
    };

    if out.status.success() {
        let name = bin.to_string_lossy().into_owned();
        // A v1 helper answers with the envelope. Anything else is treated
        // as a pre-v1 helper's plain text rather than an error — see
        // legacy_envelope.
        let body = match serde_json::from_slice::<serde_json::Value>(&out.stdout) {
            Ok(v) if v.get("version").is_some() => v,
            _ => {
                let text = String::from_utf8_lossy(&out.stdout);
                tracing::warn!(
                    "OCR helper {name} did not emit a v1 envelope; \
                     treating its output as plain text (upgrade it with `paniolo setup`)"
                );
                legacy_envelope(&name, &text, fw, fh)
            }
        };
        (
            [(header::CONTENT_TYPE, "application/json")],
            body.to_string(),
        )
            .into_response()
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("visionocr failed: {}", String::from_utf8_lossy(&out.stderr)),
        )
            .into_response()
    }
}

/// PANIOLO_TARGET (set by `paniolo video watch`/`console <target>`), or None
/// when unset/empty.
fn power_target() -> Option<String> {
    std::env::var("PANIOLO_TARGET")
        .ok()
        .filter(|t| !t.is_empty())
}

/// The 501 power endpoints return when no target is configured.
fn no_target_response() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        "PANIOLO_TARGET not set — start the daemon with: paniolo video watch <target>",
    )
        .into_response()
}

/// Run `paniolo <action…> <target>` and map its exit status to a Response. The
/// action endpoints (on/off/cycle) all funnel through here, so a request is the
/// only thing that ever changes the target's power.
async fn run_power_action(action: &[&str]) -> Response {
    let target = match power_target() {
        Some(t) => t,
        None => return no_target_response(),
    };
    let paniolo = std::env::var("PANIOLO_BIN").unwrap_or_else(|_| "paniolo".to_string());
    let mut args: Vec<&str> = action.to_vec();
    args.push(&target);
    match tokio::process::Command::new(&paniolo)
        .args(&args)
        .status()
        .await
    {
        Ok(s) if s.success() => (StatusCode::OK, "ok").into_response(),
        Ok(s) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("paniolo {} exited with {s}", action.join(" ")),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to run {paniolo}: {e}"),
        )
            .into_response(),
    }
}

/// `POST /power-cycle` — `paniolo power-cycle <target>`.
async fn power_cycle() -> Response {
    run_power_action(&["power-cycle"]).await
}

/// `POST /power-on` — `paniolo power on <target>`.
async fn power_on() -> Response {
    run_power_action(&["power", "on"]).await
}

/// `POST /power-off` — `paniolo power off <target>`.
async fn power_off() -> Response {
    run_power_action(&["power", "off"]).await
}

/// `GET /power` — capability + current state WITHOUT acting, so the dashboard
/// can probe availability and drive the on/off toggle on a timer without ever
/// toggling the target. 501 if no target; otherwise runs `paniolo power-state
/// <target>` and returns "on", "off", or "unknown".
async fn power_state() -> Response {
    let target = match power_target() {
        Some(t) => t,
        None => return no_target_response(),
    };
    let paniolo = std::env::var("PANIOLO_BIN").unwrap_or_else(|_| "paniolo".to_string());
    match tokio::process::Command::new(&paniolo)
        .args(["power-state", &target])
        .output()
        .await
    {
        Ok(o) if o.status.success() => {
            // `power-state` prints a human line like "Power ON  (pi5)"; pull the
            // on/off token out of it (case-insensitive, position-independent).
            let out = String::from_utf8_lossy(&o.stdout);
            let state = out
                .split_whitespace()
                .map(|t| t.to_ascii_lowercase())
                .find(|t| t == "on" || t == "off")
                .unwrap_or_else(|| "unknown".to_string());
            (StatusCode::OK, state).into_response()
        }
        Ok(_) => (StatusCode::OK, "unknown").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to run {paniolo}: {e}"),
        )
            .into_response(),
    }
}

async fn devices() -> Response {
    match crate::capture::enumerate() {
        Ok(list) => Json(
            list.into_iter()
                .map(|d| {
                    serde_json::json!({"index": d.index, "name": d.name, "misc": d.misc, "id": d.id})
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    }
}

fn signal_name(s: Signal) -> &'static str {
    match s {
        Signal::Stable => "stable",
        Signal::Stale => "stale",
        Signal::ModeSwitching => "mode_switching",
        Signal::NoSignal => "no_signal",
        Signal::NoDevice => "no_device",
    }
}

#[allow(unused_imports)]
use watch as _watch;

#[cfg(test)]
mod tests {
    use super::*;

    /// A pre-v1 helper's plain text must still reach the caller as an
    /// envelope. Failing instead would look to an agent like a broken capture
    /// rather than an out-of-date helper.
    #[test]
    fn legacy_envelope_wraps_plain_text() {
        let v = legacy_envelope("linuxocr", "login:\nPassword:\n", 1280, 720);
        assert_eq!(v["version"], 1);
        assert_eq!(v["text"], "login:\nPassword:");
        assert_eq!(v["width"], 1280);
        assert_eq!(v["height"], 720);
        // No invented lines: omitting them is honest, fabricating boxes is not.
        assert_eq!(v["lines"].as_array().map(|a| a.len()), Some(0));
        // The binary is named so the cause is visible in the response itself.
        assert!(v["engine_detail"].as_str().unwrap().contains("linuxocr"));
    }

    // ── Review M20: /snapshot must not spin when the capture thread is gone ──

    /// Before the fix, dropping the capture thread's `Sender` made
    /// `rx.changed()` resolve immediately with `Err` on every poll, and only
    /// the *outer* `tokio::time::timeout` was checked — so the loop spun at
    /// 100% CPU until the deadline, then answered with whatever `png_response`
    /// made of the last frame (here, "no capture device", since the receiver
    /// never saw anything else). The fix matches on the inner `Err` directly
    /// and returns "capture thread gone" without waiting out the deadline.
    /// A short deadline (200ms) keeps this test fast either way; the
    /// `tokio::time::timeout` wrapper around the whole call fails the test
    /// outright if the handler doesn't return promptly.
    #[tokio::test]
    async fn snapshot_reports_the_capture_thread_gone_instead_of_spinning() {
        use axum::body::Body;
        use axum::http::Request as HttpRequest;
        use tower::ServiceExt;

        let (tx, rx) = watch::channel(Arc::new(FrameState::no_device()));
        drop(tx); // the capture thread's Sender is gone

        let state = AppState::new(rx);
        let app = router(state, crate::auth::Auth::new("tok".into(), PUBLIC_ASSETS));

        // `wait=stable` forces `ready == false` on the first check (a
        // `no_device` frame is never `Stable`), so the handler must actually
        // reach `rx.changed()` rather than returning before ever calling it.
        let req = HttpRequest::builder()
            .uri("/snapshot?wait=stable&timeout=200")
            .header(header::HOST, "127.0.0.1:1")
            .header(header::AUTHORIZATION, "Bearer tok")
            .body(Body::empty())
            .unwrap();

        let resp = tokio::time::timeout(Duration::from_secs(2), app.oneshot(req))
            .await
            .expect("handler did not return promptly")
            .unwrap();

        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        // Distinguishes this path from the *other* 503s `png_response` can
        // give (stale / no_device), which set an `x-signal` header this path
        // does not.
        assert!(
            resp.headers()
                .get(header::HeaderName::from_static("x-signal"))
                .is_none(),
            "this 503 is the M20 path, not a png_response one"
        );
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"capture thread gone");
    }

    // ── Review M21: the OCR child is killed on timeout ───────────────────────

    /// Without a timeout, a wedged (or merely slow) `visionocr` process holds
    /// the request — and the `expensive` semaphore permit it took — forever.
    /// `sleep 5` stands in for a wedged helper: with a much shorter timeout,
    /// `wait_with_timeout` must return `TimedOut` promptly, and
    /// `kill_on_drop(true)` (set on the `Command` here, as `ocr()` sets it)
    /// must actually have killed the process by the time we check.
    #[cfg(unix)]
    #[tokio::test]
    async fn wait_with_timeout_kills_a_child_that_outlives_the_deadline() {
        let child = tokio::process::Command::new("sleep")
            .arg("5")
            .kill_on_drop(true)
            .spawn()
            .expect("spawn sleep");
        let pid = child.id().expect("pid") as i32;

        let start = Instant::now();
        let result = wait_with_timeout(child, Duration::from_millis(200)).await;
        assert!(matches!(result, Err(WaitError::TimedOut)));
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "did not return promptly: {:?}",
            start.elapsed()
        );

        // Give the kill a moment to land, then confirm the process is gone
        // (signal 0 only probes existence/permission, it sends nothing).
        tokio::time::sleep(Duration::from_millis(300)).await;
        let alive = unsafe { libc::kill(pid, 0) == 0 };
        assert!(!alive, "child pid {pid} should have been killed on timeout");
    }

    /// A child that finishes on its own, well inside the deadline, must be
    /// reported normally rather than as a timeout.
    #[cfg(unix)]
    #[tokio::test]
    async fn wait_with_timeout_returns_the_output_of_a_child_that_finishes_in_time() {
        let child = tokio::process::Command::new("echo")
            .arg("hi")
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn echo");
        let out = wait_with_timeout(child, Duration::from_secs(5))
            .await
            .unwrap_or_else(|_| panic!("should not time out"));
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }
}
