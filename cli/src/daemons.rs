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
//! Every daemon follows the same contract: it is an installed binary (the
//! paniolo libexec dir, PATH, or legacy `~/.cargo/bin` — see [`find_binary`]),
//! binds localhost (port 0 = OS-assigned), and writes a
//! discovery file `<runtime>/<name>/daemon.json` containing `{pid, port, …}`
//! where `<runtime>` is `/tmp/paniolo-<uid>` (see [`runtime_base`]). Liveness
//! is "the recorded pid still exists".

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Result};

/// The cargo-install root for paniolo's helper binaries. Binaries land in
/// `<root>/bin` (cargo appends `bin/` itself) — see [`libexec_dir`].
pub fn libexec_root() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".local/libexec/paniolo"))
}

/// Private helper dir (libexec): `~/.local/libexec/paniolo/bin`. The helpers
/// (hdmicap, serialcap, netbootd, cambrionix, hidrig, zigplug, visionocr, …)
/// live here rather than on PATH — they are paniolo's plumbing, invoked by
/// paniolo (or explicitly via `paniolo helper <name> …`), not user commands.
pub fn libexec_dir() -> Option<PathBuf> {
    libexec_root().map(|r| r.join("bin"))
}

/// Helper dir used by the Linux system packages (.deb/tarball installs to a
/// system prefix): `/usr/libexec/paniolo/bin`. Searched after the per-user
/// libexec dir, so a `make install` build overrides an installed package.
pub fn system_libexec_dir() -> PathBuf {
    PathBuf::from("/usr/libexec/paniolo/bin")
}

/// Helper dirs relative to the running CLI binary, after resolving symlinks
/// (Homebrew links `<prefix>/bin/paniolo` into the versioned keg):
/// `../libexec/bin` (Homebrew keg layout) and `../libexec/paniolo/bin`
/// (FHS-style prefix). A relocated install is self-locating without
/// enumerating package managers. Deliberately NOT the exe's own dir: for a
/// `make install` CLI that is `~/.cargo/bin`, the legacy location that must
/// stay a last-resort fallback.
fn exe_relative_dirs() -> Vec<PathBuf> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let Some(prefix) = exe.parent().and_then(|d| d.parent()) else {
        return Vec::new();
    };
    vec![
        prefix.join("libexec/bin"),
        prefix.join("libexec/paniolo/bin"),
    ]
}

/// Find an installed binary: the paniolo libexec dirs first (per-user, then
/// relative to the running CLI — Homebrew keg or other prefix install — then
/// the system package's `/usr/libexec/paniolo/bin`), then $PATH, then
/// ~/.cargo/bin (the pre-libexec install location, kept as a transitional
/// fallback). Never the in-repo build tree, so a running daemon can't point
/// at an ephemeral build artifact.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    if let Some(p) = libexec_dir().map(|d| d.join(name)) {
        if p.is_file() {
            return Some(p);
        }
    }
    for dir in exe_relative_dirs() {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    let p = system_libexec_dir().join(name);
    if p.is_file() {
        return Some(p);
    }
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

/// PATH value with the libexec dir prepended, for `sh -c` hook commands
/// (power on/off/cycle/state, hid cmd). Lab files reference helpers by bare
/// name (`zigplug …`, `cambrionix …`); prepending libexec keeps those names
/// resolving without the helpers being user-visible on PATH.
pub fn hook_path() -> std::ffi::OsString {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<PathBuf> = libexec_dir().into_iter().collect();
    paths.extend(exe_relative_dirs());
    paths.push(system_libexec_dir());
    paths.extend(std::env::split_paths(&current));
    std::env::join_paths(paths).unwrap_or(current)
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

// ── helper state/runtime-dir API ────────────────────────────────────────────
//
// Helpers must not invent their own paths (a helper writing unnamespaced
// state into ~/.config/paniolo/ collides with the lab file and each other),
// and must not re-implement the runtime-base logic above. Paniolo is the
// single source of truth: every invocation of a helper — `paniolo helper`,
// hook commands, daemon spawns — carries two environment variables:
//
//   PANIOLO_STATE_DIR    ~/.config/paniolo/helpers/<name>   durable state
//   PANIOLO_RUNTIME_DIR  /tmp/paniolo-<uid>/<name>          discovery, locks,
//                                                           logs (wiped on boot)
//
// Both directories exist by the time the helper runs. Helpers should prefer
// these over hand-rolled paths, falling back to the same literal locations
// when run standalone (documented in docs/adding-power-helpers.md).

/// Durable per-helper state base: `~/.config/paniolo/helpers`.
pub fn state_base() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".config/paniolo/helpers"))
}

