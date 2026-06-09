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
  move <dx> <dy>         Relative mouse move (accumulates into the cursor)
  moveabs <x> <y>        Absolute mouse move in a 0..32767 logical space
  click <left|right|middle>   Click a mouse button (default left)
  mdown <left|right|middle>   Press and hold a mouse button
  mup <left|right|middle>     Release a mouse button
  scroll <amount>        Scroll wheel (positive = up, negative = down)
  baud <rate>            Switch the UART to <rate> after acking at the old rate
  ping                   No-op health check
  version                Report protocol version + implementation id + caps

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
import usb_cdc
from adafruit_hid.keycode import Keycode

import config


BAUD = 115200

PROTOCOL_VERSION = 1
IMPL_ID = "kb2040-circuitpython/1.0"
# Capability tokens advertised in the `version` reply (see the protocol spec).
CAPS = "moveabs baud"

BUTTONS = {"left": 1, "right": 2, "middle": 4}

# Absolute-pointer logical range (matches the HID descriptor in boot.py); the
# host OS spreads 0..ABS_MAX across the full screen in each axis.
ABS_MAX = 32767

# --- Status NeoPixel (core neopixel_write; no /lib dependency) --------------
_px = digitalio.DigitalInOut(board.NEOPIXEL)
_px.direction = digitalio.Direction.OUTPUT


def status(r, g, b):
    neopixel_write.neopixel_write(_px, bytearray((g, r, b)))  # WS2812 is GRB


# --- HID devices -------------------------------------------------------------
kbd = None
layout = None
abs_mouse = None
_mx = ABS_MAX // 2
_my = ABS_MAX // 2
_buttons = 0
_report = bytearray(6)
_pending_baud = None

if config.ROLE in ("single", "target"):
    def _find_abs_mouse():
        """The absolute-pointer Device registered in boot.py (usage page 1, mouse)."""
        for dev in usb_hid.devices:
            if dev.usage_page == 0x01 and dev.usage == 0x02:
                return dev
        raise RuntimeError("absolute-mouse HID device not found — check boot.py")


    # Constructing Keyboard() probes the host with a no-op report, which raises
    # OSError until the target has enumerated us. Powered from the target's USB
    # port we boot in parallel with it, so retry instead of crashing.
    def make_devices():
        from adafruit_hid.keyboard import Keyboard
        from adafruit_hid.keyboard_layout_us import KeyboardLayoutUS

        while True:
            try:
                kbd_dev = Keyboard(usb_hid.devices)
                return kbd_dev, KeyboardLayoutUS(kbd_dev), _find_abs_mouse()
            except OSError:
                status(16, 0, 0)
                time.sleep(0.25)
                status(0, 0, 0)
                time.sleep(0.25)


    kbd, layout, abs_mouse = make_devices()



def keycode_for(name):
    return getattr(Keycode, name.upper())


def clamp(v, lo, hi):
    return lo if v < lo else hi if v > hi else v


def send_mouse(wheel=0):
    """Emit one absolute-pointer report at the current cursor + button state."""
    _report[0] = _buttons & 0x07
    _report[1] = _mx & 0xFF
    _report[2] = (_mx >> 8) & 0xFF
    _report[3] = _my & 0xFF
    _report[4] = (_my >> 8) & 0xFF
    _report[5] = wheel & 0xFF  # int8 two's-complement
    abs_mouse.send_report(_report, 2)


def handle_line(line):
    """Execute one command line; return extra OK-reply data or None."""
    global _mx, _my, _buttons, _pending_baud
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
        # Relative move: accumulate into the virtual absolute cursor.
        _mx = clamp(_mx + int(parts[1]), 0, ABS_MAX)
        _my = clamp(_my + int(parts[2]), 0, ABS_MAX)
        send_mouse()
    elif cmd == "moveabs":
        _mx = clamp(int(parts[1]), 0, ABS_MAX)
        _my = clamp(int(parts[2]), 0, ABS_MAX)
        send_mouse()
    elif cmd == "click":
        b = BUTTONS[parts[1].lower()] if len(parts) > 1 else 1
        _buttons |= b
        send_mouse()
        _buttons &= ~b
        send_mouse()
    elif cmd == "mdown":
        _buttons |= BUTTONS[parts[1].lower()]
        send_mouse()
    elif cmd == "mup":
        _buttons &= ~BUTTONS[parts[1].lower()]
        send_mouse()
    elif cmd == "scroll":
        send_mouse(wheel=clamp(int(parts[1]), -127, 127))
    elif cmd == "baud":
        new = int(parts[1])
        if not 1200 <= new <= 2000000:
            raise ValueError("baud out of range")
        # Defer the actual switch to the main loop, after OK is sent.
        if config.CONNECTION == "serial" or config.ROLE == "single":
            _pending_baud = new
    elif cmd == "ping":
        pass
    elif cmd == "version":
        return "%d %s %s" % (PROTOCOL_VERSION, IMPL_ID, CAPS)
    else:
        raise ValueError("unknown command: " + cmd)
    return None


# --- Setup and Main loop -----------------------------------------------------

