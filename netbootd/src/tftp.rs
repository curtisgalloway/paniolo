// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Minimal read-only TFTP server for paniolo netboot (port of `_tftp.py`).
//!
//! Read-only (RRQ) per RFC 1350, with the `blksize` (RFC 2348) and `tsize`
//! (RFC 2349) options the Raspberry Pi bootloader negotiates.
//!
//! Delivery model:
//!   * **Egress pinning** — each reply socket is tied to the netboot interface.
//!     On macOS that's `IP_BOUND_IF` (survives the brief link-flap windows where
//!     the interface IP is momentarily absent); elsewhere we bind the reply
//!     socket to the interface IP, the Python "first fix".
//!   * **Send path** — on macOS, once the DHCP handler has learned the client's
//!     MAC, every reply is injected as a raw Ethernet frame via [`BpfSender`]
//!     (we *always* prefer it when available: on Sequoia `send_to` reports
//!     success but silently misdelivers). ACKs are still received on the normal
//!     UDP reply socket. If BPF is unavailable we fall back to `send_to`.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::time::timeout;
use tracing::{info, warn};

use crate::bpf::BpfSender;

const OP_RRQ: u16 = 1;
const OP_WRQ: u16 = 2;
const OP_DATA: u16 = 3;
const OP_ACK: u16 = 4;
const OP_ERROR: u16 = 5;
const OP_OACK: u16 = 6;

const ERR_NOT_FOUND: u16 = 1;
const ERR_ACCESS: u16 = 2;
const ERR_ILLEGAL: u16 = 4;

const DEFAULT_BLKSIZE: usize = 512;
const ACK_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_RETRIES: usize = 6;

/// Per-transfer context shared by the send helpers.
#[derive(Clone)]
struct Xfer {
    host_ip: Ipv4Addr,
    bpf: Arc<BpfSender>,
    client_mac: Option<[u8; 6]>,
}

struct Rrq {
    filename: String,
    mode: String,
    blksize: Option<usize>,
    want_tsize: bool,
}

fn parse_rrq(data: &[u8]) -> Option<Rrq> {
    // Skip the 2-byte opcode; the rest is NUL-separated strings:
    // filename, mode, [opt, value]...
    let body = &data[2..];
    let mut parts = body.split(|&b| b == 0);
    let filename = String::from_utf8_lossy(parts.next()?).to_string();
    let mode = String::from_utf8_lossy(parts.next()?).to_lowercase();

    let mut blksize = None;
    let mut want_tsize = false;
    loop {
        let key = match parts.next() {
            Some(k) if !k.is_empty() => String::from_utf8_lossy(k).to_lowercase(),
            _ => break,
        };
        let Some(val) = parts.next() else { break };
        let val = String::from_utf8_lossy(val);
        match key.as_str() {
            "blksize" => {
                if let Ok(req) = val.parse::<usize>() {
                    blksize = Some(req.clamp(8, 65464));
                }
            }
            "tsize" => want_tsize = true,
            _ => {}
        }
    }
    Some(Rrq {
        filename,
        mode,
        blksize,
        want_tsize,
    })
}

fn error_packet(code: u16, msg: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(5 + msg.len());
    p.extend_from_slice(&OP_ERROR.to_be_bytes());
    p.extend_from_slice(&code.to_be_bytes());
    p.extend_from_slice(msg.as_bytes());
    p.push(0);
    p
}

/// Resolve a requested filename inside `root`, rejecting traversal outside it.
fn resolve(root: &Path, filename: &str) -> Option<PathBuf> {
    let rel = filename.trim_start_matches('/');
    let candidate = root.join(rel);
    let canon = candidate.canonicalize().ok()?;
    let root_canon = root.canonicalize().ok()?;
    canon.starts_with(&root_canon).then_some(canon)
}

/// macOS: pin a socket's traffic to `iface` via `IP_BOUND_IF`. This is the
/// documented analogue of Linux `SO_BINDTODEVICE` and, unlike binding to the
/// interface IP, keeps working when that IP momentarily disappears on a flap.
#[cfg(target_os = "macos")]
fn bind_socket_to_interface(sock: &Socket, iface: &str) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let cname = std::ffi::CString::new(iface)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "iface has NUL"))?;
    let idx = unsafe { libc::if_nametoindex(cname.as_ptr()) };
    if idx == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let idx: libc::c_uint = idx;
    let rc = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::IPPROTO_IP,
            libc::IP_BOUND_IF,
            &idx as *const libc::c_uint as *const libc::c_void,
            std::mem::size_of::<libc::c_uint>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Create a reply socket pinned to the netboot interface.
