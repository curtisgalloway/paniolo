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

"""SSH transport for driving a target on a remote control host.

This is the foundation of paniolo's distributed-control model (see
docs/distributed-control.md): the dev machine is the hub, and every command
against a remote target reaches its control host over SSH. There is no agent or
RPC server — SSH is the whole transport, which is why auth, encryption, and
identity come for free.

Three things are provided over a per-host **ControlMaster** connection (so only
the first call to a host pays the SSH handshake):

- ``run`` / ``run_passthrough`` — run a command on the host (captured, or with
  the local terminal's stdio passed through for transparent re-exec);
- ``run_interactive`` — run a command over an ``ssh -t`` PTY (for ``tio``);
- ``forward`` — hold an ``ssh -L`` tunnel to a remote port (for the dashboard);
- ``read_remote_file`` — read a remote file (e.g. a daemon discovery file).

A ``Host`` whose ``ssh`` destination is the literal ``"local"`` represents the
dev machine itself; the SSH functions reject it (callers run those commands
directly instead), but it lets the lab model express "no SSH" uniformly.
"""

from __future__ import annotations

import dataclasses
import os
import shlex
import socket
import subprocess
import time
from contextlib import contextmanager
from pathlib import Path
from typing import Iterator, Optional

# ssh destination meaning "the dev machine itself — no SSH".
LOCAL = "local"

# How long the shared master connection lingers after the last use.
_CONTROL_PERSIST = "300"
_CONNECT_TIMEOUT = "10"


@dataclasses.dataclass
class Host:
    """A control host paniolo reaches over SSH.

    ``ssh`` is the destination passed to ssh(1) (``user@host``, an ssh_config
    alias, or the sentinel ``"local"``). ``identity`` is an optional private-key
    path; ``control_path`` overrides the ControlMaster socket path (ssh %-tokens
    are allowed, e.g. ``~/.ssh/cm-%C``).
    """

    name: str
    ssh: str
    identity: Optional[str] = None
    control_path: Optional[str] = None
    # How to invoke paniolo on this host. Defaults to bare "paniolo"; set it to
    # an absolute path when paniolo isn't on the host's non-interactive ssh PATH
    # (e.g. installed under ~/.local/bin, which login-only PATHs miss).
    paniolo_cmd: Optional[str] = None

    @property
    def is_local(self) -> bool:
        return self.ssh == LOCAL or self.name == LOCAL

    @property
    def paniolo(self) -> str:
        return self.paniolo_cmd or "paniolo"


class SSHError(RuntimeError):
    """An SSH operation failed to establish (connect/forward), as opposed to a
    remote command merely exiting non-zero."""


def _control_dir() -> Path:
    """Short directory holding paniolo's default ControlMaster sockets.

    Kept deliberately short: a Unix-domain socket path has a hard limit
    (~104 chars on macOS, ~108 on Linux), and ssh appends a 40-char %C hash to
    it. XDG_RUNTIME_DIR is short on Linux; elsewhere fall back to /tmp rather
    than tempfile.gettempdir(), which on macOS is the long per-session $TMPDIR
    and overflows the limit.
    """
    base = os.environ.get("XDG_RUNTIME_DIR") or "/tmp"
    d = Path(base) / f"paniolo-{os.getuid()}"
    d.mkdir(parents=True, exist_ok=True)
    os.chmod(d, 0o700)
    return d


def _control_args(host: Host) -> list[str]:
    if host.control_path:
        cp = os.path.expanduser(host.control_path)
        # Best-effort: create the parent dir unless it contains an ssh %-token.
        parent = os.path.dirname(cp)
        if parent and "%" not in parent:
            Path(parent).mkdir(parents=True, exist_ok=True)
    else:
        cp = str(_control_dir() / "cm-%C")
    return [
        "-o",
        "ControlMaster=auto",
        "-o",
        f"ControlPath={cp}",
        "-o",
        f"ControlPersist={_CONTROL_PERSIST}",
    ]


def _base_args(
    host: Host, *, interactive: bool = False, multiplex: bool = True
) -> list[str]:
    if host.is_local:
        raise ValueError(f"_ssh called for local host '{host.name}'")
    args = ["ssh"]
    if not interactive:
        # Fail rather than block on a password prompt for non-interactive use.
        args += ["-o", "BatchMode=yes"]
    args += ["-o", f"ConnectTimeout={_CONNECT_TIMEOUT}"]
    if host.identity:
        args += ["-i", os.path.expanduser(host.identity), "-o", "IdentitiesOnly=yes"]
    if multiplex:
        args += _control_args(host)
    else:
        # A standalone connection that owns its own channel. Port forwards must
        # NOT multiplex: an `ssh -N -L` client that attaches to a ControlMaster
        # hands the forward to the master and then exits, so the process no
        # longer represents (or can tear down) the tunnel.
        args += ["-o", "ControlMaster=no", "-o", "ControlPath=none"]
    return args


