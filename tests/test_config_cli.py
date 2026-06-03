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

"""End-to-end tests for the config CRUD CLI (init/host/target/channel writes).

Drives the real Typer app against a temp lab file via CliRunner; no SSH or
hardware is touched (config writes are local and pure)."""

import tomllib

import pytest
from typer.testing import CliRunner

from paniolo import _lab
from paniolo._cli import app

runner = CliRunner()


@pytest.fixture(autouse=True)
def _isolate_lab(monkeypatch, tmp_path):
    monkeypatch.setattr(_lab, "_override_path", None)
    monkeypatch.delenv("PANIOLO_LAB", raising=False)
    monkeypatch.setattr(_lab, "DEFAULT_LAB_PATH", str(tmp_path / "absent.toml"))


def _run(lab, *args):
    return runner.invoke(app, ["--lab", str(lab), *args])


def _load(lab):
    with open(lab, "rb") as f:
        return tomllib.load(f)


def test_build_lab_via_cli(tmp_path):
    lab = tmp_path / "lab.toml"
    assert _run(lab, "host", "add", "bench1", "--ssh", "u@bench1").exit_code == 0
    assert _run(lab, "target", "add", "fortune", "--host", "bench1").exit_code == 0
    assert (
        _run(lab, "netboot", "set", "-t", "fortune", "--interface", "en0").exit_code
        == 0
    )
    assert (
        _run(
            lab, "serial", "add", "console", "-t", "fortune", "--device", "/dev/ttyUSB0"
        ).exit_code
        == 0
    )
    data = _load(lab)
    assert data["hosts"]["bench1"]["ssh"] == "u@bench1"
    assert data["targets"]["fortune"]["host"] == "bench1"
    assert data["targets"]["fortune"]["netboot"]["interface"] == "en0"
    assert data["targets"]["fortune"]["serial"][0]["device"] == "/dev/ttyUSB0"


def test_validation_error_leaves_file_unwritten(tmp_path):
    lab = tmp_path / "lab.toml"
    _run(lab, "target", "add", "fortune")
    before = lab.read_text()
    # Binding a channel to an undeclared host must fail and not persist.
    result = _run(lab, "netboot", "set", "-t", "fortune", "--host", "ghost")
    assert result.exit_code == 1
    assert lab.read_text() == before


def test_host_rm_refused_while_referenced(tmp_path):
    lab = tmp_path / "lab.toml"
    _run(lab, "host", "add", "bench1", "--ssh", "u@bench1")
    _run(lab, "target", "add", "fortune", "--host", "bench1")
    result = _run(lab, "host", "rm", "bench1")
    assert result.exit_code == 1
    assert "bench1" in _load(lab)["hosts"]


def test_serial_rm_and_set(tmp_path):
    lab = tmp_path / "lab.toml"
    _run(lab, "target", "add", "fortune")
    _run(lab, "serial", "add", "console", "-t", "fortune", "--device", "/dev/a")
    _run(lab, "serial", "set", "console", "-t", "fortune", "--baud", "230400")
    assert _load(lab)["targets"]["fortune"]["serial"][0]["baud"] == 230400
    _run(lab, "serial", "rm", "console", "-t", "fortune")
    assert "serial" not in _load(lab)["targets"]["fortune"]


def test_init_refuses_existing(tmp_path):
    lab = tmp_path / "lab.toml"
    assert _run(lab, "init").exit_code == 0
    assert _run(lab, "init").exit_code == 1


# ── doctor: _check_channel (local host, deterministic) ──────────────────────────


def _local():
    from paniolo import _ssh

    return _ssh.Host(name=_ssh.LOCAL, ssh=_ssh.LOCAL)


def test_check_channel_serial_present_and_missing():
    from paniolo import _cli

    rt = _lab.ResolvedTarget("t", "local", None, [])
    ok = _cli._check_channel(
        _local(),
        _lab.ResolvedChannel("serial", "c", "local", {"device": "/dev/null"}),
        rt,
    )
    assert ok[0] == "ok"
    bad = _cli._check_channel(
        _local(),
        _lab.ResolvedChannel("serial", "c", "local", {"device": "/dev/nope-xyz"}),
        rt,
    )
    assert bad[0] == "missing"


def test_check_channel_power_serial_ref():
    from paniolo import _cli

    serial = _lab.ResolvedChannel("serial", "console", "local", {"device": "/dev/null"})
    rt = _lab.ResolvedTarget("t", "local", None, [serial])
    good = _cli._check_channel(
        _local(),
        _lab.ResolvedChannel(
            "power", "power", "local", {"serial_interface": "console"}
        ),
        rt,
    )
    assert good[0] == "ok"
    bad = _cli._check_channel(
        _local(),
        _lab.ResolvedChannel("power", "power", "local", {"serial_interface": "nope"}),
        rt,
    )
    assert bad[0] == "missing"