fn bind_reply_socket(host_ip: Ipv4Addr, interface: Option<&str>) -> Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;

    #[cfg(target_os = "macos")]
    {
        // Egress is pinned via IP_BOUND_IF, not the bind address, so host_ip is
        // unused here. Bind a wildcard ephemeral port so we do not depend on the
        // interface IP being present at this instant.
        let _ = host_ip;
        if let Some(iface) = interface {
            if let Err(e) = bind_socket_to_interface(&sock, iface) {
                warn!("IP_BOUND_IF {iface} failed: {e}");
            }
        }
        let addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
        sock.bind(&addr.into())
            .context("bind reply socket (wildcard ephemeral)")?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = interface;
        let addr: SocketAddr = SocketAddr::new(host_ip.into(), 0);
        sock.bind(&addr.into())
            .with_context(|| format!("bind reply socket to {host_ip}:0"))?;
    }

    sock.set_nonblocking(true)?;
    Ok(UdpSocket::from_std(sock.into())?)
}

/// Send one packet to `peer`, preferring the BPF raw-frame path on macOS when a
/// client MAC is known, otherwise ordinary `send_to`.
async fn send_pkt(sock: &UdpSocket, packet: &[u8], peer: SocketAddr, xfer: &Xfer) -> Result<()> {
    // Referenced on all platforms so the fields are never "unused" on Linux.
    let _ = (&xfer.bpf, xfer.client_mac, xfer.host_ip);

    #[cfg(target_os = "macos")]
    {
        if xfer.bpf.available() {
            if let (Some(dst_mac), SocketAddr::V4(p)) = (xfer.client_mac, peer) {
                let src_port = sock.local_addr()?.port();
                if xfer
                    .bpf
                    .send_udp(dst_mac, xfer.host_ip, *p.ip(), src_port, p.port(), packet)
                {
                    return Ok(());
                }
                // BPF write failed — fall through to the kernel path.
            }
        }
    }

    sock.send_to(packet, peer).await?;
    Ok(())
}

/// Send a packet and wait for an ACK of `expect_block`, retransmitting on
/// timeout. Returns Ok(true) on ACK, Ok(false) on give-up/peer error.
async fn send_and_wait_ack(
    sock: &UdpSocket,
    packet: &[u8],
    peer: SocketAddr,
    expect_block: u16,
    xfer: &Xfer,
) -> Result<bool> {
    let mut ackbuf = [0u8; 64];
    for _ in 0..MAX_RETRIES {
        send_pkt(sock, packet, peer, xfer).await?;
        loop {
            match timeout(ACK_TIMEOUT, sock.recv_from(&mut ackbuf)).await {
                Ok(Ok((n, raddr))) => {
                    if raddr != peer || n < 4 {
                        continue;
                    }
                    let opcode = u16::from_be_bytes([ackbuf[0], ackbuf[1]]);
                    let block = u16::from_be_bytes([ackbuf[2], ackbuf[3]]);
                    if opcode == OP_ACK && block == expect_block {
                        return Ok(true);
                    }
                    if opcode == OP_ERROR {
                        warn!("ERROR from {peer} waiting for ACK of block {expect_block}");
                        return Ok(false);
                    }
                    // stray packet; keep waiting within this attempt
                }
                Ok(Err(e)) => return Err(e.into()),
                Err(_) => break, // timeout → retransmit
            }
        }
    }
    Ok(false)
}

