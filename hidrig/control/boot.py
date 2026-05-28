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
CONTROL BOARD boot.py

Enables a second USB serial channel (the "data" channel) so the host test
scripts can send commands without colliding with the CircuitPython REPL
console. Runs once at power-on / reset.

After this is in place the control board enumerates TWO serial ports:
  - console port (the REPL)
  - data port    (used by host/example.py)
"""

import usb_cdc

usb_cdc.enable(console=True, data=True)
