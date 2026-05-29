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
//! This is a plain std::thread, NOT a tokio task: nokhwa's grab is blocking and
//! must not sit on the async runtime. `watch::Sender::send` is sync, so the
//! thread publishes freely.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tracing::{info, warn};

use crate::capture::{open_backend, DeviceSpec};
use crate::frame::{ahash, is_no_signal, FrameState, Signal, STABLE_FRAMES};

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
        let mut last_hash = 0u64;
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
            let jpeg = captured.jpeg;
            let img = captured.rgb;

            let (w, h) = (img.width(), img.height());

            if (w, h) != last_dims {
                epoch += 1;
                stable_count = 0;
                last_dims = (w, h);
                last_hash = 0;
                info!("resolution -> {w}x{h} (epoch {epoch})");
            }

            let hash = ahash(&img);

            // Skip the expensive per-pixel is_no_signal scan when the frame
            // hash is unchanged and we're already Stable — static screens cost
            // almost nothing after the first pass.
            let signal = if hash == last_hash && stable_count >= STABLE_FRAMES {
                Signal::Stable
            } else if is_no_signal(&img) {
                stable_count = 0;
                Signal::NoSignal
            } else if stable_count < STABLE_FRAMES {
                stable_count += 1;
                Signal::ModeSwitching
            } else {
                Signal::Stable
            };

            last_hash = hash;
            frame_count.fetch_add(1, Ordering::Relaxed);

            // When raw JPEG bytes are available (Linux MJPEG path), store them
            // for zero-cost preview serving. The RGB is kept for snapshot/OCR
            // but only when JPEG is absent (YUYV or macOS paths).
            let (jpeg_arc, rgb_arc) = if let Some(j) = jpeg {
                (Some(j), Arc::from([] as [u8; 0]) as Arc<[u8]>)
            } else {
                let expected = w as usize * h as usize * 3;
                let mut raw = img.into_raw();
                raw.truncate(expected);
                (None, Arc::from(raw.into_boxed_slice()) as Arc<[u8]>)
            };

            let _ = tx.send(Arc::new(FrameState {
                jpeg: jpeg_arc,
                rgb: rgb_arc,
                width: w,
                height: h,
                hash,
                signal,
                resolution_epoch: epoch,
                captured_at: Instant::now(),
            }));

            // Cap to TARGET_FPS. MJPEG decode is ~50ms/frame in software, so
            // 10fps is a reasonable ceiling until the hot path uses lazy decode.
            const TARGET_INTERVAL: Duration = Duration::from_millis(1000 / 10);
            let elapsed = frame_start.elapsed();
            if elapsed < TARGET_INTERVAL {
                thread::sleep(TARGET_INTERVAL - elapsed);
            }
            frame_start = Instant::now();
        }
    }
}

fn all_receivers_gone(tx: &watch::Sender<Arc<FrameState>>) -> bool {
    tx.receiver_count() == 0
}
