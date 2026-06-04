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

_jumper = digitalio.DigitalInOut(board.D2)
_jumper.switch_to_input(pull=digitalio.Pull.UP)
DEV_MODE = not _jumper.value  # D2 grounded -> keep dev interfaces
_jumper.deinit()

usb_midi.disable()
usb_hid.enable((usb_hid.Device.KEYBOARD, usb_hid.Device.MOUSE))

if not DEV_MODE:
    storage.disable_usb_drive()
    usb_cdc.disable()
