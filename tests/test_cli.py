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

"""Tests for the CLI's location-transparent routing core: single-target
resolution, `_resolve_with_host` (env-slice precedence, lab lookup, legacy
files), and the `@remote_capable` decorator (run-here vs dispatch-over-SSH).
Lab/config/remote layers are stubbed; no SSH or filesystem is touched."""

# pylint: disable=protected-access,unused-argument

from __future__ import annotations

import pytest
import typer

from paniolo import _cli, _ssh
from paniolo._config import TargetConfig


def _cfg(name: str = "fortune") -> TargetConfig:
    return TargetConfig(name=name, interface="enx0", host_ip="192.168.99.1")


class _FakeLab:
    """Minimal stand-in for a loaded lab file."""

    def __init__(self, names, result=None, raise_exc=None):
        self._names = names
        self._result = result
        self._raise = raise_exc

    def target_names(self):
        return self._names

    def resolve_target(self, name):
        if self._raise is not None:
            raise self._raise
        return self._result


@pytest.fixture(autouse=True)
def _no_slice_env(monkeypatch):
    """Most tests exercise the lab/legacy paths; ensure the slice env that would
    short-circuit them is unset unless a test sets it explicitly."""
    monkeypatch.delenv("PANIOLO_TARGET_CONFIG", raising=False)


# ── _require_single ─────────────────────────────────────────────────────────


def test_require_single_passes_through_explicit_name():
    assert _cli._require_single("bar", ["foo"]) == "bar"


def test_require_single_resolves_sole_target():
    assert _cli._require_single(None, ["only"]) == "only"


def test_require_single_exits_when_none_configured():
    with pytest.raises(typer.Exit):
        _cli._require_single(None, [])


def test_require_single_exits_when_ambiguous():
    with pytest.raises(typer.Exit):
        _cli._require_single(None, ["a", "b"])


# ── _resolve_with_host: slice-env precedence ────────────────────────────────


def test_resolve_uses_slice_env_and_runs_local(monkeypatch):
    cfg = _cfg()
    monkeypatch.setenv("PANIOLO_TARGET_CONFIG", "/run/paniolo/slice.toml")
    monkeypatch.setattr(_cli._config, "load_target_file", lambda path: cfg)
    got_cfg, host = _cli._resolve_with_host(None)
    assert got_cfg is cfg
    assert host.is_local, "a shipped config slice always runs on the local host"


def test_resolve_slice_env_load_failure_exits(monkeypatch):
    monkeypatch.setenv("PANIOLO_TARGET_CONFIG", "/run/paniolo/slice.toml")

    def boom(path):
        raise OSError("unreadable")

    monkeypatch.setattr(_cli._config, "load_target_file", boom)
    with pytest.raises(typer.Exit):
        _cli._resolve_with_host(None)


# ── _resolve_with_host: lab lookup ──────────────────────────────────────────


def test_resolve_via_lab_returns_target_and_host(monkeypatch):
    cfg = _cfg()
    host = _ssh.Host(name="ctrl", ssh="user@ctrl")
    monkeypatch.setattr(_cli._lab, "load", lambda: _FakeLab(["fortune"], (cfg, host)))
    got_cfg, got_host = _cli._resolve_with_host(None)
    assert got_cfg is cfg
    assert got_host is host


def test_resolve_via_lab_unknown_target_exits(monkeypatch):
    monkeypatch.setattr(
        _cli._lab,
        "load",
        lambda: _FakeLab(["fortune"], raise_exc=KeyError("missing")),
    )
    with pytest.raises(typer.Exit):
        _cli._resolve_with_host("missing")


# ── _resolve_with_host: legacy per-target files (no lab) ─────────────────────


def test_resolve_legacy_returns_local_host(monkeypatch):
    cfg = _cfg()
    monkeypatch.setattr(_cli._lab, "load", lambda: None)
    monkeypatch.setattr(_cli._config, "list_targets", lambda: ["fortune"])
    monkeypatch.setattr(_cli._config, "load_target", lambda name: cfg)
    got_cfg, host = _cli._resolve_with_host(None)
    assert got_cfg is cfg
    assert host.is_local


def test_resolve_legacy_missing_target_exits(monkeypatch):
    monkeypatch.setattr(_cli._lab, "load", lambda: None)
    monkeypatch.setattr(_cli._config, "list_targets", lambda: ["fortune"])

    def missing(name):
        raise FileNotFoundError(name)

    monkeypatch.setattr(_cli._config, "load_target", missing)
    with pytest.raises(typer.Exit):
        _cli._resolve_with_host("fortune")


# ── remote_capable decorator ────────────────────────────────────────────────


def test_remote_capable_runs_body_for_local_host(monkeypatch):
    cfg = _cfg()
    local = _ssh.Host(name=_ssh.LOCAL, ssh=_ssh.LOCAL)
    monkeypatch.setattr(_cli, "_resolve_with_host", lambda name: (cfg, local))
    ran = []

    @_cli.remote_capable()
    def cmd(target=None):
        ran.append(target)
        return "local-result"

    assert cmd(target="fortune") == "local-result"
    assert ran == ["fortune"]


def test_remote_capable_dispatches_remote_and_exits_with_code(monkeypatch):
    cfg = _cfg()
    remote = _ssh.Host(name="ctrl", ssh="user@ctrl")
    monkeypatch.setattr(_cli, "_resolve_with_host", lambda name: (cfg, remote))
    seen = {}

    def fake_dispatch(host, c, mode, argv):
        seen["host"] = host
        return 7

    monkeypatch.setattr(_cli._remote, "dispatch", fake_dispatch)
    ran = []

    @_cli.remote_capable()
    def cmd(target=None):
        ran.append(target)

    with pytest.raises(typer.Exit) as ei:
        cmd(target="fortune")
    assert ei.value.exit_code == 7
    assert seen["host"] is remote
    assert not ran, "the command body never runs locally for a remote target"


def test_remote_capable_ssh_error_exits_1(monkeypatch):
    cfg = _cfg()
    remote = _ssh.Host(name="ctrl", ssh="user@ctrl")
    monkeypatch.setattr(_cli, "_resolve_with_host", lambda name: (cfg, remote))

    def boom(host, c, mode, argv):
        raise _ssh.SSHError("connection refused")

    monkeypatch.setattr(_cli._remote, "dispatch", boom)

    @_cli.remote_capable()
    def cmd(target=None):
        pass

    with pytest.raises(typer.Exit) as ei:
        cmd(target="fortune")
    assert ei.value.exit_code == 1
