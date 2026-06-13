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
const OPT_VENDOR_CLASS: u8 = 60;
const OPT_TFTP_SERVER: u8 = 66;
const OPT_BOOTFILE: u8 = 67;
const OPT_CLIENT_ARCH: u8 = 93;
const OPT_END: u8 = 255;

/// Vendor-class prefix a UEFI HTTP Boot client sends (option 60), e.g.
/// `HTTPClient:Arch:00019:UNDI:003000`. We branch on the prefix, not the full
/// string. The reply must echo a class beginning with this or EDK2 rejects the
/// offer as "not a valid HTTP boot offer".
const HTTP_CLIENT_CLASS: &str = "HTTPClient";

/// Vendor-class prefix a UEFI/legacy PXE client sends (option 60), e.g.
/// `PXEClient:Arch:00011:UNDI:003000` (arch 11 = ARM64 UEFI). Echoing it back is
/// often required for the UEFI PXE driver to accept a single-server offer.
const PXE_CLIENT_CLASS: &str = "PXEClient";

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
    /// Option 60 vendor class identifier (e.g. `HTTPClient:Arch:00019:…`), used
    /// to distinguish a UEFI HTTP Boot client from the legacy Pi bootloader.
    vendor_class: Option<String>,
    /// Option 93 client system architecture (e.g. 19 = ARM64 UEFI HTTP). Parsed
    /// for logging / future multi-arch use; the reply branches on `vendor_class`.
    arch: Option<u16>,
}

impl Request {
    /// True when this is a UEFI HTTP Boot client (option 60 begins `HTTPClient`).
    fn is_http_client(&self) -> bool {
        self.vendor_class
            .as_deref()
            .is_some_and(|c| c.starts_with(HTTP_CLIENT_CLASS))
    }

    /// True when this is a PXE client (option 60 begins `PXEClient`).
    fn is_pxe_client(&self) -> bool {
        self.vendor_class
            .as_deref()
            .is_some_and(|c| c.starts_with(PXE_CLIENT_CLASS))
    }
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
    let mut vendor_class = None;
    let mut arch = None;
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
        let val = &opts[val_start..val_end];
        match tag {
            OPT_MSG_TYPE if len >= 1 => msg_type = val[0],
            OPT_VENDOR_CLASS if len >= 1 => {
                vendor_class = Some(String::from_utf8_lossy(val).into_owned());
            }
            OPT_CLIENT_ARCH if len >= 2 => arch = Some(u16::from_be_bytes([val[0], val[1]])),
            _ => {}
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
        vendor_class,
        arch,
    })
}

fn encode_option(buf: &mut Vec<u8>, tag: u8, value: &[u8]) {
    buf.push(tag);
    buf.push(value.len() as u8);
    buf.extend_from_slice(value);
}

/// What to advertise as the boot source in a reply. The legacy Pi/TFTP path and
/// the UEFI HTTP Boot path differ only here.
struct BootAdvert<'a> {
    /// Option 67 value (and, for `tftp`, the BOOTP `file` field): a bare filename
    /// for TFTP, a full `http://…` URL for HTTP Boot.
    boot_file: &'a str,
    /// Advertise the host as the TFTP next-server — option 66 (dotted-quad
    /// string) + a non-zero `siaddr` + the bootfile copied into the fixed `file`
    /// field. False for HTTP Boot (the URL in option 67 is self-contained, and
    /// it may overrun the 128-byte `file` field).
    tftp: bool,
    /// Echo a vendor class in option 60. UEFI HTTP Boot **requires** the reply's
    /// class to begin `HTTPClient` or EDK2 rejects the offer. None on the legacy
    /// Pi path.
    vendor_class: Option<&'a str>,
}

impl<'a> BootAdvert<'a> {
    /// Legacy Raspberry Pi / generic TFTP advertisement.
    fn tftp(boot_file: &'a str) -> Self {
        Self {
            boot_file,
            tftp: true,
            vendor_class: None,
        }
    }

