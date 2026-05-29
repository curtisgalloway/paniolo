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

//! macOS raw-frame sender — the port of `_tftp.py`'s `BpfSender`.
//!
//! Why this exists: on macOS 15+ the kernel will not deliver unicast UDP to the
//! Pi bootloader even with a permanent static ARP entry, because the bootloader
//! sends from a different ephemeral source MAC than its DHCP MAC and the kernel
//! caches that wrong MAC. The only reliable fix is to stop trusting the kernel's
//! L2 resolution and inject a complete Ethernet frame addressed to the Pi's real
//! DHCP MAC (learned in-process from our own DHCP handler).
//!
//! Implementation: `pnet_datalink`'s macOS backend opens `/dev/bpfN`, sets
//! `BIOCSETIF` + `BIOCSHDRCMPLT(1)` ("don't fill in source MAC"), and writes the
//! full caller-built frame — exactly the Python recipe. Requires membership in
//! the `access_bpf` group (e.g. Wireshark's ChmodBPF) or root.
//!
//! Portability: this module compiles on every platform (pnet_datalink is
//! cross-platform) so the TFTP call sites stay type-checked, but netbootd only
//! constructs a live sender on macOS — elsewhere `BpfSender::unavailable()` is
//! used and TFTP falls back to ordinary `send_to`, matching the Python behavior.
//!
//! Known risk to verify on hardware: a long-standing macOS bug where *setting*
//! `BIOCSHDRCMPLT` can make subsequent writes fail (libpcap toggles it around
//! each inject). The Python code sets-once and works, so this is likely
//! version-dependent; if injection fails on a given macOS build, the fallback is
//! a thin `libc` BPF path that can toggle the flag per write. Not implemented
//! yet — see the project notes.

use std::net::Ipv4Addr;
use std::sync::Mutex;

use pnet_datalink::{Channel, Config, DataLinkSender};
use tracing::{info, warn};

use crate::frame::build_udp_frame;

/// Sends UDP datagrams as raw Ethernet frames, bypassing the kernel ARP table.
pub struct BpfSender {
    src_mac: Option<[u8; 6]>,
    tx: Option<Mutex<Box<dyn DataLinkSender>>>,
}

impl BpfSender {
    /// An inert sender. `available()` is always false; `send_udp()` is a no-op.
    pub fn unavailable() -> Self {
        Self {
            src_mac: None,
            tx: None,
        }
    }

    /// Open a BPF channel bound to `iface_name`. On failure (interface missing,
    /// no MAC, or insufficient privilege) returns an inert sender and logs why.
    pub fn new(iface_name: &str) -> Self {
        let Some(iface) = pnet_datalink::interfaces()
            .into_iter()
            .find(|i| i.name == iface_name)
        else {
            warn!("BPF: interface {iface_name} not found");
            return Self::unavailable();
        };
        let src_mac = match iface.mac {
            Some(m) => [m.0, m.1, m.2, m.3, m.4, m.5],
            None => {
                warn!("BPF: no MAC address for {iface_name}");
                return Self::unavailable();
            }
        };
        match pnet_datalink::channel(&iface, Config::default()) {
            Ok(Channel::Ethernet(tx, _rx)) => {
                info!("BPF sender ready on {iface_name} (src {})", mac_hex(&src_mac));
                Self {
                    src_mac: Some(src_mac),
                    tx: Some(Mutex::new(tx)),
                }
            }
            Ok(_) => {
                warn!("BPF: unsupported channel type on {iface_name}");
                Self::unavailable()
            }
            Err(e) => {
                warn!(
                    "BPF unavailable on {iface_name}: {e} — add user to the \
                     'access_bpf' group (Wireshark ChmodBPF) or run as root"
                );
                Self::unavailable()
            }
        }
    }

    pub fn available(&self) -> bool {
        self.tx.is_some() && self.src_mac.is_some()
    }

    /// Inject `payload` as a UDP datagram in a raw frame to `dst_mac`. Returns
    /// false (caller should fall back to `send_to`) if unavailable or the write
    /// fails.
    // Called only from the macOS send path; dead on a Linux build by design.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn send_udp(
        &self,
        dst_mac: [u8; 6],
        src_ip: Ipv4Addr,
        dst_ip: Ipv4Addr,
        src_port: u16,
        dst_port: u16,
        payload: &[u8],
    ) -> bool {
        let (Some(src_mac), Some(tx)) = (self.src_mac, self.tx.as_ref()) else {
            return false;
        };
        let frame = build_udp_frame(src_mac, dst_mac, src_ip, dst_ip, src_port, dst_port, payload);
        let mut guard = match tx.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        match guard.send_to(&frame, None) {
            Some(Ok(())) => true,
            Some(Err(e)) => {
                warn!("BPF write failed: {e}");
                false
            }
            None => {
                warn!("BPF write: no buffer available");
                false
            }
        }
    }
}

fn mac_hex(m: &[u8; 6]) -> String {
    m.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}
