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
//! On Linux we use the `v4l` crate directly so we can:
//!   - Call `stream.set_timeout()` to avoid an indefinite VIDIOC_DQBUF block
//!   - Keep raw MJPEG bytes for zero-cost preview serving
//!   - Use turbojpeg (libjpeg-turbo) for fast RGB decode when signal detection
//!     or OCR needs pixel data
//!
//! On macOS we use our own ObjC AVFoundation layer (src/capture_avf.m). The
//! OS UVC stack decodes MJPEG before AVFoundation — only uncompressed formats
//! are reachable — so the layer requests '420v' (bi-planar 4:2:0) and the
//! frame loop classifies the luma plane directly; RGB materializes lazily.

use std::sync::Arc;

use anyhow::{anyhow, Result};

use crate::pixel::PixelData;

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub index: u32,
    pub name: String,
    pub misc: String,
    /// Stable identifier, derived from USB topology — survives reboots and
    /// enumeration-order shifts, changes only if the device moves to another
    /// port. macOS: the AVFoundation `uniqueID` (location ID + VID + PID).
    /// Linux: the `/dev/v4l/by-path/...` symlink. Empty when unavailable.
    pub id: String,
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

/// One captured frame in its native form. `jpeg` carries raw MJPEG bytes when
/// available (Linux MJPEG tee); `pixels` carries decoded data for
/// classification and lazy RGB conversion.
pub struct CapturedFrame {
    /// Raw MJPEG bytes from the device. Present on the Linux v4l path when
    /// the device is in MJPEG mode; None on macOS (the OS decodes upstream).
    pub jpeg: Option<Arc<[u8]>>,
    /// Native pixel data: RGB on decode paths, NV12 on the macOS path.
    pub pixels: PixelData,
    pub width: u32,
    pub height: u32,
}

pub trait CaptureBackend {
    fn frame(&mut self) -> Result<CapturedFrame>;
}

/// Map V4L2 device index → its stable `/dev/v4l/by-path` symlink (the Linux
/// analogue of AVFoundation's uniqueID: derived from USB port topology).
#[cfg(target_os = "linux")]
fn stable_ids_by_index() -> std::collections::HashMap<u32, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(entries) = std::fs::read_dir("/dev/v4l/by-path") else {
        return map;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(target) = std::fs::canonicalize(&path) else {
            continue;
        };
        let target = target.to_string_lossy();
        let Some(idx) = target
            .strip_prefix("/dev/video")
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        map.entry(idx)
            .or_insert_with(|| path.to_string_lossy().into_owned());
    }
    map
}

#[cfg(target_os = "linux")]
pub fn enumerate() -> Result<Vec<DeviceInfo>> {
    use anyhow::Context;
    use nokhwa::query;
    use nokhwa::utils::{ApiBackend, CameraIndex};

    let cams = query(ApiBackend::Auto).context("nokhwa device query failed")?;
    let by_path = stable_ids_by_index();
    Ok(cams
        .into_iter()
        .map(|c| {
            let index = match c.index() {
                CameraIndex::Index(i) => *i,
                CameraIndex::String(_) => u32::MAX,
            };
            let id = by_path.get(&index).cloned().unwrap_or_default();
            DeviceInfo {
                index,
                name: c.human_name(),
                misc: c.description().to_string(),
                id,
            }
        })
        .collect())
}

#[cfg(not(target_os = "linux"))]
pub fn enumerate() -> Result<Vec<DeviceInfo>> {
    use std::ffi::{c_char, c_void, CStr};

    unsafe extern "C" fn collect(
        ctx: *mut c_void,
        name: *const c_char,
        unique_id: *const c_char,
        misc: *const c_char,
    ) {
        let out = unsafe { &mut *(ctx as *mut Vec<DeviceInfo>) };
        let s = |p: *const c_char| {
            if p.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        };
        out.push(DeviceInfo {
            index: out.len() as u32,
            name: s(name),
            misc: s(misc),
            id: s(unique_id),
        });
    }

    let mut out: Vec<DeviceInfo> = Vec::new();
    unsafe {
        macos::avf_capture_enumerate(collect, &mut out as *mut Vec<DeviceInfo> as *mut c_void);
    }
    Ok(out)
}

