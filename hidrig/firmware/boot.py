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
HID injector boot.py (Adafruit KB2040, CircuitPython 9.x)

The board's built-in USB port faces the *target* machine, so in normal
operation it must look like a plain keyboard + mouse: this file disables the
CIRCUITPY mass-storage drive, the CDC REPL console, and MIDI, leaving only
the HID keyboard + mouse interfaces.

The mouse is an **absolute pointer** (two 16-bit axes over a 0..32767 logical
range) instead of the default relative mouse, so the host can move the cursor
to an exact screen position — this is what makes the paniolo web console work
like a KVM (move-where-you-point). The firmware also tracks a virtual cursor
so relative `move` still works (it accumulates into the absolute position).

Dev mode: jumper D2 to GND (they are adjacent on the KB2040 edge) before
reset/power-on to keep CIRCUITPY + the REPL enabled for firmware updates.
Plug the board into a dev machine (not the target) for that.

boot.py only runs on a hard reset / power cycle — a soft reload (code save)
does not re-run it.
"""

import board
import digitalio
import storage
import usb_cdc
import usb_hid
import usb_midi

# Absolute-pointer report (report id 2): 1 byte buttons, two 16-bit absolute
# axes (0..32767), 1 byte relative wheel — 6 payload bytes. Keyboard keeps the
# standard report id 1, so the two HID devices share one interface cleanly.
ABS_MOUSE_DESCRIPTOR = bytes(
    (
        0x05, 0x01,        # Usage Page (Generic Desktop)
        0x09, 0x02,        # Usage (Mouse)
        0xA1, 0x01,        # Collection (Application)
        0x85, 0x02,        #   Report ID (2)
        0x09, 0x01,        #   Usage (Pointer)
        0xA1, 0x00,        #   Collection (Physical)
        0x05, 0x09,        #     Usage Page (Button)
        0x19, 0x01,        #     Usage Minimum (Button 1)
        0x29, 0x03,        #     Usage Maximum (Button 3)
        0x15, 0x00,        #     Logical Minimum (0)
        0x25, 0x01,        #     Logical Maximum (1)
        0x95, 0x03,        #     Report Count (3)
        0x75, 0x01,        #     Report Size (1)
        0x81, 0x02,        #     Input (Data, Variable, Absolute)
        0x95, 0x01,        #     Report Count (1)
        0x75, 0x05,        #     Report Size (5)
        0x81, 0x03,        #     Input (Constant) — 5-bit padding
        0x05, 0x01,        #     Usage Page (Generic Desktop)
        0x09, 0x30,        #     Usage (X)
        0x09, 0x31,        #     Usage (Y)
        0x16, 0x00, 0x00,  #     Logical Minimum (0)
        0x26, 0xFF, 0x7F,  #     Logical Maximum (32767)
        0x75, 0x10,        #     Report Size (16)
        0x95, 0x02,        #     Report Count (2)
        0x81, 0x02,        #     Input (Data, Variable, Absolute)
        0x09, 0x38,        #     Usage (Wheel)
        0x15, 0x81,        #     Logical Minimum (-127)
        0x25, 0x7F,        #     Logical Maximum (127)
        0x75, 0x08,        #     Report Size (8)
        0x95, 0x01,        #     Report Count (1)
        0x81, 0x06,        #     Input (Data, Variable, Relative)
        0xC0,              #   End Collection
        0xC0,              # End Collection
    )
)

ABS_MOUSE = usb_hid.Device(
    report_descriptor=ABS_MOUSE_DESCRIPTOR,
    usage_page=0x01,
    usage=0x02,
    report_ids=(2,),
    in_report_lengths=(6,),
    out_report_lengths=(0,),
)

import config

_jumper = digitalio.DigitalInOut(board.D2)
_jumper.switch_to_input(pull=digitalio.Pull.UP)
DEV_MODE = not _jumper.value  # D2 grounded -> keep dev interfaces
_jumper.deinit()

usb_midi.disable()

if config.ROLE == "control":
    # Control board doesn't need to emulate HID keyboard/mouse.
    # We always enable CDC data + console so it can receive commands and offer REPL/logs.
    usb_hid.disable()
    usb_cdc.enable(console=True, data=True)
    if not DEV_MODE:
        storage.disable_usb_drive()
else:
    # Target or single-board setups need the absolute mouse and keyboard HID.
    usb_hid.enable((usb_hid.Device.KEYBOARD, ABS_MOUSE))
    if not DEV_MODE:
        storage.disable_usb_drive()
        usb_cdc.disable()

