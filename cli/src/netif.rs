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

//! USB-Ethernet link plumbing for the netboot/ffx modes.
//!
//! Ported from `_netboot.py` (interface helpers) + `_netif.py` (modes). The two
//! link modes share one physical point-to-point link and are mutually
//! exclusive: **netboot** = IPv4 host_ip/24 + DHCP/TFTP; **ffx** = host IPv6
//! link-local `fe80::1/64`, no daemons. The active mode is probed from the
//! running daemon + the interface's addresses, never stored.

use std::process::Command;

use anyhow::{bail, Result};

/// The host-side IPv6 link-local that ffx talks through.
pub const FFX_HOST_LL: &str = "fe80::1";
pub const FFX_PREFIX: u8 = 64;

const SUDO_HINT: &str =
    "Ensure passwordless sudo is configured (NOPASSWD) for the control machine.";

fn run(cmd: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new(cmd[0]).args(&cmd[1..]).output()
}

fn macos() -> bool {
    cfg!(target_os = "macos")
}

// ── interface discovery / probing ───────────────────────────────────────────

pub struct EthInterface {
    pub port: String,
    pub device: String,
    pub active: bool,
}

pub fn is_interface_active(device: &str) -> bool {
    if macos() {
        run(&["ifconfig", device])
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("status: active"))
            .unwrap_or(false)
    } else {
        std::fs::read_to_string(format!("/sys/class/net/{device}/carrier"))
            .map(|s| s.trim() == "1")
            .unwrap_or(false)
    }
}

const MAC_EXCLUDE_PORTS: [&str; 6] = [
    "Wi-Fi",
    "Thunderbolt",
    "Bluetooth",
    "FireWire",
    "iPhone",
    "iPad",
];
const LINUX_SKIP_PREFIXES: [&str; 8] = [
    "lo", "docker", "veth", "br", "virbr", "vlan", "bond", "dummy",
];

/// External (non-built-in) Ethernet interfaces, active ones first. The
/// interface carrying the system default route is excluded — it can never be a
/// netboot link (the start guard refuses to reconfigure the primary NIC), so
/// surfacing it in discovery only invites a misconfiguration.
pub fn list_usb_ethernet_interfaces() -> Vec<EthInterface> {
    let primary = default_route_interface();
    let mut out: Vec<EthInterface> = Vec::new();
    if macos() {
        let Ok(o) = run(&["networksetup", "-listallhardwareports"]) else {
            return out;
        };
        let text = String::from_utf8_lossy(&o.stdout).into_owned();
        let mut port: Option<String> = None;
        for line in text.lines() {
            if let Some(p) = line.strip_prefix("Hardware Port:") {
                port = Some(p.trim().to_string());
            } else if let Some(d) = line.strip_prefix("Device:") {
                if let Some(p) = port.take() {
                    let device = d.trim().to_string();
                    let excluded = device == "bridge0"
                        || device == "lo0"
                        || primary.as_deref() == Some(device.as_str())
                        || MAC_EXCLUDE_PORTS.iter().any(|x| p.starts_with(x));
                    if !excluded {
                        let active = is_interface_active(&device);
                        out.push(EthInterface {
                            port: p,
                            device,
                            active,
                        });
                    }
                }
            }
        }
    } else if let Ok(rd) = std::fs::read_dir("/sys/class/net") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if LINUX_SKIP_PREFIXES.iter().any(|p| name.starts_with(p))
                || primary.as_deref() == Some(name.as_str())
            {
                continue;
            }
            // Type 1 = Ethernet (ARPHRD_ETHER).
            let is_ether = std::fs::read_to_string(e.path().join("type"))
                .map(|t| t.trim() == "1")
                .unwrap_or(false);
            if !is_ether {
                continue;
            }
            let active = is_interface_active(&name);
            out.push(EthInterface {
                port: name.clone(),
                device: name,
                active,
            });
        }
    }
    out.sort_by(|a, b| (!a.active, &a.device).cmp(&(!b.active, &b.device)));
    out
}

