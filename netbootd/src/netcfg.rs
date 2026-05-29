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

//! Host network glue: ARP pinning and interface-IP enforcement.
//!
//! Like the Python version, this shells out to the platform tools rather than
//! going native (netlink / route sockets). For the PoC that keeps the surface
//! small; going native is a follow-up if we want to drop the `sudo` shell-outs.

use std::net::Ipv4Addr;
use std::process::Command;
use std::time::Duration;

use tracing::warn;

/// Pin a static ARP/neighbor entry mapping `ip` → `mac`.
///
/// The Pi netboot firmware sends DHCP/TFTP but never answers ARP, so we install
/// the MAC we just saw in the DHCP frame directly. Needs root.
///
/// - macOS: `arp -s <ip> <mac>`
/// - Linux: `ip neigh replace <ip> lladdr <mac> nud permanent [dev <iface>]`
pub fn set_arp(ip: Ipv4Addr, mac: &str, interface: Option<&str>) {
    let status = if cfg!(target_os = "macos") {
        Command::new("sudo")
            .args(["arp", "-s", &ip.to_string(), mac])
            .status()
    } else {
        let mut cmd = Command::new("sudo");
        cmd.args([
            "ip", "neigh", "replace", &ip.to_string(), "lladdr", mac, "nud", "permanent",
        ]);
        if let Some(iface) = interface {
            cmd.args(["dev", iface]);
        }
        cmd.status()
    };
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => warn!("ARP pin {ip} -> {mac} exited {s}"),
        Err(e) => warn!("ARP pin {ip} -> {mac} failed to spawn: {e}"),
    }
}

/// Whether `host_ip` is currently assigned to `interface`.
fn has_interface_ip(interface: &str, host_ip: Ipv4Addr) -> bool {
    if cfg!(target_os = "macos") {
        Command::new("ifconfig")
            .arg(interface)
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout).contains(&format!("inet {host_ip} "))
            })
            .unwrap_or(false)
    } else {
        Command::new("ip")
            .args(["addr", "show", "dev", interface])
            .output()
            .map(|o| {
                let s = String::from_utf8_lossy(&o.stdout);
                s.contains(&format!("inet {host_ip}/")) || s.contains(&format!("inet {host_ip} "))
            })
            .unwrap_or(false)
    }
}

fn is_link_up(interface: &str) -> bool {
    if cfg!(target_os = "macos") {
        Command::new("ifconfig")
            .arg(interface)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("status: active"))
            .unwrap_or(false)
    } else {
        std::fs::read_to_string(format!("/sys/class/net/{interface}/carrier"))
            .map(|s| s.trim() == "1")
            .unwrap_or(false)
    }
}

fn apply_interface_ip(interface: &str, host_ip: Ipv4Addr) {
    let _ = if cfg!(target_os = "macos") {
        Command::new("sudo")
            .args([
                "ifconfig", interface, &host_ip.to_string(), "netmask", "255.255.255.0", "up",
            ])
            .status()
    } else {
        Command::new("sudo")
            .args(["ip", "addr", "add", &format!("{host_ip}/24"), "dev", interface])
            .status()
    };
}

/// Continuously enforce the static IP on `interface`.
///
/// The netboot client flaps the link on every power-cycle and at several points
/// during its own boot; macOS drops a manually-set IPv4 on flap and Linux's
/// NetworkManager may reset it. Poll and re-apply so the client's next retry
/// always finds the host reachable. Runs until the task is cancelled.
pub async fn monitor_interface(interface: String, host_ip: Ipv4Addr) {
    let mut had_ip = true;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let has_ip = has_interface_ip(&interface, host_ip);
        let active = is_link_up(&interface);
        if !has_ip && active {
            apply_interface_ip(&interface, host_ip);
            if had_ip {
                warn!("interface {interface} lost IP {host_ip} — restoring");
            }
        } else if has_ip && !had_ip {
            tracing::info!("interface {interface} restored with IP {host_ip}");
        }
        had_ip = has_ip;
    }
}
