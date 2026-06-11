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
Dual-board rig — CONTROL board boot.py (Adafruit KB2040, CircuitPython 9.x).

This board's USB faces the *control host*. It is NOT a HID device — it presents
a CDC interface to the host and routes frames downstream to the target board
over I2C1 (docs/hid-dual-board-design.md §4). So this file disables HID + MIDI
and enables both CDC endpoints:

  - console : the REPL, for editing code.py and watching debug prints
  - data    : a raw bidirectional CDC stream the host daemon will use to push
              binary frames (milestone 2+). Unused in milestone 1, but enabling
              it now keeps the USB identity stable across milestones.

Storage stays enabled (default), so this board always mounts CIRCUITPY for
edits — it never needs the D2 dev-mode jumper.
"""

import usb_cdc
import usb_hid
import usb_midi

usb_hid.disable()
usb_midi.disable()
usb_cdc.enable(console=True, data=True)
