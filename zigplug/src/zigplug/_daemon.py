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

"""zigplug daemon: a persistent owner for the ZNP coordinator session.

One-shot invocations are unreliable by construction on CC2652 sticks: every
serial-port open toggles DTR/RTS through the auto-bootloader circuit and
resets the chip (sometimes *into* the bootloader, which hangs the client),
and two concurrent invocations collide on the stateful ZNP session.

The daemon opens the coordinator once and serves operations over localhost
HTTP, serialized on a single lock with hard per-operation timeouts. It
follows paniolo's daemon contract (see cli/src/daemons.rs): it binds an
OS-assigned port on 127.0.0.1 and publishes
`/tmp/paniolo-<uid>/zigplug/daemon.json` containing `{pid, port, device}`.
The one-shot CLI auto-spawns it and proxies through it transparently, so
paniolo power hooks (`zigplug -d <dev> on <ieee>`) don't change.
"""

# Single quotes nested in double-quoted f-strings are required on Python 3.11;
# the aiohttp handlers share a (request, body) signature whether they use both
# or not.
# pylint: disable=inconsistent-quotes,unused-argument

from __future__ import annotations

import asyncio
import contextlib
import fcntl
import json
import logging
import os
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from . import _app

LOGGER = logging.getLogger(__name__)

# Hard ceiling for one radio operation (a switch + read-back takes ~1 s on a
# healthy network); cycle/permit add their own wait on top. A wedged session
# returns an error instead of hanging the power hook forever.
OP_TIMEOUT_S = 30.0

# How long a client waits for a freshly spawned daemon to come up (ZNP
# connect + network start is a few seconds; bootloader-entry wedges never
# come up — better to fail and let the next invocation respawn).
SPAWN_TIMEOUT_S = 20.0

DISCOVERY_NAME = "zigplug"


# ── runtime dir + discovery (must mirror cli/src/daemons.rs) ────────────────


def runtime_dir() -> Path:
    """`/tmp/paniolo-<uid>/zigplug`, creating the 0700 base if needed."""
    base = Path(f"/tmp/paniolo-{os.getuid()}")
    try:
        base.mkdir(mode=0o700)
    except FileExistsError:
        st = base.lstat()
        if not base.is_dir() or st.st_uid != os.getuid():
            raise _app.ZigplugError(
                f"{base} exists but is not a directory owned by uid {os.getuid()}"
            ) from None
    d = base / DISCOVERY_NAME
    d.mkdir(exist_ok=True)
    return d


def discovery_path() -> Path:
    return runtime_dir() / "daemon.json"


def log_path() -> Path:
    return runtime_dir() / "daemon.log"


def _pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except (ProcessLookupError, PermissionError):
        return False
    return True


def read_discovery() -> dict | None:
    """The running daemon's `{pid, port, device}`, or None."""
    try:
        info = json.loads(discovery_path().read_text())
    except (OSError, ValueError):
        return None
    if not _pid_alive(int(info.get("pid", -1))):
        return None
    return info


def daemon_url(device: str) -> str | None:
    """Base URL of a running daemon serving `device`, or None.

    A daemon serving a *different* device is an error, not a miss — one
    coordinator per daemon, and silently bypassing it would reintroduce
    the port collision this daemon exists to prevent.
    """
    info = read_discovery()
    if info is None:
        return None
    if info.get("device") != device:
        raise _app.ZigplugError(
            f"zigplug daemon (pid {info['pid']}) is serving {info['device']!r}, "
            f"not {device!r} — stop it first (`zigplug -d {info['device']} stop`)"
        )
    return f"http://127.0.0.1:{info['port']}"


# ── client side: proxy calls + auto-spawn ────────────────────────────────────


