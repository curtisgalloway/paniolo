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
//! signal) inline, but NEVER converts or encodes full images here. RGB/PNG/JPEG
//! materialize lazily in the handler that needs bytes, so the hot loop cost is
//! bounded and independent of resolution.

use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use crate::pixel::PixelData;

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
    /// Raw JPEG/MJPEG bytes as delivered by the capture device (Linux MJPEG
    /// tee). The preview endpoint serves these directly — no server-side
    /// decode/re-encode.
    pub jpeg: Option<Arc<[u8]>>,
    /// Native pixel data (NV12 on macOS, RGB on decode paths, Empty when
    /// `jpeg` carries the image). Handlers convert lazily via `crate::pixel`.
    pub pixels: PixelData,
    pub width: u32,
    pub height: u32,
    /// Perceptual hash (8x8 aHash over strided luma samples). Powers
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
            pixels: PixelData::Empty,
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

/// Samples per hash cell edge: each of the 64 aHash cells averages a 4x4
/// sample grid, 1024 luma reads total — resolution-independent cost.
const CELL_SAMPLES: u32 = 4;
const GRID: u32 = 8 * CELL_SAMPLES; // 32x32 sample lattice

/// One-pass strided classification: 8x8 aHash + (near-)black no-signal
/// detection from the same 1024 luma samples. `luma_at(x, y)` must return
/// FULL-RANGE luma (0-255); callers normalize video-range sources.
///
/// Replaces the old grayscale()+resize() aHash and full-image no-signal scan,
/// whose cost scaled with resolution (~hundreds of ms at 8 MP — the capture
/// loop ran at 1.4 fps against the IPEVO V4K before this).
pub fn classify<F: FnMut(u32, u32) -> u8>(w: u32, h: u32, mut luma_at: F) -> (u64, bool) {
    if w == 0 || h == 0 {
        return (0, true);
    }

    let mut cells = [0u32; 64];
    let mut sum = 0u64;
    let mut sum_sq = 0u64;

    for gy in 0..GRID {
        // Center each sample within its lattice slot.
        let y = (gy * h + h / 2) / GRID;
        for gx in 0..GRID {
            let x = (gx * w + w / 2) / GRID;
            let l = luma_at(x.min(w - 1), y.min(h - 1)) as u32;
            cells[((gy / CELL_SAMPLES) * 8 + gx / CELL_SAMPLES) as usize] += l;
            sum += l as u64;
            sum_sq += (l * l) as u64;
        }
    }

    let n = (GRID * GRID) as u64;
    let mean = sum / n;
    let var = (sum_sq / n).saturating_sub(mean * mean);
    // Thresholds carried over from the full-scan implementation: low mean +
    // low variance => HDMI blank / lens-capped. Conservative on purpose.
    let no_signal = mean < 10 && var < 64;

    // Bit per cell: cell's sample sum vs the global mean of cell sums.
    let cell_mean = (sum / 64) as u32;
    let mut bits = 0u64;
    for (i, &c) in cells.iter().enumerate() {
        if c >= cell_mean {
            bits |= 1 << i;
        }
    }
    (bits, no_signal)
}

/// Classify an NV12 luma plane ('420v', video-range: black=16). Normalizes
/// to full range so the no-signal thresholds keep their meaning.
pub fn classify_nv12(y: &[u8], w: u32, h: u32) -> (u64, bool) {
    classify(w, h, |x, yy| {
        let raw = *y
            .get((yy as usize) * (w as usize) + x as usize)
            .unwrap_or(&16);
        ((raw.saturating_sub(16) as u32 * 255) / 219).min(255) as u8
    })
}

/// Classify a packed RGB8 buffer (Rec.601 luma, integer).
pub fn classify_rgb(rgb: &[u8], w: u32, h: u32) -> (u64, bool) {
    classify(w, h, |x, y| {
        let i = ((y as usize) * (w as usize) + x as usize) * 3;
        match rgb.get(i..i + 3) {
            Some(p) => ((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as u8,
            None => 0,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_rgb(r: u8, g: u8, b: u8, w: u32, h: u32) -> Vec<u8> {
        (0..w * h).flat_map(|_| [r, g, b]).collect()
    }

    #[test]
    fn black_is_no_signal() {
        let buf = solid_rgb(0, 0, 0, 320, 240);
        let (_, no_sig) = classify_rgb(&buf, 320, 240);
        assert!(no_sig);
    }

    #[test]
    fn near_black_is_no_signal() {
        // Dark grey (luma ~7) should still register as no-signal.
        let buf = solid_rgb(8, 8, 8, 320, 240);
        let (_, no_sig) = classify_rgb(&buf, 320, 240);
        assert!(no_sig);
    }

    #[test]
    fn content_frame_is_not_no_signal() {
        // Mid-grey has enough luma.
        let buf = solid_rgb(128, 128, 128, 320, 240);
        let (_, no_sig) = classify_rgb(&buf, 320, 240);
        assert!(!no_sig);
    }

    #[test]
    fn nv12_video_range_black_is_no_signal() {
        // '420v' black is luma 16, NOT 0 — normalization must map it under
        // the threshold or HDMI blank detection breaks on the macOS path.
        let y = vec![16u8; 320 * 240];
        let (_, no_sig) = classify_nv12(&y, 320, 240);
        assert!(no_sig);
    }

    #[test]
    fn nv12_content_is_not_no_signal() {
        let y = vec![126u8; 320 * 240];
        let (_, no_sig) = classify_nv12(&y, 320, 240);
        assert!(!no_sig);
    }

    #[test]
    fn hash_same_image_stable() {
        let buf = solid_rgb(100, 150, 200, 320, 240);
        assert_eq!(
            classify_rgb(&buf, 320, 240).0,
            classify_rgb(&buf, 320, 240).0
        );
    }

    #[test]
    fn hash_different_images_differ() {
        // aHash measures structure (above/below mean): opposing horizontal
        // gradients must produce different bit patterns.
        let w = 320u32;
        let h = 240u32;
        let gradient = |left_dark: bool| -> Vec<u8> {
            (0..w * h)
                .flat_map(|i| {
                    let x = i % w;
                    let v: u8 = if (x < w / 2) == left_dark { 50 } else { 200 };
                    [v, v, v]
                })
                .collect()
        };
        let a = classify_rgb(&gradient(true), w, h).0;
        let b = classify_rgb(&gradient(false), w, h).0;
        assert_ne!(a, b);
    }

    #[test]
    fn zero_dims_is_no_signal() {
        let (hash, no_sig) = classify_rgb(&[], 0, 0);
        assert_eq!(hash, 0);
        assert!(no_sig);
    }
}
