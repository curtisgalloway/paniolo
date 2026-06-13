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

//! netbootd — single-client netboot daemon: DHCP + read-only TFTP in one
//! process.
//!
//! Proof-of-concept port of paniolo's `_dhcp.py` + `_tftp.py`. Unlike the
//! Python version (two `sudo python -m …` subprocesses coordinating through an
//! on-disk `client-mac` file), this is a single binary running both servers as
//! tokio tasks — no inter-process file handshake needed.
//!
//! Delivery on macOS uses a raw-frame ([`bpf`]) send path so TFTP reaches the
//! Pi bootloader on macOS 15+ (where the kernel misdelivers despite a static
//! ARP entry). The DHCP handler learns the client MAC and hands it to TFTP
//! in-process — no `client-mac` file. On Linux (and when BPF is unavailable)
//! TFTP uses ordinary `send_to`, matching the Python behavior.
//!
//! Privileged ports 67/69 still require root or `CAP_NET_BIND_SERVICE`, and the
//! BPF path needs `access_bpf` group membership or root — exactly as with the
//! Python servers.

mod bpf;
mod dhcp;
mod http;
mod netcfg;
mod served;
mod tftp;

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "netbootd", version, about)]
struct Cli {
    /// Interface IP — bound as the host address and advertised as the TFTP
    /// server (DHCP option 66 / siaddr).
    #[arg(long)]
    host_ip: Ipv4Addr,

    /// TFTP root directory (must exist).
    #[arg(long)]
    tftp_root: PathBuf,

    /// Bootfile advertised in DHCP option 67.
    #[arg(long, default_value = "kernel_2712.img")]
    boot_file: String,

    /// Interface device name (e.g. en11 / eth0) for ARP pinning + IP monitor.
    #[arg(long)]
    interface: Option<String>,

    #[arg(long, default_value_t = 67)]
    dhcp_port: u16,

    #[arg(long, default_value_t = 69)]
    tftp_port: u16,

    /// HTTP server port, also embedded in the UEFI HTTP Boot URL advertised in
    /// DHCP option 67. Defaults to 80 (omitted from the URL); choose an
    /// unprivileged high port to avoid needing root for the bind.
    #[arg(long, default_value_t = 80)]
    http_port: u16,

    /// `Content-Type` for HTTP responses. UEFI HTTP Boot treats
    /// `application/octet-stream` as an EFI application (the default).
    #[arg(long)]
    content_type: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .finish();
    tracing::subscriber::set_global_default(subscriber).context("install tracing subscriber")?;

    let cli = Cli::parse();

    if !cli.tftp_root.is_dir() {
        anyhow::bail!("TFTP root {} does not exist", cli.tftp_root.display());
    }

    // Refuse to run on a primary NIC: netbootd reconfigures its interface to the
    // static host IP, which would clobber the host's real networking. The
    // netboot link must be a dedicated secondary interface.
    if let Some(iface) = cli.interface.as_deref() {
        if netcfg::is_primary_interface(iface) {
            anyhow::bail!(
                "refusing to run on {iface}: it carries the system default route \
                 (a primary NIC). netboot would force {} onto it and break host \
                 networking. Use a dedicated USB-Ethernet adapter.",
                cli.host_ip
            );
        }
    }

    info!(
        host_ip = %cli.host_ip,
        tftp_root = %cli.tftp_root.display(),
        boot_file = cli.boot_file,
        interface = cli.interface.as_deref().unwrap_or("-"),
        "netbootd starting"
    );

    // Optional interface-IP enforcement (matches _dhcp.py's monitor thread).
    if let Some(iface) = cli.interface.clone() {
        let host_ip = cli.host_ip;
        tokio::spawn(async move { netcfg::monitor_interface(iface, host_ip).await });
    }

    // In-process DHCP→TFTP client-MAC handoff (replaces the on-disk file).
    let (mac_tx, mac_rx) = tokio::sync::watch::channel::<Option<[u8; 6]>>(None);

