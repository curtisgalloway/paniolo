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
    is_paniolo_child_alive,
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

# Linux: dnsmasq and other netboot tools commonly live in /usr/sbin or /sbin.
_LINUX_SBIN_PATHS = ["/usr/sbin", "/sbin"]

_EXCLUDE_PORT_PREFIXES = (
    "Wi-Fi",
    "Thunderbolt",
    "Bluetooth",
    "FireWire",
    "iPhone",
    "iPad",
)
_EXCLUDE_DEVICES = {"bridge0", "lo0"}

# Linux interfaces to skip when listing candidates for netboot.
_LINUX_SKIP_PREFIXES = ("lo", "docker", "veth", "br", "virbr", "vlan", "bond", "dummy")


def _find_bin(name: str) -> str:
    found = shutil.which(name)
    if found:
        return found
    extra = _LINUX_SBIN_PATHS if sys.platform != "darwin" else _BREW_PATHS
    for d in extra:
        p = Path(d) / name
        if p.exists():
            return str(p)
    return name


def check_deps() -> list[str]:
    # DHCP and TFTP are both pure-Python (see _dhcp.py, _tftp.py); no external
    # binaries required.
    return []


def _is_interface_active(device: str) -> bool:
    if sys.platform == "darwin":
        try:
            out = subprocess.check_output(
                ["ifconfig", device], text=True, stderr=subprocess.DEVNULL
            )
            return "status: active" in out
        except (subprocess.CalledProcessError, FileNotFoundError):
            return False
    else:
        try:
            carrier = Path(f"/sys/class/net/{device}/carrier").read_text().strip()
            return carrier == "1"
        except OSError:
            return False


def _list_linux_ethernet_interfaces() -> list[dict]:
    """Return Ethernet interfaces on Linux using sysfs.

    Each entry: {"port": str, "device": str, "active": bool}
    Skips loopback, virtual bridges, Docker, and other non-physical interfaces.
    """
    net_dir = Path("/sys/class/net")
    candidates: list[dict] = []
    try:
        entries = sorted(net_dir.iterdir())
    except OSError:
        return []
    for iface_path in entries:
        name = iface_path.name
        if any(name.startswith(p) for p in _LINUX_SKIP_PREFIXES):
            continue
        # Type 1 = Ethernet (ARPHRD_ETHER).
        try:
            if (iface_path / "type").read_text().strip() != "1":
                continue
        except OSError:
            continue
        active = _is_interface_active(name)
        candidates.append({"port": name, "device": name, "active": active})
    return sorted(candidates, key=lambda x: (not x["active"], x["device"]))


def list_usb_ethernet_interfaces() -> list[dict]:
    """Return external (non-built-in) Ethernet interfaces, active ones first.

    Each entry: {"port": str, "device": str, "active": bool}
    On macOS: queries networksetup and excludes Wi-Fi, Thunderbolt, Bluetooth, etc.
    On Linux: reads sysfs and excludes loopback and virtual interfaces.
    """
    if sys.platform != "darwin":
        return _list_linux_ethernet_interfaces()

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
    try:
        proc = subprocess.Popen(
            cmd,
            stdout=log_file,
            stderr=log_file,
            stdin=subprocess.DEVNULL,
            start_new_session=True,
            env=env,
        )
    finally:
        log_file.close()
    return proc


def _sudo_prefix() -> list[str]:
    """Return a sudo prefix for privileged subprocesses on Linux.

    On macOS, DHCP/TFTP bind to ports 67/69 without root; no prefix needed.
    On Linux they require root (or CAP_NET_BIND_SERVICE). If we're already
    running as root, no prefix needed either.

    Uses 'sudo env PYTHONUNBUFFERED=1' so the env var reaches Python through
    sudo's environment reset without requiring the SETENV sudoers option.
    Each exec in the chain (sudo → env → python) keeps the same PID, so the
    saved PID in the state file still refers to the Python process.
    """
    if sys.platform == "darwin" or os.getuid() == 0:
        return []
    return ["sudo", "env", "PYTHONUNBUFFERED=1"]


def _find_network_service(interface: str) -> str | None:
    """Return the networksetup service name for a given device (e.g. 'en11' → 'USB 10/100/1000 LAN').
    macOS only; returns None on Linux."""
    if sys.platform != "darwin":
        return None
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
    if sys.platform == "darwin":
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
    else:
        # Remove any existing addresses on this interface, then assign ours.
        subprocess.run(
            ["sudo", "ip", "addr", "flush", "dev", interface],
            capture_output=True,
            text=True,
            check=False,
        )
        result = subprocess.run(
            ["sudo", "ip", "addr", "add", f"{host_ip}/24", "dev", interface],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0 and "already assigned" not in result.stderr:
            raise RuntimeError(
                f"ip addr add {host_ip}/24 dev {interface} failed: {result.stderr.strip()}\n"
                "Ensure passwordless sudo is configured (NOPASSWD) for the control machine."
            )
        subprocess.run(
            ["sudo", "ip", "link", "set", interface, "up"],
            check=False,
        )


def _restore_interface(interface: str) -> None:
    """Release the static IP and return the interface to OS-managed networking."""
    if sys.platform == "darwin":
        service = _find_network_service(interface)
        if service:
            subprocess.run(
                ["sudo", "networksetup", "-setdhcp", service],
                check=False,
            )
    else:
        # Flush our static address; leave link up. A DHCP client (NetworkManager,
        # systemd-networkd, dhclient) will re-acquire an address if configured.
        subprocess.run(
            ["sudo", "ip", "addr", "flush", "dev", interface],
            check=False,
        )


def _tune_arp_for_silent_client() -> None:
    """Tweak OS neighbor-unreachability detection (NUD) for the netboot link.

    The Pi's bootloader sends us DHCP/TFTP but never answers ARP probes. Without
    tuning, the OS may mark the neighbor unreachable and refuse to send packets.

    macOS (26.x+): zeros arp_llreach_base and host_down_time so NUD never fires.
    Linux: no tuning needed — ARP entries installed via _dhcp._set_arp persist
    across link flaps and Linux's NUD does not block sends to permanent entries.
    """
    if sys.platform != "darwin":
        return
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
    for pid, module in (
        (state.dhcp_pid, "paniolo._dhcp"),
        (state.tftp_pid, "paniolo._tftp"),
    ):
        if is_paniolo_child_alive(pid, module):
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
    sudo = _sudo_prefix()

    dhcp = _spawn(
        sudo + [sys.executable, "-m", "paniolo._dhcp", cfg.host_ip, "--interface", cfg.interface],
        log_path,
    )
    # Pure-Python TFTP server. Binds the listen socket on the wildcard so a
    # non-root process can use port 69 on macOS; on Linux we prepend sudo
    # (see _sudo_prefix). Each reply socket is bound to cfg.host_ip so the
    # OS routes transfers out the correct secondary interface (see _tftp.py).
    tftp = _spawn(
        sudo + [sys.executable, "-m", "paniolo._tftp", cfg.host_ip, str(tftp_root),
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