async fn handle_rrq(
    root: PathBuf,
    data: Vec<u8>,
    peer: SocketAddr,
    interface: Option<String>,
    xfer: Xfer,
) {
    let sock = match bind_reply_socket(xfer.host_ip, interface.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            warn!("{e:#}");
            return;
        }
    };
    let Some(rrq) = parse_rrq(&data) else {
        let _ = send_pkt(
            &sock,
            &error_packet(ERR_ILLEGAL, "malformed request"),
            peer,
            &xfer,
        )
        .await;
        return;
    };
    if rrq.mode != "octet" {
        let _ = send_pkt(
            &sock,
            &error_packet(ERR_ILLEGAL, "unsupported mode"),
            peer,
            &xfer,
        )
        .await;
        return;
    }

    let path = match resolve(&root, &rrq.filename) {
        Some(p) if p.is_file() => p,
        _ => {
            info!("RRQ {} from {peer} -> NOT FOUND", rrq.filename);
            let _ = send_pkt(
                &sock,
                &error_packet(ERR_NOT_FOUND, "file not found"),
                peer,
                &xfer,
            )
            .await;
            return;
        }
    };

    let contents = match tokio::fs::read(&path).await {
        Ok(c) => c,
        Err(e) => {
            warn!("read {}: {e}", path.display());
            let _ = send_pkt(
                &sock,
                &error_packet(ERR_NOT_FOUND, "read error"),
                peer,
                &xfer,
            )
            .await;
            return;
        }
    };
    let size = contents.len();
    let blksize = rrq.blksize.unwrap_or(DEFAULT_BLKSIZE);

    info!(
        "RRQ {} from {peer} -> serving {size} bytes (blksize={blksize})",
        rrq.filename
    );

    // OACK if the client requested any option we honor.
    if rrq.blksize.is_some() || rrq.want_tsize {
        let mut oack = Vec::new();
        oack.extend_from_slice(&OP_OACK.to_be_bytes());
        if rrq.blksize.is_some() {
            oack.extend_from_slice(b"blksize\0");
            oack.extend_from_slice(blksize.to_string().as_bytes());
            oack.push(0);
        }
        if rrq.want_tsize {
            oack.extend_from_slice(b"tsize\0");
            oack.extend_from_slice(size.to_string().as_bytes());
            oack.push(0);
        }
        match send_and_wait_ack(&sock, &oack, peer, 0, &xfer).await {
            Ok(true) => {}
            _ => {
                warn!("no ACK for OACK from {peer}");
                return;
            }
        }
    }

    // DATA/ACK loop. Block numbers wrap at 0xFFFF.
    let mut block: u16 = 1;
    let mut offset = 0usize;
    loop {
        let end = (offset + blksize).min(size);
        let chunk = &contents[offset..end];
        let mut packet = Vec::with_capacity(4 + chunk.len());
        packet.extend_from_slice(&OP_DATA.to_be_bytes());
        packet.extend_from_slice(&block.to_be_bytes());
        packet.extend_from_slice(chunk);

        match send_and_wait_ack(&sock, &packet, peer, block, &xfer).await {
            Ok(true) => {}
            _ => {
                warn!(
                    "transfer of {} to {peer} failed at block {block}",
                    rrq.filename
                );
                return;
            }
        }
        offset = end;
        block = block.wrapping_add(1);
        if chunk.len() < blksize {
            break; // last (possibly empty) block was ACKed
        }
    }
    info!("completed {} to {peer}", rrq.filename);
}

/// Bind the main TFTP listen socket on `0.0.0.0:port`.
fn bind_server(port: u16) -> Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();
    sock.bind(&addr.into()).with_context(|| {
        format!("bind TFTP port {port} (need root/CAP_NET_BIND_SERVICE on Linux)")
    })?;
    sock.set_nonblocking(true)?;
    Ok(UdpSocket::from_std(sock.into())?)
}

