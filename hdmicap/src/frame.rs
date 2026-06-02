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

//! The single value that flows from the capture thread to every HTTP handler.
//!
//! Design rule: the capture thread does the cheap classification (dims, hash,
//! signal) inline, but NEVER encodes PNG/JPEG here. Encoding is lazy, done in
//! the handler that actually needs bytes, so the hot loop cost is bounded.

use std::sync::Arc;
use std::time::Instant;

use image::RgbImage;
use serde::Serialize;

/// How many consecutive same-resolution, non-black frames we require before
/// trusting the signal as `Stable`. A booting machine renegotiates HDMI at
/// firmware -> bootloader -> OS handoffs; the dongle emits black/torn frames
/// across each switch. This debounce stops an agent reading a black rectangle.
pub const STABLE_FRAMES: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
// `NoSignal` is the correct domain term (HDMI "no signal"); the shared `Signal`
// suffix is intentional, not the accidental redundancy this lint guards against.
#[allow(clippy::enum_variant_names)]
pub enum Signal {
    /// No capture device present / handle lost.
    NoDevice,
    /// Device is streaming but the frame is (near-)black: HDMI source off,
    /// unplugged, or mid-blank. Distinct from a stale-but-valid frame.
    NoSignal,
    /// Resolution just changed or we haven't seen enough stable frames yet.
    ModeSwitching,
    /// Frame is trustworthy for OCR / agent reading.
    Stable,
}

/// Immutable snapshot of "what's on screen right now", shared via `watch`.
#[derive(Clone)]
pub struct FrameState {
    /// Raw JPEG/MJPEG bytes as delivered by the capture device. Present when
    /// the device natively delivers MJPEG (always on Linux). The preview
    /// endpoint serves these directly — no server-side decode/re-encode.
    pub jpeg: Option<Arc<[u8]>>,
    /// Decoded RGB8, populated when jpeg is None (YUYV sources) or on demand
    /// for snapshot/OCR. Empty slice when not available.
    pub rgb: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    /// Perceptual hash of an 8x8 grayscale downscale (aHash). Powers
    /// change-detection and a secondary torn-frame check. Cheap every frame.
    pub hash: u64,
    pub signal: Signal,
    /// Bumps every time the capture resolution changes. Lets a consumer notice
    /// "the machine switched video modes" even if pixel hashes happen to match.
    pub resolution_epoch: u64,
    pub captured_at: Instant,
}

impl FrameState {
    pub fn no_device() -> Self {
        FrameState {
            jpeg: None,
            rgb: Arc::from(Vec::new().into_boxed_slice()),
            width: 0,
            height: 0,
            hash: 0,
            signal: Signal::NoDevice,
            resolution_epoch: 0,
            captured_at: Instant::now(),
        }
    }
}

/// JSON shape returned by `GET /status`. Cheap for the agent to poll.
#[derive(Serialize)]
pub struct StatusDto {
    pub signal: Signal,
    pub width: u32,
    pub height: u32,
    pub hash: String, // hex, so it round-trips cleanly into ?changed_since=
    pub resolution_epoch: u64,
    pub captured_at_ms_ago: u128,
}

impl From<&FrameState> for StatusDto {
    fn from(f: &FrameState) -> Self {
        StatusDto {
            signal: f.signal,
            width: f.width,
            height: f.height,
            hash: format!("{:016x}", f.hash),
            resolution_epoch: f.resolution_epoch,
            captured_at_ms_ago: f.captured_at.elapsed().as_millis(),
        }
    }
}

/// aHash over an 8x8 grayscale downscale (64 bits, one per pixel). Robust to
/// capture noise and cheap enough to run on every frame. Uses a bilinear
/// (Triangle) filter, which is sufficient for mode-switch discrimination.
pub fn ahash(img: &RgbImage) -> u64 {
    use image::imageops::{grayscale, resize, FilterType};
    let small = resize(&grayscale(img), 8, 8, FilterType::Triangle);
    let pixels: Vec<u8> = small.pixels().map(|p| p.0[0]).collect();
    let avg = (pixels.iter().map(|&p| p as u32).sum::<u32>() / pixels.len() as u32) as u8;
    let mut bits = 0u64;
    for (i, &p) in pixels.iter().enumerate() {
        if p >= avg {
            bits |= 1 << i;
        }
    }
    bits
}

/// (Near-)black detection: low mean luma + low variance => NoSignal.
///
/// Thresholds are intentionally conservative (mean < 10, var < 64). Tune
/// against the real MS2109 if dark-grey blanking frames cause false negatives.
pub fn is_no_signal(img: &RgbImage) -> bool {
    let mut sum = 0u64;
    let mut sum_sq = 0u64;
    let n = (img.width() * img.height()) as u64;
    if n == 0 {
        return true;
    }
    for p in img.pixels() {
        // Rec.601-ish luma, integer.
        let y = (p.0[0] as u64 * 77 + p.0[1] as u64 * 150 + p.0[2] as u64 * 29) >> 8;
        sum += y;
        sum_sq += y * y;
    }
    let mean = sum / n;
    let var = (sum_sq / n).saturating_sub(mean * mean);
    mean < 10 && var < 64
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    fn solid_image(r: u8, g: u8, b: u8, w: u32, h: u32) -> RgbImage {
        let pixels: Vec<u8> = (0..w * h).flat_map(|_| [r, g, b]).collect();
        RgbImage::from_raw(w, h, pixels).unwrap()
    }

    #[test]
    fn black_is_no_signal() {
        assert!(is_no_signal(&solid_image(0, 0, 0, 320, 240)));
    }

    #[test]
    fn near_black_is_no_signal() {
        // Dark grey (luma ~7) should still register as no-signal.
        assert!(is_no_signal(&solid_image(8, 8, 8, 320, 240)));
    }

    #[test]
    fn content_frame_is_not_no_signal() {
        // Mid-grey has enough luma.
        assert!(!is_no_signal(&solid_image(128, 128, 128, 320, 240)));
    }

    #[test]
    fn ahash_same_image_stable() {
        let img = solid_image(100, 150, 200, 320, 240);
        assert_eq!(ahash(&img), ahash(&img));
    }

    #[test]
    fn ahash_different_images_differ() {
        // aHash measures structure (above/below mean), not absolute brightness.
        // Solid images of any value all produce the same hash (all bits set),
        // so we need images with opposing gradients: left-dark/right-bright vs
        // left-bright/right-dark.
        let w = 320u32;
        let h = 240u32;
        let gradient = |left_dark: bool| {
            let pixels: Vec<u8> = (0..w * h)
                .flat_map(|i| {
                    let x = i % w;
                    let v: u8 = if (x < w / 2) == left_dark { 50 } else { 200 };
                    [v, v, v]
                })
                .collect();
            RgbImage::from_raw(w, h, pixels).unwrap()
        };
        assert_ne!(ahash(&gradient(true)), ahash(&gradient(false)));
    }
}
