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

//! Capture backend abstraction.
//!
//! On Linux we bypass nokhwa and use the `v4l` crate directly so we can:
//!   - Call `stream.set_timeout()` to avoid an indefinite VIDIOC_DQBUF block
//!   - Keep raw MJPEG bytes for zero-cost preview serving
//!   - Use turbojpeg (libjpeg-turbo) for fast RGB decode when signal detection
//!     or OCR needs pixel data
//!
//! On macOS the nokhwa + AVFoundation path is kept as-is.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use image::RgbImage;

use nokhwa::utils::{ApiBackend, CameraIndex};
use nokhwa::query;

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub index: u32,
    pub name: String,
    pub misc: String,
}

/// How the user asked us to pick a device.
#[derive(Clone, Debug)]
pub enum DeviceSpec {
    Auto,
    Index(u32),
    Name(String),
}

impl DeviceSpec {
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

const BUILTIN_HINTS: &[&str] = &["facetime", "built-in", "integrated", "isight"];

/// One captured frame. `jpeg` carries raw MJPEG bytes when available (Linux
/// MJPEG path); the preview endpoint serves these directly with zero server
/// decode/re-encode. `rgb` is always populated for signal detection (ahash,
/// is_no_signal); on the Linux path it comes from turbojpeg (fast).
pub struct CapturedFrame {
    /// Raw MJPEG bytes from the device. Present on the Linux v4l path when the
    /// device is in MJPEG mode; None on macOS or YUYV sources.
    pub jpeg: Option<Arc<[u8]>>,
    /// Decoded RGB8 pixels for signal detection. Always populated.
    pub rgb: RgbImage,
}

pub trait CaptureBackend {
    fn frame(&mut self) -> Result<CapturedFrame>;
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

pub fn resolve(spec: &DeviceSpec) -> Result<u32> {
    match spec {
        DeviceSpec::Index(i) => Ok(*i),
        _ => {
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
                    let external = devices.iter().find(|d| {
                        let n = d.name.to_lowercase();
                        !BUILTIN_HINTS.iter().any(|h| n.contains(h))
                    });
                    Ok(external.unwrap_or(&devices[0]).index)
                }
            }
        }
    }
}

pub fn open_backend(spec: &DeviceSpec) -> Result<Box<dyn CaptureBackend>> {
    #[cfg(target_os = "linux")]
    {
        linux::LinuxV4LBackend::open(spec).map(|b| Box::new(b) as Box<dyn CaptureBackend>)
    }
    #[cfg(not(target_os = "linux"))]
    {
        macos::NokhwaBackend::open(spec).map(|b| Box::new(b) as Box<dyn CaptureBackend>)
    }
}