    // Raw-frame sender: on macOS the bound /dev/bpf descriptor is obtained from
    // the setuid-root helper via SCM_RIGHTS (netbootd itself stays unprivileged).
    // Linux uses the kernel send path, matching the Python servers. Constructed
    // unconditionally as a type, so the TFTP call sites stay compiled and checked
    // on every platform.
    let bpf = Arc::new(build_bpf_sender(cli.interface.as_deref()));

    let dhcp = tokio::spawn(dhcp::serve(
        cli.host_ip,
        cli.boot_file.clone(),
        cli.interface.clone(),
        cli.dhcp_port,
        cli.http_port,
        mac_tx,
    ));
    let tftp = tokio::spawn(tftp::serve(
        cli.host_ip,
        cli.tftp_root.clone(),
        cli.tftp_port,
        cli.interface.clone(),
        bpf,
        mac_rx,
    ));
    // HTTP serves UEFI HTTP Boot clients over ordinary kernel TCP — no BPF, no
    // ARP pin (a UEFI client answers ARP, unlike the silent Pi). Always on; the
    // client picks TFTP vs HTTP by how it DHCPs.
    let http = tokio::spawn(http::serve(
        cli.tftp_root.clone(),
        cli.http_port,
        cli.content_type.clone(),
    ));

    // Any server task exiting (always an error — they loop forever) or Ctrl-C
    // brings the whole daemon down.
    tokio::select! {
        r = dhcp => match r {
            Ok(Ok(())) => {}
            Ok(Err(e)) => { error!("DHCP server failed: {e:#}"); return Err(e); }
            Err(e) => return Err(e).context("DHCP task panicked"),
        },
        r = tftp => match r {
            Ok(Ok(())) => {}
            Ok(Err(e)) => { error!("TFTP server failed: {e:#}"); return Err(e); }
            Err(e) => return Err(e).context("TFTP task panicked"),
        },
        r = http => match r {
            Ok(Ok(())) => {}
            Ok(Err(e)) => { error!("HTTP server failed: {e:#}"); return Err(e); }
            Err(e) => return Err(e).context("HTTP task panicked"),
        },
        _ = tokio::signal::ctrl_c() => {
            info!("netbootd shutting down");
        }
    }
    Ok(())
}

/// Construct the raw-frame sender. On macOS, request a bound `/dev/bpf`
/// descriptor from the privileged helper and pair it with the interface's MAC
/// (read here, unprivileged). Any failure — no interface, helper missing, helper
/// error — is non-fatal: we log it and return an inert sender so TFTP falls back
/// to the kernel `send_to` path, exactly as on Linux.
fn build_bpf_sender(interface: Option<&str>) -> bpf::BpfSender {
    #[cfg(target_os = "macos")]
    {
        let Some(iface) = interface else {
            return bpf::BpfSender::unavailable();
        };
        let Some(src_mac) = mac_of(iface) else {
            error!("no MAC address for {iface}; BPF disabled, using kernel send_to");
            return bpf::BpfSender::unavailable();
        };
        match netbootd::handoff::request_bpf_fd(iface) {
            Ok(fd) => bpf::BpfSender::from_handoff(fd, src_mac),
            Err(e) => {
                error!(
                    "BPF handoff failed ({e}); falling back to kernel send_to. \
                     Install the privileged helper with `paniolo setup`."
                );
                bpf::BpfSender::unavailable()
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = interface;
        bpf::BpfSender::unavailable()
    }
}

/// Read an interface's hardware (MAC) address. Unprivileged — no `/dev/bpf`
/// involved.
#[cfg(target_os = "macos")]
fn mac_of(iface: &str) -> Option<[u8; 6]> {
    pnet_datalink::interfaces()
        .into_iter()
        .find(|i| i.name == iface)
        .and_then(|i| i.mac)
        .map(|m| [m.0, m.1, m.2, m.3, m.4, m.5])
}
