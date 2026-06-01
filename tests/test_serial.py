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

"""Host-side tests for serial helpers — no hardware, no serialcap binary."""

from __future__ import annotations

from paniolo import _serial
from paniolo._config import SerialInterface


def test_log_cmd_defaults_to_bare_subcommand():
    assert _serial.log_cmd("serialcap") == ["serialcap", "log"]


def test_log_cmd_forwards_only_set_flags():
    cmd = _serial.log_cmd("serialcap", tail=50)
    assert cmd == ["serialcap", "log", "--tail", "50"]


def test_log_cmd_interface():
    assert _serial.log_cmd("serialcap", interface="bmc", tail=10) == [
        "serialcap", "log", "--interface", "bmc", "--tail", "10",
    ]


def test_input_url_no_pace():
    assert (
        _serial.input_url("http://127.0.0.1:8724", "console")
        == "http://127.0.0.1:8724/input?interface=console"
    )


def test_input_url_with_pace():
    assert (
        _serial.input_url("http://127.0.0.1:8724", "console", pace_ms=8)
        == "http://127.0.0.1:8724/input?interface=console&pace_ms=8"
    )


def test_input_url_zero_pace_omitted():
    assert "pace_ms" not in _serial.input_url("http://x", "bmc", pace_ms=0)


def test_interface_arg():
    assert _serial.interface_arg("console", "/dev/ttyUSB0", 115200) == "console=/dev/ttyUSB0@115200"


def test_daemon_cmd_one_per_interface():
    ifaces = [
        SerialInterface("console", "/dev/ttyUSB0", 115200),
        SerialInterface("bmc", "/dev/ttyUSB1", 9600),
    ]
    assert _serial.daemon_cmd("serialcap", ifaces, port=8724) == [
        "serialcap", "daemon", "--port", "8724",
        "--interface", "console=/dev/ttyUSB0@115200",
        "--interface", "bmc=/dev/ttyUSB1@9600",
    ]


def test_daemon_cmd_buffer_lines():
    ifaces = [SerialInterface("console", "/dev/ttyUSB0", 115200)]
    assert _serial.daemon_cmd("serialcap", ifaces, port=9, buffer_lines=1000) == [
        "serialcap", "daemon", "--port", "9", "--buffer-lines", "1000",
        "--interface", "console=/dev/ttyUSB0@115200",
    ]


def test_log_cmd_range_and_since():
    assert _serial.log_cmd("serialcap", from_seq=10, to_seq=20) == [
        "serialcap", "log", "--from", "10", "--to", "20",
    ]
    assert _serial.log_cmd("serialcap", since=7) == ["serialcap", "log", "--since", "7"]


def test_log_cmd_boolean_flags():
    cmd = _serial.log_cmd("serialcap", raw=True, as_json=True, no_pending=True)
    assert cmd == ["serialcap", "log", "--raw", "--json", "--no-pending"]
    # Defaults stay off.
    assert "--raw" not in _serial.log_cmd("serialcap", tail=1)