// ── Linux backend ─────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod linux {
    use std::io;
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::{anyhow, Context, Result};
    use image::RgbImage;
    use v4l::buffer::Type;
    use v4l::device::Device;
    use v4l::format::{Format, FourCC};
    use v4l::io::mmap::Stream;
    use v4l::io::traits::CaptureStream;
    use v4l::video::Capture;

    use super::{CaptureBackend, CapturedFrame, DeviceSpec, resolve};

    const FORMATS: &[(u32, u32, &[u8; 4])] = &[
        (1280, 720,  b"MJPG"),
        (1920, 1080, b"MJPG"),
        (1280, 720,  b"YUYV"),
        (640,  480,  b"YUYV"),
    ];

    const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

    pub struct LinuxV4LBackend {
        stream: Stream<'static>,
        dev: Box<Device>,
        dims: (u32, u32),
        is_mjpeg: bool,
    }

    impl LinuxV4LBackend {
        pub fn open(spec: &DeviceSpec) -> Result<Self> {
            let idx = resolve(spec)?;
            let dev = Box::new(
                Device::new(idx as usize).map_err(|e| anyhow!("open /dev/video{idx}: {e}"))?,
            );

            let mut last_err = anyhow!("no formats succeeded");
            for &(w, h, fourcc) in FORMATS {
                let fmt = Format::new(w, h, FourCC::new(fourcc));
                if dev.set_format(&fmt).is_err() {
                    continue;
                }
                let dev_ref: &'static Device = unsafe { &*(dev.as_ref() as *const Device) };
                match Stream::with_buffers(dev_ref, Type::VideoCapture, 4) {
                    Ok(mut stream) => {
                        stream.set_timeout(FRAME_TIMEOUT);
                        let actual_fmt = dev.format().unwrap_or(fmt);
                        let is_mjpeg = actual_fmt.fourcc == FourCC::new(b"MJPG");
                        tracing::info!(
                            "capture opened {}x{} {:?}",
                            actual_fmt.width, actual_fmt.height,
                            if is_mjpeg { "MJPEG" } else { "YUYV" }
                        );
                        return Ok(LinuxV4LBackend {
                            stream,
                            dev,
                            dims: (actual_fmt.width, actual_fmt.height),
                            is_mjpeg,
                        });
                    }
                    Err(e) => {
                        last_err = anyhow!("stream init {w}x{h}: {e}");
                    }
                }
            }
            Err(last_err)
        }
    }

    impl CaptureBackend for LinuxV4LBackend {
        fn frame(&mut self) -> Result<CapturedFrame> {
            let (buf, _meta) = self.stream.next().map_err(|e| {
                if e.kind() == io::ErrorKind::TimedOut {
                    anyhow!("frame timeout (device stalled)")
                } else {
                    anyhow!("VIDIOC_DQBUF: {e}")
                }
            })?;

            if self.is_mjpeg {
                // Keep a copy of the raw JPEG bytes for zero-cost preview serving.
                let jpeg_bytes: Arc<[u8]> = Arc::from(buf.to_vec().into_boxed_slice());

                // Decode with turbojpeg (libjpeg-turbo) for signal detection.
                // ~5ms at 720p vs ~50ms with the pure-Rust image crate.
                let rgb = turbojpeg::decompress_image::<image::Rgb<u8>>(buf)
                    .context("turbojpeg MJPEG decode failed")?;
                let (w, h) = (rgb.width(), rgb.height());
                self.dims = (w, h);

                Ok(CapturedFrame { jpeg: Some(jpeg_bytes), rgb })
            } else {
                // YUYV: no raw JPEG, decode to RGB for signal detection and storage.
                let fmt = self.dev.format().ok();
                let (w, h) = fmt
                    .as_ref()
                    .map(|f| (f.width, f.height))
                    .unwrap_or(self.dims);
                self.dims = (w, h);
                Ok(CapturedFrame { jpeg: None, rgb: yuyv_to_rgb(buf, w, h) })
            }
        }

        fn dims(&self) -> (u32, u32) {
            self.dims
        }
    }

    fn yuyv_to_rgb(buf: &[u8], w: u32, h: u32) -> RgbImage {
        let mut rgb = RgbImage::new(w, h);
        let pairs = (w * h / 2) as usize;
        for i in 0..pairs {
            let base = i * 4;
            if base + 3 >= buf.len() { break; }
            let y0 = buf[base] as f32;
            let cb = buf[base + 1] as f32 - 128.0;
            let y1 = buf[base + 2] as f32;
            let cr = buf[base + 3] as f32 - 128.0;
            let to_u8 = |v: f32| v.clamp(0.0, 255.0) as u8;
            let r = |y: f32| to_u8(y + 1.402 * cr);
            let g = |y: f32| to_u8(y - 0.344 * cb - 0.714 * cr);
            let b = |y: f32| to_u8(y + 1.772 * cb);
            let x0 = ((i * 2) % w as usize) as u32;
            let y_row = ((i * 2) / w as usize) as u32;
            if x0 < w && y_row < h {
                rgb.put_pixel(x0, y_row, image::Rgb([r(y0), g(y0), b(y0)]));
            }
            if x0 + 1 < w && y_row < h {
                rgb.put_pixel(x0 + 1, y_row, image::Rgb([r(y1), g(y1), b(y1)]));
            }
        }
        rgb
    }
}

// ── macOS backend ─────────────────────────────────────────────────────────────

#[cfg(not(target_os = "linux"))]
mod macos {
    use std::sync::Arc;

    use anyhow::{anyhow, Context, Result};
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{
        CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
    };
    use nokhwa::Camera;

    use super::{CaptureBackend, CapturedFrame, DeviceSpec, resolve};

    pub struct NokhwaBackend {
        cam: Camera,
        dims: (u32, u32),
    }

    impl NokhwaBackend {
        pub fn open(spec: &DeviceSpec) -> Result<Self> {
            let idx = resolve(spec)?;
            let format_types: &[RequestedFormatType] = &[
                RequestedFormatType::Closest(CameraFormat::new(
                    Resolution::new(1280, 720),
                    FrameFormat::MJPEG,
                    30,
                )),
                RequestedFormatType::Closest(CameraFormat::new(
                    Resolution::new(1920, 1080),
                    FrameFormat::MJPEG,
                    30,
                )),
                RequestedFormatType::AbsoluteHighestResolution,
            ];

            let mut last_err: anyhow::Error = anyhow!("no formats to try");
            for &fmt_type in format_types {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    try_open(idx, fmt_type)
                }));
                match result {
                    Ok(Ok(backend)) => return Ok(backend),
                    Ok(Err(e)) => { last_err = e; }
                    Err(_) => { last_err = anyhow!("format {:?} not supported", fmt_type); }
                }
            }
            Err(last_err)
        }
    }

    fn try_open(idx: u32, fmt_type: RequestedFormatType) -> Result<NokhwaBackend> {
        let requested = RequestedFormat::new::<RgbFormat>(fmt_type);
        let mut cam = Camera::new(CameraIndex::Index(idx), requested)
            .map_err(|e| anyhow!("failed to open capture device {idx}: {e}"))?;
        cam.open_stream()
            .map_err(|e| anyhow!("failed to open capture device {idx}: {e}"))?;
        let res = cam.resolution();
        Ok(NokhwaBackend { cam, dims: (res.width(), res.height()) })
    }

    impl CaptureBackend for NokhwaBackend {
        fn frame(&mut self) -> Result<CapturedFrame> {
            let buf = self.cam.frame().context("frame grab failed")?;
            let decoded = buf.decode_image::<RgbFormat>().context("decode failed")?;
            self.dims = (decoded.width(), decoded.height());
            Ok(CapturedFrame {
                jpeg: None,  // nokhwa gives decoded pixels, not raw JPEG
                rgb: decoded,
            })
        }

        fn dims(&self) -> (u32, u32) {
            self.dims
        }
    }
}
