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

"""Tests for the SSH transport (_ssh.py).

Two tiers:

- **Unit** (always run): argument construction, quoting, and the local-host
  guard — no SSH process is spawned.
- **Integration** (opt-in): real ssh against a destination given in
  ``PANIOLO_SSH_IT`` (e.g. ``localhost``), with an optional identity in
  ``PANIOLO_SSH_IT_IDENTITY``. Skipped otherwise, so the suite never triggers an
  ssh-agent (e.g. 1Password) confirmation on a dev box. CI sets these against a
  provisioned passphraseless key with no agent.
"""

from __future__ import annotations

import os
import socket
import threading

import pytest

from paniolo import _ssh
from paniolo._ssh import Host

# ── unit: no SSH spawned ─────────────────────────────────────────────────────


def test_is_local():
    assert Host(name="local", ssh="local").is_local
    assert Host(name="x", ssh="local").is_local
    assert not Host(name="bench1", ssh="curtisg@bench1").is_local


def test_local_host_is_rejected_before_spawning():
    local = Host(name="local", ssh="local")
    with pytest.raises(ValueError):
        _ssh.run(local, ["true"])
    with pytest.raises(ValueError):
        with _ssh.forward(local, 8723):
            pass


def test_close_master_is_noop_for_local():
    # Must not raise or spawn anything.
    _ssh.close_master(Host(name="local", ssh="local"))


def test_remote_command_quotes_each_arg():
    cmd = _ssh._remote_command(["echo", "a b", "c;d"], None)
    assert cmd == "echo 'a b' 'c;d'"


def test_remote_command_prepends_env_assignments():
    cmd = _ssh._remote_command(
        ["paniolo", "netboot", "status"], {"PANIOLO_TARGET_CONFIG": "/t/f f.toml"}
    )
    assert cmd.startswith("PANIOLO_TARGET_CONFIG='/t/f f.toml' ")
    assert cmd.endswith("paniolo netboot status")


def test_base_args_non_interactive_has_batchmode_and_controlmaster():
    args = _ssh._base_args(Host(name="b", ssh="u@b"))
    assert "BatchMode=yes" in args
    assert "ControlMaster=auto" in args
    assert any(a.startswith("ControlPath=") for a in args)
    assert f"ControlPersist={_ssh._CONTROL_PERSIST}" in args


def test_base_args_interactive_drops_batchmode():
    args = _ssh._base_args(Host(name="b", ssh="u@b"), interactive=True)
    assert "BatchMode=yes" not in args


def test_base_args_identity_uses_identities_only():
    args = _ssh._base_args(Host(name="b", ssh="u@b", identity="~/.ssh/id_lab"))
    assert "-i" in args
    assert "IdentitiesOnly=yes" in args
    # ~ is expanded so ssh resolves the path.
    idx = args.index("-i")
    assert not args[idx + 1].startswith("~")


def test_forward_args_disable_multiplexing():
    # A port forward must own a standalone connection, not attach to the master.
    args = _ssh._base_args(Host(name="b", ssh="u@b"), multiplex=False)
    assert "ControlMaster=no" in args
    assert "ControlPath=none" in args
    assert "ControlMaster=auto" not in args


def test_custom_control_path_is_honored():
    args = _ssh._base_args(Host(name="b", ssh="u@b", control_path="/tmp/cm-test"))
    assert "ControlPath=/tmp/cm-test" in args


def test_free_local_port_is_usable():
    port = _ssh._free_local_port()
    assert 1024 < port < 65536
    # Re-bindable, i.e. actually free.
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", port))


# ── integration: real ssh, opt-in ───────────────────────────────────────────

_DEST = os.environ.get("PANIOLO_SSH_IT")
_IDENT = os.environ.get("PANIOLO_SSH_IT_IDENTITY")

integration = pytest.mark.skipif(
    not _DEST,
    reason="set PANIOLO_SSH_IT=<ssh-dest> (e.g. localhost) to run SSH integration tests",
)


@pytest.fixture
def host():
    # A short control path: a deep tmp_path overflows the ~104-char Unix-socket
    # limit once ssh appends the %C hash. Unique per process; torn down below.
    h = Host(
        name="it",
        ssh=_DEST,
        identity=_IDENT,
        control_path=f"/tmp/pcm-{os.getpid()}-%C",
    )
    yield h
    _ssh.close_master(h)


@integration
def test_run_roundtrip(host):
    r = _ssh.run(host, ["echo", "hello world"])
    assert r.returncode == 0
    assert r.stdout.strip() == "hello world"


@integration
def test_run_env(host):
    r = _ssh.run(host, ["sh", "-c", "echo $FOO"], env={"FOO": "bar baz"})
    assert r.stdout.strip() == "bar baz"


@integration
def test_run_nonzero_exit(host):
    assert _ssh.run(host, ["false"]).returncode != 0


@integration
def test_run_passthrough_returns_exit_code(host):
    assert _ssh.run_passthrough(host, ["true"]) == 0
    assert _ssh.run_passthrough(host, ["false"]) != 0


@integration
def test_read_remote_file_present_and_absent(host, tmp_path):
    f = tmp_path / "probe.txt"
    f.write_text("contents-123\n")
    assert _ssh.read_remote_file(host, str(f)) == "contents-123\n"
    assert _ssh.read_remote_file(host, str(tmp_path / "nope.txt")) is None


@integration
def test_forward_reaches_a_local_listener(host):
    # A throwaway server bound on 127.0.0.1; forward() tunnels to it via ssh.
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", 0))
    server.listen(1)
    remote_port = server.getsockname()[1]

    def serve():
        # Accept repeatedly: forward()'s readiness probe opens (and drops) a
        # connection through the tunnel before the test's real one, so a
        # single-accept server would be consumed by the probe. Real daemons
        # serve many connections, so this matches production behaviour.
        while True:
            try:
                conn, _ = server.accept()
            except OSError:
                return
            conn.sendall(b"BANNER-OK")
            conn.close()

    t = threading.Thread(target=serve, daemon=True)
    t.start()
    try:
        with _ssh.forward(host, remote_port) as local_port:
            with socket.create_connection(("127.0.0.1", local_port), timeout=5) as c:
                assert c.recv(16) == b"BANNER-OK"
    finally:
        server.close()
        t.join(timeout=2)
