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

"""Tests for target config (de)serialization and serial-interface helpers."""

from __future__ import annotations

import tomllib

import pytest

from paniolo import _config
from paniolo._config import SerialInterface, TargetConfig


def roundtrip(cfg: TargetConfig) -> TargetConfig:
    return _config._from_dict(tomllib.loads(_config._to_toml(cfg)))


def test_roundtrip_multiple_interfaces():
    cfg = TargetConfig(
        name="fortune",
        interface="en3",
        tftp_root="/pxe",
        serial_interfaces=[
            SerialInterface("console", "/dev/ttyUSB0", 115200),
            SerialInterface("bmc", "/dev/ttyUSB1", 9600),
        ],
    )
    got = roundtrip(cfg)
    assert (got.name, got.interface, got.tftp_root) == ("fortune", "en3", "/pxe")
    assert [(i.name, i.device, i.baud) for i in got.serial_interfaces] == [
        ("console", "/dev/ttyUSB0", 115200),
        ("bmc", "/dev/ttyUSB1", 9600),
    ]


def test_roundtrip_no_interfaces():
    got = roundtrip(TargetConfig(name="x", interface="en0"))
    assert got.serial_interfaces == []


def test_legacy_single_serial_migrates():
    data = tomllib.loads(
        'name = "x"\ninterface = "en0"\nserial_device = "/dev/ttyUSB0"\nserial_baud = 57600\n'
    )
    cfg = _config._from_dict(data)
    assert len(cfg.serial_interfaces) == 1
    iface = cfg.serial_interfaces[0]
    assert (iface.name, iface.device, iface.baud) == (_config.DEFAULT_SERIAL_NAME, "/dev/ttyUSB0", 57600)


def test_legacy_default_baud():
    data = tomllib.loads('name = "x"\ninterface = "en0"\nserial_device = "/dev/ttyUSB0"\n')
    assert _config._from_dict(data).serial_interfaces[0].baud == 115200


def test_serial_interface_resolution():
    cfg = TargetConfig(
        name="x",
        interface="en0",
        serial_interfaces=[SerialInterface("console", "/dev/a"), SerialInterface("bmc", "/dev/b")],
    )
    assert cfg.serial_interface("bmc").device == "/dev/b"
    with pytest.raises(ValueError):
        cfg.serial_interface()  # ambiguous
    with pytest.raises(ValueError):
        cfg.serial_interface("nope")  # unknown


def test_serial_interface_single_is_default():
    cfg = TargetConfig(name="x", interface="en0", serial_interfaces=[SerialInterface("console", "/dev/a")])
    assert cfg.serial_interface().name == "console"


def test_serial_interface_none_configured():
    with pytest.raises(ValueError):
        TargetConfig(name="x", interface="en0").serial_interface()


def test_upsert_replaces_same_name():
    cfg = TargetConfig(name="x", interface="en0")
    cfg.upsert_serial_interface(SerialInterface("console", "/dev/a", 115200))
    cfg.upsert_serial_interface(SerialInterface("console", "/dev/a2", 9600))
    assert len(cfg.serial_interfaces) == 1
    assert (cfg.serial_interfaces[0].device, cfg.serial_interfaces[0].baud) == ("/dev/a2", 9600)


def test_remove_interface():
    cfg = TargetConfig(
        name="x",
        interface="en0",
        serial_interfaces=[SerialInterface("console", "/dev/a"), SerialInterface("bmc", "/dev/b")],
    )
    assert cfg.remove_serial_interface("console") is True
    assert cfg.remove_serial_interface("console") is False
    assert [i.name for i in cfg.serial_interfaces] == ["bmc"]


# --- S2: TOML control-character escaping -----------------------------------

def test_toml_roundtrip_newline_in_power_cycle_cmd():
    """Newlines in string values must survive a TOML round-trip without breaking the file."""
    cfg = TargetConfig(name="x", interface="en0", power_cycle_cmd="cmd1\ncmd2")
    got = roundtrip(cfg)
    assert got.power_cycle_cmd == "cmd1\ncmd2"


def test_toml_roundtrip_tab_and_cr():
    cfg = TargetConfig(name="x", interface="en0", power_cycle_cmd="a\tb\rc")
    got = roundtrip(cfg)
    assert got.power_cycle_cmd == "a\tb\rc"


def test_toml_kv_escapes_backslash():
    line = _config._toml_kv("k", "a\\b")
    assert line == 'k = "a\\\\b"'
    parsed = tomllib.loads(line)
    assert parsed["k"] == "a\\b"


# --- C2: unknown TOML keys are ignored gracefully --------------------------

def test_unknown_keys_in_toml_are_silently_dropped():
    data = tomllib.loads('name = "x"\ninterface = "en0"\nunknown_future_key = "foo"\n')
    cfg = _config._from_dict(data)
    assert cfg.name == "x"
    assert not hasattr(cfg, "unknown_future_key")
