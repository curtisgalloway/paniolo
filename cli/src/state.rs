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

//! Persistent runtime state — netboot PID records and liveness checks.
//!
//! The JSON shape matches the Python `_state.py` exactly so both CLIs can read
//! each other's state during the migration: for the rust engine both `*_pid`
//! fields hold the single netbootd PID.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetbootState {
    pub target: String,
    pub dhcp_pid: i32,
    pub tftp_pid: i32,
    pub started_at: f64,
    pub interface: String,
    pub tftp_root: String,
    #[serde(default = "default_engine")]
    pub engine: String,
}

fn default_engine() -> String {
    "python".to_string()
}

pub fn state_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".local/share/paniolo")
}

fn target_dir(target: &str) -> PathBuf {
    state_dir().join(target)
}

pub fn netboot_state_path(target: &str) -> PathBuf {
    target_dir(target).join("netboot.json")
}

pub fn netboot_log_path(target: &str) -> PathBuf {
    target_dir(target).join("netboot.log")
}

pub fn ensure_target_dir(target: &str) -> std::io::Result<PathBuf> {
    let d = target_dir(target);
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

pub fn save_netboot_state(state: &NetbootState) -> anyhow::Result<()> {
    ensure_target_dir(&state.target)?;
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(netboot_state_path(&state.target), json)?;
    Ok(())
}

pub fn load_netboot_state(target: &str) -> Option<NetbootState> {
    let text = std::fs::read_to_string(netboot_state_path(target)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn remove_netboot_state(target: &str) {
    let _ = std::fs::remove_file(netboot_state_path(target));
}

/// True if any process with this PID exists (signal-0 probe; EPERM = alive).
pub fn is_pid_alive(pid: i32) -> bool {
    // Safe: kill(pid, 0) only probes for existence.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The full command line of `pid`, or empty on failure.
fn pid_cmdline(pid: i32) -> String {
    if cfg!(target_os = "macos") {
        std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "args="])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    } else {
        std::fs::read(format!("/proc/{pid}/cmdline"))
            .map(|b| {
                String::from_utf8_lossy(&b)
                    .replace('\0', " ")
                    .trim()
                    .to_string()
            })
            .unwrap_or_default()
    }
}

/// True only if `pid` is alive AND its command line mentions `needle` — guards
/// against PID reuse by unrelated processes after a crash.
pub fn is_named_child_alive(pid: i32, needle: &str) -> bool {
    is_pid_alive(pid) && pid_cmdline(pid).contains(needle)
}

/// True only if the netboot process for `target` is alive (rust engine: the
/// single netbootd; legacy python engine: both children).
pub fn is_netboot_running(target: &str) -> bool {
    let Some(state) = load_netboot_state(target) else {
        return false;
    };
    if state.engine == "rust" {
        is_named_child_alive(state.dhcp_pid, "netbootd")
    } else {
        is_named_child_alive(state.dhcp_pid, "paniolo._dhcp")
            && is_named_child_alive(state.tftp_pid, "paniolo._tftp")
    }
}

pub fn now_epoch() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