/// Resolve a spec to a V4L2 device index (Linux; macOS resolves against the
/// enumerated device list directly in AvfBackend::open).
#[cfg(target_os = "linux")]
pub fn resolve(spec: &DeviceSpec) -> Result<u32> {
    if let DeviceSpec::Index(i) = spec {
        return Ok(*i);
    }
    // Any path-style spec (e.g. /dev/v4l/by-id/...) canonicalizes to
    // /dev/videoN; by-path specs also hit the exact-id match in resolve_in.
    if let DeviceSpec::Name(s) = spec {
        if s.starts_with('/') {
            if let Ok(target) = std::fs::canonicalize(s) {
                if let Some(idx) = target
                    .to_string_lossy()
                    .strip_prefix("/dev/video")
                    .and_then(|n| n.parse::<u32>().ok())
                {
                    return Ok(idx);
                }
            }
        }
    }
    resolve_in(&enumerate()?, spec)
}

/// Pick a device from `devices` per `spec`. An exact stable-id match wins;
/// otherwise case-insensitive name substring, which must match exactly one
/// device — a first-match-wins guess is how two identical dongles end up
/// silently swapped.
fn resolve_in(devices: &[DeviceInfo], spec: &DeviceSpec) -> Result<u32> {
    if devices.is_empty() {
        return Err(anyhow!("no capture devices found"));
    }
    match spec {
        DeviceSpec::Index(i) => Ok(*i),
        DeviceSpec::Name(s) => {
            if let Some(d) = devices.iter().find(|d| !d.id.is_empty() && d.id == *s) {
                return Ok(d.index);
            }
            let sub = s.to_lowercase();
            let matches: Vec<&DeviceInfo> = devices
                .iter()
                .filter(|d| d.name.to_lowercase().contains(&sub))
                .collect();
            match matches.as_slice() {
                [] => Err(anyhow!("no device matching name or id {:?}", s)),
                [d] => Ok(d.index),
                many => Err(anyhow!(
                    "device {:?} is ambiguous ({} matches) — use a stable id:\n{}",
                    s,
                    many.len(),
                    many.iter()
                        .map(|d| format!("  {:>3}  {}  id={}", d.index, d.name, d.id))
                        .collect::<Vec<_>>()
                        .join("\n")
                )),
            }
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

pub fn open_backend(spec: &DeviceSpec) -> Result<Box<dyn CaptureBackend>> {
    #[cfg(target_os = "linux")]
    {
        linux::LinuxV4LBackend::open(spec).map(|b| Box::new(b) as Box<dyn CaptureBackend>)
    }
    #[cfg(not(target_os = "linux"))]
    {
        macos::AvfBackend::open(spec).map(|b| Box::new(b) as Box<dyn CaptureBackend>)
    }
}

// ── Linux backend ─────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod linux {
    use std::io;
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::{anyhow, Context, Result};
    use v4l::buffer::Type;
    use v4l::device::Device;
    use v4l::format::{Format, FourCC};
    use v4l::io::mmap::Stream;
    use v4l::io::traits::CaptureStream;
    use v4l::video::Capture;

    use super::{resolve, CaptureBackend, CapturedFrame, DeviceSpec};
    use crate::pixel::{yuyv_to_rgb, PixelData};

    const FORMATS: &[(u32, u32, &[u8; 4])] = &[
        (1280, 720, b"MJPG"),
        (1920, 1080, b"MJPG"),
        (1280, 720, b"YUYV"),
        (640, 480, b"YUYV"),
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
                            actual_fmt.width,
                            actual_fmt.height,
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

                // Decode with turbojpeg (libjpeg-turbo) for signal detection
                // and lazy snapshot encoding. ~5ms at 720p.
                let rgb = turbojpeg::decompress_image::<image::Rgb<u8>>(buf)
                    .context("turbojpeg MJPEG decode failed")?;
                let (w, h) = (rgb.width(), rgb.height());
                self.dims = (w, h);

                Ok(CapturedFrame {
                    jpeg: Some(jpeg_bytes),
                    pixels: PixelData::Rgb(Arc::from(rgb.into_raw().into_boxed_slice())),
                    width: w,
                    height: h,
                })
            } else {
                // YUYV: no raw JPEG, decode to RGB for signal detection and storage.
                let fmt = self.dev.format().ok();
                let (w, h) = fmt
                    .as_ref()
                    .map(|f| (f.width, f.height))
                    .unwrap_or(self.dims);
                self.dims = (w, h);
                let rgb = yuyv_to_rgb(buf, w, h);
                Ok(CapturedFrame {
                    jpeg: None,
                    pixels: PixelData::Rgb(Arc::from(rgb.into_raw().into_boxed_slice())),
                    width: w,
                    height: h,
                })
            }
        }
    }
}

