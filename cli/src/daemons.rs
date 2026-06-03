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

//! Shared plumbing for paniolo's per-subsystem daemons (serialcap, hdmicap).
//!
//! Every daemon follows the same contract: it is an installed binary (PATH or
//! `~/.cargo/bin`), binds localhost (port 0 = OS-assigned), and writes a
//! discovery file `<runtime>/<name>/daemon.json` containing `{pid, port, …}`
//! where `<runtime>` is `$XDG_RUNTIME_DIR` (else the temp dir). Liveness is
//! "the recorded pid still exists".

use std::path::PathBuf;
use std::time::Duration;

/// Find an installed binary: $PATH first, then ~/.cargo/bin (where
/// `paniolo setup` installs the daemons). Never the in-repo build tree, so a
/// running daemon can't point at an ephemeral build artifact.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let cargo = dirs::home_dir()?.join(".cargo/bin").join(name);
    cargo.is_file().then_some(cargo)
}

fn runtime_base() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn pid_alive(pid: i32) -> bool {
    // Safe: kill(pid, 0) only probes for existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Base URL of the named running daemon, or None if it isn't running.
pub fn daemon_url(name: &str) -> Option<String> {
    let path = runtime_base().join(name).join("daemon.json");
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pid = v.get("pid")?.as_i64()? as i32;
    let port = v.get("port")?.as_u64()?;
    if !pid_alive(pid) {
        return None;
    }
    Some(format!("http://127.0.0.1:{port}"))
}

/// Block until the named daemon answers discovery, or time out.
pub fn wait_for_daemon(name: &str, timeout: Duration) -> Option<String> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Some(url) = daemon_url(name) {
            return Some(url);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}
