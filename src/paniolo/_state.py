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

from __future__ import annotations

import dataclasses
import json
import os
from pathlib import Path
from typing import Optional

STATE_DIR = Path.home() / ".local" / "share" / "paniolo"


@dataclasses.dataclass
class NetbootState:
    target: str
    dhcp_pid: int
    tftp_pid: int
    started_at: float
    interface: str
    tftp_root: str


def _target_dir(target: str) -> Path:
    return STATE_DIR / target


def netboot_state_path(target: str) -> Path:
    return _target_dir(target) / "netboot.json"


def netboot_log_path(target: str) -> Path:
    return _target_dir(target) / "netboot.log"


def ensure_target_dir(target: str) -> Path:
    d = _target_dir(target)
    d.mkdir(parents=True, exist_ok=True)
    return d


def save_netboot_state(state: NetbootState) -> None:
    ensure_target_dir(state.target)
    netboot_state_path(state.target).write_text(
        json.dumps(dataclasses.asdict(state), indent=2)
    )


def load_netboot_state(target: str) -> Optional[NetbootState]:
    path = netboot_state_path(target)
    if not path.exists():
        return None
    try:
        data = json.loads(path.read_text())
        return NetbootState(**data)
    except (json.JSONDecodeError, TypeError, KeyError):
        return None


def is_pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except (ProcessLookupError, PermissionError):
        return False


def is_netboot_running(target: str) -> bool:
    state = load_netboot_state(target)
    if state is None:
        return False
    return is_pid_alive(state.dhcp_pid) and is_pid_alive(state.tftp_pid)