if config.ROLE == "control":
    # -------------------------------------------------------------------------
    # CONTROL BOARD
    # -------------------------------------------------------------------------
    if usb_cdc.data is None:
        # Blink red to indicate missing USB CDC data channel configuration in boot.py
        while True:
            status(16, 0, 0)
            time.sleep(0.1)
            status(0, 0, 0)
            time.sleep(0.1)

    host_serial = usb_cdc.data

    # Initialize the downstream link to the Target board
    if config.CONNECTION == "serial":
        uart = busio.UART(
            board.TX, board.RX, baudrate=BAUD, timeout=0.001, receiver_buffer_size=512
        )
    elif config.CONNECTION == "i2c0":
        i2c = board.STEMMA_I2C()
    elif config.CONNECTION == "i2c1":
        # A1 is SCL, A0 is SDA for I2C1 on KB2040
        i2c = busio.I2C(board.A1, board.A0)
    else:
        raise ValueError("Unknown connection type: " + config.CONNECTION)

    def send_to_target(line_bytes):
        global _pending_baud
        if config.CONNECTION == "serial":
            # Drain any stale data in UART
            if uart.in_waiting:
                uart.read(uart.in_waiting)
            uart.write(line_bytes)
            # Read reply
            reply = b""
            while True:
                b = uart.read(1)
                if b:
                    reply += b
                    if b == b"\n":
                        break
                else:
                    time.sleep(0.001)

            # If the command was a baud switch and reply is OK, defer changing our own UART baud
            parts = line_bytes.strip().split(b" ")
            if parts[0].lower() == b"baud" and reply.startswith(b"OK"):
                _pending_baud = int(parts[1])
            return reply
        else:
            # I2C connection (i2c0 or i2c1)
            while not i2c.try_lock():
                time.sleep(0.001)
            try:
                i2c.writeto(config.I2C_ADDRESS, line_bytes)
            finally:
                i2c.unlock()

            # Read response
            buf = bytearray(128)
            while not i2c.try_lock():
                time.sleep(0.001)
            try:
                i2c.readfrom_into(config.I2C_ADDRESS, buf)
            finally:
                i2c.unlock()

            if b"\n" in buf:
                return buf.split(b"\n", 1)[0] + b"\n"
            else:
                return bytes(buf).rstrip(b"\x00\xff") + b"\n"

    status(0, 16, 0)  # green: up and listening
    time.sleep(0.2)
    status(0, 0, 0)

    buf = b""
    while True:
        if host_serial.in_waiting:
            buf += host_serial.read(host_serial.in_waiting)
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                try:
                    reply = send_to_target(line + b"\n")
                    host_serial.write(reply)
                    if reply.startswith(b"OK"):
                        status(0, 0, 0)
                    else:
                        status(16, 0, 0)
                except Exception as e:
                    host_serial.write(b"ERR " + str(e).encode("utf-8") + b"\n")
                    status(16, 0, 0)

        if _pending_baud is not None:
            time.sleep(0.05)  # let OK drain
            uart.deinit()
            uart = busio.UART(
                board.TX, board.RX, baudrate=_pending_baud,
                timeout=0.001, receiver_buffer_size=512,
            )
            _pending_baud = None

elif config.ROLE == "target" and config.CONNECTION in ("i2c0", "i2c1"):
    # -------------------------------------------------------------------------
    # TARGET BOARD (I2C)
    # -------------------------------------------------------------------------
    from i2ctarget import I2CTarget

    def get_i2c_target():
        if config.CONNECTION == "i2c0":
            return I2CTarget(board.SCL, board.SDA, (config.I2C_ADDRESS,))
        elif config.CONNECTION == "i2c1":
            return I2CTarget(board.A1, board.A0, (config.I2C_ADDRESS,))
        else:
            raise ValueError("Unknown I2C connection type: " + config.CONNECTION)

    device = get_i2c_target()
    reply_buffer = b"OK\n"

    status(0, 16, 0)  # green: up and listening
    time.sleep(0.2)
    status(0, 0, 0)

    while True:
        req = device.request()
        if not req:
            continue
        if req.is_read:
            req.write(reply_buffer)
        else:
            data = bytearray()
            while True:
                chunk = req.read(64)
                if not chunk:
                    break
                data.extend(chunk)
                time.sleep(0.001)
            try:
                extra = handle_line(bytes(data).decode("utf-8"))
                if extra:
                    reply_buffer = b"OK " + extra.encode("utf-8") + b"\n"
                else:
                    reply_buffer = b"OK\n"
                status(0, 0, 0)
            except Exception as e:
                reply_buffer = b"ERR " + str(e).encode("utf-8") + b"\n"
                status(16, 0, 0)

else:
    # -------------------------------------------------------------------------
    # SINGLE BOARD or TARGET BOARD (SERIAL)
    # -------------------------------------------------------------------------
    uart = busio.UART(
        board.TX, board.RX, baudrate=BAUD, timeout=0.001, receiver_buffer_size=512
    )

    status(0, 16, 0)  # green: up and listening
    time.sleep(0.2)
    status(0, 0, 0)

    buf = b""
    while True:
        data = uart.read(128)
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
        # Apply a `baud` switch only after its OK has been acked at the old rate.
        if _pending_baud is not None:
            time.sleep(0.05)  # let the OK fully drain at the current baud
            uart.deinit()
            uart = busio.UART(
                board.TX, board.RX, baudrate=_pending_baud,
                timeout=0.001, receiver_buffer_size=512,
            )
            _pending_baud = None

