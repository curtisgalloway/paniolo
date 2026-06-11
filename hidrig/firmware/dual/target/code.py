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
Dual-board rig — TARGET firmware (the "dumb pipe" relay).

This board is an I2C *peripheral* on I2C1 (D10 = GP10 = SDA, MOSI = GP19 = SCL),
address 0x41. It receives binary frames from the control board and relays HID
report frames straight to usb_hid.send_report — it never interprets the payload
(docs/hid-dual-board-design.md §3-5).

Frame format on the wire (host -> control -> target):
    [0x01][report-id][len][payload .. len bytes]   HID report frame
    [0x02][cmd][args ..]                            control frame (later)

For an HID frame it calls dev.send_report(payload, report_id), where dev is the
usb_hid.Device that owns report_id (keyboard = 1, abs mouse = 2, from boot.py).

Status NeoPixel:
    blue  = up, waiting for I2C
    green blip = a frame was received (proves the I2C link works)
    red blip = a frame arrived but send_report failed (DUT not enumerated yet)

This is milestone 1: prove the inter-board link. The control board self-drives
canned frames; no host/daemon is involved yet.
"""

import time

import board
import digitalio
import microcontroller
import neopixel_write
import usb_hid
from i2ctarget import I2CTarget

I2C_ADDR = 0x41
DEBUG = True  # prints to the REPL console (dev mode only)
RELAY_HID = True  # False = log frames but skip send_report (link-only diagnostic)

# --- Status NeoPixel (core neopixel_write; WS2812 is GRB) --------------------
_px = digitalio.DigitalInOut(board.NEOPIXEL)
_px.direction = digitalio.Direction.OUTPUT


def status(r, g, b):
    neopixel_write.neopixel_write(_px, bytearray((g, r, b)))


# --- HID report-id -> Device map (keyboard = 1, abs mouse = 2 per boot.py) ---
# usb_hid.Device doesn't expose its report ids as a readable attribute, so we
# map by HID usage instead — the one bit of descriptor knowledge the relay
# needs. The report ids (1, 2) match boot.py's enable() order/descriptor.
USAGE_TO_REPORT_ID = {
    (0x01, 0x06): 1,  # Generic Desktop / Keyboard
    (0x01, 0x02): 2,  # Generic Desktop / Mouse (our absolute pointer)
}


def build_report_map():
    m = {}
    for dev in usb_hid.devices:
        rid = USAGE_TO_REPORT_ID.get((dev.usage_page, dev.usage))
        if rid is not None:
            m[rid] = dev
    return m


reports = build_report_map()
if DEBUG:
    print("target: report ids ->", sorted(reports.keys()))


def handle_frame(data):
    """Relay one binary frame. HID frames go straight to send_report."""
    if not data:
        return
    ftype = data[0]
    if ftype == 0x01:  # HID report frame
        if len(data) < 3:
            if DEBUG:
                print("target: short HID frame", len(data))
            return
        report_id = data[1]
        length = data[2]
        payload = data[3:3 + length]
        if not RELAY_HID:
            return  # diagnostic mode: prove the I2C link without touching HID
        dev = reports.get(report_id)
        if dev is None:
            if DEBUG:
                print("target: no device for report id", report_id)
            return
        # The I2C receipt (green blip in the main loop) is the link proof; a
        # failed HID send (DUT not enumerated, wrong descriptor before boot.py
        # hard-resets) must not kill the loop, so swallow everything here.
        try:
            dev.send_report(payload, report_id)
        except Exception as e:  # noqa: BLE001 — keep relaying on any HID error
            if DEBUG:
                print("target: send_report failed:", e)
            return
    elif ftype == 0x02:  # control frame — milestone 2+
        if DEBUG:
            print("target: control frame", bytes(data))
    else:
        if DEBUG:
            print("target: unknown frame type", ftype)


# --- BOOT button (GP11) -> toggle dev / HID-only mode ----------------------
# Tap the BOOT button to flip the NVM mode flag and reset; boot.py reads the
# flag at startup (no jumper). Active-low (pressed = False). code.py runs in
# both modes, so the button is always an escape from HID-only.
button = digitalio.DigitalInOut(board.BUTTON)
button.switch_to_input(pull=digitalio.Pull.UP)
_btn_prev = True  # idle high (not pressed)


def toggle_mode_and_reset():
    cur = microcontroller.nvm[0]
    microcontroller.nvm[0:1] = bytes([0 if cur != 0 else 1])
    if DEBUG:
        print("target: BOOT pressed -> %s, resetting…"
              % ("HID-only" if cur != 0 else "dev"))
    while not button.value:  # wait for release so a held button can't enter
        time.sleep(0.01)     # the UF2 bootloader on the reset
    time.sleep(0.05)
    microcontroller.reset()


def check_button():
    global _btn_prev
    val = button.value
    if _btn_prev and not val:  # falling edge = press
        toggle_mode_and_reset()
    _btn_prev = val


# scl = MOSI (GP19), sda = D10 (GP10)  ->  I2C1, the inter-board link.
i2c = I2CTarget(board.MOSI, board.D10, (I2C_ADDR,))
status(0, 0, 16)  # blue: waiting for the controller
if DEBUG:
    print("target: I2CTarget up on 0x%02X (I2C1 = D10/MOSI), waiting…" % I2C_ADDR)

# I2C delivers each frame as a byte STREAM that fragments unpredictably across
# read() calls and request()s — read() returns whatever is momentarily in the
# FIFO, never a guaranteed whole transaction. So accumulate bytes and extract
# complete length-prefixed frames, exactly as a UART link would (transport-
# agnostic framing).
_rxbuf = bytearray()


def extract_frames():
    # Walk an index and reassign the tail — MicroPython's bytearray has no
    # slice-delete (`del buf[:n]` raises), so we can't pop from the front.
    global _rxbuf
    i = 0
    n = len(_rxbuf)
    while i < n:
        if _rxbuf[i] == 0x01:  # HID report frame: [0x01][rid][len][payload]
            if n - i < 3:
                break  # header incomplete; wait for more bytes
            need = 3 + _rxbuf[i + 2]
            if n - i < need:
                break  # payload incomplete; wait for more bytes
            frame = bytes(_rxbuf[i:i + need])
            i += need
            if DEBUG:
                print("target: frame", frame)
            handle_frame(frame)
            status(0, 16, 0)  # green blip per complete frame
            time.sleep(0.02)
            status(0, 0, 16)
        else:
            i += 1  # unframed/unknown byte (control frames TBD) — resync
    if i:
        _rxbuf = _rxbuf[i:]  # keep the unconsumed tail


while True:
    check_button()
    req = i2c.request()  # block until the controller addresses us
    if not req:
        continue
    # Do NOT use `with req:` — I2CTargetRequest's context manager calls a
    # nonexistent deinit() in CircuitPython 9.2.9 and throws on block exit,
    # crashing the loop after one frame and wedging the bus (SCL stuck low).
    if req.is_read:
        req.write(b"\x00")  # nothing to return in milestone 1
        continue
    while True:
        b = req.read(1)
        if not b:
            break
        _rxbuf.extend(b)
    extract_frames()