/// The networksetup service name for a device (macOS only).
fn find_network_service(interface: &str) -> Option<String> {
    if !macos() {
        return None;
    }
    let o = run(&["networksetup", "-listallhardwareports"]).ok()?;
    let text = String::from_utf8_lossy(&o.stdout).into_owned();
    let mut service: Option<String> = None;
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("Hardware Port:") {
            service = Some(p.trim().to_string());
        } else if let Some(d) = line.strip_prefix("Device:") {
            if d.trim() == interface {
                return service;
            }
        }
    }
    None
}

/// The interface carrying the system default route, or None.
fn default_route_interface() -> Option<String> {
    if macos() {
        let o = run(&["route", "-n", "get", "default"]).ok()?;
        let text = String::from_utf8_lossy(&o.stdout).into_owned();
        for line in text.lines() {
            if let Some(i) = line.trim().strip_prefix("interface:") {
                return Some(i.trim().to_string());
            }
        }
        None
    } else {
        let o = run(&["ip", "route", "show", "default"]).ok()?;
        let text = String::from_utf8_lossy(&o.stdout).into_owned();
        let toks: Vec<&str> = text.split_whitespace().collect();
        toks.iter()
            .position(|&t| t == "dev")
            .and_then(|i| toks.get(i + 1))
            .map(|s| s.to_string())
    }
}

/// True if `interface` carries the system default route — netboot must never
/// reconfigure the primary NIC.
pub fn is_primary_interface(interface: &str) -> bool {
    default_route_interface().as_deref() == Some(interface)
}

/// The interface's addresses: (inet, inet6) with IPv6 scope suffixes stripped.
pub fn iface_addresses(interface: &str) -> (Vec<String>, Vec<String>) {
    let mut inet = Vec::new();
    let mut inet6 = Vec::new();
    if macos() {
        if let Ok(o) = run(&["ifconfig", interface]) {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                let s = line.trim();
                if let Some(rest) = s.strip_prefix("inet ") {
                    if let Some(a) = rest.split_whitespace().next() {
                        inet.push(a.to_string());
                    }
                } else if let Some(rest) = s.strip_prefix("inet6 ") {
                    if let Some(a) = rest.split_whitespace().next() {
                        inet6.push(a.split('%').next().unwrap_or(a).to_string());
                    }
                }
            }
        }
    } else if let Ok(o) = run(&["ip", "-brief", "addr", "show", "dev", interface]) {
        if o.status.success() {
            let text = String::from_utf8_lossy(&o.stdout).into_owned();
            for addr in text.split_whitespace().skip(2) {
                if addr.contains(':') {
                    inet6.push(addr.to_string());
                } else {
                    inet.push(addr.to_string());
                }
            }
        }
    }
    (inet, inet6)
}

fn has_host_ll(inet6: &[String]) -> bool {
    inet6
        .iter()
        .any(|a| a.split('/').next().unwrap_or(a) == FFX_HOST_LL)
}

/// Link-local IPv6 neighbours on the interface (Linux only; excludes our LL).
pub fn ipv6_peers(interface: &str) -> Vec<String> {
    if macos() {
        return Vec::new();
    }
    let Ok(o) = run(&["ip", "-6", "neigh", "show", "dev", interface]) else {
        return Vec::new();
    };
    if !o.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|t| t.starts_with("fe80:") && *t != FFX_HOST_LL)
        .map(String::from)
        .collect()
}

// ── privileged interface mutation ───────────────────────────────────────────

