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

"""Tests for the tomlkit-backed editable lab document (paniolo._labfile)."""

import tomllib

import pytest

from paniolo import _labfile
from paniolo._lab import LabError


def _reparse(path):
    with open(path, "rb") as f:
        return tomllib.load(f)


def test_create_and_build_round_trips(tmp_path):
    p = str(tmp_path / "lab.toml")
    lf = _labfile.LabFile.create(p)
    lf.add_host("bench1", "curtisg@bench1.local", identity="~/.ssh/id_lab")
    lf.add_target("fortune", host="bench1", note="left adapter is console")
    lf.set_netboot("fortune", interface="enx00", tftp_root="/srv/tftp/fortune")
    lf.add_serial("fortune", "console", "/dev/ttyUSB0")
    lf.add_serial("fortune", "bmc", "/dev/ttyUSB1", baud=9600)
    lf.set_power("fortune", cycle_cmd="/bin/cycle.sh", serial_interface="console")
    lf.save()

    data = _reparse(p)
    assert data["hosts"]["bench1"]["ssh"] == "curtisg@bench1.local"
    assert data["hosts"]["bench1"]["identity"] == "~/.ssh/id_lab"
    t = data["targets"]["fortune"]
    assert t["host"] == "bench1"
    assert t["note"] == "left adapter is console"
    assert t["netboot"]["interface"] == "enx00"
    assert [s["name"] for s in t["serial"]] == ["console", "bmc"]
    assert t["serial"][1]["baud"] == 9600
    assert t["power"]["cycle_cmd"] == "/bin/cycle.sh"


def test_comments_and_formatting_preserved(tmp_path):
    p = tmp_path / "lab.toml"
    p.write_text(
        "# my hand-written lab\n"
        "[hosts.bench1]\n"
        'ssh = "curtisg@bench1.local"  # the noisy one\n\n'
        "[targets.fortune]\n"
        'host = "bench1"\n'
        "# reminder: console is on the LEFT adapter\n"
        "[[targets.fortune.serial]]\n"
        'name = "console"\n'
        'device = "/dev/ttyUSB0"\n'
        "baud = 115200\n"
    )
    lf = _labfile.LabFile.load(str(p))
    lf.add_serial("fortune", "bmc", "/dev/ttyUSB1", baud=9600)
    lf.update_host("bench1", identity="~/.ssh/id_lab")
    lf.save()

    text = p.read_text()
    assert "# my hand-written lab" in text
    assert "# the noisy one" in text
    assert "# reminder: console is on the LEFT adapter" in text
    data = _reparse(str(p))
    assert data["hosts"]["bench1"]["identity"] == "~/.ssh/id_lab"
    assert [s["name"] for s in data["targets"]["fortune"]["serial"]] == [
        "console",
        "bmc",
    ]


def test_remove_last_serial_drops_array(tmp_path):
    p = str(tmp_path / "lab.toml")
    lf = _labfile.LabFile.create(p)
    lf.add_target("fortune")
    lf.add_serial("fortune", "console", "/dev/ttyUSB0")
    lf.remove_serial("fortune", "console")
    lf.save()
    assert "serial" not in _reparse(p)["targets"]["fortune"]


def test_duplicate_serial_rejected(tmp_path):
    lf = _labfile.LabFile.create(str(tmp_path / "lab.toml"))
    lf.add_target("fortune")
    lf.add_serial("fortune", "console", "/dev/ttyUSB0")
    with pytest.raises(LabError, match="already exists"):
        lf.add_serial("fortune", "console", "/dev/ttyUSB1")


def test_remove_host_blocked_while_referenced(tmp_path):
    lf = _labfile.LabFile.create(str(tmp_path / "lab.toml"))
    lf.add_host("bench1", "curtisg@bench1.local")
    lf.add_target("fortune", host="bench1")
    with pytest.raises(LabError, match="still used by: fortune"):
        lf.remove_host("bench1")


def test_validate_rejects_unknown_host_ref(tmp_path):
    lf = _labfile.LabFile.create(str(tmp_path / "lab.toml"))
    lf.add_target("fortune", host="ghost")
    with pytest.raises(LabError, match="unknown host 'ghost'"):
        lf.save()


def test_validate_rejects_missing_ssh():
    with pytest.raises(LabError, match="missing required 'ssh'"):
        _labfile.validate({"hosts": {"bench1": {}}})


def test_validate_rejects_singleton_as_array():
    data = {"targets": {"fortune": {"netboot": [{"interface": "en0"}]}}}
    with pytest.raises(LabError, match="must be a single table"):
        _labfile.validate(data)


def test_validate_rejects_bad_sense_signal():
    data = {
        "targets": {
            "fortune": {
                "serial": [
                    {"name": "c", "device": "/dev/x", "power_sense_signal": "bogus"}
                ]
            }
        }
    }
    with pytest.raises(LabError, match="invalid power_sense_signal"):
        _labfile.validate(data)


def test_per_channel_host_validates(tmp_path):
    lf = _labfile.LabFile.create(str(tmp_path / "lab.toml"))
    lf.add_host("bench1", "u@bench1")
    lf.add_host("bench2", "u@bench2")
    lf.add_target("fortune", host="bench1")
    lf.set_video("fortune", device="/dev/video0", host="bench2")
    lf.save()  # cross-host target is now valid at the file layer
    data = _reparse(str(tmp_path / "lab.toml"))
    assert data["targets"]["fortune"]["video"]["host"] == "bench2"
