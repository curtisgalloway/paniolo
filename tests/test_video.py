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

"""Tests for _video.py: TOML config round-trip, `hdmicap devices` parsing, the
capture-device heuristic, and discovery-file / daemon-URL resolution. The
hdmicap binary, subprocess, and filesystem are all stubbed."""

from __future__ import annotations

import json
import types

from paniolo import _video
from paniolo._video import VideoConfig


def _run(stdout: str = "", returncode: int = 0):
    return types.SimpleNamespace(stdout=stdout, returncode=returncode)


# ── TOML serialization / round-trip ─────────────────────────────────────────


def test_to_toml_emits_string_kv_and_drops_none():
    assert _video._to_toml({"device": "HDMI Cap"}) == 'device = "HDMI Cap"\n'
    assert _video._to_toml({"device": "Cam0", "extra": None}) == 'device = "Cam0"\n'


def test_save_load_round_trip(tmp_path, monkeypatch):
    monkeypatch.setattr(_video._config, "CONFIG_DIR", tmp_path)
    monkeypatch.setattr(_video, "VIDEO_CONFIG_PATH", tmp_path / "video.toml")
    _video.save_video_config(VideoConfig(device="USB Capture HDMI"))
    assert _video.load_video_config() == VideoConfig(device="USB Capture HDMI")


def test_load_returns_none_when_absent(tmp_path, monkeypatch):
    monkeypatch.setattr(_video, "VIDEO_CONFIG_PATH", tmp_path / "video.toml")
    assert _video.load_video_config() is None


# ── `hdmicap devices` parsing ───────────────────────────────────────────────


def test_list_devices_parses_indexed_lines(monkeypatch):
    monkeypatch.setattr(_video, "hdmicap_binary", lambda: "/usr/bin/hdmicap")
    out = "0  FaceTime HD Camera [builtin]\n1  USB Capture HDMI [0x1234]\n"
    monkeypatch.setattr(_video.subprocess, "run", lambda *a, **k: _run(out))
    devices = _video.list_devices()
    assert devices == [
        {"index": 0, "name": "FaceTime HD Camera", "misc": "builtin"},
        {"index": 1, "name": "USB Capture HDMI", "misc": "0x1234"},
    ]


def test_list_devices_empty_when_binary_missing(monkeypatch):
    monkeypatch.setattr(_video, "hdmicap_binary", lambda: None)
    assert _video.list_devices() == []


def test_list_devices_empty_on_nonzero_exit(monkeypatch):
    monkeypatch.setattr(_video, "hdmicap_binary", lambda: "/usr/bin/hdmicap")
    monkeypatch.setattr(_video.subprocess, "run", lambda *a, **k: _run("x", 1))
    assert _video.list_devices() == []


# ── capture-device heuristic ────────────────────────────────────────────────


def test_guess_capture_device_picks_sole_non_builtin():
    devices = [
        {"index": 0, "name": "FaceTime HD Camera", "misc": ""},
        {"index": 1, "name": "USB Capture HDMI", "misc": ""},
    ]
    assert _video.guess_capture_device(devices) == devices[1]


def test_guess_capture_device_none_when_ambiguous():
    devices = [
        {"index": 0, "name": "USB Capture HDMI", "misc": ""},
        {"index": 1, "name": "Elgato HD60", "misc": ""},
    ]
    assert _video.guess_capture_device(devices) is None


def test_guess_capture_device_none_when_only_builtins():
    devices = [
        {"index": 0, "name": "FaceTime HD Camera", "misc": ""},
        {"index": 1, "name": "iPhone Camera", "misc": ""},
    ]
    assert _video.guess_capture_device(devices) is None


# ── discovery file / daemon URL ─────────────────────────────────────────────


def test_read_discovery_parses_json(tmp_path, monkeypatch):
    disc = tmp_path / "daemon.json"
    disc.write_text(json.dumps({"pid": 4321, "port": 8723}))
    monkeypatch.setattr(_video, "_discovery_path", lambda: disc)
    assert _video.read_discovery() == {"pid": 4321, "port": 8723}


def test_read_discovery_none_when_missing(tmp_path, monkeypatch):
    monkeypatch.setattr(_video, "_discovery_path", lambda: tmp_path / "nope.json")
    assert _video.read_discovery() is None


def test_read_discovery_none_when_corrupt(tmp_path, monkeypatch):
    disc = tmp_path / "daemon.json"
    disc.write_text("{not json")
    monkeypatch.setattr(_video, "_discovery_path", lambda: disc)
    assert _video.read_discovery() is None


def test_daemon_url_when_pid_alive(monkeypatch):
    monkeypatch.setattr(_video, "read_discovery", lambda: {"pid": 10, "port": 8723})
    monkeypatch.setattr(_video.os, "kill", lambda pid, sig: None)
    assert _video.daemon_url() == "http://127.0.0.1:8723"


def test_daemon_url_none_when_pid_dead(monkeypatch):
    monkeypatch.setattr(_video, "read_discovery", lambda: {"pid": 10, "port": 8723})

    def gone(pid, sig):
        raise ProcessLookupError

    monkeypatch.setattr(_video.os, "kill", gone)
    assert _video.daemon_url() is None


def test_daemon_url_none_when_no_discovery(monkeypatch):
    monkeypatch.setattr(_video, "read_discovery", lambda: None)
    assert _video.daemon_url() is None