/// Run the TFTP server until the task is cancelled.
pub async fn serve(
    host_ip: Ipv4Addr,
    root: PathBuf,
    port: u16,
    interface: Option<String>,
    bpf: Arc<BpfSender>,
    mac_rx: watch::Receiver<Option<[u8; 6]>>,
) -> Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("TFTP root {} does not exist", root.display()))?;
    let sock = bind_server(port)?;
    info!(
        %host_ip,
        root = %root.display(),
        bpf = bpf.available(),
        "TFTP listening on 0.0.0.0:{port}"
    );

    let mut buf = vec![0u8; 4096];
    loop {
        let (n, peer) = match sock.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                warn!("TFTP recv_from: {e}");
                continue;
            }
        };
        if n < 2 {
            continue;
        }
        let opcode = u16::from_be_bytes([buf[0], buf[1]]);
        // Snapshot the client MAC learned by DHCP (stable for the transfer).
        let xfer = Xfer {
            host_ip,
            bpf: bpf.clone(),
            client_mac: *mac_rx.borrow(),
        };
        match opcode {
            OP_RRQ => {
                let data = buf[..n].to_vec();
                let root = root.clone();
                let interface = interface.clone();
                tokio::spawn(async move { handle_rrq(root, data, peer, interface, xfer).await });
            }
            OP_WRQ => {
                // Read-only server: reject writes.
                if let Ok(err_sock) = bind_reply_socket(host_ip, interface.as_deref()) {
                    let _ = send_pkt(
                        &err_sock,
                        &error_packet(ERR_ACCESS, "read-only server"),
                        peer,
                        &xfer,
                    )
                    .await;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A unique, freshly-created temp dir (mirrors the sibling crates' pattern —
    /// no `tempfile` dependency).
    fn tmp() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "netbootd-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// Build an RRQ payload: opcode + filename\0mode\0[opt\0val\0]...
    fn rrq(filename: &str, mode: &str, opts: &[(&str, &str)]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&OP_RRQ.to_be_bytes());
        p.extend_from_slice(filename.as_bytes());
        p.push(0);
        p.extend_from_slice(mode.as_bytes());
        p.push(0);
        for (k, v) in opts {
            p.extend_from_slice(k.as_bytes());
            p.push(0);
            p.extend_from_slice(v.as_bytes());
            p.push(0);
        }
        p
    }

    #[test]
    fn parse_rrq_basic() {
        let r = parse_rrq(&rrq("kernel_2712.img", "octet", &[])).expect("parses");
        assert_eq!(r.filename, "kernel_2712.img");
        assert_eq!(r.mode, "octet");
        assert_eq!(r.blksize, None);
        assert!(!r.want_tsize);
    }

    #[test]
    fn parse_rrq_lowercases_mode() {
        // RFC 1350: the mode string is case-insensitive.
        let r = parse_rrq(&rrq("f", "OCTET", &[])).unwrap();
        assert_eq!(r.mode, "octet");
    }

    #[test]
    fn parse_rrq_honors_blksize_and_tsize() {
        let r = parse_rrq(&rrq("f", "octet", &[("blksize", "1024"), ("tsize", "0")])).unwrap();
        assert_eq!(r.blksize, Some(1024));
        assert!(r.want_tsize);
    }

    #[test]
    fn parse_rrq_clamps_blksize() {
        // Below the RFC 2348 floor (8) and above our ceiling (65464) are clamped.
        assert_eq!(
            parse_rrq(&rrq("f", "octet", &[("blksize", "1")]))
                .unwrap()
                .blksize,
            Some(8)
        );
        assert_eq!(
            parse_rrq(&rrq("f", "octet", &[("blksize", "70000")]))
                .unwrap()
                .blksize,
            Some(65464)
        );
    }

    #[test]
    fn parse_rrq_ignores_unparseable_blksize() {
        let r = parse_rrq(&rrq("f", "octet", &[("blksize", "huge")])).unwrap();
        assert_eq!(
            r.blksize, None,
            "non-numeric blksize is dropped, not defaulted"
        );
    }

    #[test]
    fn parse_rrq_ignores_unknown_options() {
        let r = parse_rrq(&rrq("f", "octet", &[("windowsize", "4"), ("tsize", "0")])).unwrap();
        assert!(r.want_tsize);
        assert_eq!(r.blksize, None);
    }

    #[test]
    fn parse_rrq_rejects_missing_mode() {
        // Only a filename, no NUL-terminated mode field.
        let mut p = Vec::new();
        p.extend_from_slice(&OP_RRQ.to_be_bytes());
        p.extend_from_slice(b"file");
        assert!(parse_rrq(&p).is_none());
    }

    #[test]
    fn error_packet_layout() {
        let p = error_packet(ERR_NOT_FOUND, "file not found");
        assert_eq!(u16::from_be_bytes([p[0], p[1]]), OP_ERROR);
        assert_eq!(u16::from_be_bytes([p[2], p[3]]), ERR_NOT_FOUND);
        assert_eq!(&p[4..p.len() - 1], b"file not found");
        assert_eq!(*p.last().unwrap(), 0, "error message is NUL-terminated");
    }

    #[test]
    fn resolve_accepts_file_inside_root() {
        let root = tmp();
        fs::write(root.join("kernel.img"), b"x").unwrap();
        let got = resolve(&root, "kernel.img").expect("file in root resolves");
        assert!(got.ends_with("kernel.img"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_strips_leading_slash() {
        let root = tmp();
        fs::write(root.join("boot.img"), b"x").unwrap();
        // An absolute-looking request is treated as relative to root, never as
        // a host filesystem path.
        assert!(resolve(&root, "/boot.img").is_some());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_rejects_traversal_outside_root() {
        // Lay out  base/secret  and serve from  base/served . A "../secret"
        // request must be rejected even though the target genuinely exists.
        let base = tmp();
        let served = base.join("served");
        fs::create_dir_all(&served).unwrap();
        fs::write(base.join("secret"), b"top secret").unwrap();

        assert!(
            resolve(&served, "../secret").is_none(),
            "must not escape root"
        );
        assert!(resolve(&served, "../../etc/passwd").is_none());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn resolve_rejects_missing_file() {
        let root = tmp();
        // canonicalize() fails for a nonexistent path -> None (no info leak about
        // whether a sibling outside root exists).
        assert!(resolve(&root, "nope.img").is_none());
        fs::remove_dir_all(&root).ok();
    }

    // ── Loopback transfer tests ──────────────────────────────────────────────
    //
    // These drive the real async DATA/ACK engine (`handle_rrq` →
    // `send_and_wait_ack` → `send_pkt`) over a 127.0.0.1 UDP pair. `BpfSender`
    // is the inert `unavailable()` form on every platform, so `send_pkt` always
    // takes the ordinary `send_to` path — no hardware, no raw frames, no root.

    /// An `Xfer` that always uses the kernel `send_to` path (BPF unavailable).
    fn loopback_xfer() -> Xfer {
        Xfer {
            host_ip: Ipv4Addr::LOCALHOST,
            bpf: Arc::new(BpfSender::unavailable()),
            client_mac: None,
        }
    }

    fn ack(block: u16) -> [u8; 4] {
        let mut a = [0u8; 4];
        a[..2].copy_from_slice(&OP_ACK.to_be_bytes());
        a[2..].copy_from_slice(&block.to_be_bytes());
        a
    }

    /// Drive the read side of a transfer: ACK every OACK/DATA, accumulate the
    /// payload and the block-number sequence. `blksize` is the negotiated size,
    /// used to spot the final (short) block. Returns (bytes, blocks_seen).
    async fn recv_transfer(sock: &UdpSocket, blksize: usize) -> (Vec<u8>, Vec<u16>) {
        let mut data = Vec::new();
        let mut blocks = Vec::new();
        let mut buf = vec![0u8; 2048];
        loop {
            let (n, from) = sock.recv_from(&mut buf).await.unwrap();
            match u16::from_be_bytes([buf[0], buf[1]]) {
                OP_OACK => {
                    sock.send_to(&ack(0), from).await.unwrap();
                }
                OP_DATA => {
                    let blk = u16::from_be_bytes([buf[2], buf[3]]);
                    blocks.push(blk);
                    data.extend_from_slice(&buf[4..n]);
                    sock.send_to(&ack(blk), from).await.unwrap();
                    if n - 4 < blksize {
                        break;
                    }
                }
                other => panic!("unexpected opcode {other} during transfer"),
            }
        }
        (data, blocks)
    }

    /// Bind a loopback client socket and return it with its address (the `peer`
    /// the server sends to).
    async fn client_socket() -> (UdpSocket, SocketAddr) {
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = sock.local_addr().unwrap();
        (sock, addr)
    }

    #[tokio::test]
    async fn transfer_delivers_multi_block_file() {
        let root = tmp();
        // 1300 bytes over the default 512-byte blksize -> blocks 1(512),2(512),3(276).
        let contents: Vec<u8> = (0..1300u32).map(|i| i as u8).collect();
        fs::write(root.join("k.img"), &contents).unwrap();

        let (sock, peer) = client_socket().await;
        let server = handle_rrq(
            root.clone(),
            rrq("k.img", "octet", &[]),
            peer,
            None,
            loopback_xfer(),
        );
        let (_, (got, blocks)) = tokio::join!(server, recv_transfer(&sock, DEFAULT_BLKSIZE));

        assert_eq!(got, contents, "reassembled bytes must match the file");
        assert_eq!(blocks, vec![1, 2, 3]);
        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn transfer_exact_multiple_sends_trailing_empty_block() {
        let root = tmp();
        // Exactly 2*512: the last data block is full, so RFC 1350 requires one
        // more (empty) block to signal end-of-file.
        let contents = vec![0xABu8; 1024];
        fs::write(root.join("k.img"), &contents).unwrap();

        let (sock, peer) = client_socket().await;
        let server = handle_rrq(
            root.clone(),
            rrq("k.img", "octet", &[]),
            peer,
            None,
            loopback_xfer(),
        );
        let (_, (got, blocks)) = tokio::join!(server, recv_transfer(&sock, DEFAULT_BLKSIZE));

        assert_eq!(got, contents);
        assert_eq!(blocks, vec![1, 2, 3], "trailing empty block 3 terminates");
        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn transfer_of_empty_file_is_one_empty_block() {
        let root = tmp();
        fs::write(root.join("empty"), b"").unwrap();

        let (sock, peer) = client_socket().await;
        let server = handle_rrq(
            root.clone(),
            rrq("empty", "octet", &[]),
            peer,
            None,
            loopback_xfer(),
        );
        let (_, (got, blocks)) = tokio::join!(server, recv_transfer(&sock, DEFAULT_BLKSIZE));

        assert!(got.is_empty());
        assert_eq!(blocks, vec![1], "a single empty DATA block");
        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn transfer_with_oack_negotiates_blksize_and_tsize() {
        let root = tmp();
        let contents = vec![0x5Au8; 40];
        fs::write(root.join("k.img"), &contents).unwrap();

        let (sock, peer) = client_socket().await;
        // Custom client: inspect the OACK before ACKing, then read the transfer.
        let client = async {
            let mut buf = vec![0u8; 2048];
            let (n, from) = sock.recv_from(&mut buf).await.unwrap();
            assert_eq!(u16::from_be_bytes([buf[0], buf[1]]), OP_OACK);
            // OACK body: blksize\016\0tsize\040\0
            let body = &buf[2..n];
            let s = String::from_utf8_lossy(body);
            let fields: Vec<&str> = s.split('\0').collect();
            assert!(fields.contains(&"blksize") && fields.contains(&"16"));
            assert!(fields.contains(&"tsize") && fields.contains(&"40"));
            sock.send_to(&ack(0), from).await.unwrap();
            // Now read DATA at the negotiated 16-byte blksize: 40 -> 16,16,8.
            let mut data = Vec::new();
            let mut blocks = Vec::new();
            loop {
                let (n, from) = sock.recv_from(&mut buf).await.unwrap();
                assert_eq!(u16::from_be_bytes([buf[0], buf[1]]), OP_DATA);
                let blk = u16::from_be_bytes([buf[2], buf[3]]);
                blocks.push(blk);
                data.extend_from_slice(&buf[4..n]);
                sock.send_to(&ack(blk), from).await.unwrap();
                if n - 4 < 16 {
                    break;
                }
            }
            (data, blocks)
        };
        let server = handle_rrq(
            root.clone(),
            rrq("k.img", "octet", &[("blksize", "16"), ("tsize", "0")]),
            peer,
            None,
            loopback_xfer(),
        );
        let (_, (got, blocks)) = tokio::join!(server, client);

        assert_eq!(got, contents);
        assert_eq!(blocks, vec![1, 2, 3], "16-byte blocks: 16+16+8");
        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn retransmits_data_when_first_ack_is_dropped() {
        let root = tmp();
        let contents = vec![0x11u8; 100];
        fs::write(root.join("k.img"), &contents).unwrap();

        let (sock, peer) = client_socket().await;
        // Client ignores the first copy of block 1, forcing an ACK_TIMEOUT-driven
        // retransmit, then behaves normally. We expect to see block 1 twice.
        let client = async {
            let mut buf = vec![0u8; 2048];
            let mut blocks = Vec::new();
            let mut dropped_once = false;
            let mut data = Vec::new();
            loop {
                let (n, from) = sock.recv_from(&mut buf).await.unwrap();
                let blk = u16::from_be_bytes([buf[2], buf[3]]);
                blocks.push(blk);
                if blk == 1 && !dropped_once {
                    dropped_once = true; // swallow this DATA; do NOT ACK
                    continue;
                }
                data.extend_from_slice(&buf[4..n]);
                sock.send_to(&ack(blk), from).await.unwrap();
                if n - 4 < DEFAULT_BLKSIZE {
                    break;
                }
            }
            (data, blocks)
        };
        let server = handle_rrq(
            root.clone(),
            rrq("k.img", "octet", &[]),
            peer,
            None,
            loopback_xfer(),
        );
        let (_, (got, blocks)) = tokio::join!(server, client);

        assert_eq!(got, contents, "transfer still completes after a loss");
        assert_eq!(
            blocks.iter().filter(|&&b| b == 1).count(),
            2,
            "block 1 was retransmitted exactly once"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn missing_file_returns_error_packet() {
        let root = tmp();
        let (sock, peer) = client_socket().await;
        let server = handle_rrq(
            root.clone(),
            rrq("nope.img", "octet", &[]),
            peer,
            None,
            loopback_xfer(),
        );
        let client = async {
            let mut buf = vec![0u8; 2048];
            let (n, _) = sock.recv_from(&mut buf).await.unwrap();
            (
                u16::from_be_bytes([buf[0], buf[1]]),
                u16::from_be_bytes([buf[2], buf[3]]),
                n,
            )
        };
        let (_, (opcode, code, _)) = tokio::join!(server, client);
        assert_eq!(opcode, OP_ERROR);
        assert_eq!(code, ERR_NOT_FOUND);
        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn non_octet_mode_returns_error_packet() {
        let root = tmp();
        fs::write(root.join("k.img"), b"hello").unwrap();
        let (sock, peer) = client_socket().await;
        let server = handle_rrq(
            root.clone(),
            rrq("k.img", "netascii", &[]),
            peer,
            None,
            loopback_xfer(),
        );
        let client = async {
            let mut buf = vec![0u8; 2048];
            let _ = sock.recv_from(&mut buf).await.unwrap();
            (
                u16::from_be_bytes([buf[0], buf[1]]),
                u16::from_be_bytes([buf[2], buf[3]]),
            )
        };
        let (_, (opcode, code)) = tokio::join!(server, client);
        assert_eq!(opcode, OP_ERROR);
        assert_eq!(code, ERR_ILLEGAL);
        fs::remove_dir_all(&root).ok();
    }

    // ~14 s: 65536 lock-step round trips are the only way to reach the wrap.
    // Excluded from the default run; exercise with `cargo test -- --ignored`.
    #[ignore = "stress test: 65k round trips (~14s); run with --ignored"]
    #[tokio::test]
    async fn block_number_wraps_past_0xffff() {
        // The block counter is a u16 with `wrapping_add`. To exercise the wrap we
        // must transfer more than 65535 blocks; an 8-byte blksize keeps the file
        // small (~512 KiB) while still forcing 65536+ round trips. We assert the
        // transfer completes AND that block 0 (only reachable by wrapping) appears.
        let root = tmp();
        let blksize = 8usize;
        let n_full_blocks = 65536usize; // blocks 1..=65535, then 0
        let contents = vec![0x7Eu8; n_full_blocks * blksize + 4]; // + a final short block
        fs::write(root.join("big.img"), &contents).unwrap();

        let (sock, peer) = client_socket().await;
        let server = handle_rrq(
            root.clone(),
            rrq("big.img", "octet", &[("blksize", "8")]),
            peer,
            None,
            loopback_xfer(),
        );
        let (_, (got, blocks)) = tokio::join!(server, recv_transfer(&sock, blksize));

        assert_eq!(
            got.len(),
            contents.len(),
            "all bytes delivered across the wrap"
        );
        assert_eq!(got, contents);
        assert!(
            blocks.contains(&0),
            "block 0 must appear, proving the 0xFFFF->0 wrap"
        );
        // Sanity: the sequence starts 1,2,3 and contains the wrap boundary 65535,0.
        assert_eq!(&blocks[..3], &[1, 2, 3]);
        let wrap = blocks.windows(2).position(|w| w == [0xFFFF, 0]);
        assert!(wrap.is_some(), "0xFFFF must be immediately followed by 0");
        fs::remove_dir_all(&root).ok();
    }
}
