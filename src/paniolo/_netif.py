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

"""Switch the target's USB-Ethernet link between netboot and ffx modes.

Both modes share one physical point-to-point link (the USB-Ethernet dongle to
the Pi's RP1 GEM). They are mutually exclusive:

  netboot — IPv4 ``host_ip``/24 + DHCP + TFTP (the daemons managed by
            ``_netboot``). The Pi TFTP-boots a kernel + ramdisk.

  ffx     — IPv6 link-local (``fe80::1``/64) on the host interface, with **no**
            DHCP/TFTP. The Pi boots from its SD card and is reached over ffx at
            ``fe80::<dev-slaac>%<iface>``.

``netif mode`` makes the transition atomic and removes the two highest-cost
seams of doing it by hand:

  1. Switching to ``ffx`` stops netboot *first*, so the next power-cycle falls
     through to the SD card instead of silently TFTP-booting a stale image.
  2. Switching to ``ffx`` adds the host-side ``fe80::1``/64 that ffx needs and
     that nothing else sets up — no manual ``sudo ip -6 addr add``.

Every mode is idempotent and re-runnable. The IPv6 link-local is ephemeral
(lost on a control-host reboot), so ``mode ffx`` simply re-adds it when absent.
The active mode is *probed* (from the running daemons and the interface's
addresses), not stored, so it stays correct even after a reboot clears things.
"""

from __future__ import annotations

import subprocess
import sys

from ._config import TargetConfig
from . import _netboot
from ._state import is_netboot_running

# The host-side IPv6 link-local that ffx talks through. Any LL works on a
# point-to-point link; ::1 is the conventional, easy-to-type choice and matches
# the manual recipe (`sudo ip -6 addr add fe80::1/64 dev <iface>`).
FFX_HOST_LL = "fe80::1"
FFX_PREFIX = 64

MODES = ("netboot", "ffx", "off")

_SUDO_HINT = (
    "Ensure passwordless sudo is configured (NOPASSWD) for the control machine."
)


# ── interface address probing ──────────────────────────────────────────────


def iface_addresses(interface: str) -> dict:
    """Return the interface's assigned addresses as {"inet": [...], "inet6": [...]}.

    IPv6 addresses keep their scope suffix stripped (``fe80::1%enx0`` → ``fe80::1``)
    but retain any prefix length (``fe80::1/64``) so callers can match exactly.
    """
    inet: list[str] = []
    inet6: list[str] = []
    if sys.platform == "darwin":
        result = subprocess.run(["ifconfig", interface], capture_output=True, text=True)
        for line in result.stdout.splitlines():
            s = line.strip()
            if s.startswith("inet "):
                inet.append(s.split()[1])
            elif s.startswith("inet6 "):
                inet6.append(s.split()[1].split("%", 1)[0])
        return {"inet": inet, "inet6": inet6}

    result = subprocess.run(
        ["ip", "-brief", "addr", "show", "dev", interface],
        capture_output=True,
        text=True,
    )
    if result.returncode == 0:
        # `ip -brief` columns: <iface> <state> <addr> <addr> ...
        for addr in result.stdout.split()[2:]:
            (inet6 if ":" in addr else inet).append(addr)
    return {"inet": inet, "inet6": inet6}


def _has_host_ll(addrs: dict) -> bool:
    """True if the ffx host link-local is currently assigned to the interface."""
    return any(a.split("/", 1)[0] == FFX_HOST_LL for a in addrs["inet6"])


