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
Configuration for KB2040 HID Injector.
This file is imported by both boot.py and code.py.
"""

# ROLE can be:
#   - "single": Single-board setup (default). USB connects to target, serial (UART) to host.
#   - "control": Host-facing board of a two-board setup. USB connects to host, I2C/serial to target board.
#   - "target": Target-facing board of a two-board setup. USB connects to target, I2C/serial to control board.
ROLE = "single"

# CONNECTION can be:
#   - "serial": UART serial connection (TX/RX pins).
#   - "i2c0": I2C0 connection using STEMMA/QT port (SDA/SCL pins).
#   - "i2c1": I2C1 connection using A0/A1 pins (A0=SDA, A1=SCL).
CONNECTION = "i2c1"

# I2C address used for the two-board communication
I2C_ADDRESS = 0x41