def call(base_url: str, method: str, path: str, body: dict | None, timeout: float):
    """One JSON request to the daemon; raises ZigplugError on error replies."""
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        f"{base_url}{path}",
        data=data,
        method=method,
        headers={"content-type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read() or b"{}")
    except urllib.error.HTTPError as exc:
        try:
            message = json.loads(exc.read()).get("error", str(exc))
        except ValueError:
            message = str(exc)
        raise _app.ZigplugError(message) from exc
    except (urllib.error.URLError, TimeoutError, ConnectionError) as exc:
        raise _app.ZigplugError(
            f"zigplug daemon unreachable ({exc}) — it may have died; "
            "the next invocation will restart it"
        ) from exc


def spawn(device: str, db_path: Path) -> str:
    """Start a detached daemon for `device` and wait for it; returns its URL.

    Serialized on a lock file so two concurrent one-shots can't both spawn
    (the loser of the race finds the winner's daemon via discovery).
    """
    lock_file = (runtime_dir() / "spawn.lock").open("w")
    try:
        fcntl.flock(lock_file, fcntl.LOCK_EX)
        url = daemon_url(device)  # someone else may have won the race
        if url is not None:
            return url
        log = log_path().open("w")
        with contextlib.redirect_stdout(sys.stderr):
            print(f"starting zigplug daemon for {device}…")
        subprocess.Popen(  # noqa: consider-using-with — outlives us by design
            [
                sys.executable,
                "-m",
                "zigplug",
                "--device",
                device,
                "--db",
                str(db_path),
                "serve",
                "--foreground",
            ],
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=log,
            start_new_session=True,
        )
        deadline = time.monotonic() + SPAWN_TIMEOUT_S
        while time.monotonic() < deadline:
            url = daemon_url(device)
            if url is not None:
                try:
                    call(url, "GET", "/healthz", None, timeout=2.0)
                    return url
                except _app.ZigplugError:
                    pass
            time.sleep(0.25)
        tail = ""
        with contextlib.suppress(OSError):
            tail = "\n  ".join(log_path().read_text().splitlines()[-5:])
        raise _app.ZigplugError(
            f"zigplug daemon did not start within {SPAWN_TIMEOUT_S:.0f}s"
            + (f"; last log lines:\n  {tail}" if tail else "")
        )
    finally:
        fcntl.flock(lock_file, fcntl.LOCK_UN)
        lock_file.close()


def ensure(device: str, db_path: Path) -> str:
    """URL of a daemon serving `device`, spawning one if needed."""
    return daemon_url(device) or spawn(device, db_path)


# ── server side ──────────────────────────────────────────────────────────────


async def serve(device: str, db_path: Path) -> int:
    """Run the daemon in the foreground until stopped; returns an exit code.

    Exit code 1 (e.g. on radio connection loss) tells the wrapper layers the
    session died abnormally; the discovery file is removed either way so the
    next one-shot respawns cleanly.
    """
    # Local import: the server is the only consumer, and one-shot fallback
    # paths must not require it at import time.
    from aiohttp import web  # pylint: disable=import-outside-toplevel

    config = _app.build_config(device, db_path)
    try:
        app = await _app.ControllerApplication.new(config)
    except Exception as exc:  # surface a one-line reason in the daemon log
        LOGGER.error("coordinator startup failed: %s", exc)
        raise
    started = time.monotonic()
    lock = asyncio.Lock()
    stop_event = asyncio.Event()
    exit_code = 0

    class RadioListener:
        """Exit (code 1) when the ZNP session dies so a fresh daemon respawns."""

        def connection_lost(self, exc: Exception | None = None) -> None:
            nonlocal exit_code
            LOGGER.error("radio connection lost: %s", exc)
            exit_code = 1
            stop_event.set()

    app.add_listener(RadioListener())

    def fail(status: int, message: str) -> web.Response:
        return web.json_response({"error": message}, status=status)

    def handler(fn, *, extra_timeout: float = 0.0):
        async def wrapped(request: web.Request) -> web.Response:
            body = {}
            if request.can_read_body:
                try:
                    body = await request.json()
                except ValueError:
                    return fail(400, "invalid JSON body")
            timeout = OP_TIMEOUT_S + extra_timeout_from(body) + extra_timeout
            try:
                async with lock:
                    result = await asyncio.wait_for(fn(request, body), timeout)
            except _app.ZigplugError as exc:
                return fail(400, str(exc))
            except asyncio.TimeoutError:
                return fail(504, f"operation timed out after {timeout:.0f}s")
            except Exception as exc:  # pylint: disable=broad-exception-caught
                LOGGER.exception("operation failed")
                return fail(500, f"{type(exc).__name__}: {exc}")
            return web.json_response(result)

        return wrapped

    def extra_timeout_from(body: dict) -> float:
        # cycle holds the lock for its off-delay; permit for its window.
        return float(body.get("delay_ms", 0)) / 1000.0 + float(body.get("time_s", 0))

    def cluster_for(ieee_text: str):
        ieee = _app.parse_ieee(ieee_text)
        device_obj = _app.find_device(app, ieee)
        return _app.on_off_cluster(device_obj)

    async def h_healthz(request, body):
        info = app.state.network_info
        return {
            "device": device,
            "channel": info.channel,
            "pan_id": f"0x{info.pan_id:04x}",
            "uptime_s": round(time.monotonic() - started, 1),
        }

    async def h_state(request, body):
        cluster = cluster_for(request.query["ieee"])
        return {"state": "on" if await _app.read_on_off(cluster) else "off"}

    async def h_on(request, body):
        await _app.set_on_off(cluster_for(body["ieee"]), True)
        return {"state": "on"}

    async def h_off(request, body):
        await _app.set_on_off(cluster_for(body["ieee"]), False)
        return {"state": "off"}

    async def h_cycle(request, body):
        cluster = cluster_for(body["ieee"])
        delay_ms = int(body.get("delay_ms", 3000))
        await _app.set_on_off(cluster, False)
        await asyncio.sleep(delay_ms / 1000.0)
        await _app.set_on_off(cluster, True)
        return {"state": "on", "held_off_ms": delay_ms}

    async def h_list(request, body):
        plugs = []
        for dev in _app.plug_devices(app):
            try:
                cluster = _app.on_off_cluster(dev)
                state = "on" if await _app.read_on_off(cluster) else "off"
            except (_app.ZigplugError, asyncio.TimeoutError, OSError):
                state = "?"
            plugs.append(
                {
                    "ieee": str(dev.ieee),
                    "nwk": f"0x{dev.nwk:04x}",
                    "manufacturer": dev.manufacturer,
                    "model": dev.model,
                    "state": state,
                }
            )
        return {"plugs": plugs}

    async def h_permit(request, body):
        time_s = int(body.get("time_s", 60))
        joined: list[dict] = []

        class Listener:

            def device_initialized(self, dev) -> None:
                joined.append(
                    {
                        "ieee": str(dev.ieee),
                        "manufacturer": dev.manufacturer,
                        "model": dev.model,
                    }
                )

        app.add_listener(Listener())
        await app.permit(time_s=time_s)
        await asyncio.sleep(time_s)
        return {"joined": joined}

    async def h_remove(request, body):
        ieee = _app.parse_ieee(body["ieee"])
        _app.find_device(app, ieee)
        await app.remove(ieee)
        await asyncio.sleep(2)  # let the background leave request go out
        return {"removed": str(ieee)}

    async def h_backup(request, body):
        backup = app.backups.create_backup(load_devices=True)
        return backup.as_dict()

    async def h_stop(request, body):
        stop_event.set()
        return {"stopping": True}

    web_app = web.Application()
    web_app.router.add_get("/healthz", handler(h_healthz))
    web_app.router.add_get("/state", handler(h_state))
    web_app.router.add_post("/on", handler(h_on))
    web_app.router.add_post("/off", handler(h_off))
    web_app.router.add_post("/cycle", handler(h_cycle))
    web_app.router.add_get("/list", handler(h_list))
    web_app.router.add_post("/permit", handler(h_permit))
    web_app.router.add_post("/remove", handler(h_remove))
    web_app.router.add_get("/backup", handler(h_backup))
    web_app.router.add_post("/stop", handler(h_stop))

    runner = web.AppRunner(web_app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    port = runner.addresses[0][1]  # the OS-assigned port

    discovery = discovery_path()
    discovery.write_text(
        json.dumps({"pid": os.getpid(), "port": port, "device": device})
    )
    LOGGER.info("zigplug daemon up: %s port %d", device, port)

    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, stop_event.set)

    try:
        await stop_event.wait()
    finally:
        with contextlib.suppress(OSError):
            discovery.unlink()
        await runner.cleanup()
        await app.shutdown()
    return exit_code
