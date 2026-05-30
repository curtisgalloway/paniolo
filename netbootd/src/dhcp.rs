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

//! Minimal DHCP server for paniolo netboot (PoC port of `_dhcp.py`).
//!
//! Broadcast-only replies — no raw sockets, no BPF. Handles
//! DISCOVER→OFFER and REQUEST→ACK for a single netboot client, advertising
//! the host as both TFTP server (option 66 / `siaddr`) and bootfile source.

use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tracing::{info, warn};

use crate::netcfg;

const BOOTREQUEST: u8 = 1;
const BOOTREPLY: u8 = 2;
const HTYPE_ETHERNET: u8 = 1;
const MAGIC: [u8; 4] = [0x63, 0x82, 0x53, 0x63];

const OPT_SUBNET: u8 = 1;
const OPT_ROUTER: u8 = 3;
const OPT_LEASE: u8 = 51;
const OPT_MSG_TYPE: u8 = 53;
const OPT_SERVER_ID: u8 = 54;
const OPT_TFTP_SERVER: u8 = 66;
const OPT_BOOTFILE: u8 = 67;
const OPT_END: u8 = 255;

const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_ACK: u8 = 5;

const LEASE_SECONDS: u32 = 12 * 3600;
/// Fixed single-client lease (matches `_dhcp.py`).
const ASSIGNED_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 99, 100);

/// Parsed view of the fields we care about from an inbound BOOTP/DHCP packet.
struct Request {
    xid: [u8; 4],
    chaddr: [u8; 16],
    msg_type: u8,
}

fn parse_request(data: &[u8]) -> Option<Request> {
    // Fixed BOOTP header is 236 bytes; options (magic + TLVs) follow.
    if data.len() < 240 || data[0] != BOOTREQUEST {
        return None;
    }
    let mut xid = [0u8; 4];
    xid.copy_from_slice(&data[4..8]);
    let mut chaddr = [0u8; 16];
    chaddr.copy_from_slice(&data[28..44]);

    let opts = &data[236..];
    if opts.len() < 4 || opts[0..4] != MAGIC {
        return None;
    }
    let mut msg_type = 0u8;
    let mut i = 4;
    while i < opts.len() {
        let tag = opts[i];
        if tag == OPT_END {
            break;
        }
        if tag == 0 {
            i += 1;
            continue;
        }
        if i + 1 >= opts.len() {
            break;
        }
        let len = opts[i + 1] as usize;
        let val_start = i + 2;
        let val_end = val_start + len;
        if val_end > opts.len() {
            break;
        }
        if tag == OPT_MSG_TYPE && len >= 1 {
            msg_type = opts[val_start];
        }
        i = val_end;
    }
    if msg_type == 0 {
        return None;
    }
    Some(Request {
        xid,
        chaddr,
        msg_type,
    })
}

fn encode_option(buf: &mut Vec<u8>, tag: u8, value: &[u8]) {
    buf.push(tag);
    buf.push(value.len() as u8);
    buf.extend_from_slice(value);
}

fn build_reply(req: &Request, msg_type: u8, server_ip: Ipv4Addr, boot_file: &str) -> Vec<u8> {
    let server_b = server_ip.octets();
    let client_b = ASSIGNED_IP.octets();

    let mut opts = Vec::with_capacity(64);
    opts.extend_from_slice(&MAGIC);
    encode_option(&mut opts, OPT_MSG_TYPE, &[msg_type]);
    encode_option(&mut opts, OPT_SERVER_ID, &server_b);
    encode_option(&mut opts, OPT_LEASE, &LEASE_SECONDS.to_be_bytes());
    encode_option(&mut opts, OPT_SUBNET, &[255, 255, 255, 0]);
    encode_option(&mut opts, OPT_ROUTER, &server_b);
    encode_option(&mut opts, OPT_TFTP_SERVER, server_ip.to_string().as_bytes());
    encode_option(&mut opts, OPT_BOOTFILE, boot_file.as_bytes());
    opts.push(OPT_END);

    let mut pkt = Vec::with_capacity(236 + opts.len());
    pkt.extend_from_slice(&[BOOTREPLY, HTYPE_ETHERNET, 6, 0]); // op, htype, hlen, hops
    pkt.extend_from_slice(&req.xid);
    pkt.extend_from_slice(&[0, 0]); // secs
    pkt.extend_from_slice(&[0x80, 0x00]); // flags: broadcast
    pkt.extend_from_slice(&[0; 4]); // ciaddr
    pkt.extend_from_slice(&client_b); // yiaddr
    pkt.extend_from_slice(&server_b); // siaddr (next-server = TFTP)
    pkt.extend_from_slice(&[0; 4]); // giaddr
    pkt.extend_from_slice(&req.chaddr); // chaddr (16)
    pkt.extend_from_slice(&[0; 64]); // sname
    let mut file = [0u8; 128]; // file (null-padded)
    let fb = boot_file.as_bytes();
    let n = fb.len().min(127);
    file[..n].copy_from_slice(&fb[..n]);
    pkt.extend_from_slice(&file);
    pkt.extend_from_slice(&opts);
    pkt
}

