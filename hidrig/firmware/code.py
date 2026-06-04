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
HID injector firmware (Adafruit KB2040, CircuitPython 9.x)

Reference implementation of the paniolo HID serial protocol, version 1 —
the normative spec is docs/hid-serial-protocol.md in the paniolo repo.

Single-board USB keyboard/mouse injector: the built-in USB port is a
device-mode HID keyboard + mouse plugged into the *target* machine; the
control host drives it with line-based text commands over the UART on the
TX/RX pins (115200 8N1, 3.3 V logic), typically via a USB-serial adapter.

Wiring:
  - USB         -> target machine (this is the HID link; it also powers us)
  - TX/RX + GND -> control host's USB-serial adapter (cross TX<->RX)
  - D2 -> GND   -> optional dev-mode jumper at boot (see boot.py)

Requirements:
  - CircuitPython 9.x
  - adafruit_hid library bundle copied to /lib
  - boot.py from this folder (clean HID-only USB identity)

Command protocol (one command per line, terminated by '\n'; see the spec
for the authoritative definition):
  type <text>            Type a string of text
  key <NAME>             Tap a key (press then release), e.g. "key ENTER"
  combo <NAME> <NAME>... Chord: press all, then release all, e.g. "combo LEFT_CONTROL C"
  down <NAME>            Press and hold a key
  up <NAME>              Release a held key
  releaseall             Release all held keys
  move <dx> <dy>         Relative mouse move in pixels
  click <left|right|middle>   Click a mouse button (default left)
  mdown <left|right|middle>   Press and hold a mouse button
  mup <left|right|middle>     Release a mouse button
  scroll <amount>        Scroll wheel (positive = up, negative = down)
  ping                   No-op health check
  version                Report protocol version + implementation id

<NAME> values are adafruit_hid Keycode names: A-Z, ZERO..NINE, ENTER, TAB,
SPACE, ESCAPE, BACKSPACE, LEFT_CONTROL, LEFT_SHIFT, LEFT_ALT, LEFT_GUI,
UP_ARROW, DOWN_ARROW, F1..F12, etc.

The board replies "OK\n" (or "OK <data>\n") on success or "ERR <message>\n"
on failure.

Status NeoPixel: green blip at startup, red while the last command failed
(cleared by the next successful one), red blink while waiting for the
target's USB to enumerate.
"""

import time

import board
import busio
import digitalio
import neopixel_write
import usb_hid
from adafruit_hid.keycode import Keycode

BAUD = 115200

PROTOCOL_VERSION = 1
IMPL_ID = "kb2040-circuitpython/1.0"

BUTTONS = {"left": 1, "right": 2, "middle": 4}

# --- Status NeoPixel (core neopixel_write; no /lib dependency) --------------
_px = digitalio.DigitalInOut(board.NEOPIXEL)
_px.direction = digitalio.Direction.OUTPUT


def status(r, g, b):
    neopixel_write.neopixel_write(_px, bytearray((g, r, b)))  # WS2812 is GRB


# --- HID devices -------------------------------------------------------------
# Constructing Keyboard()/Mouse() probes the host with a no-op report, which
# raises OSError until the target has enumerated us. Powered from the target's
# USB port we boot in parallel with it, so retry instead of crashing.
def make_devices():
    from adafruit_hid.keyboard import Keyboard
    from adafruit_hid.keyboard_layout_us import KeyboardLayoutUS
    from adafruit_hid.mouse import Mouse

    while True:
        try:
            kbd = Keyboard(usb_hid.devices)
            mouse = Mouse(usb_hid.devices)
            return kbd, KeyboardLayoutUS(kbd), mouse
        except OSError:
            status(16, 0, 0)
            time.sleep(0.25)
            status(0, 0, 0)
            time.sleep(0.25)


kbd, layout, mouse = make_devices()


def keycode_for(name):
    return getattr(Keycode, name.upper())


def handle_line(line):
    """Execute one command line; return extra OK-reply data or None."""
    parts = line.strip().split(" ")
    cmd = parts[0].lower()
    if not cmd:
        return None

    if cmd == "type":
        layout.write(line.split(" ", 1)[1] if " " in line else "")
    elif cmd == "key":
        kc = keycode_for(parts[1])
        kbd.press(kc)
        kbd.release(kc)
    elif cmd == "combo":
        kcs = [keycode_for(p) for p in parts[1:]]
        kbd.press(*kcs)
        kbd.release(*kcs)
    elif cmd == "down":
        kbd.press(keycode_for(parts[1]))
    elif cmd == "up":
        kbd.release(keycode_for(parts[1]))
    elif cmd == "releaseall":
        kbd.release_all()
    elif cmd == "move":
        # adafruit_hid splits moves beyond int8 into multiple reports itself.
        mouse.move(x=int(parts[1]), y=int(parts[2]))
    elif cmd == "click":
        b = BUTTONS[parts[1].lower()] if len(parts) > 1 else 1
        mouse.press(b)
        mouse.release(b)
    elif cmd == "mdown":
        mouse.press(BUTTONS[parts[1].lower()])
    elif cmd == "mup":
        mouse.release(BUTTONS[parts[1].lower()])
    elif cmd == "scroll":
        mouse.move(wheel=int(parts[1]))
    elif cmd == "ping":
        pass
    elif cmd == "version":
        return "%d %s" % (PROTOCOL_VERSION, IMPL_ID)
    else:
        raise ValueError("unknown command: " + cmd)
    return None


uart = busio.UART(
    board.TX, board.RX, baudrate=BAUD, timeout=0.01, receiver_buffer_size=512
)

status(0, 16, 0)  # green: up and listening
time.sleep(0.2)
status(0, 0, 0)

buf = b""
while True:
    data = uart.read(64)
    if not data:
        continue
    buf += data
    while b"\n" in buf:
        line, buf = buf.split(b"\n", 1)
        try:
            extra = handle_line(line.decode("utf-8"))
            if extra:
                uart.write(b"OK " + extra.encode("utf-8") + b"\n")
            else:
                uart.write(b"OK\n")
            status(0, 0, 0)
        except Exception as e:  # report back instead of dropping the line
            uart.write(b"ERR " + str(e).encode("utf-8") + b"\n")
            status(16, 0, 0)
