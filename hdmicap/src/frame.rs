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

/// How old the newest frame may be before it stops describing the screen.
///
/// The capture loop publishes only on success: a frame error does `continue`,
/// leaving the previous `FrameState` in the watch channel with its previous
/// label. So a dead capture path reads as `Stable` forever, and callers acted
/// on it — a machine whose mains had been cut kept reporting `stable` on its
/// pre-cut desktop. Age is the only thing that distinguishes "the screen has
/// not changed" from "we have stopped being told what the screen is".
///
/// Generous next to the 10 fps capture cap, so an occasional slow frame is not
/// reported as a fault, and far below the minutes-long staleness observed.
pub const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(3);

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
    /// The last frame is too old to describe what is on screen now. The
    /// capture loop has stopped delivering — the device errored, the source
    /// lost power, or a mode change is in flight — and what remains in the
    /// watch channel is whatever was last seen.
    Stale,
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
    /// The signal as it applies *now*, downgrading a frame that has aged out.
    ///
    /// Read this rather than the `signal` field. The stored value describes the
    /// frame when it was captured; this describes whether it still describes
    /// the screen. `NoDevice` and `NoSignal` are already terminal answers and
    /// pass through unchanged.
    pub fn effective_signal(&self) -> Signal {
        match self.signal {
            Signal::Stable | Signal::ModeSwitching if self.captured_at.elapsed() > STALE_AFTER => {
                Signal::Stale
            }
            other => other,
        }
    }

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
            signal: f.effective_signal(),
            width: f.width,
            height: f.height,
            hash: format!("{:016x}", f.hash),
            resolution_epoch: f.resolution_epoch,
            captured_at_ms_ago: f.captured_at.elapsed().as_millis(),
        }
    }
}

/// Samples per hash cell edge: each of the 64 aHash cells averages an 8x8
/// sample grid, 4096 luma reads total — resolution-independent cost.
///
/// This was 4 (a 32x32 lattice, 1024 reads) and that was too coarse to *see*
/// console text. On a 1280x720 Gigaboot screen — 1.35% of pixels at luma 255 —
/// a 32x32 lattice landed on no glyph at all: max luma seen 20, so the frame
/// classified as blank and hdmicap reported `no_signal` on a perfectly
/// readable screen. At 64x64 the same frame yields max luma 255 across 42
/// samples. See evals/ocr/dataset/README.md for the measurements.
///
/// The 8x8 cell grid, and therefore the 64-bit hash, is unchanged.
const CELL_SAMPLES: u32 = 8;
const GRID: u32 = 8 * CELL_SAMPLES; // 64x64 sample lattice

/// A luma level that cannot come from an unlit panel. Well above the noise of a
/// black frame from a capture dongle, well below any legible text.
const BRIGHT: u32 = 64;

