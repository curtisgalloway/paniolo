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

"""Minimal read-only TFTP server for paniolo netboot.

Read-only (RRQ) TFTP per RFC 1350, with the blksize (RFC 2348) and tsize
(RFC 2349) options the Raspberry Pi bootloader negotiates.

Why a custom server instead of an off-the-shelf one (e.g. tftp-now): on macOS
a non-root process can bind a privileged port (69) only on the wildcard
address 0.0.0.0, NOT on a specific interface IP. But the host we serve sits on
a *secondary* USB-Ethernet interface, and a reply socket left on 0.0.0.0 lets
macOS pick the wrong egress (the primary interface) -> sendto() fails with
EHOSTUNREACH ("no route to host"). The first fix is to listen on the wildcard
while binding each reply socket to the specific interface IP on an ephemeral
port, pinning egress to the right NIC.

However, on macOS 15+ (Sequoia / "macOS 26") the kernel refuses to unicast to
the Pi bootloader even with a permanent static ARP entry, because the bootloader
sends TFTP packets from a different ephemeral source MAC than the one it used for
DHCP. The kernel learns that ephemeral MAC as the host route for the client IP,
then won't deliver frames to it (the bootloader only receives on its real MAC).
The second fix is a BPF raw-frame sender: when sendto returns EHOSTUNREACH we
write a complete Ethernet/IPv4/UDP frame directly to /dev/bpf, using the
bootloader's real DHCP MAC (shared via /tmp/paniolo-client-mac by _dhcp.py) as
the destination, bypassing the kernel's ARP table entirely.

Usage (as subprocess):
    python -m paniolo._tftp <host_ip> <root> [--port 69] [--interface <iface>]
"""

from __future__ import annotations

import argparse
import errno
import fcntl
import logging
import os
import re
import socket
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path

_OP_RRQ = 1
_OP_WRQ = 2
_OP_DATA = 3
_OP_ACK = 4
_OP_ERROR = 5
_OP_OACK = 6

_ERR_NOT_FOUND = 1
_ERR_ACCESS = 2
_ERR_ILLEGAL = 4

_DEFAULT_BLKSIZE = 512
_ACK_TIMEOUT = 1.0
_MAX_RETRIES = 6
_ARP_RESOLVE_TIMEOUT = 4.0

# Shared with _dhcp.py: the DHCP server writes the client MAC here so we can
# use it as the BPF frame destination, bypassing the kernel's ARP table.
# Placed in the user state dir (not /tmp) to prevent symlink and spoofing attacks.
_CLIENT_MAC_FILE = Path.home() / ".local" / "share" / "paniolo" / "client-mac"

# macOS BPF ioctl constants (64-bit).  Used by BpfSender below.
_BIOCSETIF = 0x8020426C  # bind BPF fd to an interface (struct ifreq, 32 B)
_BIOCSHDRCMPLT = 0x80044275  # tell kernel we write complete L2 headers

log = logging.getLogger(__name__)


# ── BPF raw-frame sender ──────────────────────────────────────────────────────


def _inet_checksum(data: bytes) -> int:
    if len(data) % 2:
        data += b"\x00"
    total = sum(struct.unpack(f"!{len(data) // 2}H", data))
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return ~total & 0xFFFF


def _build_udp_frame(
    src_mac: bytes,
    dst_mac: bytes,
    src_ip: str,
    dst_ip: str,
    src_port: int,
    dst_port: int,
    payload: bytes,
) -> bytes:
    """Construct a raw Ethernet/IPv4/UDP frame."""
    src_a = socket.inet_aton(src_ip)
    dst_a = socket.inet_aton(dst_ip)
    udp_len = 8 + len(payload)
    ip_len = 20 + udp_len

    ip_hdr = struct.pack(
        "!BBHHHBBH4s4s",
        0x45,
        0,
        ip_len,
        0,
        0x4000,
        64,
        17,
        0,
        src_a,
        dst_a,
    )
    ip_ck = _inet_checksum(ip_hdr)
    ip_hdr = ip_hdr[:10] + struct.pack("!H", ip_ck) + ip_hdr[12:]

    udp_hdr_no_ck = struct.pack("!HHH", src_port, dst_port, udp_len)
    pseudo = src_a + dst_a + b"\x00\x11" + struct.pack("!H", udp_len)
    udp_ck = _inet_checksum(pseudo + udp_hdr_no_ck + b"\x00\x00" + payload)
    udp_hdr = udp_hdr_no_ck + struct.pack("!H", udp_ck)

    return dst_mac + src_mac + b"\x08\x00" + ip_hdr + udp_hdr + payload


