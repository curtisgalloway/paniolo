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

"""Tests for netif mode switching: address probing, mode resolution, and the
order/teardown of mode transitions. No privileged subprocess actually runs —
subprocess and the netboot hooks are stubbed."""

from __future__ import annotations

import types

import pytest

from paniolo import _netif
from paniolo._config import TargetConfig


def _cfg(interface: str = "enx0") -> TargetConfig:
    return TargetConfig(name="fortune", interface=interface, host_ip="192.168.99.1")


def _completed(stdout: str = "", returncode: int = 0, stderr: str = ""):
    return types.SimpleNamespace(stdout=stdout, returncode=returncode, stderr=stderr)


# ── address / peer parsing (Linux `ip` output) ─────────────────────────────


def test_iface_addresses_parses_brief_output(monkeypatch):
    monkeypatch.setattr(_netif.sys, "platform", "linux")
    out = "enx0             UP             192.168.99.1/24 fe80::1/64 fe80::abcd/64"
    monkeypatch.setattr(_netif.subprocess, "run", lambda *a, **k: _completed(out))
    addrs = _netif.iface_addresses("enx0")
    assert addrs["inet"] == ["192.168.99.1/24"]
    assert addrs["inet6"] == ["fe80::1/64", "fe80::abcd/64"]


def test_iface_addresses_empty_when_command_fails(monkeypatch):
    monkeypatch.setattr(_netif.sys, "platform", "linux")
    monkeypatch.setattr(
        _netif.subprocess, "run", lambda *a, **k: _completed(returncode=1)
    )
    assert _netif.iface_addresses("enx0") == {"inet": [], "inet6": []}


def test_has_host_ll_matches_with_and_without_prefix():
    assert _netif._has_host_ll({"inet6": ["fe80::1/64"]})
    assert _netif._has_host_ll({"inet6": ["fe80::1"]})
    assert not _netif._has_host_ll({"inet6": ["fe80::abcd/64"]})
    assert not _netif._has_host_ll({"inet6": []})


def test_ipv6_peers_excludes_host_ll(monkeypatch):
    monkeypatch.setattr(_netif.sys, "platform", "linux")
    out = (
        "fe80::1 lladdr aa:bb:cc:dd:ee:ff router STALE\n"
        "fe80::fc33:fca2:96e0:6dbe lladdr 11:22:33:44:55:66 REACHABLE\n"
    )
    monkeypatch.setattr(_netif.subprocess, "run", lambda *a, **k: _completed(out))
    assert _netif.ipv6_peers("enx0") == ["fe80::fc33:fca2:96e0:6dbe"]


def test_ipv6_peers_empty_on_macos(monkeypatch):
    monkeypatch.setattr(_netif.sys, "platform", "darwin")
    assert _netif.ipv6_peers("enx0") == []


# ── mode resolution in get_status ───────────────────────────────────────────


@pytest.mark.parametrize(
    "running, inet6, expected",
    [
        (True, ["fe80::1/64"], "netboot"),  # daemons win even if LL present
        (True, [], "netboot"),
        (False, ["fe80::1/64"], "ffx"),
        (False, ["fe80::abcd/64"], "off"),  # some other LL, not ours
        (False, [], "off"),
    ],
)
def test_get_status_mode_resolution(monkeypatch, running, inet6, expected):
    monkeypatch.setattr(_netif, "is_netboot_running", lambda name: running)
    monkeypatch.setattr(
        _netif, "iface_addresses", lambda iface: {"inet": [], "inet6": inet6}
    )
    monkeypatch.setattr(_netif, "ipv6_peers", lambda iface: ["fe80::dead"])
    s = _netif.get_status(_cfg())
    assert s["mode"] == expected
    # peers are only probed in ffx mode.
    assert s["peers"] == (["fe80::dead"] if expected == "ffx" else [])
    assert s["host_ll"] == ("fe80::1" if "fe80::1/64" in inet6 else None)


# ── mode transitions: order and teardown ────────────────────────────────────


@pytest.fixture
def calls(monkeypatch):
    """Record the sequence of side-effecting hooks each transition invokes."""
    seq: list[str] = []
    monkeypatch.setattr(_netif, "_add_host_ll", lambda i: seq.append("add_ll"))
    monkeypatch.setattr(_netif, "_del_host_ll", lambda i: seq.append("del_ll"))
    monkeypatch.setattr(_netif, "_del_host_ip", lambda i, ip: seq.append("del_ip"))
    monkeypatch.setattr(
        _netif._netboot,
        "start",
        lambda cfg, engine="rust": seq.append(f"start:{engine}"),
    )
    monkeypatch.setattr(_netif._netboot, "stop", lambda name: seq.append("stop"))
    return seq


def test_mode_ffx_stops_netboot_then_adds_ll(monkeypatch, calls):
    monkeypatch.setattr(_netif, "is_netboot_running", lambda name: True)
    _netif.mode_ffx(_cfg())
    assert calls == ["stop", "add_ll"]


def test_mode_ffx_idempotent_readds_ll_when_not_running(monkeypatch, calls):
    monkeypatch.setattr(_netif, "is_netboot_running", lambda name: False)
    _netif.mode_ffx(_cfg())
    assert calls == ["add_ll"]


def test_mode_netboot_clears_ll_then_starts(monkeypatch, calls):
    monkeypatch.setattr(_netif, "is_netboot_running", lambda name: False)
    # No engine arg: the rust engine is the default.
    _netif.mode_netboot(_cfg())
    assert calls == ["del_ll", "start:rust"]


def test_mode_netboot_idempotent_when_already_running(monkeypatch, calls):
    monkeypatch.setattr(_netif, "is_netboot_running", lambda name: True)
    _netif.mode_netboot(_cfg())
    # clears the ffx LL but does not try to start a second netboot.
    assert calls == ["del_ll"]


def test_mode_off_tears_down_everything(monkeypatch, calls):
    monkeypatch.setattr(_netif, "is_netboot_running", lambda name: True)
    _netif.mode_off(_cfg())
    assert calls == ["stop", "del_ll", "del_ip"]


def test_mode_off_when_not_running_skips_stop(monkeypatch, calls):
    monkeypatch.setattr(_netif, "is_netboot_running", lambda name: False)
    _netif.mode_off(_cfg())
    assert calls == ["del_ll", "del_ip"]
