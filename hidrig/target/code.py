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
TARGET BOARD  (KB2040 plugged into the Raspberry Pi via USB)

Role:
  - Presents itself to the Pi as a USB HID keyboard + mouse.
  - Acts as an I2C *target* (peripheral) on the STEMMA QT connector.
  - Receives command packets from the control board and replays them as
    HID events into the Pi.

Wiring:
  - USB-C  -> Raspberry Pi (this is the HID link)
  - STEMMA QT -> control board (this is the I2C link)

Requirements:
  - CircuitPython 9.x (i2ctarget is a built-in core module on RP2040)
  - adafruit_hid library bundle copied to /lib

Notes:
  - The RP2040 i2ctarget implementation uses I2C clock stretching. That is
    fine here because the I2C bus is RP2040 <-> RP2040. (The Pi, which is
    poor at clock stretching, is on USB, not on this I2C bus.)
  - RP2040 supports a single I2C target address only.
"""

import board
import time
import usb_hid
from i2ctarget import I2CTarget
from adafruit_hid.keyboard import Keyboard
from adafruit_hid.mouse import Mouse
from adafruit_hid.keyboard_layout_us import KeyboardLayoutUS

I2C_ADDRESS = 0x41

kbd = Keyboard(usb_hid.devices)
layout = KeyboardLayoutUS(kbd)
mouse = Mouse(usb_hid.devices)

# --- Opcodes (MUST match control/code.py) ---------------------------------
OP_KEY_PRESS = 0x01        # payload: one or more HID keycode bytes
OP_KEY_RELEASE = 0x02      # payload: one or more HID keycode bytes
OP_KEY_RELEASE_ALL = 0x03  # payload: none
OP_TYPE = 0x04             # payload: UTF-8 text bytes
OP_MOUSE_MOVE = 0x10       # payload: dx (int8), dy (int8)
OP_MOUSE_PRESS = 0x11      # payload: button mask (1=L, 2=R, 4=M)
OP_MOUSE_RELEASE = 0x12    # payload: button mask
OP_MOUSE_SCROLL = 0x13     # payload: amount (int8)


def s8(b):
    """Decode an unsigned byte as a signed int8."""
    return b - 256 if b > 127 else b


def handle(packet):
    if not packet:
        return
    op, payload = packet[0], packet[1:]

    if op == OP_KEY_PRESS:
        for kc in payload:
            kbd.press(kc)
    elif op == OP_KEY_RELEASE:
        for kc in payload:
            kbd.release(kc)
    elif op == OP_KEY_RELEASE_ALL:
        kbd.release_all()
    elif op == OP_TYPE:
        layout.write(payload.decode("utf-8", "replace"))
    elif op == OP_MOUSE_MOVE:
        dx = s8(payload[0]) if len(payload) > 0 else 0
        dy = s8(payload[1]) if len(payload) > 1 else 0
        mouse.move(x=dx, y=dy)
    elif op == OP_MOUSE_PRESS:
        mouse.press(payload[0])
    elif op == OP_MOUSE_RELEASE:
        mouse.release(payload[0])
    elif op == OP_MOUSE_SCROLL:
        mouse.move(wheel=s8(payload[0]))


# board.SCL / board.SDA are the STEMMA QT pins on the KB2040.
with I2CTarget(board.SCL, board.SDA, (I2C_ADDRESS,)) as device:
    while True:
        req = device.request()
        if not req:
            continue
        if req.is_read:
            # Handshake: controller is polling for ready — we only reach here
            # after handle() has returned, so we are ready for the next command.
            req.write(b'\x01')
        else:
            # Drain the RX FIFO in chunks until req.read() returns nothing.
            # The RP2040 FIFO is 16 bytes deep; for packets larger than that
            # we must read in a loop — a single read only returns what's
            # buffered so far, even with clock stretching active.
            data = bytearray()
            while True:
                chunk = req.read(64)
                if not chunk:
                    break
                data.extend(chunk)
                time.sleep(0.001)
            handle(bytes(data))
