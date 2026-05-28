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

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tracing::{info, warn};

use crate::capture::{CaptureBackend, DeviceSpec, NokhwaBackend};
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
        let mut backend = match NokhwaBackend::open(&spec) {
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
                thread::sleep(Duration::from_millis(750));
                continue;
            }
        };

        let mut last_dims = (0u32, 0u32);
        let mut epoch = 0u64;
        let mut stable_count = 0u32;

        loop {
            if all_receivers_gone(&tx) {
                info!("no receivers left; capture thread exiting");
                return;
            }

            let img = match backend.frame() {
                Ok(img) => img,
                Err(e) => {
                    // A dropped frame is normal during mode switches; only
                    // treat a sustained failure as device loss.
                    warn!("frame error: {e:#}");
                    let _ = tx.send(Arc::new(FrameState::no_device()));
                    break; // fall back to the reconnect loop
                }
            };

            let (w, h) = (img.width(), img.height());

            if (w, h) != last_dims {
                epoch += 1;
                stable_count = 0;
                last_dims = (w, h);
                info!("resolution -> {w}x{h} (epoch {epoch})");
            }

            let hash = ahash(&img);

            let signal = if is_no_signal(&img) {
                stable_count = 0;
                Signal::NoSignal
            } else if stable_count < STABLE_FRAMES {
                stable_count += 1;
                Signal::ModeSwitching
            } else {
                Signal::Stable
            };

            // Some backends (e.g. nokhwa's AVFoundation YUYV decoder) emit a
            // buffer with stride padding beyond width*height*3. Trim to exact
            // size so PNG and JPEG encoders don't assert on the extra bytes.
            let expected = w as usize * h as usize * 3;
            let mut raw = img.into_raw();
            raw.truncate(expected);
            let rgb: Arc<[u8]> = Arc::from(raw.into_boxed_slice());

            let _ = tx.send(Arc::new(FrameState {
                rgb,
                width: w,
                height: h,
                hash,
                signal,
                resolution_epoch: epoch,
                captured_at: Instant::now(),
            }));
        }
    }
}

fn all_receivers_gone(tx: &watch::Sender<Arc<FrameState>>) -> bool {
    tx.receiver_count() == 0
}
