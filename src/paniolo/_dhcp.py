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

"""Minimal DHCP server for paniolo netboot.

Sends broadcast DHCP responses (no BPF required, no root required on macOS 14+).
Handles DISCOVER→OFFER and REQUEST→ACK for a single netboot client.

Usage (as subprocess):
    python -m paniolo._dhcp <host_ip> [--boot-file <filename>]

host_ip is both the interface address and the TFTP/siaddr advertised to clients.
"""

from __future__ import annotations

import argparse
import logging
import socket
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path

_BOOTREQUEST = 1
_BOOTREPLY = 2
_HTYPE_ETHERNET = 1
_MAGIC = b"\x63\x82\x53\x63"

_OPT_SUBNET = 1
_OPT_ROUTER = 3
_OPT_LEASE = 51
_OPT_MSG_TYPE = 53
_OPT_SERVER_ID = 54
_OPT_TFTP_SERVER = 66
_OPT_BOOTFILE = 67
_OPT_END = 255

_DHCP_DISCOVER = 1
_DHCP_OFFER = 2
_DHCP_REQUEST = 3
_DHCP_ACK = 5
_DHCP_NAK = 6

_LEASE_SECONDS = 12 * 3600
_ASSIGNED_IP = "192.168.99.100"

# Shared file written by this DHCP server, read by the co-process TFTP server.
# The TFTP server needs the client's real MAC to build BPF raw frames (the Pi
# bootloader sends TFTP from a different ephemeral MAC than the one it used for
# DHCP, which causes macOS to install the wrong ARP entry — see _tftp.py).
_CLIENT_MAC_FILE = Path("/tmp/paniolo-client-mac")

log = logging.getLogger(__name__)


def _parse_options(options: bytes) -> dict[int, bytes]:
    result: dict[int, bytes] = {}
    if options[:4] != _MAGIC:
        return result
    i = 4
    while i < len(options):
        tag = options[i]
        if tag == _OPT_END:
            break
        if tag == 0:
            i += 1
            continue
        if i + 1 >= len(options):
            break
        length = options[i + 1]
        result[tag] = options[i + 2 : i + 2 + length]
        i += 2 + length
    return result


def _encode_option(tag: int, value: bytes) -> bytes:
    return bytes([tag, len(value)]) + value


def _build_reply(
    xid: bytes,
    chaddr: bytes,
    msg_type: int,
    server_ip: str,
    assigned_ip: str,
    boot_file: str,
) -> bytes:
    server_b = socket.inet_aton(server_ip)
    client_b = socket.inet_aton(assigned_ip)

    opts = _MAGIC
    opts += _encode_option(_OPT_MSG_TYPE, bytes([msg_type]))
    opts += _encode_option(_OPT_SERVER_ID, server_b)
    opts += _encode_option(_OPT_LEASE, struct.pack("!I", _LEASE_SECONDS))
    opts += _encode_option(_OPT_SUBNET, socket.inet_aton("255.255.255.0"))
    opts += _encode_option(_OPT_ROUTER, server_b)
    opts += _encode_option(_OPT_TFTP_SERVER, server_ip.encode())
    opts += _encode_option(_OPT_BOOTFILE, boot_file.encode())
    opts += bytes([_OPT_END])

    pkt = struct.pack("!BBBB", _BOOTREPLY, _HTYPE_ETHERNET, 6, 0)
    pkt += xid
    pkt += struct.pack("!HH", 0, 0x8000)
    pkt += b"\x00" * 4  # ciaddr
    pkt += client_b  # yiaddr
    pkt += server_b  # siaddr (next-server = TFTP)
    pkt += b"\x00" * 4  # giaddr
    pkt += chaddr[:16]  # chaddr (padded to 16)
    pkt += b"\x00" * 64  # sname
    file_bytes = boot_file.encode()[:127]
    pkt += file_bytes + b"\x00" * (128 - len(file_bytes))  # file (null-padded)
    pkt += opts
    return pkt


def _set_arp(ip: str, mac: str) -> None:
    """Pin a static ARP entry mapping the client IP to the MAC we just saw in a
    DHCP packet.

    The Pi's netboot firmware sends us DHCP/TFTP but does NOT answer ARP
    requests, so macOS cannot resolve its MAC dynamically and every unicast
    reply fails with EHOSTUNREACH ("no route to host"). We already know the MAC
    from the DHCP frame, so install it directly. `arp -s` replaces any existing
    entry, so calling this on each DHCP exchange tracks the active MAC (the Pi
    cycles through several across boot phases). Needs root, like the interface
    IP assignment; both are unavoidable OS-level network config on macOS.
    """
    r = subprocess.run(
        ["sudo", "arp", "-s", ip, mac], capture_output=True, text=True
    )
    if r.returncode != 0:
        log.warning("arp -s %s %s failed: %s", ip, mac, r.stderr.strip() or r.stdout.strip())
    # Share with the co-process TFTP server so it can build BPF raw frames
    # addressed to the Pi's real MAC (which the kernel may have overridden with a
    # per-packet ephemeral MAC the bootloader only uses for sending, not receiving).
    try:
        _CLIENT_MAC_FILE.write_text(mac)
    except OSError as exc:
        log.debug("could not write client MAC file: %s", exc)