def ipv6_peers(interface: str) -> list[str]:
    """Link-local IPv6 neighbours discovered on the interface (Linux only).

    Surfaces the device's own ``fe80::…`` address for a ready-to-paste
    ``ffx target add fe80::…%<iface>`` without scraping the serial log. Excludes
    our own host LL. Returns [] on macOS or when nothing has been discovered.
    """
    if sys.platform == "darwin":
        return []
    result = subprocess.run(
        ["ip", "-6", "neigh", "show", "dev", interface],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        return []
    peers: list[str] = []
    for line in result.stdout.splitlines():
        toks = line.split()
        if toks and toks[0].startswith("fe80:") and toks[0] != FFX_HOST_LL:
            peers.append(toks[0])
    return peers


# ── privileged interface mutation ──────────────────────────────────────────


def _add_host_ll(interface: str) -> None:
    """Add the ffx host link-local to the interface (idempotent)."""
    if sys.platform == "darwin":
        # macOS auto-assigns a link-local already; aliasing in fe80::1 is
        # best-effort and harmless if it already exists.
        subprocess.run(
            [
                "sudo",
                "ifconfig",
                interface,
                "inet6",
                FFX_HOST_LL,
                "prefixlen",
                str(FFX_PREFIX),
                "up",
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        return

    # IPv6 may be disabled on a fresh secondary interface; enable it. Use the
    # '/'-separated sysctl key form so interface names are passed literally.
    subprocess.run(
        ["sudo", "sysctl", "-w", f"net/ipv6/conf/{interface}/disable_ipv6=0"],
        capture_output=True,
        text=True,
        check=False,
    )
    subprocess.run(["sudo", "ip", "link", "set", interface, "up"], check=False)
    result = subprocess.run(
        [
            "sudo",
            "ip",
            "-6",
            "addr",
            "add",
            f"{FFX_HOST_LL}/{FFX_PREFIX}",
            "dev",
            interface,
        ],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0 and "exists" not in result.stderr.lower():
        raise RuntimeError(
            f"ip -6 addr add {FFX_HOST_LL}/{FFX_PREFIX} dev {interface} failed: "
            f"{result.stderr.strip()}\n{_SUDO_HINT}"
        )


def _del_host_ll(interface: str) -> None:
    """Remove the ffx host link-local from the interface (best-effort)."""
    if sys.platform == "darwin":
        subprocess.run(
            ["sudo", "ifconfig", interface, "inet6", FFX_HOST_LL, "-alias"],
            capture_output=True,
            text=True,
            check=False,
        )
        return
    subprocess.run(
        [
            "sudo",
            "ip",
            "-6",
            "addr",
            "del",
            f"{FFX_HOST_LL}/{FFX_PREFIX}",
            "dev",
            interface,
        ],
        capture_output=True,
        text=True,
        check=False,
    )


def _del_host_ip(interface: str, host_ip: str) -> None:
    """Remove a lingering netboot IPv4 from the interface (Linux, best-effort).

    Targets only ``host_ip``/24 so we never touch an unrelated address. macOS
    teardown goes through ``networksetup -setdhcp`` in ``_netboot`` instead.
    """
    if sys.platform == "darwin":
        return
    subprocess.run(
        ["sudo", "ip", "addr", "del", f"{host_ip}/24", "dev", interface],
        capture_output=True,
        text=True,
        check=False,
    )


# ── mode transitions ───────────────────────────────────────────────────────


def mode_netboot(cfg: TargetConfig, engine: str = "rust") -> None:
    """Put the link in netboot mode: tear down ffx, start DHCP+TFTP.

    Idempotent: if netboot is already running it is left as-is (the ffx
    link-local is still cleared). ``_netboot.start`` itself enforces the
    primary-NIC guard and configures the IPv4 host address.
    """
    _del_host_ll(cfg.interface)
    if is_netboot_running(cfg.name):
        return
    _netboot.start(cfg, engine=engine)


def mode_ffx(cfg: TargetConfig) -> None:
    """Put the link in ffx mode: stop netboot, add the host IPv6 link-local.

    Stopping netboot first is the point of this command — it means the next
    power-cycle boots from SD instead of TFTP. Re-running re-adds the ephemeral
    link-local if a control-host reboot cleared it.
    """
    if is_netboot_running(cfg.name):
        _netboot.stop(cfg.name)
    _add_host_ll(cfg.interface)


def mode_off(cfg: TargetConfig) -> None:
    """Drop both configs: stop netboot, remove the ffx LL and any stale IPv4."""
    if is_netboot_running(cfg.name):
        # _netboot.stop restores the interface (flushes the IPv4 host address).
        _netboot.stop(cfg.name)
    _del_host_ll(cfg.interface)
    _del_host_ip(cfg.interface, cfg.host_ip)


# ── status ─────────────────────────────────────────────────────────────────


def get_status(cfg: TargetConfig) -> dict:
    """Probe and report the active mode plus the interface's addresses.

    ``mode`` is derived, not stored: netboot daemons running → ``netboot``; else
    the ffx host LL present → ``ffx``; else ``off``.
    """
    netboot_running = is_netboot_running(cfg.name)
    addrs = iface_addresses(cfg.interface)
    has_ll = _has_host_ll(addrs)

    if netboot_running:
        mode = "netboot"
    elif has_ll:
        mode = "ffx"
    else:
        mode = "off"

    return {
        "target": cfg.name,
        "interface": cfg.interface,
        "mode": mode,
        "netboot_running": netboot_running,
        "host_ip": cfg.host_ip,
        "host_ll": FFX_HOST_LL if has_ll else None,
        "inet": addrs["inet"],
        "inet6": addrs["inet6"],
        "peers": ipv6_peers(cfg.interface) if mode == "ffx" else [],
    }