/// One-pass strided classification: 8x8 aHash + (near-)black no-signal
/// detection from the same 4096 luma samples. `luma_at(x, y)` must return
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
    let mut max = 0u32;

    for gy in 0..GRID {
        // Center each sample within its lattice slot.
        let y = (gy * h + h / 2) / GRID;
        for gx in 0..GRID {
            let x = (gx * w + w / 2) / GRID;
            let l = luma_at(x.min(w - 1), y.min(h - 1)) as u32;
            cells[((gy / CELL_SAMPLES) * 8 + gx / CELL_SAMPLES) as usize] += l;
            sum += l as u64;
            sum_sq += (l * l) as u64;
            max = max.max(l);
        }
    }

    let n = (GRID * GRID) as u64;
    let mean = sum / n;
    let var = (sum_sq / n).saturating_sub(mean * mean);
    // Low mean + low variance => HDMI blank / lens-capped. The max-luma term is
    // the guard that mean and variance cannot provide: a screen showing a
    // handful of bright glyphs on black has a near-zero mean and, if the
    // lattice happens to miss them, a near-zero variance too. One sample above
    // BRIGHT is enough to prove something is lit, whatever the average says.
    // Conservative on purpose — this decides whether an agent is allowed to
    // read the screen at all.
    let no_signal = mean < 10 && var < 64 && max < BRIGHT;

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
    use std::time::Duration;

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

    /// A boot screen is mostly black, and that must not read as "no signal".
    ///
    /// This is the regression for a real failure: hdmicap reported `no_signal`
    /// for minutes at a time across a NUC's PXE/Gigaboot phase, and labelled a
    /// clean, fully readable capture `no_signal` too. The screen was 1.35%
    /// bright pixels on black, and the sampling lattice was coarse enough to
    /// land on none of them — so mean, variance and max all said "blank".
    ///
    /// The frame below mimics that: thin bright text rows on black, at roughly
    /// the same bright fraction, positioned off the old lattice. It fails with
    /// the pre-fix `CELL_SAMPLES = 4`.
    #[test]
    fn sparse_bright_text_on_black_is_not_no_signal() {
        let (w, h) = (1280u32, 720u32);
        let mut buf = vec![0u8; (w * h * 3) as usize];
        // Nine 2px text rows, at y positions the PRE-FIX 32x32 lattice misses
        // entirely and the 64x64 lattice hits. Calibrated deliberately: a row
        // spacing chosen by eye happens to collide with the old lattice, the
        // test passes against the old code, and the regression it is supposed
        // to guard goes unguarded. If the lattice geometry changes, recompute
        // these rather than assuming they still straddle it.
        const ROWS: [u32; 9] = [4, 83, 162, 240, 319, 398, 477, 555, 634];
        let mut lit = 0u32;
        for y0 in ROWS {
            for y in y0..(y0 + 2).min(h) {
                // Dashes with gaps, like glyphs rather than a solid rule.
                for x in (0..w).filter(|x| (x / 4) % 3 != 0) {
                    let i = ((y * w + x) * 3) as usize;
                    buf[i] = 255;
                    buf[i + 1] = 255;
                    buf[i + 2] = 255;
                    lit += 1;
                }
            }
        }
        // Sanity: this is a sparse screen, not a bright one.
        let frac = lit as f64 / (w * h) as f64;
        assert!(frac < 0.03, "test frame should be sparse, got {frac:.4}");

        let (_, no_sig) = classify_rgb(&buf, w, h);
        assert!(
            !no_sig,
            "a readable boot screen classified as no_signal ({:.2}% of pixels lit)",
            frac * 100.0
        );
    }

    /// A frame that has aged out must stop claiming to be the screen.
    ///
    /// The capture loop publishes only on success, so a stalled capture leaves
    /// the last good frame in place with its old label. Observed twice on real
    /// hardware: across a mode change, and on a machine whose mains had been
    /// cut — which kept reporting `stable` on its pre-cut desktop. `--stable`
    /// and `/ocr` both gate on this, so an agent trusted a dead screen.
    #[test]
    fn an_aged_out_frame_is_stale_not_stable() {
        let fresh = FrameState {
            jpeg: None,
            pixels: PixelData::Empty,
            width: 1280,
            height: 720,
            hash: 1,
            signal: Signal::Stable,
            resolution_epoch: 0,
            captured_at: Instant::now(),
        };
        assert_eq!(fresh.effective_signal(), Signal::Stable);

        let stalled = FrameState {
            captured_at: Instant::now() - (STALE_AFTER + Duration::from_millis(1)),
            ..fresh.clone()
        };
        assert_eq!(
            stalled.effective_signal(),
            Signal::Stale,
            "a frame older than STALE_AFTER must not read as Stable"
        );

        // A device that is genuinely gone keeps saying so; staleness does not
        // overwrite a terminal answer with a vaguer one.
        let gone = FrameState {
            signal: Signal::NoDevice,
            captured_at: Instant::now() - (STALE_AFTER * 10),
            ..fresh.clone()
        };
        assert_eq!(gone.effective_signal(), Signal::NoDevice);
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
