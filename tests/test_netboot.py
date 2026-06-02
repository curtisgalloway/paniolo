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

"""Tests for _netboot.py's pure parsing + the safety-critical primary-NIC guard.

The guard (`_is_primary_interface`) is what refuses to reconfigure the host's
main NIC to the static netboot IP. We exercise the default-route parsing on both
macOS (`route -n get default`) and Linux (`ip route show default`), and the
USB-Ethernet listing's exclusion/sort logic — all with subprocess stubbed, so
the same suite runs identically on either platform's CI."""

from __future__ import annotations

import subprocess

from paniolo import _netboot


def _raise(exc):
    def f(*a, **k):
        raise exc

    return f


# ── default-route interface parsing ─────────────────────────────────────────


def test_default_route_macos_parses_interface_line(monkeypatch):
    monkeypatch.setattr(_netboot.sys, "platform", "darwin")
    out = "   route to: default\n  gateway: 192.168.1.1\n  interface: en0\n  flags: \n"
    monkeypatch.setattr(_netboot.subprocess, "check_output", lambda *a, **k: out)
    assert _netboot._default_route_interface() == "en0"


def test_default_route_macos_none_when_no_interface_line(monkeypatch):
    monkeypatch.setattr(_netboot.sys, "platform", "darwin")
    monkeypatch.setattr(
        _netboot.subprocess, "check_output", lambda *a, **k: "  gateway: 1.2.3.4\n"
    )
    assert _netboot._default_route_interface() is None


def test_default_route_macos_none_on_command_failure(monkeypatch):
    monkeypatch.setattr(_netboot.sys, "platform", "darwin")
    monkeypatch.setattr(
        _netboot.subprocess, "check_output", _raise(FileNotFoundError())
    )
    assert _netboot._default_route_interface() is None


def test_default_route_linux_parses_dev_token(monkeypatch):
    monkeypatch.setattr(_netboot.sys, "platform", "linux")
    out = "default via 192.168.1.1 dev eth0 proto dhcp src 192.168.1.50 metric 100"
    monkeypatch.setattr(_netboot.subprocess, "check_output", lambda *a, **k: out)
    assert _netboot._default_route_interface() == "eth0"


def test_default_route_linux_none_without_dev(monkeypatch):
    monkeypatch.setattr(_netboot.sys, "platform", "linux")
    monkeypatch.setattr(_netboot.subprocess, "check_output", lambda *a, **k: "")
    assert _netboot._default_route_interface() is None


def test_default_route_linux_none_on_command_failure(monkeypatch):
    monkeypatch.setattr(_netboot.sys, "platform", "linux")
    monkeypatch.setattr(
        _netboot.subprocess,
        "check_output",
        _raise(subprocess.CalledProcessError(1, "ip")),
    )
    assert _netboot._default_route_interface() is None


# ── primary-NIC guard ───────────────────────────────────────────────────────


def test_primary_interface_true_for_default_route_nic(monkeypatch):
    monkeypatch.setattr(_netboot, "_default_route_interface", lambda: "en0")
    assert _netboot._is_primary_interface("en0") is True


def test_primary_interface_false_for_secondary_nic(monkeypatch):
    monkeypatch.setattr(_netboot, "_default_route_interface", lambda: "en0")
    assert _netboot._is_primary_interface("enx_usb_dongle") is False


def test_primary_interface_false_when_no_default_route(monkeypatch):
    monkeypatch.setattr(_netboot, "_default_route_interface", lambda: None)
    assert _netboot._is_primary_interface("enx_usb_dongle") is False


# ── USB-Ethernet listing (macOS branch: exclusions + active-first sort) ──────


_NETWORKSETUP = """Hardware Port: Wi-Fi
Device: en0
Ethernet Address: aa:bb:cc:dd:ee:ff

Hardware Port: USB 10/100/1000 LAN
Device: en7
Ethernet Address: 11:22:33:44:55:66

Hardware Port: Thunderbolt Bridge
Device: bridge0
Ethernet Address: N/A

Hardware Port: AX88179A
Device: en10
Ethernet Address: 77:88:99:aa:bb:cc
"""


def test_list_usb_ethernet_macos_excludes_and_sorts_active_first(monkeypatch):
    monkeypatch.setattr(_netboot.sys, "platform", "darwin")
    monkeypatch.setattr(
        _netboot.subprocess, "check_output", lambda *a, **k: _NETWORKSETUP
    )
    # en10 active, en7 inactive -> active sorts first.
    active = {"en10": True, "en7": False}
    monkeypatch.setattr(
        _netboot, "_is_interface_active", lambda dev: active.get(dev, False)
    )
    result = _netboot.list_usb_ethernet_interfaces()
    devices = [r["device"] for r in result]

    assert "en0" not in devices, "Wi-Fi port excluded"
    assert "bridge0" not in devices, "Thunderbolt Bridge / bridge0 excluded"
    assert devices == ["en10", "en7"], "active interface listed before inactive"
    assert result[0]["port"] == "AX88179A"


def test_list_usb_ethernet_macos_empty_on_command_failure(monkeypatch):
    monkeypatch.setattr(_netboot.sys, "platform", "darwin")
    monkeypatch.setattr(
        _netboot.subprocess, "check_output", _raise(FileNotFoundError())
    )
    assert _netboot.list_usb_ethernet_interfaces() == []


def test_list_usb_ethernet_linux_delegates_to_sysfs(monkeypatch):
    monkeypatch.setattr(_netboot.sys, "platform", "linux")
    sentinel = [{"port": "eth0", "device": "eth0", "active": True}]
    monkeypatch.setattr(_netboot, "_list_linux_ethernet_interfaces", lambda: sentinel)
    assert _netboot.list_usb_ethernet_interfaces() is sentinel