/// Assign the static netboot host IP to the interface (sudo).
pub fn configure_interface(interface: &str, host_ip: &str) -> Result<()> {
    if macos() {
        if let Some(service) = find_network_service(interface) {
            let _ = run(&[
                "sudo",
                "networksetup",
                "-setmanual",
                &service,
                host_ip,
                "255.255.255.0",
            ]);
        }
        let o = run(&[
            "sudo",
            "ifconfig",
            interface,
            host_ip,
            "netmask",
            "255.255.255.0",
            "up",
        ])?;
        if !o.status.success() {
            bail!(
                "ifconfig {interface} failed: {}\n{SUDO_HINT}",
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
    } else {
        let _ = run(&["sudo", "ip", "addr", "flush", "dev", interface]);
        let cidr = format!("{host_ip}/24");
        let o = run(&["sudo", "ip", "addr", "add", &cidr, "dev", interface])?;
        let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
        if !o.status.success() && !stderr.contains("already assigned") {
            bail!(
                "ip addr add {cidr} dev {interface} failed: {}\n{SUDO_HINT}",
                stderr.trim()
            );
        }
        let _ = run(&["sudo", "ip", "link", "set", interface, "up"]);
    }
    Ok(())
}

/// Release the static IP and return the interface to OS-managed networking.
pub fn restore_interface(interface: &str) {
    if macos() {
        if let Some(service) = find_network_service(interface) {
            let _ = run(&["sudo", "networksetup", "-setdhcp", &service]);
        }
    } else {
        let _ = run(&["sudo", "ip", "addr", "flush", "dev", interface]);
    }
}

/// macOS: stop neighbour-unreachability detection from blocking sends to the
/// Pi's bootloader (it never answers ARP probes). No-op on Linux.
pub fn tune_arp_for_silent_client() {
    if !macos() {
        return;
    }
    for kv in [
        "net.link.ether.inet.arp_llreach_base=0",
        "net.link.ether.inet.host_down_time=0",
    ] {
        let _ = run(&["sudo", "sysctl", "-w", kv]);
    }
}

fn add_host_ll(interface: &str) -> Result<()> {
    if macos() {
        let _ = run(&[
            "sudo",
            "ifconfig",
            interface,
            "inet6",
            FFX_HOST_LL,
            "prefixlen",
            "64",
            "up",
        ]);
        return Ok(());
    }
    let sysctl_key = format!("net/ipv6/conf/{interface}/disable_ipv6=0");
    let _ = run(&["sudo", "sysctl", "-w", &sysctl_key]);
    let _ = run(&["sudo", "ip", "link", "set", interface, "up"]);
    let cidr = format!("{FFX_HOST_LL}/{FFX_PREFIX}");
    let o = run(&["sudo", "ip", "-6", "addr", "add", &cidr, "dev", interface])?;
    let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
    if !o.status.success() && !stderr.to_lowercase().contains("exists") {
        bail!(
            "ip -6 addr add {cidr} dev {interface} failed: {}\n{SUDO_HINT}",
            stderr.trim()
        );
    }
    Ok(())
}

fn del_host_ll(interface: &str) {
    if macos() {
        let _ = run(&[
            "sudo",
            "ifconfig",
            interface,
            "inet6",
            FFX_HOST_LL,
            "-alias",
        ]);
    } else {
        let cidr = format!("{FFX_HOST_LL}/{FFX_PREFIX}");
        let _ = run(&["sudo", "ip", "-6", "addr", "del", &cidr, "dev", interface]);
    }
}

fn del_host_ip(interface: &str, host_ip: &str) {
    if macos() {
        // Release the static IP by returning the service to DHCP — an
        // `ifconfig` delete won't unset a `networksetup -setmanual` IP, so
        // `mode off` from `link` mode (no netboot stop to restore it) would
        // otherwise leave the host IP assigned.
        if let Some(service) = find_network_service(interface) {
            let _ = run(&["sudo", "networksetup", "-setdhcp", &service]);
        }
        return;
    }
    let cidr = format!("{host_ip}/24");
    let _ = run(&["sudo", "ip", "addr", "del", &cidr, "dev", interface]);
}

/// Best-effort: disable Wake-on-LAN so the NIC doesn't keep the PHY (and the
/// peer's carrier) alive after the interface is taken down. Per-NIC via ethtool
/// on Linux; macOS WoL is a system-wide pref (`pmset womp`), so there we rely on
/// admin-down alone (USB-Ethernet adapters drop link on `ifconfig down`).
fn disable_wol(interface: &str) {
    if macos() {
        return;
    }
    let _ = run(&["sudo", "ethtool", "-s", interface, "wol", "d"]);
}

/// Administratively bring the interface down — drops carrier on the peer.
fn admin_down(interface: &str) {
    if macos() {
        let _ = run(&["sudo", "ifconfig", interface, "down"]);
    } else {
        let _ = run(&["sudo", "ip", "link", "set", interface, "down"]);
    }
}

// ── modes ───────────────────────────────────────────────────────────────────

pub const MODES: [&str; 4] = ["netboot", "link", "ffx", "off"];

/// netboot mode: tear down ffx, start DHCP+TFTP (idempotent).
pub fn mode_netboot(
    target: &str,
    interface: &str,
    host_ip: &str,
    tftp_root: &str,
    opts: &crate::netboot::BootOptions,
) -> Result<()> {
    del_host_ll(interface);
    if crate::state::is_netboot_running(target) {
        return Ok(());
    }
    crate::netboot::start(target, interface, host_ip, tftp_root, opts)
}

/// link mode: assign just the IPv4 host IP to the interface — no DHCP/TFTP
/// daemon and no ffx link-local. This is the bare host side of the link, for
/// testing it up/down (`mode link` brings the host IP up, `mode off` releases
/// it). Idempotent. Note `mode off` only releases the IP; it does not force the
/// physical carrier down (the NIC can hold the link up for Wake-on-LAN — see
/// docs/netif.md).
pub fn mode_link(target: &str, interface: &str, host_ip: &str) -> Result<()> {
    if crate::state::is_netboot_running(target) {
        crate::netboot::stop(target)?;
    }
    del_host_ll(interface);
    configure_interface(interface, host_ip)
}

/// ffx mode: stop netboot first, then add the host IPv6 link-local.
pub fn mode_ffx(target: &str, interface: &str) -> Result<()> {
    if crate::state::is_netboot_running(target) {
        crate::netboot::stop(target)?;
    }
    add_host_ll(interface)
}

/// off: stop netboot, remove the ffx LL and any stale IPv4.
pub fn mode_off(target: &str, interface: &str, host_ip: &str) -> Result<()> {
    if crate::state::is_netboot_running(target) {
        crate::netboot::stop(target)?;
    }
    del_host_ll(interface);
    del_host_ip(interface, host_ip);
    Ok(())
}

/// Force the link down *hard*: do everything `mode off` does (stop netboot,
/// release the host IP + ffx LL), then disable Wake-on-LAN and admin-down the
/// interface so the peer sees carrier loss. `mode off` alone only releases the
/// host IP and can leave the carrier up (a WoL-capable NIC keeps the PHY
/// energized) — use this when you need the target to actually *detect* link
/// loss. Bring the link back with `mode link`/`mode netboot`; WoL stays disabled
/// until re-enabled (`ethtool -s <iface> wol g`) or the adapter is replugged.
pub fn down_hard(target: &str, interface: &str, host_ip: &str) -> Result<()> {
    mode_off(target, interface, host_ip)?;
    disable_wol(interface);
    admin_down(interface);
    Ok(())
}

pub struct NetifStatus {
    pub mode: &'static str,
    pub carrier: bool,
    pub inet: Vec<String>,
    pub inet6: Vec<String>,
    pub peers: Vec<String>,
}

/// Probe the active mode (never stored): daemons running → netboot; host LL
/// present → ffx; the static host IP present (but no daemon/LL) → link; else
/// off. `carrier` is the physical link state (independent of the mode — it can
/// read up in `off` if the NIC keeps the PHY alive for Wake-on-LAN).
pub fn get_status(target: &str, interface: &str, host_ip: &str) -> NetifStatus {
    let netboot_running = crate::state::is_netboot_running(target);
    let (inet, inet6) = iface_addresses(interface);
    let has_ll = has_host_ll(&inet6);
    let has_host_ip = inet
        .iter()
        .any(|a| a.split('/').next().unwrap_or(a) == host_ip);
    let mode = if netboot_running {
        "netboot"
    } else if has_ll {
        "ffx"
    } else if has_host_ip {
        "link"
    } else {
        "off"
    };
    let peers = if mode == "ffx" {
        ipv6_peers(interface)
    } else {
        Vec::new()
    };
    NetifStatus {
        mode,
        carrier: is_interface_active(interface),
        inet,
        inet6,
        peers,
    }
}