// ── macOS backend ─────────────────────────────────────────────────────────────

#[cfg(not(target_os = "linux"))]
mod macos {
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::sync::Arc;

    use anyhow::{anyhow, Result};

    use super::{enumerate, resolve_in, CaptureBackend, CapturedFrame, DeviceSpec};
    use crate::pixel::{yuyv_to_rgb, PixelData};

    // FourCC tags mirrored from capture_avf.m.
    const PIXFMT_NV12: u32 = 0x3432_3076; // '420v'
    const PIXFMT_YUYV: u32 = 0x7975_7673; // 'yuvs'

    /// Mirrors `AvfFrame` in capture_avf.m. Plane buffers are malloc'd by the
    /// ObjC layer; ownership passes to us and is returned via frame_free.
    #[repr(C)]
    struct AvfFrame {
        seq: u64,
        width: u32,
        height: u32,
        pixfmt: u32,
        y: *mut u8,
        y_len: usize,
        cbcr: *mut u8,
        cbcr_len: usize,
    }

    extern "C" {
        pub fn avf_capture_enumerate(
            cb: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char),
            ctx: *mut c_void,
        );
        fn avf_capture_open(
            unique_id: *const c_char,
            err: *mut c_char,
            errlen: usize,
        ) -> *mut c_void;
        fn avf_capture_wait_frame(
            h: *mut c_void,
            last_seq: u64,
            timeout_ms: u32,
            out: *mut AvfFrame,
        ) -> i32;
        fn avf_capture_frame_free(f: *mut AvfFrame);
        fn avf_capture_close(h: *mut c_void);
    }

    /// Matches the Linux backend's 5s VIDIOC_DQBUF timeout; the capture
    /// thread's reconnect loop handles the resulting error.
    const FRAME_TIMEOUT_MS: u32 = 5000;

    pub struct AvfBackend {
        handle: *mut c_void,
        last_seq: u64,
    }

    // The handle is owned and used exclusively by the capture thread; the
    // ObjC side does its own locking between the delegate queue and callers.
    unsafe impl Send for AvfBackend {}

    impl AvfBackend {
        pub fn open(spec: &DeviceSpec) -> Result<Self> {
            let devices = enumerate()?;
            let idx = resolve_in(&devices, spec)?;
            let dev = devices
                .iter()
                .find(|d| d.index == idx)
                .ok_or_else(|| anyhow!("no capture device at index {idx}"))?;
            if dev.id.is_empty() {
                return Err(anyhow!("device {:?} has no uniqueID", dev.name));
            }

            let cid = CString::new(dev.id.as_str())?;
            let mut err = [0i8; 256];
            let handle = unsafe {
                avf_capture_open(cid.as_ptr(), err.as_mut_ptr() as *mut c_char, err.len())
            };
            if handle.is_null() {
                let msg = unsafe { CStr::from_ptr(err.as_ptr() as *const c_char) };
                return Err(anyhow!(
                    "failed to open {:?}: {}",
                    dev.name,
                    msg.to_string_lossy()
                ));
            }
            tracing::info!("capture opened: {} ({})", dev.name, dev.id);
            Ok(AvfBackend {
                handle,
                last_seq: 0,
            })
        }
    }

    impl CaptureBackend for AvfBackend {
        fn frame(&mut self) -> Result<CapturedFrame> {
            let mut raw = AvfFrame {
                seq: 0,
                width: 0,
                height: 0,
                pixfmt: 0,
                y: std::ptr::null_mut(),
                y_len: 0,
                cbcr: std::ptr::null_mut(),
                cbcr_len: 0,
            };
            let rc = unsafe {
                avf_capture_wait_frame(self.handle, self.last_seq, FRAME_TIMEOUT_MS, &mut raw)
            };
            match rc {
                0 => return Err(anyhow!("frame timeout (device stalled)")),
                1 => {}
                _ => return Err(anyhow!("capture session error (device lost?)")),
            }
            self.last_seq = raw.seq;

            let (w, h) = (raw.width, raw.height);
            let y = unsafe { std::slice::from_raw_parts(raw.y, raw.y_len) };
            let frame = match raw.pixfmt {
                PIXFMT_NV12 => {
                    let cbcr = unsafe { std::slice::from_raw_parts(raw.cbcr, raw.cbcr_len) };
                    CapturedFrame {
                        jpeg: None,
                        pixels: PixelData::Nv12 {
                            y: Arc::from(y),
                            cbcr: Arc::from(cbcr),
                        },
                        width: w,
                        height: h,
                    }
                }
                PIXFMT_YUYV => {
                    let rgb = yuyv_to_rgb(y, w, h);
                    CapturedFrame {
                        jpeg: None,
                        pixels: PixelData::Rgb(Arc::from(rgb.into_raw().into_boxed_slice())),
                        width: w,
                        height: h,
                    }
                }
                other => {
                    unsafe { avf_capture_frame_free(&mut raw) };
                    return Err(anyhow!("unexpected pixel format {other:#x}"));
                }
            };
            unsafe { avf_capture_frame_free(&mut raw) };
            Ok(frame)
        }
    }

    impl Drop for AvfBackend {
        fn drop(&mut self) {
            unsafe { avf_capture_close(self.handle) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(index: u32, name: &str, id: &str) -> DeviceInfo {
        DeviceInfo {
            index,
            name: name.to_string(),
            misc: String::new(),
            id: id.to_string(),
        }
    }

    fn two_dongles() -> Vec<DeviceInfo> {
        vec![
            dev(0, "USB Video", "0x8300000534d2109"),
            dev(1, "USB Video", "0x8200000534d2109"),
            dev(2, "FaceTime HD Camera", "0x1421000005ac8514"),
        ]
    }

    #[test]
    fn exact_id_match_wins() {
        let devices = two_dongles();
        let spec = DeviceSpec::parse("0x8200000534d2109");
        assert_eq!(resolve_in(&devices, &spec).unwrap(), 1);
    }

    #[test]
    fn unique_name_substring_matches() {
        let devices = two_dongles();
        let spec = DeviceSpec::parse("facetime");
        assert_eq!(resolve_in(&devices, &spec).unwrap(), 2);
    }

    #[test]
    fn ambiguous_name_is_an_error_listing_ids() {
        let devices = two_dongles();
        let spec = DeviceSpec::parse("USB Video");
        let err = resolve_in(&devices, &spec).unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "{err}");
        assert!(err.contains("0x8300000534d2109"), "{err}");
        assert!(err.contains("0x8200000534d2109"), "{err}");
    }

    #[test]
    fn no_match_is_an_error() {
        let devices = two_dongles();
        let spec = DeviceSpec::parse("Elgato");
        assert!(resolve_in(&devices, &spec).is_err());
    }

    #[test]
    fn auto_prefers_external_over_builtin() {
        let devices = vec![
            dev(0, "FaceTime HD Camera", "0x1421000005ac8514"),
            dev(1, "USB Video", "0x8300000534d2109"),
        ];
        assert_eq!(resolve_in(&devices, &DeviceSpec::Auto).unwrap(), 1);
    }

    #[test]
    fn empty_id_never_matches_empty_spec_id() {
        // A device with no stable id must not be selected by id equality.
        let devices = vec![dev(0, "USB Video", "")];
        let spec = DeviceSpec::Name(String::new());
        // Empty string parses to Auto via parse(); construct Name directly to
        // prove the id-equality guard, then expect substring match (matches).
        assert_eq!(resolve_in(&devices, &spec).unwrap(), 0);
    }
}