def _get_if_mac(iface: str) -> bytes:
    if sys.platform != "darwin":
        # Linux: read directly from sysfs (no ifconfig needed).
        addr = (
            Path(f"/sys/class/net/{iface}/address").read_text(encoding="utf-8").strip()
        )
        return bytes(int(b, 16) for b in addr.split(":"))
    out = subprocess.check_output(
        ["ifconfig", iface], text=True, stderr=subprocess.DEVNULL
    )
    m = re.search(r"\bether\s+((?:[0-9a-f]{2}:){5}[0-9a-f]{2})\b", out)
    if not m:
        raise ValueError(f"No ether address for {iface}")
    return bytes(int(b, 16) for b in m.group(1).split(":"))


def _open_bpf_fd(iface: str) -> int | None:
    """Open a writable BPF device bound to iface. macOS only; returns None elsewhere."""
    if sys.platform != "darwin":
        return None
    for n in range(10):
        try:
            fd = os.open(f"/dev/bpf{n}", os.O_RDWR)
        except OSError:
            continue
        try:
            ifreq = bytearray(32)
            ifreq[: len(iface)] = iface.encode()
            fcntl.ioctl(fd, _BIOCSETIF, ifreq)
            fcntl.ioctl(fd, _BIOCSHDRCMPLT, struct.pack("I", 1))
            return fd
        except OSError as exc:
            os.close(fd)
            log.debug("BPF /dev/bpf%d bind %s: %s", n, iface, exc)
    return None


class BpfSender:
    """Sends UDP packets as raw Ethernet frames via /dev/bpf, bypassing the
    kernel ARP table.  Used on macOS when sendto returns EHOSTUNREACH because
    the kernel has installed the wrong destination MAC for the Pi bootloader.
    On Linux, BPF is not available; `available` is always False."""

    def __init__(self, iface: str, host_ip: str) -> None:
        self._host_ip = host_ip
        self._fd: int | None = None
        self._src_mac: bytes | None = None
        self._lock = threading.Lock()
        if sys.platform != "darwin":
            return
        try:
            self._src_mac = _get_if_mac(iface)
            self._fd = _open_bpf_fd(iface)
            if self._fd is not None:
                log.info(
                    "BPF sender ready on %s (src %s)",
                    iface,
                    self._src_mac.hex(":"),
                )
            else:
                log.warning(
                    "BPF unavailable on %s — check /dev/bpf* permissions or "
                    "add user to 'access_bpf' group",
                    iface,
                )
        except Exception as exc:  # pylint: disable=broad-exception-caught
            log.warning("BPF init failed: %s", exc)

    @property
    def available(self) -> bool:
        return self._fd is not None and self._src_mac is not None

    def _read_client_mac(self) -> bytes | None:
        try:
            mac_str = _CLIENT_MAC_FILE.read_text(encoding="utf-8").strip()
            return bytes(int(b, 16) for b in mac_str.split(":"))
        except Exception:  # pylint: disable=broad-exception-caught
            return None

    def send(self, sock: socket.socket, packet: bytes, peer: tuple) -> bool:
        """Send packet as a raw frame. sock supplies the ephemeral src port."""
        if not self.available:
            return False
        dst_mac = self._read_client_mac()
        if dst_mac is None:
            log.warning("BPF: no client MAC in %s", _CLIENT_MAC_FILE)
            return False
        src_port = sock.getsockname()[1]
        dst_ip, dst_port = peer
        try:
            frame = _build_udp_frame(
                self._src_mac,
                dst_mac,  # type: ignore[arg-type]
                self._host_ip,
                dst_ip,
                src_port,
                dst_port,
                packet,
            )
            with self._lock:
                os.write(self._fd, frame)  # type: ignore[arg-type]
            log.debug(
                "BPF sent %d B to %s:%d (dst MAC %s)",
                len(frame),
                dst_ip,
                dst_port,
                dst_mac.hex(":"),
            )
            return True
        except OSError as exc:
            log.warning("BPF write failed: %s", exc)
            return False

    def close(self) -> None:
        with self._lock:
            if self._fd is not None:
                os.close(self._fd)
                self._fd = None


