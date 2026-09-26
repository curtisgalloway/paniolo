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

Tools for measuring and verifying the KB2040 injector (an Adafruit RP2040 board
that acts as a USB keyboard and mouse, or HID device) with its USB plugged into
the **same Mac** that drives the control link. This tests the HID path on its
own, without the video feedback of the full KVM.

Build the C/Objective-C tools:

```bash
make            # builds hid_capture_usb and hid_seize_reports
```

The Python tools run under `uv` and need no build step.

| Tool | Use it for | Works with |
|---|---|---|
| `hid_capture_usb` | Leak-safe capture of the injector's reports (use this one) | Either firmware |
| `hid_seize_reports` | Passive timestamped tap; **not** exclusive | Either firmware |
| `hid_bench.py` | Latency and throughput | Retired single-board firmware only |
| `leak_check.py` | Assert that injection does not move the real cursor | Retired single-board firmware only |

## hid_capture_usb — leak-safe HID capture

**Always start this tool before injecting.** Otherwise the reports leak into
your live session.

It detaches the injector from the macOS HID stack entirely, using IOUSBHost
whole-device capture (`IOUSBHostObjectInitOptionsDeviceCapture`; running as
root passes the same gate as the `com.apple.vm.device-access` entitlement). It
then reads the interrupt-IN endpoint and prints each report with timestamps.
Because the device is detached, injected keystrokes and mouse moves reach
**only** this tool, never the focused app or the real cursor.

```bash
sudo ./hid_capture_usb            # defaults to the injector serial
sudo ./hid_capture_usb <serial>   # there may be >1 KB2040 (same VID/PID) attached
HID_CAPTURE_PROBE=1 sudo -E ./hid_capture_usb   # hold the capture and sleep, for hidutil/leak inspection
```

Each line is `report ts=<sec.usec> dt=<usec-since-prev> len=<n>: <hex>`. The
first payload byte is the report ID (1 = keyboard, 2 = absolute mouse). To
confirm the device is detached, run `hidutil list` while the tool runs: the
injector disappears.

## hid_seize_reports — passive raw-report tap (NOT exclusive)

The older approach: `IOHIDDeviceOpen(..., kIOHIDOptionsTypeSeizeDevice)`. On
Darwin 24/25 the seize is **not** exclusive. The open succeeds and reports
arrive here, but the system event path is not detached, so injected mouse moves
still move the real cursor. Use it only as a passive timestamped tap; use
`hid_capture_usb` when you need exclusivity. Requires `sudo` plus an Input
Monitoring grant in System Settings.

`hid_capture_usb` and `hid_seize_reports` observe the DUT-facing HID board
(the board that plugs into the device under test). It has the same VID/PID on
the retired single board and on the dual-board target board, so both tools work
with either firmware when that board is plugged into this Mac.

## hid_bench.py — latency / throughput (retired single-board path)

> **Note:** `hid_bench.py` and `leak_check.py` speak only the retired
> single-board firmware's line protocol over a USB-serial adapter (`OK`/`ERR`
> replies, 115200→460800 baud negotiation). The current dual-board rig's
> control link is a USB-CDC (USB virtual serial) port speaking binary frames
> with **no baud and no text replies**, so these two tools cannot drive it.
> They are kept for the retired firmware (paniolo-hardware `hidrig-kb2040/firmware/single-board/{boot,code,config}.py`,
> https://github.com/curtisgalloway/paniolo-hardware) and as a reference for a
> future bench port.

It drives the UART directly (setting `IOSSDATALAT` itself) and times command
round trips. For a leak-safe run, start `hid_capture_usb` first.

```bash
uv run --with pyserial hid_bench.py --device /dev/cu.usbserial-XXXX \
    --baud 460800 --latency-us 1 --mode latency --count 100
```

| Mode | Measures |
|---|---|
| `latency` | per-sample cmd→OK round trip + percentiles |
| `rr` | back-to-back request/reply rate |
| `pipe` | windowed pipelining |

`--latency-us` sets the macOS serial read-latency timer (1 ≈ floor; 0 leaves
the default).

Reference numbers (this bench, with the latency fix): `ping` ~3 ms, `moveabs`
~8 ms (USB `bInterval` floor), moveabs throughput ~123/s (bInterval-capped).

## leak_check.py — assert no leak (retired single-board path)

Injects one centered `moveabs` and checks whether the real cursor moved,
restoring it if so. Expect `NO LEAK` while `hid_capture_usb` holds the device.

```bash
uv run --with pyserial --with pyobjc-framework-Quartz leak_check.py \
    --device /dev/cu.usbserial-XXXX
```
