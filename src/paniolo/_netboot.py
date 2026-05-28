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

from __future__ import annotations

import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

from ._config import TargetConfig
from ._state import (
    NetbootState,
    ensure_target_dir,
    is_netboot_running,
    is_pid_alive,
    load_netboot_state,
    netboot_log_path,
    netboot_state_path,
    save_netboot_state,
)

_BREW_PATHS = [
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
    "/usr/local/bin",
    "/usr/local/sbin",
]

_EXCLUDE_PORT_PREFIXES = (
    "Wi-Fi",
    "Thunderbolt",
    "Bluetooth",
    "FireWire",
    "iPhone",
    "iPad",
)
_EXCLUDE_DEVICES = {"bridge0", "lo0"}


def _find_bin(name: str) -> str:
    found = shutil.which(name)
    if found:
        return found
    for d in _BREW_PATHS:
        p = Path(d) / name
        if p.exists():
            return str(p)
    return name


def check_deps() -> list[str]:
    # DHCP and TFTP are both pure-Python (see _dhcp.py, _tftp.py); no external
    # binaries required.
    return []


def _is_interface_active(device: str) -> bool:
    try:
        out = subprocess.check_output(
            ["ifconfig", device], text=True, stderr=subprocess.DEVNULL
        )
        return "status: active" in out
    except (subprocess.CalledProcessError, FileNotFoundError):
        return False