def _sendto(
    sock: socket.socket, packet: bytes, peer, bpf: "BpfSender | None" = None
) -> bool:
    """sendto() with BPF raw-frame fallback for EHOSTUNREACH.

    macOS 15+ refuses to deliver unicast UDP to the Pi bootloader even with a
    permanent static ARP entry, because the bootloader's TFTP packets arrive
    from a random source MAC (different from its DHCP MAC), and the kernel
    installs that ephemeral MAC as the host route.  When sendto hits
    EHOSTUNREACH we fall back to a /dev/bpf raw frame addressed to the real
    DHCP MAC (written by _dhcp.py to _CLIENT_MAC_FILE).  If BPF is not
    available, retry for _ARP_RESOLVE_TIMEOUT seconds in case the ARP entry
    heals on its own (covers older macOS and the brief post-link-flap window).
    """
    if bpf is not None and bpf.available:
        # With arp_llreach_base=0 (NUD disabled), sendto() to the Pi "succeeds"
        # even when the kernel's ARP table has the wrong ephemeral source MAC the
        # bootloader used for TFTP (not its DHCP/receive MAC).  The packet is sent
        # but the Pi never receives it.  Always use BPF when available so we bypass
        # the ARP table entirely and address frames to the real DHCP MAC directly.
        if bpf.send(sock, packet, peer):
            return True
        log.debug("BPF failed, falling back to kernel sendto %s:%d", peer[0], peer[1])

    deadline = time.monotonic() + _ARP_RESOLVE_TIMEOUT
    while True:
        try:
            sock.sendto(packet, peer)
            return True
        except OSError as exc:
            if exc.errno == errno.EHOSTUNREACH and time.monotonic() < deadline:
                time.sleep(0.1)
                continue
            log.warning("sendto %s:%d failed: %s", peer[0], peer[1], exc)
            return False


def _parse_rrq(data: bytes) -> tuple[str, str, dict[str, str]] | None:
    """Return (filename, mode, options) from an RRQ payload, or None if malformed."""
    parts = data[2:].split(b"\x00")
    if len(parts) < 2:
        return None
    filename = parts[0].decode("latin-1")
    mode = parts[1].decode("latin-1").lower()
    options: dict[str, str] = {}
    rest = parts[2:]
    for i in range(0, len(rest) - 1, 2):
        key = rest[i].decode("latin-1").lower()
        if key:
            options[key] = rest[i + 1].decode("latin-1")
    return filename, mode, options


def _error_packet(code: int, msg: str) -> bytes:
    return struct.pack("!HH", _OP_ERROR, code) + msg.encode("latin-1") + b"\x00"


def _resolve(root: Path, filename: str) -> Path | None:
    """Resolve a requested filename inside root, rejecting traversal outside it."""
    candidate = (root / filename.lstrip("/")).resolve()
    try:
        candidate.relative_to(root.resolve())
    except ValueError:
        return None
    return candidate


def _send_and_wait_ack(
    sock: socket.socket,
    packet: bytes,
    peer,
    expect_block: int,
    bpf: "BpfSender | None" = None,
) -> bool:
    """Send a packet and wait for ACK of expect_block, retransmitting on timeout."""
    for attempt in range(_MAX_RETRIES):
        if not _sendto(sock, packet, peer, bpf):
            log.warning(
                "sendto %s:%d failed (attempt %d/%d), retrying",
                peer[0],
                peer[1],
                attempt + 1,
                _MAX_RETRIES,
            )
            time.sleep(0.05)
            continue
        sock.settimeout(_ACK_TIMEOUT)
        try:
            while True:
                resp, raddr = sock.recvfrom(4)
                if raddr != peer:
                    continue
                if len(resp) < 4:
                    continue
                opcode, block = struct.unpack("!HH", resp[:4])
                if opcode == _OP_ACK and block == expect_block:
                    return True
                if opcode == _OP_ERROR:
                    log.warning(
                        "ERROR from %s:%d (code=%d) waiting for ACK of block %d",
                        peer[0],
                        peer[1],
                        block,
                        expect_block,
                    )
                    return False
        except socket.timeout:
            continue
    return False


def _handle_rrq(
    host_ip: str,
    root: Path,
    data: bytes,
    peer,
    bpf: "BpfSender | None" = None,
) -> None:
    try:
        _do_rrq(host_ip, root, data, peer, bpf)
    except Exception:  # pylint: disable=broad-exception-caught  # noqa: BLE001
        log.exception("RRQ handler from %s:%d crashed", peer[0], peer[1])


def _bind_reply_socket(host_ip: str) -> socket.socket | None:
    """Create a reply socket bound to host_ip:ephemeral. Retries briefly because
    the interface IP may be momentarily absent while a link flap is being
    repaired (bind would otherwise fail with EADDRNOTAVAIL)."""
    deadline = time.monotonic() + _ARP_RESOLVE_TIMEOUT
    while True:
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            sock.bind((host_ip, 0))
            return sock
        except OSError as exc:
            sock.close()
            if exc.errno == errno.EADDRNOTAVAIL and time.monotonic() < deadline:
                time.sleep(0.1)
                continue
            log.warning("cannot bind reply socket to %s: %s", host_ip, exc)
            return None


