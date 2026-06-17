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
//! binds localhost (port 0 = OS-assigned), and writes a discovery file
//! `{pid, port, …}` under `<runtime>/<name>[/<target>]/daemon.json`. The
//! optional `<target>` segment lets per-target capture daemons (serialcap,
//! hdmicap, hid) coexist on one host; host-singleton daemons (zigplug,
//! cambrionix, netbootd) omit it (see [`runtime_rel`]). `<runtime>` is
//! `<base>/paniolo-<uid>` where `<base>` honors `$PANIOLO_RUNTIME_BASE`
//! (default `/tmp`; see [`runtime_root`]). Liveness is "the recorded pid
//! still exists".

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

/// The paniolo helper directories, in resolution order: the per-user libexec
/// dir (`~/.local/libexec/paniolo/bin`), then dirs relative to the running CLI
/// (Homebrew keg / prefix install — see [`exe_relative_dirs`]), then the
/// system package dir (`/usr/libexec/paniolo/bin`). These are the only places
/// paniolo ships helpers into: [`find_binary`] searches them (before falling
/// back to `$PATH` and `~/.cargo/bin`), [`hook_path`] prepends them, and
/// `paniolo helper` lists them. A per-user `make install` thus shadows an
/// installed system package. The system dir is always present, so this is
/// never empty.
pub fn helper_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = libexec_dir().into_iter().collect();
    dirs.extend(exe_relative_dirs());
    dirs.push(system_libexec_dir());
    dirs
}

/// Find an installed binary: the paniolo helper dirs first (per-user, then
/// relative to the running CLI — Homebrew keg or other prefix install — then
/// the system package's `/usr/libexec/paniolo/bin`; see [`helper_dirs`]), then
/// $PATH, then ~/.cargo/bin (the pre-libexec install location, kept as a
/// transitional fallback). Never the in-repo build tree, so a running daemon
/// can't point at an ephemeral build artifact.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    for dir in helper_dirs() {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
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
    let mut paths: Vec<PathBuf> = helper_dirs();
    paths.extend(std::env::split_paths(&current));
    std::env::join_paths(paths).unwrap_or(current)
}

