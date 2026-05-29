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

"""Video capture helpers — delegates to the hdmicap daemon."""

from __future__ import annotations

import dataclasses
import json
import os
import re
import shutil
import subprocess
import tempfile
import tomllib
from pathlib import Path
from typing import Optional

from . import _config
from ._config import _toml_kv

VIDEO_CONFIG_PATH = _config.CONFIG_DIR / "video.toml"

_BUILTIN_NAMES = ("FaceTime", "Capture screen", "iSight", "iPhone", "iPad")


@dataclasses.dataclass
class VideoConfig:
    """Saved configuration for the HDMI/USB capture device."""

    device: str


def _to_toml(data: dict) -> str:
    lines = [_toml_kv(k, v) for k, v in data.items() if v is not None]
    return "\n".join(lines) + "\n"


def save_video_config(cfg: VideoConfig) -> None:
    _config.CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    VIDEO_CONFIG_PATH.write_text(_to_toml(dataclasses.asdict(cfg)))


def load_video_config() -> Optional[VideoConfig]:
    if not VIDEO_CONFIG_PATH.exists():
        return None
    with open(VIDEO_CONFIG_PATH, "rb") as f:
        data = tomllib.load(f)
    return VideoConfig(device=data["device"])


def hdmicap_binary() -> Optional[str]:
    """Return the installed hdmicap path: PATH, then ~/.cargo/bin. None if absent.

    Installed by `paniolo setup` (cargo install). Never resolved from the in-repo
    build tree, so a running daemon can't point at an ephemeral build artifact.
    """
    found = shutil.which("hdmicap")
    if found:
        return found
    cargo_bin = Path.home() / ".cargo" / "bin" / "hdmicap"
    return str(cargo_bin) if cargo_bin.exists() else None


_DEVICE_RE = re.compile(r"^\s*(\d+)\s+(.+?)\s+\[([^\]]*)\]")


def list_devices() -> list[dict]:
    """Return [{index, name, misc}, ...] via `hdmicap devices`."""
    binary = hdmicap_binary()
    if not binary:
        return []
    try:
        result = subprocess.run(
            [binary, "devices"],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            return []
        devices = []
        for line in result.stdout.splitlines():
            m = _DEVICE_RE.match(line)
            if m:
                devices.append({"index": int(m.group(1)), "name": m.group(2), "misc": m.group(3)})
        return devices
    except FileNotFoundError:
        return []


def guess_capture_device(devices: list[dict]) -> Optional[dict]:
    """Return the one non-built-in device, or None if ambiguous."""
    candidates = [d for d in devices if not any(s in d["name"] for s in _BUILTIN_NAMES)]
    return candidates[0] if len(candidates) == 1 else None


def _discovery_path() -> Path:
    """Path where hdmicap writes its daemon.json discovery file.

    Mirrors hdmicap/src/daemon.rs::runtime_dir(): prefer $XDG_RUNTIME_DIR
    (set by systemd on Linux), fall back to tempfile.gettempdir().
    """
    base = os.environ.get("XDG_RUNTIME_DIR") or tempfile.gettempdir()
    return Path(base) / "hdmicap" / "daemon.json"


def read_discovery() -> Optional[dict]:
    """Read hdmicap's discovery file, returning {pid, port} or None."""
    path = _discovery_path()
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return None


def daemon_url() -> Optional[str]:
    """Return the base URL of the running daemon, or None if not running."""
    disc = read_discovery()
    if disc is None:
        return None
    try:
        os.kill(int(disc["pid"]), 0)
    except (ProcessLookupError, PermissionError, KeyError):
        return None
    return f"http://127.0.0.1:{disc['port']}"


def start_daemon(
    cfg: VideoConfig,
    port: int = 8723,
    ocr_bin: Optional[str] = None,
    target_name: Optional[str] = None,
) -> subprocess.Popen:
    """Start hdmicap daemon in the background; caller should poll daemon_url().

    ocr_bin is exported as PANIOLO_VISIONOCR for the /ocr endpoint.
    target_name is exported as PANIOLO_TARGET so the /power-cycle endpoint
    can call `paniolo power-cycle <target>`.
    """
    binary = hdmicap_binary()
    if not binary:
        raise FileNotFoundError("hdmicap not found in PATH or project build dir")
    env = dict(os.environ)
    if ocr_bin:
        env["PANIOLO_VISIONOCR"] = ocr_bin
    if target_name:
        env["PANIOLO_TARGET"] = target_name
    return subprocess.Popen(
        [binary, "daemon", "--device", cfg.device, "--port", str(port)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
        env=env,
    )


def stop_daemon() -> bool:
    """Ask the running hdmicap daemon to stop. Returns True if it was running."""
    binary = hdmicap_binary()
    if not binary:
        return False
    result = subprocess.run([binary, "stop"], check=False,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return result.returncode == 0
