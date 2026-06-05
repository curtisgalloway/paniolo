<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-->

# HID injector host tools (macOS)

Bench and capture tools for measuring and verifying the KB2040 injector with
its USB plugged into the **same Mac** that drives the control UART. This
isolates the HID path from the video-feedback path of the full KVM.

Build the C/Objective-C tools:

```bash
make            # builds hid_capture_usb and hid_seize_reports
```

The Python tools run under `uv` and need no build step.

## hid_capture_usb — leak-safe HID capture

Detaches the injector from the macOS HID stack entirely via IOUSBHost
whole-device capture (`IOUSBHostObjectInitOptionsDeviceCapture`; running as
root passes the same gate as the `com.apple.vm.device-access` entitlement),
then reads its interrupt-IN endpoint and prints each report with timestamps.
Because the device is detached, injected keystrokes/mouse-moves reach **only**
this tool — never the focused app or the real cursor.

```bash
sudo ./hid_capture_usb            # defaults to the injector serial
sudo ./hid_capture_usb <serial>   # there may be >1 KB2040 (same VID/PID) attached
HID_CAPTURE_PROBE=1 sudo -E ./hid_capture_usb   # hold the capture and sleep, for hidutil/leak inspection
```

Each line: `report ts=<sec.usec> dt=<usec-since-prev> len=<n>: <hex>`. The first
payload byte is the report ID (1 = keyboard, 2 = absolute mouse). Verify the
device is detached while the tool runs with `hidutil list` (the injector
disappears).

**Always start this tool before injecting** — otherwise the reports leak into
your live session.

## hid_seize_reports — passive raw-report tap (NOT exclusive)

The older approach: `IOHIDDeviceOpen(..., kIOHIDOptionsTypeSeizeDevice)`. On
Darwin 24/25 the seize is **not** exclusive — the open succeeds and reports
arrive here, but the system event path is not detached, so injected mouse moves
still move the real cursor. Keep it only as a passive timestamped tap; use
`hid_capture_usb` when you need true exclusivity. Requires `sudo` plus an Input
Monitoring grant in System Settings.

## hid_bench.py — latency / throughput

Drives the UART directly (it sets `IOSSDATALAT` itself) and times command
round trips. Run leak-safe by starting `hid_capture_usb` first.

```bash
uv run --with pyserial hid_bench.py --device /dev/cu.usbserial-XXXX \
    --baud 460800 --latency-us 1 --mode latency --count 100
```

Modes: `latency` (per-sample cmd→OK round trip + percentiles), `rr`
(back-to-back request/reply rate), `pipe` (windowed pipelining). `--latency-us`
sets the macOS serial read-latency timer (1 ≈ floor; 0 leaves the default).

Reference numbers (this bench, with the latency fix): `ping` ~3 ms, `moveabs`
~8 ms (USB `bInterval` floor), moveabs throughput ~123/s (bInterval-capped).

## leak_check.py — assert no leak

Injects one centered `moveabs` and checks whether the real cursor moved,
restoring it if so. Expect `NO LEAK` while `hid_capture_usb` holds the device.

```bash
uv run --with pyserial --with pyobjc-framework-Quartz leak_check.py \
    --device /dev/cu.usbserial-XXXX
```