/// The temp root beneath which paniolo's per-uid runtime base lives. Honors
/// `$PANIOLO_RUNTIME_BASE` (default `/tmp`), so the location is configurable
/// without resorting to `$TMPDIR`: macOS hands each environment a different
/// TMPDIR (GUI terminal vs SSH vs sandboxed agent shells), which would make a
/// daemon started in one environment invisible to the others — the bug the
/// hardcoded `/tmp` originally fixed. `$XDG_RUNTIME_DIR` is likewise avoided
/// (systemd removes `/run/user/<uid>` when the user's last session ends,
/// breaking daemons that outlive the SSH session that started them).
pub fn runtime_root() -> PathBuf {
    std::env::var_os("PANIOLO_RUNTIME_BASE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Stable per-user runtime base: `<root>/paniolo-<uid>`, identical in every
/// environment of the same user. The per-uid namespace and its 0700 ownership
/// check (see [`ensure_runtime_dir`]) are always applied beneath the root.
/// Keep in sync with `runtime_dir()` in hdmicap/src/daemon.rs and
/// serialcap/src/daemon.rs.
fn runtime_base() -> PathBuf {
    // Safe: getuid is always successful.
    let uid = unsafe { libc::getuid() };
    runtime_root().join(format!("paniolo-{uid}"))
}

/// Sanitize an instance key (a target name, user-chosen) into a single safe
/// path component: keep alphanumerics, `-`, `_`, `.`; collapse anything else
/// to `_`. Mirrors serialcap's interface-name sanitizer. An empty result
/// falls back to `_`.
fn sanitize_component(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "_".to_string()
    } else {
        out
    }
}

/// The runtime subdir for a daemon, relative to the per-uid base: `<name>` for
/// a single-instance daemon, or `<name>/<sanitized-instance>` for a per-target
/// (multi-instance) daemon. Both the local path helpers and the remote
/// discovery lookup (`dispatch::remote_daemon_port`) build paths through this,
/// so a daemon's writer and reader always agree on the location.
pub fn runtime_rel(name: &str, instance: Option<&str>) -> String {
    match instance {
        Some(i) => format!("{name}/{}", sanitize_component(i)),
        None => name.to_string(),
    }
}

/// Create (0700) and validate the runtime base, then `<base>/<name>[/<inst>]`.
/// The ownership check guards against a squatter pre-creating the /tmp path.
/// `instance` is `Some(target)` for per-target capture daemons (serialcap,
/// hdmicap, hid), `None` for host-singleton daemons (zigplug, cambrionix, …).
pub fn ensure_runtime_dir(name: &str, instance: Option<&str>) -> Result<PathBuf> {
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
    let dir = base.join(runtime_rel(name, instance));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Where a spawned daemon's stderr is captured (truncated on each start).
pub fn log_path(name: &str, instance: Option<&str>) -> PathBuf {
    runtime_base()
        .join(runtime_rel(name, instance))
        .join("daemon.log")
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
/// directories created. `instance` is `Some(target)` for per-target capture
/// daemons (so each target gets its own runtime + state dir) and `None` for
/// host-singleton helpers. Failures degrade to omitting the affected var — the
/// helper's own fallback then applies.
pub fn helper_env(name: &str, instance: Option<&str>) -> Vec<(&'static str, PathBuf)> {
    let mut env = Vec::new();
    if let Some(state) = state_base().map(|b| b.join(runtime_rel(name, instance))) {
        if std::fs::create_dir_all(&state).is_ok() {
            env.push(("PANIOLO_STATE_DIR", state));
        }
    }
    if let Ok(runtime) = ensure_runtime_dir(name, instance) {
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
pub fn start_failure(name: &str, instance: Option<&str>, timeout: Duration) -> anyhow::Error {
    let log = std::fs::read_to_string(log_path(name, instance)).unwrap_or_default();
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
    /// Target name for per-target (multi-instance) daemons; `None` for
    /// host-singleton daemons.
    pub instance: Option<String>,
    pub pid: i32,
    pub port: Option<u16>,
    /// Daemon-specific detail (e.g. zigplug's serial device), if published.
    pub detail: String,
}

/// Parse `<dir>/daemon.json` into a live [`DaemonInfo`], or `None` if the file
/// is absent, unparseable, or names a dead pid (stale).
fn read_discovery(
    dir: &std::path::Path,
    name: &str,
    instance: Option<String>,
) -> Option<DaemonInfo> {
    let text = std::fs::read_to_string(dir.join("daemon.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pid = v.get("pid")?.as_i64()?;
    if !pid_alive(pid as i32) {
        return None;
    }
    Some(DaemonInfo {
        name: name.to_string(),
        instance,
        pid: pid as i32,
        port: v.get("port").and_then(|p| p.as_u64()).map(|p| p as u16),
        detail: v
            .get("device")
            .and_then(|d| d.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

/// Every daemon currently publishing a live discovery file. Stale files
/// (dead pid) are skipped, mirroring [`daemon_port`]'s liveness rule. Handles
/// both layouts: `<name>/daemon.json` (host-singleton: zigplug, cambrionix,
/// netbootd) and `<name>/<target>/daemon.json` (per-target: serialcap,
/// hdmicap, hid).
pub fn list_discovered() -> Vec<DaemonInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(runtime_base()) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let dir = entry.path();
        // Host-singleton: a discovery file sits directly in <name>/.
        if let Some(info) = read_discovery(&dir, &name, None) {
            out.push(info);
            continue;
        }
        // Otherwise look one level down for per-target instances.
        let Ok(subs) = std::fs::read_dir(&dir) else {
            continue;
        };
        for sub in subs.flatten() {
            let inst = sub.file_name().to_string_lossy().into_owned();
            if let Some(info) = read_discovery(&sub.path(), &name, Some(inst)) {
                out.push(info);
            }
        }
    }
    out.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.instance.cmp(&b.instance))
    });
    out
}

/// Processes executing out of the libexec dir that are NOT in `exclude_pids` —
/// stray helper invocations (e.g. wedged one-shots holding a serial port).
pub fn list_stray_helpers(exclude_pids: &[i32]) -> Vec<(i32, String)> {
    let needles: Vec<String> = helper_dirs()
        .iter()
        .map(|d| d.to_string_lossy().into_owned())
        .collect();
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
            let from_libexec = needles.iter().any(|n| args.contains(n.as_str()));
            if !from_libexec || pid == me || exclude_pids.contains(&pid) {
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

/// Listen port of the named running daemon instance, or None if it isn't
/// running. `instance` selects a per-target daemon (`None` = host-singleton).
pub fn daemon_port(name: &str, instance: Option<&str>) -> Option<u16> {
    let path = runtime_base()
        .join(runtime_rel(name, instance))
        .join("daemon.json");
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pid = v.get("pid")?.as_i64()? as i32;
    let port = v.get("port")?.as_u64()?;
    if !pid_alive(pid) {
        return None;
    }
    u16::try_from(port).ok()
}

/// Base URL of the named running daemon instance, or None if it isn't running.
pub fn daemon_url(name: &str, instance: Option<&str>) -> Option<String> {
    daemon_port(name, instance).map(|port| format!("http://127.0.0.1:{port}"))
}

/// Block until the named daemon instance answers discovery, or time out.
pub fn wait_for_daemon(name: &str, instance: Option<&str>, timeout: Duration) -> Option<String> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Some(url) = daemon_url(name, instance) {
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

    #[test]
    fn runtime_rel_singleton_vs_per_target() {
        assert_eq!(runtime_rel("zigplug", None), "zigplug");
        assert_eq!(runtime_rel("serialcap", Some("pi5")), "serialcap/pi5");
    }

    #[test]
    fn runtime_rel_sanitizes_instance() {
        // Path separators and other unsafe chars in a target name collapse to
        // `_`, so the instance is always a single path component.
        assert_eq!(runtime_rel("hdmicap", Some("a/b")), "hdmicap/a_b");
        assert_eq!(runtime_rel("hdmicap", Some("../x")), "hdmicap/.._x");
        assert_eq!(runtime_rel("hid", Some("")), "hid/_");
        assert_eq!(runtime_rel("hid", Some("nova-1.2")), "hid/nova-1.2");
    }

    #[test]
    fn runtime_root_honors_env_default_tmp() {
        // Default is /tmp; the override is read live, so just assert the
        // default path shape (the env var is process-global in tests).
        if std::env::var_os("PANIOLO_RUNTIME_BASE").is_none() {
            assert_eq!(runtime_root(), PathBuf::from("/tmp"));
        }
    }
}