def list_usb_ethernet_interfaces() -> list[dict]:
    """Return external (non-built-in) Ethernet interfaces, active ones first.

    Each entry: {"port": str, "device": str, "active": bool}
    Excludes Wi-Fi, Thunderbolt, Bluetooth, FireWire, and virtual bridges.
    """
    try:
        out = subprocess.check_output(
            ["networksetup", "-listallhardwareports"],
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return []

    candidates: list[dict] = []
    port: str | None = None
    for line in out.splitlines():
        if line.startswith("Hardware Port:"):
            port = line.split(":", 1)[1].strip()
        elif line.startswith("Device:") and port is not None:
            device = line.split(":", 1)[1].strip()
            if device not in _EXCLUDE_DEVICES and not any(
                port.startswith(p) for p in _EXCLUDE_PORT_PREFIXES
            ):
                candidates.append(
                    {
                        "port": port,
                        "device": device,
                        "active": _is_interface_active(device),
                    }
                )
            port = None

    return sorted(candidates, key=lambda x: (not x["active"], x["device"]))




def _spawn(cmd: list[str], log_path: Path, append: bool = False) -> subprocess.Popen:
    if not append:
        log_path.unlink(missing_ok=True)
    log_file = open(log_path, "a")
    env = {**os.environ, "PYTHONUNBUFFERED": "1"}
    return subprocess.Popen(
        cmd,
        stdout=log_file,
        stderr=log_file,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
        env=env,
    )


def _find_network_service(interface: str) -> str | None:
    """Return the networksetup service name for a given device (e.g. 'en11' → 'USB 10/100/1000 LAN')."""
    try:
        out = subprocess.check_output(
            ["networksetup", "-listallhardwareports"],
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None
    service: str | None = None
    for line in out.splitlines():
        if line.startswith("Hardware Port:"):
            service = line.split(":", 1)[1].strip()
        elif line.startswith("Device:"):
            if line.split(":", 1)[1].strip() == interface:
                return service
    return None


def _configure_interface(interface: str, host_ip: str) -> None:
    service = _find_network_service(interface)
    if service:
        subprocess.run(
            ["sudo", "networksetup", "-setmanual", service, host_ip, "255.255.255.0"],
            check=False,
        )
    result = subprocess.run(
        ["sudo", "ifconfig", interface, host_ip, "netmask", "255.255.255.0", "up"],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"ifconfig {interface} failed: {result.stderr.strip()}\n"
            "Ensure passwordless sudo is configured (NOPASSWD) for the control machine."
        )


def _restore_interface(interface: str) -> None:
    """Return the interface to DHCP so macOS resumes normal network management."""
    service = _find_network_service(interface)
    if service:
        subprocess.run(
            ["sudo", "networksetup", "-setdhcp", service],
            check=False,
        )


def _tune_arp_for_silent_client() -> None:
    """Disable macOS neighbor-unreachability detection (NUD) for the netboot link.

    The Pi's bootloader sends us DHCP/TFTP but never answers ARP probes. Even
    with a permanent static ARP entry, recent macOS (26.x) refuses to transmit
    to such a neighbor — sendto() returns EHOSTUNREACH — because NUD marks it
    unreachable. Zeroing arp_llreach_base makes arp_llreach_reachable() always
    return true (NUD disabled), and host_down_time=0 removes the 20s host-down
    penalty. These are global sysctls; harmless on a dedicated netboot host.
    """
    for key, val in (
        ("net.link.ether.inet.arp_llreach_base", "0"),
        ("net.link.ether.inet.host_down_time", "0"),
    ):
        subprocess.run(["sudo", "sysctl", "-w", f"{key}={val}"], capture_output=True, text=True)


def _cleanup_stale(target: str) -> None:
    """Kill any lingering pids from a previous crashed netboot session."""
    state = load_netboot_state(target)
    if state is None:
        return
    for pid in (state.dhcp_pid, state.tftp_pid):
        if is_pid_alive(pid):
            try:
                os.kill(pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
    netboot_state_path(target).unlink(missing_ok=True)


def start(cfg: TargetConfig) -> None:
    if is_netboot_running(cfg.name):
        raise RuntimeError(f"netboot already running for '{cfg.name}'")

    missing = check_deps()
    if missing:
        raise RuntimeError(
            f"Missing required tools: {', '.join(missing)}\n"
            "Run: paniolo setup"
        )

    if not cfg.tftp_root:
        raise RuntimeError("No tftp_root configured. Run: paniolo target set <name> --tftp-root <path>")
    tftp_root = Path(cfg.tftp_root)
    if not tftp_root.exists():
        raise RuntimeError(f"TFTP root does not exist: {tftp_root}")

    _cleanup_stale(cfg.name)
    _configure_interface(cfg.interface, cfg.host_ip)
    _tune_arp_for_silent_client()

    ensure_target_dir(cfg.name)
    log_path = netboot_log_path(cfg.name)

    dhcp = _spawn(
        [sys.executable, "-m", "paniolo._dhcp", cfg.host_ip, "--interface", cfg.interface],
        log_path,
    )
    # Pure-Python TFTP server. It listens on the wildcard (rootless even on
    # privileged port 69) but binds each reply socket to cfg.host_ip so macOS
    # routes transfers out the correct secondary interface. See _tftp.py for
    # the full rationale (off-the-shelf servers bound to 0.0.0.0 hit
    # EHOSTUNREACH on the reply because egress selection picks the wrong NIC).
    tftp = _spawn(
        [sys.executable, "-m", "paniolo._tftp", cfg.host_ip, str(tftp_root),
         "--interface", cfg.interface],
        log_path,
        append=True,
    )

    save_netboot_state(NetbootState(
        target=cfg.name,
        dhcp_pid=dhcp.pid,
        tftp_pid=tftp.pid,
        started_at=time.time(),
        interface=cfg.interface,
        tftp_root=str(tftp_root),
    ))


def stop(target: str) -> None:
    state = load_netboot_state(target)
    if state is None:
        raise RuntimeError(f"No netboot state for '{target}'")

    for pid in (state.dhcp_pid, state.tftp_pid):
        if is_pid_alive(pid):
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            except PermissionError:
                subprocess.run(["sudo", "kill", "-TERM", str(pid)], check=False)

    deadline = time.time() + 3.0
    while time.time() < deadline:
        if not is_pid_alive(state.dhcp_pid) and not is_pid_alive(state.tftp_pid):
            break
        time.sleep(0.1)

    netboot_state_path(target).unlink(missing_ok=True)
    _restore_interface(state.interface)


def get_status(target: str) -> dict:
    state = load_netboot_state(target)
    if state is None:
        return {"running": False, "target": target}

    dhcp_alive = is_pid_alive(state.dhcp_pid)
    tftp_alive = is_pid_alive(state.tftp_pid)

    return {
        "running": dhcp_alive and tftp_alive,
        "target": target,
        "dhcp_pid": state.dhcp_pid,
        "dhcp_alive": dhcp_alive,
        "tftp_pid": state.tftp_pid,
        "tftp_alive": tftp_alive,
        "interface": state.interface,
        "tftp_root": state.tftp_root,
        "started_at": state.started_at,
        "uptime_seconds": time.time() - state.started_at if (dhcp_alive and tftp_alive) else None,
    }