/// The `(var, value)` environment pairs for invoking helper `name`, with both
/// directories created. Failures degrade to omitting the affected var — the
/// helper's own fallback then applies.
pub fn helper_env(name: &str) -> Vec<(&'static str, PathBuf)> {
    let mut env = Vec::new();
    if let Some(state) = state_base().map(|b| b.join(name)) {
        if std::fs::create_dir_all(&state).is_ok() {
            env.push(("PANIOLO_STATE_DIR", state));
        }
    }
    if let Ok(runtime) = ensure_runtime_dir(name) {
        env.push(("PANIOLO_RUNTIME_DIR", runtime));
    }
    env
}

/// The helper name for an opaque hook command: the basename of its first
/// shell token (`zigplug -d … on …` → `zigplug`, `/path/to/script.sh …` →
/// `script.sh`). Hooks are opaque strings, so this is a convention, not an
/// inspection — documented in docs/adding-power-helpers.md.
pub fn hook_helper_name(cmd: &str) -> Option<String> {
    let first = cmd.split_whitespace().next()?;
    let name = first.rsplit('/').next()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
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

/// One live daemon found via its discovery file under the runtime base.
pub struct DaemonInfo {
    /// Discovery dir name (serialcap, hdmicap, hid, zigplug, …).
    pub name: String,
    pub pid: i32,
    pub port: Option<u16>,
    /// Daemon-specific detail (e.g. zigplug's serial device), if published.
    pub detail: String,
}

/// Every daemon currently publishing a live discovery file. Stale files
/// (dead pid) are skipped, mirroring [`daemon_port`]'s liveness rule.
pub fn list_discovered() -> Vec<DaemonInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(runtime_base()) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path().join("daemon.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(pid) = v.get("pid").and_then(|p| p.as_i64()) else {
            continue;
        };
        if !pid_alive(pid as i32) {
            continue;
        }
        out.push(DaemonInfo {
            name: entry.file_name().to_string_lossy().into_owned(),
            pid: pid as i32,
            port: v.get("port").and_then(|p| p.as_u64()).map(|p| p as u16),
            detail: v
                .get("device")
                .and_then(|d| d.as_str())
                .unwrap_or_default()
                .to_string(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Processes executing out of the libexec dir that are NOT in `exclude_pids` —
/// stray helper invocations (e.g. wedged one-shots holding a serial port).
pub fn list_stray_helpers(exclude_pids: &[i32]) -> Vec<(i32, String)> {
    let Some(libexec) = libexec_dir() else {
        return Vec::new();
    };
    let needle = libexec.to_string_lossy().into_owned();
    let Ok(out) = std::process::Command::new("ps")
        .args(["-axo", "pid=,args="])
        .output()
    else {
        return Vec::new();
    };
    let me = std::process::id() as i32;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid_s, args) = line.split_once(' ')?;
            let pid: i32 = pid_s.parse().ok()?;
            if !args.contains(&needle) || pid == me || exclude_pids.contains(&pid) {
                return None;
            }
            Some((pid, args.trim().to_string()))
        })
        .collect()
}

/// Send `signal` to `pid` (best-effort).
pub fn signal_pid(pid: i32, signal: i32) {
    // Safe: sending a signal to a pid we just enumerated; failure is fine.
    unsafe {
        libc::kill(pid, signal);
    }
}

/// Listen port of the named running daemon, or None if it isn't running.
pub fn daemon_port(name: &str) -> Option<u16> {
    let path = runtime_base().join(name).join("daemon.json");
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pid = v.get("pid")?.as_i64()? as i32;
    let port = v.get("port")?.as_u64()?;
    if !pid_alive(pid) {
        return None;
    }
    u16::try_from(port).ok()
}

/// Base URL of the named running daemon, or None if it isn't running.
pub fn daemon_url(name: &str) -> Option<String> {
    daemon_port(name).map(|port| format!("http://127.0.0.1:{port}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_helper_name_takes_first_token_basename() {
        assert_eq!(
            hook_helper_name("zigplug -d /dev/x on 1").as_deref(),
            Some("zigplug")
        );
        assert_eq!(
            hook_helper_name("/usr/local/bin/script.sh --flag").as_deref(),
            Some("script.sh")
        );
        assert_eq!(
            hook_helper_name("  cambrionix state 4").as_deref(),
            Some("cambrionix")
        );
        assert_eq!(hook_helper_name(""), None);
        assert_eq!(hook_helper_name("   "), None);
    }
}
