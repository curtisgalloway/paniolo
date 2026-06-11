# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""
Dual-board rig — CONTROL firmware (milestone 1: self-driving link test).

This board is the I2C *controller* on I2C1 (D10 = GP10 = SDA, MOSI = GP19 = SCL).
To prove the inter-board link before any host/daemon exists, it composes a
canned HID frame and writes it to the target board (address 0x41) once a second.

Pick the stimulus with TEST below:
  "mouse" — absolute-mouse wiggle between two points (VISIBLE: it moves the
            cursor on whatever the target board is plugged into — only use this
            if the target's USB is on a spare machine you don't mind twitching)
  "noop"  — an all-zero keyboard report (proves send_report with NO visible
            effect; rely on the NeoPixels for the success signal)

Status NeoPixel:
    green blip = target ACKed the I2C write (link OK)
    red blip   = no ACK (pull-ups? target code not running? wrong addr/pins?)

In milestone 2 this loop is replaced by "read a frame from usb_cdc.data, route
by the type byte, relay HID frames over I2C" — the host daemon composes them.
"""

import time

import board
import busio
import digitalio
import neopixel_write

I2C_ADDR = 0x41
ABS_MAX = 32767
TEST = "noop"  # "noop" (no visible effect; safe default) or "mouse" (cursor wiggle)

# --- Status NeoPixel (core neopixel_write; WS2812 is GRB) --------------------
_px = digitalio.DigitalInOut(board.NEOPIXEL)
_px.direction = digitalio.Direction.OUTPUT


def status(r, g, b):
    neopixel_write.neopixel_write(_px, bytearray((g, r, b)))


def hid_mouse_frame(x, y, buttons=0, wheel=0):
    """type 0x01, report id 2 (abs mouse), 6-byte payload."""
    payload = bytes(
        (
            buttons & 0x07,
            x & 0xFF,
            (x >> 8) & 0xFF,
            y & 0xFF,
            (y >> 8) & 0xFF,
            wheel & 0xFF,
        )
    )
    return bytes((0x01, 0x02, len(payload))) + payload


def hid_keyboard_noop_frame():
    """type 0x01, report id 1 (keyboard), 8 zero bytes — no keys pressed."""
    payload = bytes(8)
    return bytes((0x01, 0x01, len(payload))) + payload


# scl = MOSI (GP19), sda = D10 (GP10)  ->  I2C1, the inter-board link.
i2c = busio.I2C(board.MOSI, board.D10, frequency=100000)

_mid = ABS_MAX // 2
_points = [_mid - 2000, _mid + 2000]
_idx = 0

print("control: I2C1 controller up, poking target 0x%02X every 1s (TEST=%s)" % (I2C_ADDR, TEST))

while True:
    if TEST == "mouse":
        frame = hid_mouse_frame(_points[_idx], _mid)
        _idx ^= 1
    else:
        frame = hid_keyboard_noop_frame()

    while not i2c.try_lock():
        pass
    try:
        i2c.writeto(I2C_ADDR, frame)
        status(0, 16, 0)  # green: target ACKed
        ok = True
    except OSError as e:
        status(16, 0, 0)  # red: no ACK
        ok = False
        print("control: write failed:", e)
    finally:
        i2c.unlock()

    if ok:
        print("control: sent", bytes(frame))
    time.sleep(0.5)
    status(0, 0, 0)
    time.sleep(0.5)
