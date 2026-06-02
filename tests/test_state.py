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

"""Tests for netboot state: path construction, JSON round-trip (including the
back-compat `engine` default), PID liveness probes, and the per-engine
`is_netboot_running` logic. No real processes or state dir are touched —
STATE_DIR is redirected to a tmp dir and os/subprocess probes are stubbed."""

# pylint: disable=redefined-outer-name,unused-argument

from __future__ import annotations

import json

import pytest

from paniolo import _state
from paniolo._state import NetbootState


@pytest.fixture
def state_dir(tmp_path, monkeypatch):
    """Redirect the on-disk state dir into a throwaway tmp path."""
    monkeypatch.setattr(_state, "STATE_DIR", tmp_path)
    return tmp_path


def _state_obj(**over) -> NetbootState:
    base = dict(
        target="fortune",
        dhcp_pid=111,
        tftp_pid=222,
        started_at=1234.5,
        interface="enx0",
        tftp_root="/srv/tftp",
        engine="rust",
    )
    base.update(over)
    return NetbootState(**base)


# ── path construction ───────────────────────────────────────────────────────


def test_paths_are_under_state_dir(state_dir):
    assert (
        _state.netboot_state_path("fortune") == state_dir / "fortune" / "netboot.json"
    )
    assert _state.netboot_log_path("fortune") == state_dir / "fortune" / "netboot.log"


def test_ensure_target_dir_creates_it(state_dir):
    d = _state.ensure_target_dir("fortune")
    assert d == state_dir / "fortune"
    assert d.is_dir()


# ── save / load round-trip ──────────────────────────────────────────────────


def test_save_load_round_trip_preserves_all_fields(state_dir):
    original = _state_obj()
    _state.save_netboot_state(original)
    assert _state.load_netboot_state("fortune") == original


def test_load_missing_state_returns_none(state_dir):
    assert _state.load_netboot_state("ghost") is None


def test_load_corrupt_json_returns_none(state_dir):
    p = _state.netboot_state_path("broken")
    p.parent.mkdir(parents=True)
    p.write_text("{not valid json")
    assert _state.load_netboot_state("broken") is None


def test_load_rejects_unknown_keys(state_dir):
    # An extra key makes NetbootState(**data) raise TypeError -> None, rather
    # than silently constructing a partial object.
    p = _state.netboot_state_path("weird")
    p.parent.mkdir(parents=True)
    p.write_text(json.dumps({"target": "weird", "bogus": 1}))
    assert _state.load_netboot_state("weird") is None


def test_load_without_engine_field_defaults_to_python(state_dir):
    # A state file written before the `engine` field existed must deserialize to
    # the legacy engine ("python"), since that was the sole engine then. The
    # field default exists ONLY for this back-compat path (see _state.py).
    p = _state.netboot_state_path("legacy")
    p.parent.mkdir(parents=True)
    p.write_text(
        json.dumps(
            {
                "target": "legacy",
                "dhcp_pid": 1,
                "tftp_pid": 2,
                "started_at": 0.0,
                "interface": "enx0",
                "tftp_root": "/srv",
            }
        )
    )
    st = _state.load_netboot_state("legacy")
    assert st is not None
    assert st.engine == "python"


def test_fresh_rust_state_round_trips_as_rust(state_dir):
    _state.save_netboot_state(_state_obj(engine="rust"))
    assert _state.load_netboot_state("fortune").engine == "rust"


# ── PID liveness ────────────────────────────────────────────────────────────


def test_is_pid_alive_branches(monkeypatch):
    monkeypatch.setattr(_state.os, "kill", lambda pid, sig: None)
    assert _state.is_pid_alive(123) is True

    def gone(pid, sig):
        raise ProcessLookupError

    monkeypatch.setattr(_state.os, "kill", gone)
    assert _state.is_pid_alive(123) is False

    def denied(pid, sig):
        # PID exists but we cannot signal it — still alive.
        raise PermissionError

    monkeypatch.setattr(_state.os, "kill", denied)
    assert _state.is_pid_alive(123) is True


def test_child_alive_requires_module_in_cmdline(monkeypatch):
    monkeypatch.setattr(_state, "is_pid_alive", lambda pid: True)
    monkeypatch.setattr(
        _state, "_pid_cmdline", lambda pid: "python -m paniolo._tftp 192.168.99.1"
    )
    assert _state.is_paniolo_child_alive(99, "paniolo._tftp") is True
    assert _state.is_paniolo_child_alive(99, "netbootd") is False


def test_child_alive_short_circuits_when_pid_dead(monkeypatch):
    monkeypatch.setattr(_state, "is_pid_alive", lambda pid: False)
    checked = []
    monkeypatch.setattr(_state, "_pid_cmdline", lambda pid: checked.append(pid) or "")
    assert _state.is_paniolo_child_alive(99, "anything") is False
    assert not checked, "cmdline must not be read once the PID is known dead"


# ── is_netboot_running (per-engine) ─────────────────────────────────────────


def test_running_false_when_no_state(state_dir):
    assert _state.is_netboot_running("ghost") is False


def test_running_rust_checks_single_netbootd_pid(state_dir, monkeypatch):
    _state.save_netboot_state(
        _state_obj(target="r", engine="rust", dhcp_pid=500, tftp_pid=500)
    )
    seen = []
    monkeypatch.setattr(
        _state,
        "is_paniolo_child_alive",
        lambda pid, module: seen.append((pid, module)) or True,
    )
    assert _state.is_netboot_running("r") is True
    assert seen == [(500, "netbootd")], "rust engine probes one netbootd PID"


def test_running_python_requires_both_children(state_dir, monkeypatch):
    _state.save_netboot_state(
        _state_obj(target="p", engine="python", dhcp_pid=10, tftp_pid=20)
    )
    # tftp child reports dead -> overall not running.
    monkeypatch.setattr(
        _state,
        "is_paniolo_child_alive",
        lambda pid, module: module == "paniolo._dhcp",
    )
    assert _state.is_netboot_running("p") is False


def test_running_python_true_when_both_alive(state_dir, monkeypatch):
    _state.save_netboot_state(
        _state_obj(target="p", engine="python", dhcp_pid=10, tftp_pid=20)
    )
    monkeypatch.setattr(_state, "is_paniolo_child_alive", lambda pid, module: True)
    assert _state.is_netboot_running("p") is True
