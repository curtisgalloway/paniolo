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

//! Capture backend abstraction + the nokhwa implementation.
//!
//! Even though nokhwa already unifies V4L2/AVFoundation, we wrap it so a
//! `ffmpeg-next` fallback or an in-memory fake (for tests) can slot in behind
//! the same trait. Do NOT hardcode the MS2109's 534D:2109 anywhere — select by
//! capability so any UVC grabber works.

use anyhow::{anyhow, Context, Result};
use image::RgbImage;

use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
    Resolution,
};
use nokhwa::{query, Camera};

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub index: u32,
    pub name: String,
    pub misc: String,
}

/// How the user asked us to pick a device.
#[derive(Clone, Debug)]
pub enum DeviceSpec {
    /// Highest-resolution external (non-built-in) capture device.
    Auto,
    Index(u32),
    /// Case-insensitive substring match on the device name.
    Name(String),
}

impl DeviceSpec {
    /// Parse the CLI `--device` value: empty/"auto" -> Auto, all-digits ->
    /// Index, else Name.
    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("auto") {
            DeviceSpec::Auto
        } else if let Ok(i) = s.parse::<u32>() {
            DeviceSpec::Index(i)
        } else if let Some(idx) = s
            .strip_prefix("/dev/video")
            .and_then(|n| n.parse::<u32>().ok())
        {
            DeviceSpec::Index(idx)
        } else {
            DeviceSpec::Name(s.to_string())
        }
    }
}

/// Names we treat as built-in webcams and skip under `Auto`.
const BUILTIN_HINTS: &[&str] = &["facetime", "built-in", "integrated", "isight"];

pub trait CaptureBackend {
    /// One blocking grab + decode to RGB8. Errors are normal (dropped frames
    /// during mode switches) and the caller maps them to NoSignal.
    fn frame(&mut self) -> Result<RgbImage>;
    fn dims(&self) -> (u32, u32);
}

pub fn enumerate() -> Result<Vec<DeviceInfo>> {
    let cams = query(ApiBackend::Auto).context("nokhwa device query failed")?;
    Ok(cams
        .into_iter()
        .map(|c| DeviceInfo {
            index: match c.index() {
                CameraIndex::Index(i) => *i,
                CameraIndex::String(_) => u32::MAX,
            },
            name: c.human_name(),
            misc: c.description().to_string(),
        })
        .collect())
}

/// Resolve a `DeviceSpec` to a concrete camera index.
fn resolve(spec: &DeviceSpec) -> Result<u32> {
    let devices = enumerate()?;
    if devices.is_empty() {
        return Err(anyhow!("no capture devices found"));
    }
    match spec {
        DeviceSpec::Index(i) => Ok(*i),
        DeviceSpec::Name(sub) => {
            let sub = sub.to_lowercase();
            devices
                .iter()
                .find(|d| d.name.to_lowercase().contains(&sub))
                .map(|d| d.index)
                .ok_or_else(|| anyhow!("no device matching name {:?}", sub))
        }
        DeviceSpec::Auto => {
            // Prefer the first non-built-in device. If everything looks
            // built-in, fall back to index 0.
            let external = devices.iter().find(|d| {
                let n = d.name.to_lowercase();
                !BUILTIN_HINTS.iter().any(|h| n.contains(h))
            });
            Ok(external.unwrap_or(&devices[0]).index)
        }
    }
}

pub struct NokhwaBackend {
    cam: Camera,
    dims: (u32, u32),
}

impl NokhwaBackend {
    pub fn open(spec: &DeviceSpec) -> Result<Self> {
        let idx = resolve(spec)?;

        // Some UVC dongles (e.g. MS2109) throw an NSException when AVFoundation
        // tries to set a format they advertise but can't actually deliver. Try
        // formats in preference order, catching panics from nokhwa's Obj-C
        // exception handlers, until one succeeds.
        //
        // Preference: MJPEG (dongle-native) > YUYV > absolute-highest.
        // For text/OCR capture we want max resolution; fps doesn't matter.
        // Preference order: 1080p MJPEG (dongle-native), 720p MJPEG, 1080p YUYV, absolute highest.
        let format_types: &[RequestedFormatType] = &[
            RequestedFormatType::Closest(CameraFormat::new(
                Resolution::new(1920, 1080),
                FrameFormat::MJPEG,
                30,
            )),
            RequestedFormatType::Closest(CameraFormat::new(
                Resolution::new(1280, 720),
                FrameFormat::MJPEG,
                30,
            )),
            RequestedFormatType::Closest(CameraFormat::new(
                Resolution::new(1920, 1080),
                FrameFormat::YUYV,
                30,
            )),
            RequestedFormatType::AbsoluteHighestResolution,
        ];

        let mut last_err: anyhow::Error = anyhow!("no formats to try");
        for &fmt_type in format_types {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| try_open(idx, fmt_type)));
            match result {
                Ok(Ok(backend)) => return Ok(backend),
                Ok(Err(e)) => {
                    tracing::debug!("format {:?} error: {e:#}", fmt_type);
                    last_err = e;
                }
                Err(_panic) => {
                    tracing::debug!("format {:?} panicked (unsupported by AVFoundation)", fmt_type);
                    last_err = anyhow!("format {:?} not supported by device {idx}", fmt_type);
                }
            }
        }
        Err(last_err)
    }
}

fn try_open(idx: u32, fmt_type: RequestedFormatType) -> Result<NokhwaBackend> {
    let requested = RequestedFormat::new::<RgbFormat>(fmt_type);
    let mut cam = Camera::new(CameraIndex::Index(idx), requested)
        .map_err(|e| classify_open_error(e, idx))?;
    cam.open_stream()
        .map_err(|e| classify_open_error(e, idx))?;
    let res = cam.resolution();
    Ok(NokhwaBackend {
        cam,
        dims: (res.width(), res.height()),
    })
}

impl CaptureBackend for NokhwaBackend {
    fn frame(&mut self) -> Result<RgbImage> {
        let buf = self.cam.frame().context("frame grab failed")?;
        let decoded = buf
            .decode_image::<RgbFormat>()
            .context("MJPEG/YUV decode failed")?;
        self.dims = (decoded.width(), decoded.height());
        Ok(decoded)
    }

    fn dims(&self) -> (u32, u32) {
        self.dims
    }
}

/// Turn nokhwa's open errors into something actionable. The macOS "device in
/// use" case is real: Chrome and UVCAssistant can hold handles on the MS2109.
fn classify_open_error(e: nokhwa::NokhwaError, idx: u32) -> anyhow::Error {
    let s = e.to_string().to_lowercase();
    if s.contains("busy") || s.contains("in use") || s.contains("access") {
        anyhow!(
            "capture device {idx} is busy (likely held by another app, e.g. \
             a browser or Apple's UVCAssistant on macOS). Close it and retry. \
             underlying: {e}"
        )
    } else {
        anyhow!("failed to open capture device {idx}: {e}")
    }
}
