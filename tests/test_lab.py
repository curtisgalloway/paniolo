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

"""Tests for the lab-file model (_lab.py): parsing, host resolution, the
single-host (multi-host-rejecting) constraint, and lab-path selection."""

from __future__ import annotations

import tomllib

import pytest

from paniolo import _lab
from paniolo._lab import Lab, LabError

# A representative single-host lab spanning every resource kind.
_LAB = {
    "hosts": {
        "bench1": {"ssh": "curtisg@bench1.local", "identity": "~/.ssh/id_lab"},
        "bench2": {"ssh": "curtisg@bench2.local"},
    },
    "targets": {
        "fortune": {
            "host": "bench1",
            "netboot": {
                "interface": "enx00e04c08d9a0",
                "tftp_root": "/srv/tftp/fortune",
            },
            "serial": [
                {"name": "console", "device": "/dev/ttyUSB0", "baud": 115200},
                {"name": "bmc", "device": "/dev/ttyUSB1", "baud": 9600},
            ],
            "power": {"cycle_cmd": "/bin/cycle.sh", "serial_interface": "console"},
        },
        "apple": {"host": "bench2", "netboot": {"interface": "en5"}},
    },
}


def test_resolve_single_host_target_flattens_to_config_and_host():
    lab = Lab.from_dict(_LAB)
    cfg, host = lab.resolve_target("fortune")
    assert cfg.name == "fortune"
    assert cfg.interface == "enx00e04c08d9a0"
    assert cfg.tftp_root == "/srv/tftp/fortune"
    assert cfg.host_ip == "192.168.99.1"  # default applied
    assert cfg.power_cycle_cmd == "/bin/cycle.sh"
    assert cfg.power_serial_interface == "console"
    assert [(s.name, s.device, s.baud) for s in cfg.serial_interfaces] == [
        ("console", "/dev/ttyUSB0", 115200),
        ("bmc", "/dev/ttyUSB1", 9600),
    ]
    assert host.name == "bench1"
    assert host.ssh == "curtisg@bench1.local"
    assert host.identity == "~/.ssh/id_lab"
    assert not host.is_local


def test_target_names_sorted():
    assert Lab.from_dict(_LAB).target_names() == ["apple", "fortune"]


def test_resource_inherits_target_default_host():
    # serial has no explicit host → inherits target host bench1; still single-host.
    _, host = Lab.from_dict(_LAB).resolve_target("fortune")
    assert host.name == "bench1"


def test_target_without_host_resolves_to_local():
    lab = Lab.from_dict({"targets": {"t": {"netboot": {"interface": "en0"}}}})
    _, host = lab.resolve_target("t")
    assert host.is_local


def test_empty_target_resolves_to_local():
    lab = Lab.from_dict({"targets": {"t": {}}})
    cfg, host = lab.resolve_target("t")
    assert host.is_local
    assert cfg.interface == ""  # no netboot section


def test_unknown_target_raises_keyerror():
    with pytest.raises(KeyError):
        Lab.from_dict(_LAB).resolve_target("ghost")


def test_unknown_host_reference_raises():
    lab = Lab.from_dict(
        {"targets": {"t": {"host": "nope", "power": {"cycle_cmd": "x"}}}}
    )
    with pytest.raises(LabError, match="unknown host 'nope'"):
        lab.resolve_target("t")


def test_multi_host_target_is_rejected():
    lab = Lab.from_dict(
        {
            "hosts": {"a": {"ssh": "u@a"}, "b": {"ssh": "u@b"}},
            "targets": {
                "split": {
                    "netboot": {"interface": "en0", "host": "a"},
                    "serial": [{"name": "c", "device": "/dev/x", "host": "b"}],
                }
            },
        }
    )
    with pytest.raises(LabError, match="multiple hosts"):
        lab.resolve_target("split")


def test_host_missing_ssh_raises():
    with pytest.raises(LabError, match="missing required 'ssh'"):
        Lab.from_dict({"hosts": {"bad": {"identity": "k"}}})


def test_serial_missing_fields_raises():
    lab = Lab.from_dict({"targets": {"t": {"serial": [{"name": "c"}]}}})
    with pytest.raises(LabError, match="name . device"):
        lab.resolve_target("t")


def test_propose_target_block_picks_carrier_up_and_first_serial():
    inv = {
        "ethernet": [
            {"device": "eth0", "active": True},
            {"device": "enx00e0", "active": True},
        ],
        "serial": ["/dev/ttyUSB0", "/dev/ttyUSB1"],
    }
    block = _lab.propose_target_block("fortune", "bench1", inv)
    # Parses as valid TOML with the expected structure (one value chosen per
    # field; the other interface + serial are commented out, not parsed).
    t = tomllib.loads(block)["targets"]["fortune"]
    assert t["host"] == "bench1"
    assert t["netboot"]["interface"] in ("eth0", "enx00e0")
    assert t["serial"][0] == {
        "name": "console",
        "device": "/dev/ttyUSB0",
        "baud": 115200,
    }
    assert len(t["serial"]) == 1  # the second device is a comment, not an entry
    assert "# another serial device: /dev/ttyUSB1" in block
    assert "cycle_cmd" in block  # commented power stub


def test_propose_target_block_handles_empty_inventory():
    block = _lab.propose_target_block("t", "h", {"ethernet": [], "serial": []})
    assert "no USB-Ethernet interface discovered" in block
    assert "no serial devices discovered" in block
    tomllib.loads(block)  # still valid TOML (commented-out resource sections)


def test_load_lab_parses_a_real_toml_file(tmp_path):
    f = tmp_path / "lab.toml"
    f.write_text(
        '[hosts.bench1]\nssh = "u@bench1"\n\n'
        '[targets.fortune]\nhost = "bench1"\n\n'
        '[targets.fortune.netboot]\ninterface = "en0"\n'
    )
    lab = _lab.load_lab(str(f))
    cfg, host = lab.resolve_target("fortune")
    assert cfg.interface == "en0"
    assert host.ssh == "u@bench1"


# ── lab-path selection ───────────────────────────────────────────────────────


@pytest.fixture(autouse=True)
def _reset_override(monkeypatch):
    monkeypatch.setattr(_lab, "_override_path", None)
    monkeypatch.delenv("PANIOLO_LAB", raising=False)


def test_load_is_none_without_a_configured_lab():
    assert _lab.load() is None


def test_lab_path_prefers_override_then_env(monkeypatch):
    monkeypatch.setenv("PANIOLO_LAB", "/from/env.toml")
    assert _lab.lab_path() == "/from/env.toml"
    _lab.set_lab_path("/from/flag.toml")
    assert _lab.lab_path() == "/from/flag.toml"


def test_load_raises_on_missing_file():
    _lab.set_lab_path("/no/such/lab.toml")
    with pytest.raises(LabError, match="not found"):
        _lab.load()
