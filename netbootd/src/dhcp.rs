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

#[cfg(test)]
mod tests {
    use super::*;

    const XID: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];
    const MAC6: [u8; 6] = [0xdc, 0xa6, 0x32, 0x01, 0x02, 0x03];

    /// Build a minimal but well-formed BOOTREQUEST/DHCP packet with the given
    /// message type. `trailing_opts` are appended (TLV bytes) before OPT_END.
    fn request_packet(op: u8, msg_type: Option<u8>, trailing_opts: &[u8]) -> Vec<u8> {
        let mut pkt = vec![0u8; 236];
        pkt[0] = op;
        pkt[1] = HTYPE_ETHERNET;
        pkt[2] = 6; // hlen
        pkt[4..8].copy_from_slice(&XID);
        pkt[28..34].copy_from_slice(&MAC6); // chaddr (first 6 = MAC)
        pkt.extend_from_slice(&MAGIC);
        if let Some(mt) = msg_type {
            pkt.extend_from_slice(&[OPT_MSG_TYPE, 1, mt]);
        }
        pkt.extend_from_slice(trailing_opts);
        pkt.push(OPT_END);
        pkt
    }

    /// Collect the options TLVs from a reply packet into (tag -> value) pairs,
    /// in wire order (so duplicates would be visible).
    fn reply_options(pkt: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let opts = &pkt[236..];
        assert_eq!(&opts[0..4], &MAGIC, "reply must start options with magic");
        let mut out = Vec::new();
        let mut i = 4;
        while i < opts.len() {
            let tag = opts[i];
            if tag == OPT_END {
                break;
            }
            let len = opts[i + 1] as usize;
            out.push((tag, opts[i + 2..i + 2 + len].to_vec()));
            i += 2 + len;
        }
        out
    }

    #[test]
    fn parse_discover_extracts_xid_mac_and_type() {
        let pkt = request_packet(BOOTREQUEST, Some(DHCP_DISCOVER), &[]);
        let req = parse_request(&pkt).expect("valid DISCOVER should parse");
        assert_eq!(req.xid, XID);
        assert_eq!(&req.chaddr[..6], &MAC6);
        assert_eq!(req.msg_type, DHCP_DISCOVER);
    }

    #[test]
    fn parse_skips_pad_options_before_msg_type() {
        // A leading run of pad (0) options must not derail the TLV walk: the
        // msg_type option sits after them.
        let mut p = vec![0u8; 236];
        p[0] = BOOTREQUEST;
        p[28..34].copy_from_slice(&MAC6);
        p.extend_from_slice(&MAGIC);
        p.extend_from_slice(&[0u8, 0, 0]); // three pad options
        p.extend_from_slice(&[OPT_MSG_TYPE, 1, DHCP_REQUEST]);
        p.push(OPT_END);
        let req = parse_request(&p).expect("pad-prefixed packet parses");
        assert_eq!(req.msg_type, DHCP_REQUEST);
    }

    #[test]
    fn parse_rejects_too_short() {
        assert!(parse_request(&[0u8; 239]).is_none(), "under 240 bytes");
    }

    #[test]
    fn parse_rejects_non_bootrequest() {
        let pkt = request_packet(BOOTREPLY, Some(DHCP_DISCOVER), &[]);
        assert!(parse_request(&pkt).is_none(), "op must be BOOTREQUEST");
    }

    #[test]
    fn parse_rejects_bad_magic() {
        let mut pkt = request_packet(BOOTREQUEST, Some(DHCP_DISCOVER), &[]);
        pkt[236] ^= 0xff; // corrupt the magic cookie
        assert!(
            parse_request(&pkt).is_none(),
            "wrong magic must be rejected"
        );
    }

    #[test]
    fn parse_rejects_missing_msg_type() {
        let pkt = request_packet(BOOTREQUEST, None, &[]);
        assert!(parse_request(&pkt).is_none(), "no option 53 -> reject");
    }

    #[test]
    fn parse_stops_at_truncated_tlv() {
        // A TLV whose length runs past the buffer must not panic or over-read;
        // the walk breaks and (with no msg_type seen) the packet is rejected.
        let bogus = [OPT_TFTP_SERVER, 200]; // claims 200 bytes, none follow
        let mut p = vec![0u8; 236];
        p[0] = BOOTREQUEST;
        p[28..34].copy_from_slice(&MAC6);
        p.extend_from_slice(&MAGIC);
        p.extend_from_slice(&bogus);
        // no OPT_END, no msg_type
        assert!(parse_request(&p).is_none());
    }

    #[test]
    fn build_offer_header_fields() {
        let req = parse_request(&request_packet(BOOTREQUEST, Some(DHCP_DISCOVER), &[])).unwrap();
        let server = Ipv4Addr::new(192, 168, 99, 1);
        let reply = build_reply(&req, DHCP_OFFER, server, "kernel_2712.img");

        assert_eq!(reply[0], BOOTREPLY, "op = BOOTREPLY");
        assert_eq!(reply[1], HTYPE_ETHERNET);
        assert_eq!(reply[2], 6, "hlen");
        assert_eq!(&reply[4..8], &XID, "xid echoed back");
        assert_eq!(&reply[10..12], &[0x80, 0x00], "broadcast flag set");
        assert_eq!(&reply[16..20], &ASSIGNED_IP.octets(), "yiaddr = assigned");
        assert_eq!(&reply[20..24], &server.octets(), "siaddr = next-server");
        assert_eq!(&reply[28..34], &MAC6, "chaddr echoed");
        // file field (offset 108, 128 bytes) carries the bootfile, null-padded.
        assert_eq!(&reply[108..123], b"kernel_2712.img");
        assert_eq!(reply[123], 0, "file field null-padded");
    }

    #[test]
    fn build_offer_options_match_spec() {
        let req = parse_request(&request_packet(BOOTREQUEST, Some(DHCP_DISCOVER), &[])).unwrap();
        let server = Ipv4Addr::new(10, 0, 0, 5);
        let reply = build_reply(&req, DHCP_OFFER, server, "boot.img");
        let opts = reply_options(&reply);

        let get = |tag: u8| opts.iter().find(|(t, _)| *t == tag).map(|(_, v)| v.clone());
        assert_eq!(get(OPT_MSG_TYPE), Some(vec![DHCP_OFFER]));
        assert_eq!(get(OPT_SERVER_ID), Some(server.octets().to_vec()));
        assert_eq!(get(OPT_LEASE), Some(LEASE_SECONDS.to_be_bytes().to_vec()));
        assert_eq!(get(OPT_SUBNET), Some(vec![255, 255, 255, 0]));
        assert_eq!(get(OPT_ROUTER), Some(server.octets().to_vec()));
        // Option 66 advertises the TFTP server as a dotted-quad STRING (not raw
        // bytes) — the Pi bootloader expects the text form.
        assert_eq!(get(OPT_TFTP_SERVER), Some(b"10.0.0.5".to_vec()));
        assert_eq!(get(OPT_BOOTFILE), Some(b"boot.img".to_vec()));
    }

    #[test]
    fn request_yields_ack_type() {
        let req = parse_request(&request_packet(BOOTREQUEST, Some(DHCP_REQUEST), &[])).unwrap();
        let reply = build_reply(&req, DHCP_ACK, Ipv4Addr::new(192, 168, 99, 1), "k.img");
        let opts = reply_options(&reply);
        let mt = opts.iter().find(|(t, _)| *t == OPT_MSG_TYPE).unwrap();
        assert_eq!(mt.1, vec![DHCP_ACK]);
    }

    #[test]
    fn build_reply_truncates_overlong_bootfile_in_file_field() {
        let req = parse_request(&request_packet(BOOTREQUEST, Some(DHCP_DISCOVER), &[])).unwrap();
        let long = "a".repeat(200);
        let reply = build_reply(&req, DHCP_OFFER, Ipv4Addr::new(1, 2, 3, 4), &long);
        // The fixed 128-byte file field holds at most 127 chars + a NUL terminator.
        assert_eq!(reply[108..108 + 127], long.as_bytes()[..127]);
        assert_eq!(reply[108 + 127], 0, "127th byte reserved for the NUL");
    }

    #[test]
    fn mac_string_formats_first_six_octets() {
        let mut chaddr = [0u8; 16];
        chaddr[..6].copy_from_slice(&MAC6);
        assert_eq!(mac_string(&chaddr), "dc:a6:32:01:02:03");
    }

    #[test]
    fn encode_option_writes_tag_len_value() {
        let mut buf = Vec::new();
        encode_option(&mut buf, OPT_LEASE, &[0, 0, 0x0e, 0x10]);
        assert_eq!(buf, vec![OPT_LEASE, 4, 0, 0, 0x0e, 0x10]);
    }
}
