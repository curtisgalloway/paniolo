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

//! Netboot lifecycle — start/stop/status of the `netbootd` daemon (DHCP + TFTP
//! over the dedicated USB-Ethernet link).
//!
//! Ported from `_netboot.py`, rust engine only (the legacy pure-Python
//! DHCP/TFTP engine stays behind in the Python tree). On macOS netbootd runs
//! unprivileged (its raw-frame send path uses the setuid bpf-helper installed
//! beside it); on Linux ports 67/69 need root so the spawn gets a sudo prefix.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Result};

use crate::daemons;
use crate::netif;
use crate::state::{self, NetbootState};

fn resolve_netbootd() -> Result<std::path::PathBuf> {
    daemons::find_binary("netbootd")
        .ok_or_else(|| anyhow!("netbootd not found — build and install it with `paniolo setup`"))
}

/// Optional UEFI boot parameters forwarded to `netbootd` as flags. All default
/// inside `netbootd` when unset (boot_file → `kernel_2712.img`, http_port → 80,
/// content_type → `application/octet-stream`).
#[derive(Default, Clone)]
pub struct BootOptions {
    pub boot_file: Option<String>,
    pub http_port: Option<String>,
    pub content_type: Option<String>,
}

/// Kill any lingering netbootd from a previous crashed session for `target`.
fn cleanup_stale(target: &str) {
    if let Some(s) = state::load_netboot_state(target) {
        if s.engine == "rust" && state::is_named_child_alive(s.dhcp_pid, "netbootd") {
            unsafe { libc::kill(s.dhcp_pid, libc::SIGTERM) };
        }
    }
    state::remove_netboot_state(target);
}

/// Start netbootd for `target` on `interface`, serving `tftp_root` at `host_ip`.
pub fn start(
    target: &str,
    interface: &str,
    host_ip: &str,
    tftp_root: &str,
    opts: &BootOptions,
) -> Result<()> {
    if state::is_netboot_running(target) {
        bail!("netboot already running for '{target}'");
    }
    if tftp_root.is_empty() {
        bail!("no tftp_root configured (paniolo netboot set -t {target} --tftp-root <path>)");
    }
    if !Path::new(tftp_root).exists() {
        bail!("TFTP root does not exist: {tftp_root}");
    }
    if netif::is_primary_interface(interface) {
        bail!(
            "refusing to start netboot on '{interface}': it carries the system default \
             route (your primary network interface). netboot reconfigures it to \
             {host_ip} and would break host networking. Use a dedicated USB-Ethernet \
             adapter for the netboot link."
        );
    }

    cleanup_stale(target);
    netif::configure_interface(interface, host_ip)?;
    netif::tune_arp_for_silent_client();

    state::ensure_target_dir(target)?;
    let log_path = state::netboot_log_path(target);
    let _ = std::fs::remove_file(&log_path);
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let log_err = log.try_clone()?;

    let netbootd = resolve_netbootd()?;
    // Linux needs root for ports 67/69; sudo resets the env, so NO_COLOR rides
    // through `env` in the prefix. macOS runs unprivileged (bpf-helper).
    let mut cmd = if cfg!(target_os = "macos") || unsafe { libc::getuid() } == 0 {
        let mut c = Command::new(&netbootd);
        c.env("NO_COLOR", "1");
        c
    } else {
        let mut c = Command::new("sudo");
        c.arg("env").arg("NO_COLOR=1").arg(&netbootd);
        c
    };
    cmd.arg("--host-ip")
        .arg(host_ip)
        .arg("--tftp-root")
        .arg(tftp_root)
        .arg("--interface")
        .arg(interface);
    // Optional UEFI boot params; netbootd defaults each when the flag is absent.
    if let Some(bf) = &opts.boot_file {
        cmd.arg("--boot-file").arg(bf);
    }
    if let Some(p) = &opts.http_port {
        cmd.arg("--http-port").arg(p);
    }
    if let Some(ct) = &opts.content_type {
        cmd.arg("--content-type").arg(ct);
    }
    cmd.stdin(Stdio::null()).stdout(log).stderr(log_err);
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let child = cmd.spawn()?;

    state::save_netboot_state(&NetbootState {
        target: target.to_string(),
        // Single process; both pid fields hold the netbootd PID (state-file compat).
        dhcp_pid: child.id() as i32,
        tftp_pid: child.id() as i32,
        started_at: state::now_epoch(),
        interface: interface.to_string(),
        tftp_root: tftp_root.to_string(),
        engine: "rust".to_string(),
    })?;
    Ok(())
}

/// Stop the netboot session for `target` and restore its interface.
pub fn stop(target: &str) -> Result<()> {
    let s = state::load_netboot_state(target)
        .ok_or_else(|| anyhow!("no netboot state for '{target}'"))?;
    for pid in [s.dhcp_pid, s.tftp_pid] {
        if state::is_pid_alive(pid) {
            let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
            if rc != 0 {
                // Likely EPERM (started under sudo on Linux) — escalate.
                let _ = Command::new("sudo")
                    .args(["kill", "-TERM", &pid.to_string()])
                    .status();
            }
        }
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if !state::is_pid_alive(s.dhcp_pid) && !state::is_pid_alive(s.tftp_pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    state::remove_netboot_state(target);
    netif::restore_interface(&s.interface);
    Ok(())
}

pub struct Status {
    pub running: bool,
    pub state: Option<NetbootState>,
    pub uptime_seconds: Option<f64>,
}

pub fn status(target: &str) -> Status {
    let Some(s) = state::load_netboot_state(target) else {
        return Status {
            running: false,
            state: None,
            uptime_seconds: None,
        };
    };
    let alive = state::is_named_child_alive(s.dhcp_pid, "netbootd");
    let uptime = alive.then(|| state::now_epoch() - s.started_at);
    Status {
        running: alive,
        state: Some(s),
        uptime_seconds: uptime,
    }
}
