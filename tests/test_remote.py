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

"""Tests for transparent remote re-exec (_remote.py).

Unit tests cover argv rewriting (always run). Integration tests cover the real
ship-config + re-exec round-trip and a full CLI dispatch; they are gated on
PANIOLO_SSH_IT (see test_ssh.py) so the suite never needs an ssh-agent.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tomllib
import types
from pathlib import Path

import pytest

from paniolo import _cli, _config, _remote, _ssh
from paniolo._config import TargetConfig
from paniolo._ssh import Host

# ── unit: argv rewriting ─────────────────────────────────────────────────────


def test_strip_lab_option_spaced_and_equals_forms():
    assert _remote.strip_lab_option(["--lab", "/x.toml", "netboot", "status"]) == [
        "netboot",
        "status",
    ]
    assert _remote.strip_lab_option(["--lab=/x.toml", "power-cycle", "t"]) == [
        "power-cycle",
        "t",
    ]


def test_strip_lab_option_noop_without_lab():
    assert _remote.strip_lab_option(["serial", "log", "-i", "console"]) == [
        "serial",
        "log",
        "-i",
        "console",
    ]


def test_remote_argv_replaces_launcher_and_defaults_to_paniolo():
    argv = ["/usr/local/bin/paniolo", "--lab", "/x.toml", "netboot", "status", "t"]
    assert _remote.remote_argv(argv) == ["paniolo", "netboot", "status", "t"]


def test_remote_argv_honors_host_paniolo_cmd():
    argv = ["/local/paniolo", "netif", "status", "t"]
    assert _remote.remote_argv(argv, "/opt/paniolo/bin/paniolo") == [
        "/opt/paniolo/bin/paniolo",
        "netif",
        "status",
        "t",
    ]


def test_host_paniolo_property_default_and_override():
    assert Host(name="h", ssh="u@h").paniolo == "paniolo"
    assert Host(name="h", ssh="u@h", paniolo_cmd="/x/paniolo").paniolo == "/x/paniolo"


# ── unit: remote console plumbing ────────────────────────────────────────────


def _completed(stdout="", returncode=0, stderr=""):
    return types.SimpleNamespace(stdout=stdout, returncode=returncode, stderr=stderr)


def test_remote_daemon_port_parses_discovery_json(monkeypatch):
    monkeypatch.setattr(
        _remote._ssh, "run", lambda *a, **k: _completed('{"pid": 5, "port": 8723}')
    )
    assert _remote.remote_daemon_port(Host(name="h", ssh="u@h"), "hdmicap") == 8723


def test_remote_daemon_port_none_when_absent_or_bad(monkeypatch):
    h = Host(name="h", ssh="u@h")
    monkeypatch.setattr(_remote._ssh, "run", lambda *a, **k: _completed("", 1))
    assert _remote.remote_daemon_port(h, "hdmicap") is None
    monkeypatch.setattr(_remote._ssh, "run", lambda *a, **k: _completed("not json"))
    assert _remote.remote_daemon_port(h, "hdmicap") is None


def test_dashboard_url_builds_query():
    base = "http://127.0.0.1:9001"
    assert _cli._dashboard_url(base, None, None) == base
    assert (
        _cli._dashboard_url(base, "ws://127.0.0.1:9002/stream", None)
        == f"{base}/?serialws=ws://127.0.0.1:9002/stream"
    )
    assert (
        _cli._dashboard_url(base, "ws://127.0.0.1:9002/stream", "console")
        == f"{base}/?serialws=ws://127.0.0.1:9002/stream&interface=console"
    )


# ── integration: real ssh re-exec, opt-in ───────────────────────────────────

_DEST = os.environ.get("PANIOLO_SSH_IT")
_IDENT = os.environ.get("PANIOLO_SSH_IT_IDENTITY")
# The paniolo matching the code under test — the console script next to the
# running interpreter (the venv), not whatever happens to be first on PATH.
_colocated = Path(sys.executable).parent / "paniolo"
_PANIOLO = str(_colocated) if _colocated.exists() else shutil.which("paniolo")

integration = pytest.mark.skipif(
    not _DEST, reason="set PANIOLO_SSH_IT to run remote re-exec integration tests"
)


@pytest.fixture
def host():
    h = Host(
        name="it",
        ssh=_DEST,
        identity=_IDENT,
        control_path=f"/tmp/pcm-remote-{os.getpid()}-%C",
        paniolo_cmd=_PANIOLO,
    )
    yield h
    _ssh.close_master(h)


def _cfg() -> TargetConfig:
    return TargetConfig(name="t", interface="lo", tftp_root="/tmp/tftp-t")


@integration
def test_ship_config_round_trips(host):
    remote_path = _remote.ship_config(host, _cfg())
    try:
        text = _ssh.read_remote_file(host, remote_path)
        assert text is not None
        back = _config._from_dict(tomllib.loads(text))
        assert back.name == "t"
        assert back.interface == "lo"
        assert back.tftp_root == "/tmp/tftp-t"
    finally:
        _ssh.run(host, ["rm", "-f", remote_path])


@integration
@pytest.mark.skipif(not _PANIOLO, reason="paniolo not installed on PATH")
def test_full_cli_dispatch_runs_on_remote(host, tmp_path):
    # A lab whose single target lives on the (remote) integration host.
    ident = f'identity = "{_IDENT}"\n' if _IDENT else ""
    lab = tmp_path / "lab.toml"
    lab.write_text(
        f'[hosts.h]\nssh = "{_DEST}"\n{ident}paniolo_cmd = "{_PANIOLO}"\n\n'
        f'[targets.t]\nhost = "h"\n\n[targets.t.netboot]\ninterface = "lo"\n'
    )
    result = subprocess.run(
        [_PANIOLO, "--lab", str(lab), "netboot", "status", "t"],
        capture_output=True,
        text=True,
        timeout=60,
    )
    assert result.returncode == 0, result.stderr
    # The status text is produced by the *remote* paniolo and passed through.
    assert "stopped" in result.stdout or "running" in result.stdout
