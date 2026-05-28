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
CONTROL BOARD  (KB2040 or USB Trinkey QT2040 plugged into the test computer)

Role:
  - Reads line-based text commands from the host over the USB CDC data channel.
  - Translates them into compact binary packets.
  - Acts as the I2C *controller* on STEMMA QT and writes packets to the target.

Wiring:
  - USB -> test computer (command link)
  - STEMMA QT -> target board (I2C link)

Requirements:
  - CircuitPython 9.x
  - adafruit_hid library bundle copied to /lib (used here only for the
    Keycode name->value table, so we can accept human-readable key names)
  - boot.py from this folder (enables the usb_cdc data channel)

Command protocol (one command per line, terminated by '\n'):
  type <text>            Type a string of text
  key <NAME>             Tap a key (press then release), e.g. "key ENTER"
  combo <NAME> <NAME>... Chord: press all, then release all, e.g. "combo LEFT_CONTROL C"
  down <NAME>            Press and hold a key
  up <NAME>              Release a held key
  releaseall             Release all held keys
  move <dx> <dy>         Relative mouse move in pixels (auto-split into HID steps)
  click <left|right|middle>   Click a mouse button (default left)
  mdown <left|right|middle>   Press and hold a mouse button
  mup <left|right|middle>     Release a mouse button
  scroll <amount>        Scroll wheel (positive = up, negative = down)

<NAME> values are adafruit_hid Keycode names: A-Z, ZERO..NINE, ENTER, TAB,
SPACE, ESCAPE, BACKSPACE, LEFT_CONTROL, LEFT_SHIFT, LEFT_ALT, LEFT_GUI,
UP_ARROW, DOWN_ARROW, F1..F12, etc.

The board replies "OK\n" on success or "ERR <message>\n" on failure.
"""

import board
import time
import usb_cdc
from adafruit_hid.keycode import Keycode

TARGET_ADDRESS = 0x41

serial = usb_cdc.data
i2c = board.STEMMA_I2C()   # fallback if needed: busio.I2C(board.SCL, board.SDA)

# --- Opcodes (MUST match target/code.py) ----------------------------------
OP_KEY_PRESS, OP_KEY_RELEASE, OP_KEY_RELEASE_ALL, OP_TYPE = 0x01, 0x02, 0x03, 0x04
OP_MOUSE_MOVE, OP_MOUSE_PRESS, OP_MOUSE_RELEASE, OP_MOUSE_SCROLL = 0x10, 0x11, 0x12, 0x13

BUTTONS = {"left": 1, "right": 2, "middle": 4}

MAX_TYPE_CHUNK = 30


def send(packet):
    while not i2c.try_lock():
        pass
    try:
        i2c.writeto(TARGET_ADDRESS, bytes(packet))
    finally:
        i2c.unlock()
    # Handshake: poll a 1-byte read from the target until it returns 0x01,
    # meaning handle() has finished and it is ready for the next command.
    buf = bytearray(1)
    while True:
        while not i2c.try_lock():
            pass
        try:
            i2c.readfrom_into(TARGET_ADDRESS, buf)
            if buf[0] == 0x01:
                break
        except OSError:
            time.sleep(0.001)
        finally:
            i2c.unlock()


def keycode_for(name):
    return getattr(Keycode, name.upper())


def clamp(v, lo, hi):
    return max(lo, min(hi, v))


def u8(v):
    """Two's-complement encode a signed value into an unsigned byte."""
    return v & 0xFF


def do_move(dx, dy):
    # HID relative movement is int8 per report; split larger moves into steps.
    while dx or dy:
        sx, sy = clamp(dx, -127, 127), clamp(dy, -127, 127)
        send([OP_MOUSE_MOVE, u8(sx), u8(sy)])
        dx -= sx
        dy -= sy


def handle_line(line):
    parts = line.strip().split(" ")
    cmd = parts[0].lower()
    if not cmd:
        return

    if cmd == "type":
        data = (line.split(" ", 1)[1] if " " in line else "").encode("utf-8")
        for i in range(0, len(data), MAX_TYPE_CHUNK):
            send([OP_TYPE] + list(data[i:i + MAX_TYPE_CHUNK]))
    elif cmd == "key":
        kc = keycode_for(parts[1])
        send([OP_KEY_PRESS, kc])
        send([OP_KEY_RELEASE, kc])
    elif cmd == "combo":
        kcs = [keycode_for(p) for p in parts[1:]]
        send([OP_KEY_PRESS] + kcs)
        send([OP_KEY_RELEASE] + kcs)
    elif cmd == "down":
        send([OP_KEY_PRESS, keycode_for(parts[1])])
    elif cmd == "up":
        send([OP_KEY_RELEASE, keycode_for(parts[1])])
    elif cmd == "releaseall":
        send([OP_KEY_RELEASE_ALL])
    elif cmd == "move":
        do_move(int(parts[1]), int(parts[2]))
    elif cmd == "click":
        b = BUTTONS[parts[1].lower()] if len(parts) > 1 else 1
        send([OP_MOUSE_PRESS, b])
        send([OP_MOUSE_RELEASE, b])
    elif cmd == "mdown":
        send([OP_MOUSE_PRESS, BUTTONS[parts[1].lower()]])
    elif cmd == "mup":
        send([OP_MOUSE_RELEASE, BUTTONS[parts[1].lower()]])
    elif cmd == "scroll":
        send([OP_MOUSE_SCROLL, u8(clamp(int(parts[1]), -127, 127))])
    else:
        raise ValueError("unknown command: " + cmd)


buf = b""
while True:
    if serial.in_waiting:
        buf += serial.read(serial.in_waiting)
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            try:
                handle_line(line.decode("utf-8"))
                serial.write(b"OK\n")
            except Exception as e:  # report back instead of dropping the line
                serial.write(b"ERR " + str(e).encode("utf-8") + b"\n")