fn mac_string(chaddr: &[u8; 16]) -> String {
    chaddr[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Bind a broadcast-capable UDP socket on `0.0.0.0:67`.
///
/// Wildcard bind keeps this rootless on macOS 14+; on Linux port 67 still
/// needs root or `CAP_NET_BIND_SERVICE`.
fn bind_server(port: u16) -> Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    sock.set_broadcast(true)?;
    let addr: std::net::SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();
    sock.bind(&addr.into()).with_context(|| {
        format!("bind DHCP port {port} (need root/CAP_NET_BIND_SERVICE on Linux)")
    })?;
    sock.set_nonblocking(true)?;
    Ok(UdpSocket::from_std(sock.into())?)
}

/// Run the DHCP server until the task is cancelled.
///
/// `mac_tx` publishes the client's hardware address (from `chaddr`) to the TFTP
/// task in-process, so the BPF send path can address frames to the Pi's real
/// DHCP MAC without the on-disk `client-mac` file the Python version needs.
pub async fn serve(
    host_ip: Ipv4Addr,
    boot_file: String,
    interface: Option<String>,
    port: u16,
    mac_tx: watch::Sender<Option<[u8; 6]>>,
) -> Result<()> {
    let bcast = {
        let o = host_ip.octets();
        Ipv4Addr::new(o[0], o[1], o[2], 255)
    };
    let sock = bind_server(port)?;
    info!(%host_ip, %bcast, boot_file, "DHCP listening on 0.0.0.0:{port}");

    let mut buf = vec![0u8; 4096];
    loop {
        let (n, _peer) = match sock.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                warn!("DHCP recv_from: {e}");
                continue;
            }
        };
        let Some(req) = parse_request(&buf[..n]) else {
            continue;
        };
        let mac = mac_string(&req.chaddr);

        let (reply_type, label) = match req.msg_type {
            DHCP_DISCOVER => (DHCP_OFFER, "DHCPDISCOVER"),
            DHCP_REQUEST => (DHCP_ACK, "DHCPREQUEST"),
            _ => continue,
        };
        info!("{label} from {mac}");

        // Publish the client MAC to the TFTP task (for the macOS BPF send path).
        let mut mac_bytes = [0u8; 6];
        mac_bytes.copy_from_slice(&req.chaddr[..6]);
        let _ = mac_tx.send_replace(Some(mac_bytes));

        // Pin the client's MAC so the host kernel can deliver to the silent
        // Pi bootloader (it never answers ARP). On macOS 15+ the kernel path is
        // unreliable for TFTP regardless — that's what the BPF send path covers.
        netcfg::set_arp(ASSIGNED_IP, &mac, interface.as_deref());

        let reply = build_reply(&req, reply_type, host_ip, &boot_file);
        if let Err(e) = sock.send_to(&reply, (bcast, 68)).await {
            warn!("DHCP send_to {bcast}:68 failed: {e}");
            continue;
        }
        match reply_type {
            DHCP_OFFER => {
                info!("DHCPOFFER -> {mac}  ip={ASSIGNED_IP}  tftp={host_ip}  file={boot_file}")
            }
            _ => info!("DHCPACK -> {mac}  ip={ASSIGNED_IP}"),
        }
    }
}