    /// UEFI HTTP Boot: a `http://…` URL in option 67, `HTTPClient` echoed in
    /// option 60, no TFTP next-server.
    fn http(url: &'a str) -> Self {
        Self {
            boot_file: url,
            tftp: false,
            vendor_class: Some(HTTP_CLIENT_CLASS),
        }
    }

    /// PXE: the legacy TFTP advertisement plus the `PXEClient` option-60 echo.
    fn pxe(boot_file: &'a str) -> Self {
        Self {
            boot_file,
            tftp: true,
            vendor_class: Some(PXE_CLIENT_CLASS),
        }
    }
}

/// Construct the `http://host[:port]/file` URL advertised to a UEFI HTTP Boot
/// client in option 67. The port is omitted when it is the default 80, keeping
/// the common URL clean.
fn http_boot_url(host_ip: Ipv4Addr, port: u16, boot_file: &str) -> String {
    let file = boot_file.trim_start_matches('/');
    if port == 80 {
        format!("http://{host_ip}/{file}")
    } else {
        format!("http://{host_ip}:{port}/{file}")
    }
}

fn build_reply(req: &Request, msg_type: u8, server_ip: Ipv4Addr, advert: &BootAdvert) -> Vec<u8> {
    let server_b = server_ip.octets();
    let client_b = ASSIGNED_IP.octets();

    let mut opts = Vec::with_capacity(64);
    opts.extend_from_slice(&MAGIC);
    encode_option(&mut opts, OPT_MSG_TYPE, &[msg_type]);
    encode_option(&mut opts, OPT_SERVER_ID, &server_b);
    encode_option(&mut opts, OPT_LEASE, &LEASE_SECONDS.to_be_bytes());
    encode_option(&mut opts, OPT_SUBNET, &[255, 255, 255, 0]);
    encode_option(&mut opts, OPT_ROUTER, &server_b);
    if advert.tftp {
        encode_option(&mut opts, OPT_TFTP_SERVER, server_ip.to_string().as_bytes());
    }
    if let Some(vc) = advert.vendor_class {
        encode_option(&mut opts, OPT_VENDOR_CLASS, vc.as_bytes());
    }
    encode_option(&mut opts, OPT_BOOTFILE, advert.boot_file.as_bytes());
    opts.push(OPT_END);

    // siaddr (next-server) only for the TFTP path; HTTP carries the full URL.
    let siaddr = if advert.tftp { server_b } else { [0u8; 4] };

    let mut pkt = Vec::with_capacity(236 + opts.len());
    pkt.extend_from_slice(&[BOOTREPLY, HTYPE_ETHERNET, 6, 0]); // op, htype, hlen, hops
    pkt.extend_from_slice(&req.xid);
    pkt.extend_from_slice(&[0, 0]); // secs
    pkt.extend_from_slice(&[0x80, 0x00]); // flags: broadcast
    pkt.extend_from_slice(&[0; 4]); // ciaddr
    pkt.extend_from_slice(&client_b); // yiaddr
    pkt.extend_from_slice(&siaddr); // siaddr (next-server = TFTP, else 0)
    pkt.extend_from_slice(&[0; 4]); // giaddr
    pkt.extend_from_slice(&req.chaddr); // chaddr (16)
    pkt.extend_from_slice(&[0; 64]); // sname
    let mut file = [0u8; 128]; // file (null-padded)
    if advert.tftp {
        let fb = advert.boot_file.as_bytes();
        let n = fb.len().min(127);
        file[..n].copy_from_slice(&fb[..n]);
    }
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
    http_port: u16,
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

        // Branch on the client's vendor class (option 60):
        //   * HTTPClient → self-contained http:// URL in option 67 + the required
        //     HTTPClient echo, served over HTTP;
        //   * PXEClient  → the legacy TFTP reply plus a PXEClient option-60 echo;
        //   * neither (the silent Pi, generic TFTP) → the legacy reply unchanged.
        let url;
        let (advert, style) = if req.is_http_client() {
            url = http_boot_url(host_ip, http_port, &boot_file);
            (BootAdvert::http(&url), "http-boot")
        } else if req.is_pxe_client() {
            (BootAdvert::pxe(&boot_file), "pxe")
        } else {
            (BootAdvert::tftp(&boot_file), "tftp")
        };

        let reply = build_reply(&req, reply_type, host_ip, &advert);
        if let Err(e) = sock.send_to(&reply, (bcast, 68)).await {
            warn!("DHCP send_to {bcast}:68 failed: {e}");
            continue;
        }
        let what = match reply_type {
            DHCP_OFFER => "DHCPOFFER",
            _ => "DHCPACK",
        };
        info!(
            "{what} -> {mac}  ip={ASSIGNED_IP}  {style}  arch={:?}  boot={}",
            req.arch, advert.boot_file
        );
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
        let reply = build_reply(
            &req,
            DHCP_OFFER,
            server,
            &BootAdvert::tftp("kernel_2712.img"),
        );

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
        let reply = build_reply(&req, DHCP_OFFER, server, &BootAdvert::tftp("boot.img"));
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
        let reply = build_reply(
            &req,
            DHCP_ACK,
            Ipv4Addr::new(192, 168, 99, 1),
            &BootAdvert::tftp("k.img"),
        );
        let opts = reply_options(&reply);
        let mt = opts.iter().find(|(t, _)| *t == OPT_MSG_TYPE).unwrap();
        assert_eq!(mt.1, vec![DHCP_ACK]);
    }

    #[test]
    fn build_reply_truncates_overlong_bootfile_in_file_field() {
        let req = parse_request(&request_packet(BOOTREQUEST, Some(DHCP_DISCOVER), &[])).unwrap();
        let long = "a".repeat(200);
        let reply = build_reply(
            &req,
            DHCP_OFFER,
            Ipv4Addr::new(1, 2, 3, 4),
            &BootAdvert::tftp(&long),
        );
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

    // ── UEFI HTTP Boot path ──────────────────────────────────────────────────

    /// Trailing option TLVs for an ARM64 UEFI HTTP Boot client: option 60
    /// vendor class + option 93 arch = 0x0013.
    fn http_client_opts() -> Vec<u8> {
        let class = b"HTTPClient:Arch:00019:UNDI:003000";
        let mut t = vec![OPT_VENDOR_CLASS, class.len() as u8];
        t.extend_from_slice(class);
        t.extend_from_slice(&[OPT_CLIENT_ARCH, 2, 0x00, 0x13]);
        t
    }

    #[test]
    fn parse_extracts_vendor_class_and_arch() {
        let pkt = request_packet(BOOTREQUEST, Some(DHCP_DISCOVER), &http_client_opts());
        let req = parse_request(&pkt).expect("HTTPClient DISCOVER parses");
        assert_eq!(
            req.vendor_class.as_deref(),
            Some("HTTPClient:Arch:00019:UNDI:003000")
        );
        assert_eq!(req.arch, Some(0x0013));
        assert!(req.is_http_client());
    }

    #[test]
    fn classless_request_is_not_http_client() {
        let req = parse_request(&request_packet(BOOTREQUEST, Some(DHCP_DISCOVER), &[])).unwrap();
        assert_eq!(req.vendor_class, None);
        assert_eq!(req.arch, None);
        assert!(!req.is_http_client(), "the Pi/legacy path is not HTTP");
    }

    #[test]
    fn http_reply_echoes_class_and_url_without_tftp_fields() {
        let req = parse_request(&request_packet(BOOTREQUEST, Some(DHCP_DISCOVER), &[])).unwrap();
        let server = Ipv4Addr::new(192, 168, 99, 1);
        let url = http_boot_url(server, 80, "grubaa64.efi");
        let reply = build_reply(&req, DHCP_OFFER, server, &BootAdvert::http(&url));
        let opts = reply_options(&reply);
        let get = |tag: u8| opts.iter().find(|(t, _)| *t == tag).map(|(_, v)| v.clone());

        // The mandatory HTTPClient echo (EDK2 rejects the offer without it) and
        // the full URL as the bootfile.
        assert_eq!(get(OPT_VENDOR_CLASS), Some(b"HTTPClient".to_vec()));
        assert_eq!(
            get(OPT_BOOTFILE),
            Some(b"http://192.168.99.1/grubaa64.efi".to_vec())
        );
        // No TFTP next-server: option 66 absent, siaddr zeroed, file field empty.
        assert_eq!(
            get(OPT_TFTP_SERVER),
            None,
            "HTTP path must not set option 66"
        );
        assert_eq!(&reply[20..24], &[0, 0, 0, 0], "siaddr zero on HTTP path");
        assert_eq!(reply[108], 0, "file field empty for HTTP boot");
    }

    #[test]
    fn http_boot_url_formats() {
        let ip = Ipv4Addr::new(192, 168, 99, 1);
        assert_eq!(
            http_boot_url(ip, 80, "boot.efi"),
            "http://192.168.99.1/boot.efi",
            "port 80 omitted"
        );
        assert_eq!(
            http_boot_url(ip, 80, "/boot.efi"),
            "http://192.168.99.1/boot.efi",
            "leading slash trimmed"
        );
        assert_eq!(
            http_boot_url(ip, 8080, "boot.efi"),
            "http://192.168.99.1:8080/boot.efi",
            "non-default port kept"
        );
    }

    // ── PXE path ─────────────────────────────────────────────────────────────

    /// Trailing option TLVs for an ARM64 UEFI PXE client: option 60 vendor class
    /// + option 93 arch = 0x000B (11).
    fn pxe_client_opts() -> Vec<u8> {
        let class = b"PXEClient:Arch:00011:UNDI:003000";
        let mut t = vec![OPT_VENDOR_CLASS, class.len() as u8];
        t.extend_from_slice(class);
        t.extend_from_slice(&[OPT_CLIENT_ARCH, 2, 0x00, 0x0B]);
        t
    }

    #[test]
    fn parse_classifies_pxe_client() {
        let pkt = request_packet(BOOTREQUEST, Some(DHCP_DISCOVER), &pxe_client_opts());
        let req = parse_request(&pkt).expect("PXEClient DISCOVER parses");
        assert_eq!(req.arch, Some(0x000B));
        assert!(req.is_pxe_client());
        assert!(!req.is_http_client(), "PXE is not HTTP");
    }

    #[test]
    fn pxe_reply_echoes_class_and_keeps_tftp_fields() {
        let req = parse_request(&request_packet(BOOTREQUEST, Some(DHCP_DISCOVER), &[])).unwrap();
        let server = Ipv4Addr::new(192, 168, 99, 1);
        let reply = build_reply(&req, DHCP_OFFER, server, &BootAdvert::pxe("ipxe.efi"));
        let opts = reply_options(&reply);
        let get = |tag: u8| opts.iter().find(|(t, _)| *t == tag).map(|(_, v)| v.clone());

        // PXEClient echo, alongside the legacy TFTP next-server fields.
        assert_eq!(get(OPT_VENDOR_CLASS), Some(b"PXEClient".to_vec()));
        assert_eq!(get(OPT_TFTP_SERVER), Some(b"192.168.99.1".to_vec()));
        assert_eq!(get(OPT_BOOTFILE), Some(b"ipxe.efi".to_vec()));
        assert_eq!(&reply[20..24], &server.octets(), "siaddr = next-server");
        assert_eq!(&reply[108..116], b"ipxe.efi", "bootfile in the file field");
    }
}
