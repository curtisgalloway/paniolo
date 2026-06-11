#!/usr/bin/env python3
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

"""host_send.py — milestone-2 test driver for the dual-board HID rig.

Pushes length-prefixed frames to the CONTROL board's usb_cdc.data endpoint,
which relays HID frames over I2C1 to the target board (which injects them as
USB-HID into whatever it is plugged into).

Run with pyserial available, e.g.:
    uv run --with pyserial python host_send.py --port /dev/cu.usbmodemXXXX mouse 16000 16000

Commands:
    mouse <x> <y> [left|right|middle]   absolute pointer (0..32767); a button
                                        name makes it a click at (x, y)
    key <NAME|0xNN>                     tap one key (e.g. ENTER, A, 0x28)
    type <text...>                      type an ASCII string (a-z 0-9 space .,-/)
    ping                               liveness check (expects an ack)
    version                            report the control firmware id

--port is the control board's *data* CDC interface — the second usbmodem of the
board's pair (the first is the REPL console). Composition proper lives in the M3
Rust daemon; the keymap here is just enough to demo keystrokes.
"""

import argparse
import time

import serial

ABS_MAX = 32767
BTN = {"left": 1, "right": 2, "middle": 4}

# Minimal US-keyboard usage map for the type/key test commands.
_KC = {}
for _i, _c in enumerate("abcdefghijklmnopqrstuvwxyz"):
    _KC[_c] = 0x04 + _i
for _i, _c in enumerate("1234567890"):
    _KC[_c] = 0x1E + _i
_KC[" "] = 0x2C
_KC["\n"] = 0x28
_KC["-"] = 0x2D
_KC["."] = 0x37
_KC[","] = 0x36
_KC["/"] = 0x38
NAMED = {"ENTER": 0x28, "TAB": 0x2B, "ESC": 0x29, "SPACE": 0x2C, "BACKSPACE": 0x2A}


def kbd_frame(modifier=0, keycode=0):
    """type 0x01, report-id 1 (keyboard), 8-byte report."""
    report = bytes([modifier & 0xFF, 0, keycode & 0xFF, 0, 0, 0, 0, 0])
    return bytes((0x01, 0x01, len(report))) + report


def mouse_abs_frame(x, y, buttons=0, wheel=0):
    """type 0x01, report-id 2 (absolute pointer), 6-byte report."""
    p = bytes(
        (buttons & 7, x & 0xFF, (x >> 8) & 0xFF, y & 0xFF, (y >> 8) & 0xFF, wheel & 0xFF)
    )
    return bytes((0x01, 0x02, len(p))) + p


def ctrl_frame(cmd, args=b""):
    return bytes((0x02, cmd, len(args))) + args


def keycode_for(name):
    up = name.upper()
    if up in NAMED:
        return NAMED[up], 0
    if name.lower().startswith("0x"):
        return int(name, 16), 0
    if len(name) == 1 and name.lower() in _KC:
        mod = 0x02 if name.isupper() else 0
        return _KC[name.lower()], mod
    raise SystemExit("unknown key: %s" % name)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", required=True, help="control board data CDC port")
    ap.add_argument("cmd")
    ap.add_argument("args", nargs="*")
    a = ap.parse_args()
    s = serial.Serial(a.port, 115200, timeout=0.5)

    if a.cmd == "mouse":
        x, y = int(a.args[0]), int(a.args[1])
        if len(a.args) > 2:
            b = BTN.get(a.args[2], 0)
            s.write(mouse_abs_frame(x, y, b))
            time.sleep(0.02)
            s.write(mouse_abs_frame(x, y, 0))  # release -> a click at (x, y)
        else:
            s.write(mouse_abs_frame(x, y, 0))
    elif a.cmd == "key":
        kc, mod = keycode_for(a.args[0])
        s.write(kbd_frame(mod, kc))
        time.sleep(0.01)
        s.write(kbd_frame(0, 0))  # release
    elif a.cmd == "type":
        for ch in " ".join(a.args):
            lc = ch.lower()
            if lc not in _KC:
                continue
            mod = 0x02 if ch.isupper() else 0
            s.write(kbd_frame(mod, _KC[lc]))
            time.sleep(0.008)
            s.write(kbd_frame(0, 0))
            time.sleep(0.008)
    elif a.cmd == "ping":
        s.write(ctrl_frame(0x01))
        time.sleep(0.15)
        print("reply:", s.read(8))
    elif a.cmd == "version":
        s.write(ctrl_frame(0x02))
        time.sleep(0.15)
        print("reply:", s.read(32))
    else:
        raise SystemExit("unknown command: %s" % a.cmd)
    s.close()


if __name__ == "__main__":
    main()