def _do_rrq(
    host_ip: str,
    root: Path,
    data: bytes,
    peer,
    bpf: "BpfSender | None" = None,
) -> None:
    parsed = _parse_rrq(data)
    # Bind the reply socket to the specific interface IP (ephemeral port) so
    # macOS routes the transfer out the correct (secondary) interface.
    xfer = _bind_reply_socket(host_ip)
    if xfer is None:
        return
    try:
        if parsed is None:
            _sendto(xfer, _error_packet(_ERR_ILLEGAL, "malformed request"), peer, bpf)
            return
        filename, mode, options = parsed
        if mode != "octet":
            _sendto(
                xfer, _error_packet(_ERR_ILLEGAL, f"unsupported mode {mode}"), peer, bpf
            )
            return

        path = _resolve(root, filename)
        if path is None or not path.is_file():
            log.info("RRQ %s from %s:%d -> NOT FOUND", filename, peer[0], peer[1])
            _sendto(xfer, _error_packet(_ERR_NOT_FOUND, "file not found"), peer, bpf)
            return

        size = path.stat().st_size
        blksize = _DEFAULT_BLKSIZE
        oack_opts: dict[str, str] = {}
        if "blksize" in options:
            try:
                req = int(options["blksize"])
                blksize = max(8, min(req, 65464))
                oack_opts["blksize"] = str(blksize)
            except ValueError:
                pass
        if "tsize" in options:
            oack_opts["tsize"] = str(size)

        log.info(
            "RRQ %s from %s:%d -> serving %d bytes (blksize=%d)",
            filename,
            peer[0],
            peer[1],
            size,
            blksize,
        )

        if oack_opts:
            payload = struct.pack("!H", _OP_OACK)
            for k, v in oack_opts.items():
                payload += k.encode("latin-1") + b"\x00" + v.encode("latin-1") + b"\x00"
            if not _send_and_wait_ack(xfer, payload, peer, 0, bpf):
                log.warning("no ACK for OACK from %s:%d", peer[0], peer[1])
                return

        with path.open("rb") as f:
            block = 1
            while True:
                chunk = f.read(blksize)
                packet = struct.pack("!HH", _OP_DATA, block & 0xFFFF) + chunk
                if not _send_and_wait_ack(xfer, packet, peer, block & 0xFFFF, bpf):
                    log.warning(
                        "transfer of %s to %s:%d failed at block %d",
                        filename,
                        peer[0],
                        peer[1],
                        block,
                    )
                    return
                block += 1
                if len(chunk) < blksize:
                    break
        log.info("completed %s to %s:%d", filename, peer[0], peer[1])
    finally:
        xfer.close()


def serve(
    host_ip: str, root: str, port: int = 69, interface: str | None = None
) -> None:
    root_path = Path(root).resolve()

    bpf: BpfSender | None = None
    if interface is not None:
        bpf = BpfSender(interface, host_ip)

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
    # Wildcard bind on 0.0.0.0: rootless on macOS 14+ for privileged ports.
    # On Linux, port 69 requires root or CAP_NET_BIND_SERVICE.
    try:
        sock.bind(("", port))
    except PermissionError:
        log.error(
            "Cannot bind to port %d (TFTP). On Linux, run paniolo as root or "
            "grant CAP_NET_BIND_SERVICE.",
            port,
        )
        raise
    log.info(
        "TFTP listening on 0.0.0.0:%d  reply_src=%s  root=%s  bpf=%s",
        port,
        host_ip,
        root_path,
        "yes" if (bpf and bpf.available) else "no",
    )

    while True:
        try:
            data, peer = sock.recvfrom(4096)
        except OSError as exc:
            log.error("recvfrom: %s", exc)
            continue
        if len(data) < 2:
            continue
        opcode = struct.unpack("!H", data[:2])[0]
        if opcode == _OP_RRQ:
            t = threading.Thread(
                target=_handle_rrq,
                args=(host_ip, root_path, data, peer, bpf),
                daemon=True,
            )
            t.start()
        elif opcode == _OP_WRQ:
            err_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            try:
                err_sock.bind((host_ip, 0))
                _sendto(
                    err_sock, _error_packet(_ERR_ACCESS, "read-only server"), peer, bpf
                )
            except OSError as exc:
                log.debug("WRQ error reply failed: %s", exc)
            finally:
                err_sock.close()


def main() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
        stream=sys.stderr,
    )
    parser = argparse.ArgumentParser(
        description="Paniolo minimal read-only TFTP server"
    )
    parser.add_argument("host_ip", help="Interface IP to bind reply sockets to")
    parser.add_argument("root", help="TFTP root directory")
    parser.add_argument("--port", type=int, default=69)
    parser.add_argument(
        "--interface", help="Interface name for BPF raw-frame fallback (e.g. en14)"
    )
    args = parser.parse_args()
    serve(args.host_ip, args.root, args.port, args.interface)


if __name__ == "__main__":
    main()
