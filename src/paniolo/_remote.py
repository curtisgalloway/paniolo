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

"""Transparent re-exec of a command on a target's remote control host.

When a command targets a host other than the dev machine, paniolo re-runs the
*same* invocation on that host over SSH (see docs/distributed-control.md). The
control host is stateless, so the resolved TargetConfig is shipped to it as a
temp file and pointed at with PANIOLO_TARGET_CONFIG; the remote paniolo then
runs against that slice locally. stdin/stdout/stderr and the exit code pass
through, so the command behaves as if it ran here.

This module holds the transport-shaped helpers (pure where possible, for
testing); the ``@remote_capable`` decorator that calls them lives in _cli.py
next to the command definitions and target resolution.
"""

from __future__ import annotations

import json
from typing import Optional

from . import _config, _ssh
from ._config import TargetConfig
from ._ssh import Host

TARGET_CONFIG_ENV = "PANIOLO_TARGET_CONFIG"

# Re-exec dispatch modes.
REEXEC = "reexec"  # non-interactive: stdio passed through (run_passthrough)
INTERACTIVE = "interactive"  # PTY over ssh -t (e.g. tio)


def strip_lab_option(args: list[str]) -> list[str]:
    """Drop the global ``--lab PATH`` / ``--lab=PATH`` option from an argv tail.

    The dev machine's lab path is meaningless on the control host — it gets the
    config via PANIOLO_TARGET_CONFIG instead — so it must not travel with the
    re-exec'd command.
    """
    out: list[str] = []
    skip = False
    for arg in args:
        if skip:
            skip = False
            continue
        if arg == "--lab":
            skip = True
            continue
        if arg.startswith("--lab="):
            continue
        out.append(arg)
    return out


def remote_argv(argv: list[str], paniolo_cmd: str = "paniolo") -> list[str]:
    """Build the command to run on the host from this process's ``sys.argv``.

    argv[0] (the local launcher path) becomes the host's paniolo command (bare
    ``paniolo`` by default, or a path the host pins); the ``--lab`` option is
    stripped.
    """
    return [paniolo_cmd] + strip_lab_option(argv[1:])


# A POSIX-sh one-liner: make a temp file, write stdin into it, print its path.
_SHIP_SCRIPT = (
    'f=$(mktemp "${TMPDIR:-/tmp}/paniolo-cfg.XXXXXX") && cat > "$f" && printf %s "$f"'
)


def ship_config(host: Host, cfg: TargetConfig) -> str:
    """Write ``cfg`` to a temp file on ``host`` and return its remote path."""
    toml = _config._to_toml(cfg)
    result = _ssh.run(host, ["sh", "-c", _SHIP_SCRIPT], stdin=toml)
    path = result.stdout.strip()
    if result.returncode != 0 or not path:
        raise _ssh.SSHError(
            f"failed to ship target config to {host.name}: {result.stderr.strip()}"
        )
    return path


def dispatch(host: Host, cfg: TargetConfig, mode: str, argv: list[str]) -> int:
    """Run this invocation on ``host`` over SSH; return the remote exit code.

    Ships ``cfg`` as the remote's PANIOLO_TARGET_CONFIG slice, re-execs the
    (lab-stripped) command, and removes the slice afterward.
    """
    remote_path = ship_config(host, cfg)
    env = {TARGET_CONFIG_ENV: remote_path}
    cmd = remote_argv(argv, host.paniolo)
    try:
        if mode == INTERACTIVE:
            return _ssh.run_interactive(host, cmd, env=env)
        return _ssh.run_passthrough(host, cmd, env=env)
    finally:
        _ssh.run(host, ["rm", "-f", remote_path])


def run_subcommand(host: Host, cfg: TargetConfig, subargs: list[str]):
    """Run ``paniolo <subargs>`` on ``host`` against a shipped config slice.

    Captured (returns a CompletedProcess); used to drive helper commands on the
    host, e.g. starting the streaming daemons before tunnelling to them.
    """
    remote_path = ship_config(host, cfg)
    env = {TARGET_CONFIG_ENV: remote_path}
    try:
        return _ssh.run(host, [host.paniolo, *subargs], env=env)
    finally:
        _ssh.run(host, ["rm", "-f", remote_path])


# Resolve the daemon discovery dir the same way the daemons do (see
# _video/_serial._discovery_path): $XDG_RUNTIME_DIR, else $TMPDIR, else /tmp.
_REMOTE_RUNTIME = "${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}"


def remote_daemon_port(host: Host, subdir: str) -> Optional[int]:
    """Read the TCP port of a daemon's discovery file on ``host``, or None.

    ``subdir`` is the daemon's runtime subdir ("hdmicap" / "serialcap"). The
    path is resolved by a remote shell so the host's own XDG_RUNTIME_DIR applies.
    """
    script = f'cat "{_REMOTE_RUNTIME}/{subdir}/daemon.json" 2>/dev/null'
    result = _ssh.run(host, ["sh", "-c", script])
    if result.returncode != 0 or not result.stdout.strip():
        return None
    try:
        return int(json.loads(result.stdout)["port"])
    except (ValueError, KeyError, json.JSONDecodeError):
        return None
