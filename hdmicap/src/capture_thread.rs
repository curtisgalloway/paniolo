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

//! The capture thread. Owns the device, runs the warm decode loop, classifies
//! each frame, and publishes the latest FrameState into a `watch` channel.
//!
//! This is a plain std::thread, NOT a tokio task: both backends' frame waits
//! block (V4L2 DQBUF; the AVFoundation layer's condvar) and must not sit on
//! the async runtime. `watch::Sender::send` is sync, so the thread publishes
//! freely.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tracing::{info, warn};

use crate::capture::{open_backend, DeviceSpec};
use crate::frame::{classify_nv12, classify_rgb, FrameState, Signal, STABLE_FRAMES};
use crate::pixel::PixelData;

pub type FrameRx = watch::Receiver<Arc<FrameState>>;

/// Spawn the capture thread. Returns the receiver end and the JoinHandle.
pub fn spawn(spec: DeviceSpec) -> (FrameRx, thread::JoinHandle<()>) {
    let (tx, rx) = watch::channel(Arc::new(FrameState::no_device()));

    let handle = thread::Builder::new()
        .name("capture".into())
        .spawn(move || capture_loop(spec, tx))
        .expect("failed to spawn capture thread");

    (rx, handle)
}

fn capture_loop(spec: DeviceSpec, tx: watch::Sender<Arc<FrameState>>) {
    // Reconnect loop: if the device is absent or vanishes mid-run, publish
    // NoDevice and keep retrying so hot-plug just works.
    loop {
        let mut backend = match open_backend(&spec) {
            Ok(b) => {
                info!("capture device opened");
                b
            }
            Err(e) => {
                warn!("open failed: {e:#}");
                let _ = tx.send(Arc::new(FrameState::no_device()));
                if all_receivers_gone(&tx) {
                    return;
                }
                // Brief pause before retry. The MS2109 firmware needs time to
                // reset its isochronous endpoint state after a stream stop;
                // immediately reopening can catch it mid-reset and cause stalls.
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        // Watchdog: fallback for any stall the v4l timeout doesn't catch.
        // The cancel flag is set when we exit the inner loop normally so the
        // watchdog doesn't fire across reconnect iterations.
        let frame_count = Arc::new(AtomicU64::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let frame_count = frame_count.clone();
            let cancelled = cancelled.clone();
            thread::Builder::new()
                .name("stall-watchdog".into())
                .spawn(move || {
                    const GRACE: Duration = Duration::from_secs(12);
                    const POLL: Duration = Duration::from_secs(4);
                    thread::sleep(GRACE);
                    if cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    let mut prev = frame_count.load(Ordering::Relaxed);
                    if prev == 0 {
                        warn!("no frames in {GRACE:?} after device open — exiting for restart");
                        std::process::exit(1);
                    }
                    loop {
                        thread::sleep(POLL);
                        if cancelled.load(Ordering::Relaxed) {
                            return;
                        }
                        let cur = frame_count.load(Ordering::Relaxed);
                        if cur == prev {
                            warn!("capture stalled ({POLL:?} with no new frames) — exiting for restart");
                            std::process::exit(1);
                        }
                        prev = cur;
                    }
                })
                .ok();
        }

        let mut last_dims = (0u32, 0u32);
        let mut epoch = 0u64;
        let mut stable_count = 0u32;
        // Linux only: the loop is rate-capped there (see below). On macOS the
        // backend blocks until the device delivers the next frame, so the
        // device's own cadence paces us.
        #[cfg(target_os = "linux")]
        let mut frame_start = Instant::now();
        // Consecutive decode errors. Transient errors (bad buffer after open,
        // UVC flush frames) are tolerated; only a sustained run triggers reconnect.
        let mut consecutive_errors = 0u32;
        const MAX_CONSECUTIVE_ERRORS: u32 = 8;

        loop {
            if all_receivers_gone(&tx) {
                info!("no receivers left; capture thread exiting");
                return;
            }

            let captured = match backend.frame() {
                Ok(f) => {
                    consecutive_errors = 0;
                    f
                }
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        warn!("frame error ({consecutive_errors} consecutive): {e:#}");
                        cancelled.store(true, Ordering::Relaxed);
                        let _ = tx.send(Arc::new(FrameState::no_device()));
                        break;
                    }
                    // Transient: skip this frame and try again.
                    tracing::debug!("transient frame error (#{consecutive_errors}): {e:#}");
                    continue;
                }
            };
            let (w, h) = (captured.width, captured.height);

            if (w, h) != last_dims {
                epoch += 1;
                stable_count = 0;
                last_dims = (w, h);
                info!("resolution -> {w}x{h} (epoch {epoch})");
            }

            // One-pass strided classification: hash + no-signal from ~1k luma
            // samples, resolution-independent (the old full-image pass cost
            // hundreds of ms at 8 MP).
            let (hash, no_signal) = match &captured.pixels {
                PixelData::Nv12 { y, .. } => classify_nv12(y, w, h),
                PixelData::Rgb(buf) => classify_rgb(buf, w, h),
                PixelData::Empty => (0, true),
            };

            let signal = if no_signal {
                stable_count = 0;
                Signal::NoSignal
            } else if stable_count < STABLE_FRAMES {
                stable_count += 1;
                Signal::ModeSwitching
            } else {
                Signal::Stable
            };

            frame_count.fetch_add(1, Ordering::Relaxed);

            // When raw JPEG bytes are available (Linux MJPEG path), the
            // preview serves them directly and snapshot/OCR re-decode on
            // demand — don't carry a redundant RGB copy in every FrameState.
            let pixels = if captured.jpeg.is_some() {
                PixelData::Empty
            } else {
                captured.pixels
            };

            let _ = tx.send(Arc::new(FrameState {
                jpeg: captured.jpeg,
                pixels,
                width: w,
                height: h,
                hash,
                signal,
                resolution_epoch: epoch,
                captured_at: Instant::now(),
            }));

            // Linux: cap to 10fps — the v4l device delivers as fast as we
            // dequeue, and the per-frame turbojpeg decode has real cost.
            // macOS: no cap; the backend blocks until the next frame.
            #[cfg(target_os = "linux")]
            {
                const TARGET_INTERVAL: Duration = Duration::from_millis(1000 / 10);
                let elapsed = frame_start.elapsed();
                if elapsed < TARGET_INTERVAL {
                    thread::sleep(TARGET_INTERVAL - elapsed);
                }
                frame_start = Instant::now();
            }
        }
    }
}

fn all_receivers_gone(tx: &watch::Sender<Arc<FrameState>>) -> bool {
    tx.receiver_count() == 0
}