def _remote_command(argv: list[str], env: Optional[dict[str, str]]) -> str:
    """Quote argv (and optional env assignments) into one remote shell command.

    Quoting each token preserves argument boundaries through the remote shell, so
    paths with spaces or globbing characters survive intact.
    """
    parts: list[str] = []
    if env:
        for key, val in env.items():
            parts.append(f"{key}={shlex.quote(val)}")
    parts.extend(shlex.quote(a) for a in argv)
    return " ".join(parts)


def run(
    host: Host,
    argv: list[str],
    *,
    stdin: Optional[str] = None,
    env: Optional[dict[str, str]] = None,
    timeout: Optional[float] = None,
) -> subprocess.CompletedProcess:
    """Run ``argv`` on ``host`` and capture its output.

    Returns the CompletedProcess (text mode); never raises on a non-zero remote
    exit. Use for programmatic calls — reading a discovery file, probing state.
    """
    full = _base_args(host) + [host.ssh, _remote_command(argv, env)]
    return subprocess.run(
        full, input=stdin, capture_output=True, text=True, timeout=timeout
    )


def run_passthrough(
    host: Host, argv: list[str], *, env: Optional[dict[str, str]] = None
) -> int:
    """Run ``argv`` on ``host`` with the local terminal's stdio passed through.

    No PTY is allocated, so pipes and non-interactive semantics are preserved.
    This is the transparent re-exec path: the remote command's stdin/stdout/
    stderr *are* the local ones. Returns the exit code.
    """
    full = _base_args(host) + [host.ssh, _remote_command(argv, env)]
    return subprocess.run(full).returncode


def run_interactive(
    host: Host, argv: list[str], *, env: Optional[dict[str, str]] = None
) -> int:
    """Run ``argv`` on ``host`` over an ``ssh -t`` PTY (for interactive tools).

    Used for ``serial connect`` (tio) — SSH's own pseudo-terminal is the
    transport, so no tunnel is needed. Returns the exit code.
    """
    full = _base_args(host, interactive=True)
    full += ["-t", host.ssh, _remote_command(argv, env)]
    return subprocess.run(full).returncode


def read_remote_file(host: Host, path: str) -> Optional[str]:
    """Return the contents of a remote file, or None if it doesn't exist."""
    result = run(host, ["cat", path])
    if result.returncode != 0:
        return None
    return result.stdout


def _free_local_port() -> int:
    """Pick a currently-free local TCP port (small TOCTOU window is acceptable)."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _wait_for_port(port: int, proc: subprocess.Popen, timeout: float) -> None:
    """Block until 127.0.0.1:port accepts a connection, or raise.

    Fails fast if the ssh forwarder process exits early (e.g. auth failure).
    """
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            stderr = proc.stderr.read() if proc.stderr else ""
            raise SSHError(
                f"ssh forward exited early (rc={proc.returncode}): {stderr.strip()}"
            )
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return
        except OSError:
            time.sleep(0.1)
    raise SSHError(f"timed out waiting for forwarded port {port}")


@contextmanager
def forward(
    host: Host,
    remote_port: int,
    *,
    remote_bind: str = "127.0.0.1",
    timeout: float = 10.0,
) -> Iterator[int]:
    """Hold an ``ssh -L`` tunnel to ``remote_bind:remote_port`` on ``host``.

    Yields the local port the tunnel listens on (127.0.0.1). The forwarder
    process is torn down on exit. It attaches to the shared ControlMaster
    connection, so it doesn't pay a second handshake.
    """
    local_port = _free_local_port()
    spec = f"{local_port}:{remote_bind}:{remote_port}"
    args = _base_args(host, multiplex=False) + ["-N", "-L", spec, host.ssh]
    proc = subprocess.Popen(
        args, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True
    )
    try:
        _wait_for_port(local_port, proc, timeout)
        yield local_port
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()


def close_master(host: Host) -> None:
    """Tear down the shared ControlMaster connection to ``host`` if one is open."""
    if host.is_local:
        return
    subprocess.run(
        _base_args(host) + ["-O", "exit", host.ssh],
        capture_output=True,
        text=True,
    )
