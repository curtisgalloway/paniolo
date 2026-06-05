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

use std::io::Cursor;
use std::process::Stdio;
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use image::{ImageBuffer, Rgb};
use serde::Deserialize;
use tokio::sync::watch;

use crate::capture_thread::FrameRx;
use crate::frame::{FrameState, Signal, StatusDto};
use crate::pixel::{nv12_to_rgb, nv12_to_rgb_half, PixelData};

#[derive(Clone)]
pub struct AppState {
    pub frames: FrameRx,
}

pub fn router(state: AppState) -> Router {
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
        .with_state(state)
}

async fn index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
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
                (true, _) => f.signal == Signal::Stable,
                (_, Some(h)) => f.hash != h,
                _ => true,
            }
        };

        if ready {
            let f = rx.borrow().clone();
            return png_response(&f, false);
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let f = rx.borrow().clone();
            return png_response(&f, true);
        }
        if tokio::time::timeout(remaining, rx.changed()).await.is_err() {
            let f = rx.borrow().clone();
            return png_response(&f, true);
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

/// Encode the frame to PNG bytes. Shared by /snapshot and /ocr.
fn encode_png(f: &FrameState) -> Option<Vec<u8>> {
    let img = decode_rgb(f)?;
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .ok()?;
    Some(bytes)
}

/// Lazily encode the current RGB buffer to PNG. PNG for agent snapshots: text
/// edges matter for OCR and the dongle already adds MJPEG artifacts.
fn png_response(f: &FrameState, timed_out: bool) -> Response {
    if f.signal == Signal::NoDevice || f.width == 0 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::HeaderName::from_static("x-signal"), "no_device")],
            "no capture device",
        )
            .into_response();
    }

    let bytes = match encode_png(f) {
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
                match encode_preview_jpeg(&f) {
                    Some(b) => b,
                    None => continue,
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

/// OCR the current warm frame by shelling out to the `visionocr` tool (Apple
/// Vision). The daemon doesn't link Vision itself — it pipes a PNG to whatever
/// `PANIOLO_VISIONOCR` points at (paniolo sets this), falling back to PATH.
async fn ocr(State(s): State<AppState>) -> Response {
    let f = s.frames.borrow().clone();
    if f.signal == Signal::NoDevice || f.width == 0 {
        return (StatusCode::SERVICE_UNAVAILABLE, "no capture device").into_response();
    }
    let png = match encode_png(&f) {
        Some(p) => p,
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "png encode failed").into_response(),
    };

    let bin = std::env::var("PANIOLO_VISIONOCR").unwrap_or_else(|_| "visionocr".to_string());
    let mut child = match tokio::process::Command::new(&bin)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::NOT_IMPLEMENTED,
                format!("visionocr unavailable ({bin}): {e}"),
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

    match child.wait_with_output().await {
        Ok(out) if out.status.success() => (
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            out.stdout,
        )
            .into_response(),
        Ok(out) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("visionocr failed: {}", String::from_utf8_lossy(&out.stderr)),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("visionocr wait: {e}"),
        )
            .into_response(),
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
        Signal::ModeSwitching => "mode_switching",
        Signal::NoSignal => "no_signal",
        Signal::NoDevice => "no_device",
    }
}

#[allow(unused_imports)]
use watch as _watch;
