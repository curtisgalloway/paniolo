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

"""Serial console helpers for paniolo targets.

Two paths share this module:
- `tio` for an interactive terminal in the current shell (`paniolo serial connect`)
- the `serialcap` daemon, which owns the port and fans it out over a localhost
  WebSocket for the combined video+serial dashboard (`paniolo serial watch`)
"""

from __future__ import annotations

import glob
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path
from typing import TYPE_CHECKING, Optional, Sequence

if TYPE_CHECKING:
    from ._config import SerialInterface


def list_serial_devices() -> list[str]:
    """Return available serial device paths on this platform."""
    if sys.platform == "darwin":
        paths = glob.glob("/dev/tty.usbserial-*") + glob.glob("/dev/tty.usbmodem*")
    else:
        paths = glob.glob("/dev/ttyUSB*") + glob.glob("/dev/ttyACM*")
    return sorted(paths)


def tio_binary() -> str | None:
    """Return the path to tio, or None if not found."""
    return shutil.which("tio")


def connect_cmd(device: str, baud: int = 115200) -> list[str]:
    """Build the tio command to open an interactive serial terminal."""
    binary = tio_binary()
    if not binary:
        raise FileNotFoundError("tio not found in PATH")
    return [binary, "--baudrate", str(baud), device]


def log_cmd(
    binary: str,
    *,
    interface: Optional[str] = None,
    tail: Optional[int] = None,
    from_seq: Optional[int] = None,
    to_seq: Optional[int] = None,
    since: Optional[int] = None,
    raw: bool = False,
    as_json: bool = False,
    no_pending: bool = False,
) -> list[str]:
    """Build the `serialcap log` argv for the captured-output reader.

    serialcap reads its own on-disk capture log, so this works whether or not the
    daemon is running. `interface` selects which interface's log to read (optional
    when only one was captured). Only set flags are forwarded; the binary applies
    its own defaults (most recent lines, ANSI-stripped, pending line included)."""
    cmd = [binary, "log"]
    if interface is not None:
        cmd += ["--interface", interface]
    if tail is not None:
        cmd += ["--tail", str(tail)]
    if from_seq is not None:
        cmd += ["--from", str(from_seq)]
    if to_seq is not None:
        cmd += ["--to", str(to_seq)]
    if since is not None:
        cmd += ["--since", str(since)]
    if raw:
        cmd.append("--raw")
    if as_json:
        cmd.append("--json")
    if no_pending:
        cmd.append("--no-pending")
    return cmd


def serialcap_binary() -> Optional[str]:
    """Return the installed serialcap path: PATH, then ~/.cargo/bin. None if absent.

    Installed by `paniolo setup` (cargo install). Never resolved from the in-repo
    build tree, so a running daemon can't point at an ephemeral build artifact.
    """
    found = shutil.which("serialcap")
    if found:
        return found
    cargo_bin = Path.home() / ".cargo" / "bin" / "serialcap"
    return str(cargo_bin) if cargo_bin.exists() else None


def _discovery_path() -> Path:
    """Path where serialcap writes its daemon.json discovery file.

    Mirrors serialcap/src/daemon.rs::runtime_dir(): prefer $XDG_RUNTIME_DIR
    (set by systemd on Linux), fall back to tempfile.gettempdir().
    """
    base = os.environ.get("XDG_RUNTIME_DIR") or tempfile.gettempdir()
    return Path(base) / "serialcap" / "daemon.json"


def read_discovery() -> Optional[dict]:
    """Read serialcap's discovery file, returning {pid, port, device, baud} or None."""
    path = _discovery_path()
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return None


def daemon_url() -> Optional[str]:
    """Return the base URL of the running serialcap daemon, or None if stopped."""
    disc = read_discovery()
    if disc is None:
        return None
    try:
        os.kill(int(disc["pid"]), 0)
    except (ProcessLookupError, PermissionError, KeyError):
        return None
    return f"http://127.0.0.1:{disc['port']}"


def interface_arg(name: str, device: str, baud: int, power_sense_signal: Optional[str] = None) -> str:
    """Format one interface for the daemon's repeatable --interface flag.

    Format: NAME=DEVICE[@BAUD][:SENSE]
    SENSE is one of cts, dsr, dcd, ri — the FTDI modem-control input wired to
    the target's 3.3 V rail for power-state sensing.
    """
    arg = f"{name}={device}@{baud}"
    if power_sense_signal:
        arg += f":{power_sense_signal}"
    return arg


def daemon_cmd(
    binary: str,
    interfaces: "Sequence[SerialInterface]",
    port: int = 8724,
    buffer_lines: Optional[int] = None,
) -> list[str]:
    """Build the `serialcap daemon` argv owning every given interface."""
    cmd = [binary, "daemon", "--port", str(port)]
    if buffer_lines is not None:
        cmd += ["--buffer-lines", str(buffer_lines)]
    for iface in interfaces:
        cmd += [
            "--interface",
            interface_arg(iface.name, iface.device, iface.baud, iface.power_sense_signal),
        ]
    return cmd


def wait_power_off(daemon_url: str, interface_name: str, timeout_s: float = 10.0) -> bool:
    """Poll GET /status until power_on == False or timeout.

    Returns True if the power-off was confirmed by the sense signal before the
    timeout.  Returns False if the sense signal is not configured for this
    interface (power_on is null in the response) or if the timeout expires.
    """
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            url = f"{daemon_url}/status?interface={interface_name}"
            req = urllib.request.Request(url)
            with urllib.request.urlopen(req, timeout=2) as resp:
                data = json.loads(resp.read())
                if data.get("power_on") is False:
                    return True
        except Exception:
            pass
        time.sleep(0.5)
    return False


def read_power_state(daemon_url: str, interface_name: str) -> Optional[bool]:
    """Return the current power state from the daemon status, or None if unknown."""
    try:
        url = f"{daemon_url}/status?interface={interface_name}"
        req = urllib.request.Request(url)
        with urllib.request.urlopen(req, timeout=2) as resp:
            data = json.loads(resp.read())
            return data.get("power_on")
    except Exception:
        return None


def start_daemon(
    interfaces: "Sequence[SerialInterface]",
    port: int = 8724,
    buffer_lines: Optional[int] = None,
) -> subprocess.Popen:
    """Start the serialcap daemon (owning all interfaces) detached; caller should
    poll daemon_url()."""
    binary = serialcap_binary()
    if not binary:
        raise FileNotFoundError("serialcap not found in PATH or project build dir")
    return subprocess.Popen(
        daemon_cmd(binary, interfaces, port, buffer_lines),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
