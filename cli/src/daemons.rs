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
//! where `<runtime>` is `/tmp/paniolo-<uid>` (see [`runtime_base`]). Liveness
//! is "the recorded pid still exists".

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Result};

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

/// Stable per-user runtime base: `/tmp/paniolo-<uid>`, identical in every
/// environment of the same user. Deliberately NOT `$TMPDIR`/`temp_dir()`
/// (macOS hands each environment a different TMPDIR — GUI terminal vs SSH vs
/// sandboxed agent shells — so a running daemon was invisible from the
/// others) and NOT `$XDG_RUNTIME_DIR` (systemd removes `/run/user/<uid>`
/// when the user's last session ends, breaking daemons that outlive the SSH
/// session that started them). Keep in sync with `runtime_dir()` in
/// hdmicap/src/daemon.rs and serialcap/src/daemon.rs.
fn runtime_base() -> PathBuf {
    // Safe: getuid is always successful.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/paniolo-{uid}"))
}

/// Create (0700) and validate the runtime base, then `<base>/<name>`.
/// The ownership check guards against a squatter pre-creating the /tmp path.
pub fn ensure_runtime_dir(name: &str) -> Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let base = runtime_base();
    match std::fs::DirBuilder::new().mode(0o700).create(&base) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let uid = unsafe { libc::getuid() };
            let md = std::fs::symlink_metadata(&base)?;
            if !md.is_dir() || md.uid() != uid {
                return Err(anyhow!(
                    "{} exists but is not a directory owned by uid {uid}",
                    base.display()
                ));
            }
        }
        Err(e) => return Err(e.into()),
    }
    let dir = base.join(name);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Where a spawned daemon's stderr is captured (truncated on each start).
pub fn log_path(name: &str) -> PathBuf {
    runtime_base().join(name).join("daemon.log")
}

/// Error for a daemon that didn't publish discovery in time, carrying the
/// tail of its stderr log so the failure is diagnosable.
pub fn start_failure(name: &str, timeout: Duration) -> anyhow::Error {
    let log = std::fs::read_to_string(log_path(name)).unwrap_or_default();
    let mut tail: Vec<&str> = log.lines().rev().take(5).collect();
    tail.reverse();
    if tail.is_empty() {
        anyhow!(
            "{name} daemon did not start within {} s (no stderr captured)",
            timeout.as_secs()
        )
    } else {
        anyhow!(
            "{name} daemon did not start within {} s; last stderr:\n  {}",
            timeout.as_secs(),
            tail.join("\n  ")
        )
    }
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
