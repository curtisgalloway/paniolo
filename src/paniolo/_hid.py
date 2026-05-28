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

"""Host control client for the KB2040 HID rig (see hidrig/).

Sends line-based text commands to the control board over its USB CDC *data*
port; the control board parses them and relays HID keyboard/mouse events to the
target board, which injects them into the Pi over USB. The board owns the wire
protocol — `hidrig/control/code.py` and `hidrig/README.md` are the source of
truth. This module is a thin text-command client plus host-side sequencing.
"""

from __future__ import annotations

import dataclasses
import glob
import sys
import time
import tomllib
from pathlib import Path
from typing import Callable, Optional

from . import _config

HID_CONFIG_PATH = _config.CONFIG_DIR / "hid.toml"

DEFAULT_BAUD = 115200  # irrelevant over USB CDC, but pyserial requires a value

# Absolute-mouse logical range the OS spreads across the screen (HID convention).
ABS_MAX = 32767


@dataclasses.dataclass
class HidConfig:
    """Saved configuration for the HID control board."""

    port: str


def _to_toml(data: dict) -> str:
    lines = []
    for key, value in data.items():
        if value is None:
            continue
        if isinstance(value, str):
            escaped = value.replace("\\", "\\\\").replace('"', '\\"')
            lines.append(f'{key} = "{escaped}"')
        elif isinstance(value, bool):
            lines.append(f'{key} = {"true" if value else "false"}')
        else:
            lines.append(f"{key} = {value}")
    return "\n".join(lines) + "\n"


def save_hid_config(cfg: HidConfig) -> None:
    _config.CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    HID_CONFIG_PATH.write_text(_to_toml(dataclasses.asdict(cfg)))


def load_hid_config() -> Optional[HidConfig]:
    if not HID_CONFIG_PATH.exists():
        return None
    with open(HID_CONFIG_PATH, "rb") as f:
        data = tomllib.load(f)
    return HidConfig(port=data["port"])


def list_serial_ports() -> list[str]:
    """Candidate USB CDC ports for the control board."""
    if sys.platform == "darwin":
        return sorted(glob.glob("/dev/cu.usbmodem*"))
    return sorted(glob.glob("/dev/ttyACM*"))


def guess_data_port() -> Optional[str]:
    """Best guess at the control board's *data* CDC port.

    The board exposes two CDC ports (console + data); the data port is
    conventionally the higher-numbered node. Returns None if no candidates.
    """
    ports = list_serial_ports()
    return ports[-1] if ports else None


def scale_to_logical(px: int, screen_px: int) -> int:
    """Map a pixel coordinate to the 0..32767 absolute-mouse logical range.

    The host OS maps that range across the full screen dimension, so callers
    scale each pixel axis against the screen's size in that axis. Clamped.
    """
    if screen_px <= 1:
        return 0
    v = round(px * ABS_MAX / (screen_px - 1))
    return max(0, min(ABS_MAX, v))


class HidRig:
    """Text-command client for the control board over USB serial.

    Pass `transport` (any object with `write(bytes)`, `readline() -> bytes`,
    `close()`) to drive it without real hardware (used by tests). Otherwise a
    `pyserial` Serial port is opened lazily on the given `port`.
    """

    def __init__(
        self,
        port: Optional[str] = None,
        baud: int = DEFAULT_BAUD,
        timeout: float = 1.0,
        transport=None,
    ):
        if transport is not None:
            self._transport = transport
            return
        try:
            import serial  # lazy: only the live path needs pyserial
        except ImportError as exc:
            raise RuntimeError(
                "pyserial not installed — install the hid extra: "
                "uv sync --extra hid  (or: pip install 'paniolo[hid]')"
            ) from exc
        if not port:
            raise ValueError("no serial port given")
        self._transport = serial.Serial(port, baud, timeout=timeout)
        time.sleep(0.2)
        self._transport.reset_input_buffer()

    def cmd(self, text: str) -> str:
        """Send one command line; return the board's reply, raise on ERR."""
        self._transport.write((text + "\n").encode("utf-8"))
        reply = self._transport.readline().decode("utf-8", "replace").strip()
        if reply.startswith("ERR"):
            raise RuntimeError(f"control board rejected '{text}': {reply}")
        return reply

    # Command wrappers — mirror hidrig/control/code.py's text protocol.
    def type(self, text: str) -> str:
        return self.cmd(f"type {text}")

    def key(self, name: str) -> str:
        return self.cmd(f"key {name}")

    def combo(self, *names: str) -> str:
        return self.cmd("combo " + " ".join(names))

    def down(self, name: str) -> str:
        return self.cmd(f"down {name}")

    def up(self, name: str) -> str:
        return self.cmd(f"up {name}")

    def releaseall(self) -> str:
        return self.cmd("releaseall")

    def move(self, dx: int, dy: int) -> str:
        return self.cmd(f"move {dx} {dy}")

    def click(self, button: str = "left") -> str:
        return self.cmd(f"click {button}")

    def mdown(self, button: str = "left") -> str:
        return self.cmd(f"mdown {button}")

    def mup(self, button: str = "left") -> str:
        return self.cmd(f"mup {button}")

    def scroll(self, amount: int) -> str:
        return self.cmd(f"scroll {amount}")

    def close(self) -> None:
        self._transport.close()


# --- Host-side sequencing / timing (the board firmware stays dumb) ----------

def parse_sequence(text: str) -> list[tuple[str, object]]:
    """Parse a command file into steps.

    Each non-blank, non-`#`-comment line is either a command or a timing
    directive: `delay <ms>` or `sleep <seconds>`. Returns a list of
    `("cmd", line)` / `("delay", seconds)` tuples.
    """
    steps: list[tuple[str, object]] = []
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        head, _, rest = line.partition(" ")
        low = head.lower()
        if low == "delay":
            steps.append(("delay", float(rest) / 1000.0))
        elif low == "sleep":
            steps.append(("delay", float(rest)))
        else:
            steps.append(("cmd", line))
    return steps


def run_sequence(
    rig: HidRig,
    steps: list[tuple[str, object]],
    default_delay: float = 0.0,
    sleep: Callable[[float], None] = time.sleep,
) -> None:
    """Execute parsed steps against `rig`. `sleep` is injectable for tests."""
    for kind, value in steps:
        if kind == "delay":
            sleep(float(value))
        else:
            rig.cmd(str(value))
            if default_delay:
                sleep(default_delay)


def repeat_key(
    rig: HidRig,
    name: str,
    count: int,
    delay: float = 0.0,
    sleep: Callable[[float], None] = time.sleep,
) -> None:
    """Tap a key `count` times with an inter-tap delay (auto-repeat)."""
    for i in range(count):
        rig.key(name)
        if delay and i < count - 1:
            sleep(delay)
