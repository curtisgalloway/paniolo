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

"""Power control helpers: FTDI DTR line via the serialcap daemon or pyserial fallback."""  # pylint: disable=line-too-long

from __future__ import annotations

import time
import urllib.error
import urllib.request


def dtr_button_press(daemon_url: str, interface_name: str, duration_ms: int) -> None:
    """Assert DTR (J2 power button) for duration_ms ms via the serialcap daemon.

    The daemon owns the serial port exclusively and drives the DTR line on its
    supervisor task.  This call blocks until the press completes.

    duration_ms guidance (Raspberry Pi 5 / DA9091 PMIC):
      ≤500 ms  — soft reset signal; OS handles it (graceful reboot or halt)
      ≥3000 ms — hard power-off; follow with another call to power the board on

    Raises RuntimeError on HTTP error, OSError on network failure.
    """
    url = f"{daemon_url}/button?interface={interface_name}&ms={duration_ms}"
    req = urllib.request.Request(url, method="POST", data=b"")
    try:
        with urllib.request.urlopen(
            req, timeout=max(15, duration_ms // 1000 + 5)
        ) as resp:
            resp.read()
    except urllib.error.HTTPError as exc:
        raise RuntimeError(
            f"serialcap /button returned {exc.code}: {exc.reason}"
        ) from exc


def dtr_direct_button_press(device: str, duration_ms: int) -> None:
    """Assert DTR (J2 power button) for duration_ms milliseconds directly via pyserial.

    Fallback for when the serialcap daemon is not running.  Opens the serial
    port, asserts DTR for the requested duration, then releases and closes.

    Raises RuntimeError if pyserial is not installed or on serial errors.
    """
    try:
        import serial as _serial  # pylint: disable=import-outside-toplevel
    except ImportError as exc:
        raise RuntimeError(
            "pyserial is required for direct DTR control. "
            "Install it with: uv add pyserial"
        ) from exc

    port = _serial.Serial()
    port.port = device
    port.baudrate = 115200
    port.open()
    try:
        port.dtr = False
        time.sleep(0.05)  # brief settle after open
        port.dtr = True
        time.sleep(duration_ms / 1000.0)
        port.dtr = False
    finally:
        port.close()
