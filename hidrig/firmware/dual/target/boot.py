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
Dual-board rig — TARGET board boot.py (Adafruit KB2040, CircuitPython 9.x).

This is the board whose USB faces the *DUT*. It must look like a plain
keyboard + absolute-pointer mouse, so this file registers the HID descriptor
(the host↔rig contract, per docs/hid-dual-board-design.md §5) and — outside
dev mode — strips the CIRCUITPY drive, the CDC console, and MIDI.

The descriptor here is identical to the single-board firmware's boot.py: report
id 1 = keyboard, report id 2 = absolute mouse (0..32767 in each axis). The host
daemon composes report bytes to match it exactly; this board never interprets
them — it just relays them to send_report (see code.py).

Mode selection (no jumper needed in normal use):
  - The mode lives in NVM byte 0: 0 = HID-only (production), anything else =
    dev (CIRCUITPY + REPL + HID). Erased NVM reads 0xFF, so a fresh board boots
    in dev mode — safe for editing.
  - Tap the BOOT button (GP11) while running and code.py flips the flag and
    resets, so a press toggles dev <-> HID-only with no jumper.
  - Grounding D2 at reset forces dev mode regardless of the flag — a hardware
    fallback so a wedged code.py can never strand the board in HID-only.

boot.py only runs on a hard reset / power cycle — a soft reload does not re-run it.
"""

import board
import digitalio
import microcontroller
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

# Mode flag in NVM byte 0 (0 = HID-only, else dev), with D2-to-GND as a
# hardware override that forces dev mode even if code.py is wedged.
_d2 = digitalio.DigitalInOut(board.D2)
_d2.switch_to_input(pull=digitalio.Pull.UP)
_d2_forces_dev = not _d2.value
_d2.deinit()
DEV_MODE = (microcontroller.nvm[0] != 0) or _d2_forces_dev

usb_midi.disable()
usb_hid.enable((usb_hid.Device.KEYBOARD, ABS_MOUSE))

if not DEV_MODE:
    storage.disable_usb_drive()
    usb_cdc.disable()