def _monitor_interface(interface: str, host_ip: str) -> None:
    """Continuously enforce the static IP on the interface.

    macOS drops a manually-set IPv4 every time the USB-Ethernet link flaps —
    and the netboot client flaps the link on every power-cycle and at several
    points during its own boot. While the interface has no IP there is no route
    to the client, so DHCP/TFTP replies fail with EHOSTUNREACH ("no route to
    host"). We poll fast and re-apply the IP immediately so the window where
    the client can't reach us is at most a couple hundred milliseconds — short
    enough that the client's next retry lands.
    """
    had_ip = True
    while True:
        time.sleep(0.25)
        try:
            out = subprocess.check_output(
                ["ifconfig", interface], text=True, stderr=subprocess.DEVNULL
            )
        except subprocess.CalledProcessError:
            out = ""
        has_ip = f"inet {host_ip} " in out
        is_active = "status: active" in out

        if not has_ip and is_active:
            # Re-apply every poll until it sticks (idempotent); a single attempt
            # can lose a race with macOS still tearing the config down.
            subprocess.run(
                ["sudo", "ifconfig", interface, host_ip, "netmask", "255.255.255.0", "up"],
                check=False,
            )
            if had_ip:
                log.warning("interface %s lost IP %s — restoring", interface, host_ip)
        elif has_ip and not had_ip:
            log.info("interface %s restored with IP %s", interface, host_ip)

        had_ip = has_ip


def serve(
    host_ip: str, boot_file: str = "kernel_2712.img", interface: str | None = None
) -> None:
    prefix = host_ip.rsplit(".", 1)[0]
    bcast = f"{prefix}.255"

    if interface:
        t = threading.Thread(
            target=_monitor_interface, args=(interface, host_ip), daemon=True
        )
        t.start()

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    sock.bind(("", 67))
    log.info(
        "DHCP listening on 0.0.0.0:67  host_ip=%s  bcast=%s  boot_file=%s",
        host_ip,
        bcast,
        boot_file,
    )

    while True:
        try:
            data, _addr = sock.recvfrom(4096)
        except OSError as exc:
            log.error("recvfrom: %s", exc)
            continue

        if len(data) < 240:
            continue
        op = data[0]
        if op != _BOOTREQUEST:
            continue

        xid = data[4:8]
        chaddr = data[28:44]
        mac = data[28:34].hex(":")
        options = _parse_options(data[236:])

        msg_type = options.get(_OPT_MSG_TYPE, b"")
        if not msg_type:
            continue
        msg_type_val = msg_type[0]

        if msg_type_val == _DHCP_DISCOVER:
            log.info("DHCPDISCOVER from %s", mac)
            _set_arp(_ASSIGNED_IP, mac)
            reply = _build_reply(xid, chaddr, _DHCP_OFFER, host_ip, _ASSIGNED_IP, boot_file)
            sock.sendto(reply, (bcast, 68))
            log.info(
                "DHCPOFFER → %s  ip=%s  tftp=%s  file=%s",
                mac,
                _ASSIGNED_IP,
                host_ip,
                boot_file,
            )

        elif msg_type_val == _DHCP_REQUEST:
            log.info("DHCPREQUEST from %s", mac)
            _set_arp(_ASSIGNED_IP, mac)
            reply = _build_reply(xid, chaddr, _DHCP_ACK, host_ip, _ASSIGNED_IP, boot_file)
            sock.sendto(reply, (bcast, 68))
            log.info("DHCPACK → %s  ip=%s", mac, _ASSIGNED_IP)


def main() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
        stream=sys.stderr,
    )
    parser = argparse.ArgumentParser(description="Paniolo minimal DHCP server")
    parser.add_argument("host_ip", help="Interface IP (also advertised as TFTP server)")
    parser.add_argument("--boot-file", default="kernel_2712.img")
    parser.add_argument("--interface", help="Interface device name (e.g. en11) for IP monitoring")
    args = parser.parse_args()
    serve(args.host_ip, args.boot_file, args.interface)


if __name__ == "__main__":
    main()
