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
//! `{pid, port, token, …}` under `<runtime>/<name>[/<target>]/daemon.json`.
//! `token` is the bearer secret every request to that daemon must carry (see
//! [`Endpoint`]); the file is owner-only, so only the operator's own processes
//! can read it. The
//! optional `<target>` segment lets per-target capture daemons (serialcap,
//! hdmicap, hid) coexist on one host; host-singleton daemons (zigplug,
//! cambrionix, netbootd) omit it (see [`runtime_rel`]). `<runtime>` is
//! `<base>/paniolo-<uid>` where `<base>` honors `$PANIOLO_RUNTIME_BASE`
//! (default `/tmp`; see [`runtime_root`]). Liveness is "the recorded pid
//! still exists".

use std::path::{Path, PathBuf};
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

/// Candidate file names for a helper in a directory.
///
/// Windows requires the `.exe` suffix to execute a file and the packaged
/// helpers carry it, but every caller names helpers bare (`"hdmicap"`), so the
/// suffixed name is tried first and the bare one kept as a fallback for a
/// cross-built or extension-less binary. On Unix there is only ever one name.
fn binary_names(name: &str) -> Vec<String> {
    if cfg!(windows) && !name.ends_with(".exe") {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    }
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
    let mut dirs = vec![
        prefix.join("libexec/bin"),
        prefix.join("libexec/paniolo/bin"),
    ];
    // The portable Windows layout is `paniolo\paniolo.exe` with the helpers in
    // `paniolo\libexec` beside it — the exe's own directory is the install
    // prefix there, since Windows has no bin/libexec split to hang them off.
    if cfg!(windows) {
        if let Some(exe_dir) = exe.parent() {
            dirs.push(exe_dir.join("libexec"));
        }
    }
    dirs
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
    let names = binary_names(name);
    for dir in helper_dirs() {
        for n in &names {
            let p = dir.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            for n in &names {
                let p = dir.join(n);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    let cargo_bin = dirs::home_dir()?.join(".cargo/bin");
    names
        .iter()
        .map(|n| cargo_bin.join(n))
        .find(|p| p.is_file())
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
        .unwrap_or_else(crate::platform::default_runtime_root)
}

/// Stable per-user runtime base: `<root>/paniolo-<uid>`, identical in every
/// environment of the same user. The per-uid namespace and its 0700 ownership
/// check (see [`ensure_runtime_dir`]) are always applied beneath the root.
/// Keep in sync with `runtime_dir()` in hdmicap/src/daemon.rs and
/// serialcap/src/daemon.rs.
fn runtime_base() -> PathBuf {
    let uid = crate::platform::current_uid();
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
/// discovery lookup (`dispatch::remote_daemon_endpoint`) build paths through this,
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
    let base = runtime_base();
    crate::platform::ensure_private_dir(&base)?;
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

/// Create (truncating) the daemon's stderr log at [`log_path`], private to
/// this user, with the runtime dir ensured first. The log is the daemon's
/// tracing output — device paths, remote hostnames, whatever a hook printed —
/// so on Unix it is created 0600 rather than umask-default, the same as
/// serialcap's capture files.
pub fn create_log(name: &str, instance: Option<&str>) -> Result<std::fs::File> {
    let dir = ensure_runtime_dir(name, instance)?;
    let mut o = std::fs::OpenOptions::new();
    o.create(true).write(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    Ok(o.open(dir.join("daemon.log"))?)
}

/// The runtime base, only if it can be trusted: it exists, is a real
/// directory (not a symlink), is owned by this user, and is closed to
/// everyone else — the same conditions [`ensure_runtime_dir`] establishes
/// when it creates the base. `Ok(None)` when it does not exist yet (nothing
/// has ever started on this host); `Err` names the problem. Every reader of
/// discovery files goes through this, so a `daemon.json` planted under a
/// squatted or world-accessible base can never hand the CLI a port to
/// connect to or a pid to signal. A base that is ours but was left too open
/// is tightened to 0700 here, as the writer does; only a symlink or another
/// owner is refused outright.
fn trusted_runtime_base() -> std::result::Result<Option<PathBuf>, String> {
    let base = runtime_base();
    if std::fs::symlink_metadata(&base).is_err() {
        return Ok(None);
    }
    if crate::platform::is_private_dir(&base) {
        return Ok(Some(base));
    }
    // Our own directory left too open — created by an older paniolo, or by
    // the ssh ControlMaster path before it went through the private-dir
    // check. Tighten it exactly as the write path does rather than pretend
    // nothing is running; only a symlink or another owner is refused.
    if crate::platform::ensure_private_dir(&base).is_ok() && crate::platform::is_private_dir(&base)
    {
        Ok(Some(base))
    } else {
        Err(format!(
            "{} is not a private directory owned by this user (a symlink, another \
             owner, or group/world-accessible); ignoring the discovery files under \
             it — `chmod 700` it if it is yours",
            base.display()
        ))
    }
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
    crate::platform::pid_alive(pid)
}

// ── binary staleness ─────────────────────────────────────────────────────────
//
// A daemon is a long-lived process running a snapshot of its helper binary. An
// upgrade (`apt install`, `make install`) or a dev rebuild (`cargo install`)
// replaces that binary on disk, but the running process keeps the old code —
// and its on-disk runtime/protocol contract can diverge from the new CLI's
// (the per-target-daemon move did exactly this, stranding old daemons that
// held capture devices while the new CLI couldn't see them). The package can't
// reap these (they're per-user processes, not packaged services), so the CLI
// records the binary's identity at spawn and flags a running daemon as stale
// once the binary changes underneath it.

/// Identity of the binary a daemon was spawned from. Written next to the
/// daemon's discovery file as `binmeta.json` by [`record_binmeta`].
#[derive(serde::Serialize, serde::Deserialize)]
struct BinMeta {
    /// Helper basename, used to re-resolve the *current* install for comparison.
    bin: String,
    /// Absolute path the daemon was launched from.
    path: String,
    /// Modification time, nanoseconds since the Unix epoch.
    mtime_ns: u128,
    /// Size in bytes.
    size: u64,
}

/// `(mtime_ns, size)` for `path`, or `None` if it can't be stat'd.
fn stat_identity(path: &Path) -> Option<(u128, u64)> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((mtime, md.len()))
}

/// True when the binary now at `current` differs from what `meta` recorded —
/// a different path, or the same path with a changed mtime/size. Pure, so the
/// comparison is unit-testable without spawning anything.
fn meta_differs(meta: &BinMeta, current: &Path) -> bool {
    match stat_identity(current) {
        Some((mtime_ns, size)) => {
            current.to_string_lossy() != meta.path || mtime_ns != meta.mtime_ns || size != meta.size
        }
        // The recorded binary is gone — definitely not what's running now.
        None => true,
    }
}

/// Record the identity of the binary `bin` we are about to spawn for the daemon
/// instance, so [`binary_is_stale`] can later tell whether it changed. Best
/// effort: on any failure staleness simply reports "unknown" (never a false
/// positive), so a stamping hiccup can't block a daemon from starting.
pub fn record_binmeta(bin: &Path, name: &str, instance: Option<&str>) {
    let Some((mtime_ns, size)) = stat_identity(bin) else {
        return;
    };
    let meta = BinMeta {
        bin: bin
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: bin.to_string_lossy().into_owned(),
        mtime_ns,
        size,
    };
    if let Ok(dir) = ensure_runtime_dir(name, instance) {
        if let Ok(text) = serde_json::to_string(&meta) {
            let _ = std::fs::write(dir.join("binmeta.json"), text);
        }
    }
}

/// Whether the daemon instance's binary has changed on disk since it started
/// (an upgrade or rebuild left the running process stale). `None` when it can't
/// be determined — no recorded identity, e.g. a daemon started by an older CLI;
/// callers treat unknown as "not stale" so the signal never cries wolf.
pub fn binary_is_stale(name: &str, instance: Option<&str>) -> Option<bool> {
    let dir = runtime_base().join(runtime_rel(name, instance));
    let text = std::fs::read_to_string(dir.join("binmeta.json")).ok()?;
    let meta: BinMeta = serde_json::from_str(&text).ok()?;
    // Compare against the binary the CLI would resolve today; fall back to the
    // recorded path when the helper no longer resolves by name.
    let current = find_binary(&meta.bin).unwrap_or_else(|| PathBuf::from(&meta.path));
    Some(meta_differs(&meta, &current))
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
    /// `Some(true)` if the binary changed on disk since the daemon started
    /// (upgrade/rebuild — the running process is stale), `Some(false)` if it
    /// matches, `None` if unknown (no recorded identity).
    pub stale: Option<bool>,
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
    let stale = binary_is_stale(name, instance.as_deref());
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
        stale,
    })
}

/// Every daemon currently publishing a live discovery file. Stale files
/// (dead pid) are skipped, mirroring [`daemon_endpoint`]'s liveness rule. Handles
/// both layouts: `<name>/daemon.json` (host-singleton: zigplug, cambrionix,
/// netbootd) and `<name>/<target>/daemon.json` (per-target: serialcap,
/// hdmicap, hid).
pub fn list_discovered() -> Vec<DaemonInfo> {
    let mut out = Vec::new();
    let base = match trusted_runtime_base() {
        Ok(Some(base)) => base,
        Ok(None) => return out,
        Err(why) => {
            // The human-facing inventory is the one place this is said aloud;
            // the per-daemon readers just answer "not running".
            eprintln!("warning: {why}");
            return out;
        }
    };
    let Ok(entries) = std::fs::read_dir(&base) else {
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

/// Processes launched from a paniolo helper dir that are neither in
/// `exclude_pids` nor descended from one of them — stray helper invocations
/// (e.g. a wedged one-shot holding a serial port).
///
/// "Launched from" means the process's program path (argv[0]) sits directly
/// in one of [`helper_dirs`]; the rest of the command line is not consulted,
/// so the `cargo install --root …/libexec/paniolo …` that `make install`
/// runs, an editor, or an `ls` that merely *mentions* the dir is not a stray
/// and is not TERMed by `daemons stop --all`. Descendants of an excluded pid
/// are excluded too, because a recorded pid is not always the helper itself:
/// on Linux netbootd runs under `sudo`, so the recorded pid is sudo's and the
/// real netbootd is its child, and hdmicap spawns its OCR helper the same way.
pub fn list_stray_helpers(exclude_pids: &[i32]) -> Vec<(i32, String)> {
    let Ok(out) = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,args="])
        .output()
    else {
        return Vec::new();
    };
    strays_in_ps(
        &String::from_utf8_lossy(&out.stdout),
        &helper_dirs(),
        std::process::id() as i32,
        exclude_pids,
    )
}

/// One row of `ps -axo pid=,ppid=,args=`: `(pid, ppid, command line)`. The
/// numeric columns are right-aligned, so each is taken up to the next run of
/// blanks rather than a single space.
fn parse_ps_row(line: &str) -> Option<(i32, i32, &str)> {
    let rest = line.trim_start();
    let (pid, rest) = rest.split_once(' ')?;
    let rest = rest.trim_start();
    let (ppid, rest) = rest.split_once(' ')?;
    Some((pid.parse().ok()?, ppid.parse().ok()?, rest.trim()))
}

/// True when argv[0] of `args` is a file directly inside one of `helper_dirs`.
fn launched_from(args: &str, helper_dirs: &[PathBuf]) -> bool {
    let Some(program) = args.split_whitespace().next() else {
        return false;
    };
    Path::new(program)
        .parent()
        .is_some_and(|dir| helper_dirs.iter().any(|d| d.as_path() == dir))
}

/// The classification behind [`list_stray_helpers`], over the text of a
/// `ps` listing: pure, so it is unit-testable with sample rows.
fn strays_in_ps(
    ps: &str,
    helper_dirs: &[PathBuf],
    me: i32,
    exclude_pids: &[i32],
) -> Vec<(i32, String)> {
    let rows: Vec<(i32, i32, &str)> = ps.lines().filter_map(parse_ps_row).collect();
    // Close the excluded set over parent → child, so a helper's own children
    // (sudo's netbootd, hdmicap's OCR helper) go with it.
    let mut excluded: std::collections::HashSet<i32> = exclude_pids.iter().copied().collect();
    loop {
        let before = excluded.len();
        for (pid, ppid, _) in &rows {
            if excluded.contains(ppid) {
                excluded.insert(*pid);
            }
        }
        if excluded.len() == before {
            break;
        }
    }
    rows.into_iter()
        .filter(|(pid, _, args)| {
            *pid != me && !excluded.contains(pid) && launched_from(args, helper_dirs)
        })
        .map(|(pid, _, args)| (pid, args.to_string()))
        .collect()
}

/// A running daemon's HTTP endpoint, as read from its discovery file: the
/// loopback port and the bearer token every request must carry. `token` is
/// `None` for a daemon started by a paniolo older than the token — it accepts
/// unauthenticated requests, and `paniolo daemons restart --stale` replaces
/// it with one that does not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub pid: i32,
    pub port: u16,
    pub token: Option<String>,
}

impl Endpoint {
    /// Parse a `daemon.json` body (read locally, or fetched over SSH by
    /// `dispatch::remote_daemon_endpoint`). `None` without a pid and a port.
    /// Liveness is the caller's concern.
    pub fn from_json(text: &str) -> Option<Endpoint> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let pid = i32::try_from(v.get("pid")?.as_i64()?).ok()?;
        let port = u16::try_from(v.get("port")?.as_u64()?).ok()?;
        let token = v
            .get("token")
            .and_then(|t| t.as_str())
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        Some(Endpoint { pid, port, token })
    }

    /// `http://127.0.0.1:<port>` — the base for API calls.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// A GET of `path` (which may carry a query) with the token attached as
    /// `Authorization: Bearer`.
    pub fn get(&self, path: &str) -> ureq::Request {
        self.authorize(ureq::get(&format!("{}{path}", self.base_url())))
    }

    /// A POST to `path`, likewise.
    pub fn post(&self, path: &str) -> ureq::Request {
        self.authorize(ureq::post(&format!("{}{path}", self.base_url())))
    }

    fn authorize(&self, req: ureq::Request) -> ureq::Request {
        match &self.token {
            Some(t) => req.set("Authorization", &format!("Bearer {t}")),
            None => req,
        }
    }

    /// `scheme://127.0.0.1:<port><path>` with the token as a `token=` query
    /// parameter — the form a browser needs, since a page load, an `<img>`
    /// or a WebSocket upgrade cannot set a header. `path` may already carry
    /// a query.
    fn url_with_token(&self, scheme: &str, path: &str) -> String {
        let mut url = format!("{scheme}://127.0.0.1:{}{path}", self.port);
        if let Some(t) = &self.token {
            url.push(if path.contains('?') { '&' } else { '?' });
            url.push_str("token=");
            url.push_str(t);
        }
        url
    }

    /// The URL a human opens in a browser (the dashboard is `GET /`).
    pub fn http_url(&self, path: &str) -> String {
        self.url_with_token("http", path)
    }

    /// The URL the dashboard page connects to (`/stream`, `/hid`).
    pub fn ws_url(&self, path: &str) -> String {
        self.url_with_token("ws", path)
    }
}

/// `<dir>/daemon.json` as a live [`Endpoint`], or None if the file is absent,
/// unparseable, or names a dead pid.
fn endpoint_at(dir: &Path) -> Option<Endpoint> {
    let text = std::fs::read_to_string(dir.join("daemon.json")).ok()?;
    let ep = Endpoint::from_json(&text)?;
    pid_alive(ep.pid).then_some(ep)
}

/// The named running daemon instance's endpoint (port + token), or None if
/// it isn't running. `instance` selects a per-target daemon (`None` =
/// host-singleton).
pub fn daemon_endpoint(name: &str, instance: Option<&str>) -> Option<Endpoint> {
    let base = trusted_runtime_base().ok().flatten()?;
    endpoint_at(&base.join(runtime_rel(name, instance)))
}

/// PID of the named running daemon instance, or None if it isn't running.
pub fn daemon_pid(name: &str, instance: Option<&str>) -> Option<i32> {
    daemon_endpoint(name, instance).map(|ep| ep.pid)
}

/// Base URL of the named running daemon instance, or None if it isn't running.
pub fn daemon_url(name: &str, instance: Option<&str>) -> Option<String> {
    daemon_endpoint(name, instance).map(|ep| ep.base_url())
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
    fn meta_differs_detects_a_changed_binary() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("paniolo-binmeta-test-{}", std::process::id()));
        std::fs::write(&path, b"v1").unwrap();
        let (mtime_ns, size) = stat_identity(&path).unwrap();
        let meta = BinMeta {
            bin: "test-bin".to_string(),
            path: path.to_string_lossy().into_owned(),
            mtime_ns,
            size,
        };
        // Unchanged file: not stale.
        assert!(!meta_differs(&meta, &path));
        // A different path (even if it exists) counts as changed.
        let other = dir.join(format!("paniolo-binmeta-other-{}", std::process::id()));
        std::fs::write(&other, b"v1").unwrap();
        assert!(meta_differs(&meta, &other));
        // Replacing the file (new size, new mtime) is stale.
        std::fs::write(&path, b"v2-larger").unwrap();
        assert!(meta_differs(&meta, &path));
        // A vanished binary is stale.
        std::fs::remove_file(&path).unwrap();
        assert!(meta_differs(&meta, &path));
        std::fs::remove_file(&other).ok();
    }

    /// Helper lookup has to try `<name>.exe` on Windows: every caller names
    /// helpers bare (`"hdmicap"`), and the packaged binaries carry the suffix,
    /// so without this no helper resolves at all on a Windows install.
    #[test]
    fn binary_names_adds_the_windows_suffix() {
        let names = binary_names("hdmicap");
        if cfg!(windows) {
            assert_eq!(names, vec!["hdmicap.exe", "hdmicap"]);
        } else {
            assert_eq!(names, vec!["hdmicap"]);
        }
        // An explicit .exe is never doubled up.
        assert_eq!(binary_names("hdmicap.exe"), vec!["hdmicap.exe"]);
    }

    /// The lookup is exercised through $PATH, which `find_binary` searches on
    /// every platform — so this runs the real resolution code rather than
    /// asserting on the shape of a name.
    #[test]
    fn find_binary_resolves_a_helper_through_path() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "paniolo-fake-helper.exe"
        } else {
            "paniolo-fake-helper"
        };
        let path = dir.path().join(name);
        std::fs::write(&path, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }

        let prev = std::env::var_os("PATH");
        let mut paths = vec![dir.path().to_path_buf()];
        if let Some(p) = &prev {
            paths.extend(std::env::split_paths(p));
        }
        // Safe: single-threaded test process; PATH is restored below.
        unsafe { std::env::set_var("PATH", std::env::join_paths(paths).unwrap()) };

        // Named bare, as every caller names helpers.
        let found = find_binary("paniolo-fake-helper");

        match prev {
            Some(p) => unsafe { std::env::set_var("PATH", p) },
            None => unsafe { std::env::remove_var("PATH") },
        }

        assert_eq!(
            found.as_deref(),
            Some(path.as_path()),
            "a bare helper name must resolve to the real file"
        );
    }

    #[test]
    fn endpoint_parses_the_token_and_tolerates_its_absence() {
        let ep = Endpoint::from_json(r#"{"pid":42,"port":8724,"token":"abc"}"#).unwrap();
        assert_eq!(
            ep,
            Endpoint {
                pid: 42,
                port: 8724,
                token: Some("abc".into())
            }
        );
        // A daemon older than the token: still discoverable, no credential.
        let old = Endpoint::from_json(r#"{"pid":42,"port":8724}"#).unwrap();
        assert_eq!(old.token, None);
        assert!(Endpoint::from_json(r#"{"pid":42}"#).is_none());
        assert!(Endpoint::from_json("not json").is_none());
    }

    #[test]
    fn endpoint_urls_carry_the_token_only_where_a_browser_needs_it() {
        let ep = Endpoint {
            pid: 1,
            port: 5555,
            token: Some("t0k".into()),
        };
        assert_eq!(ep.base_url(), "http://127.0.0.1:5555");
        assert_eq!(ep.http_url("/"), "http://127.0.0.1:5555/?token=t0k");
        assert_eq!(
            ep.ws_url("/stream?interface=console"),
            "ws://127.0.0.1:5555/stream?interface=console&token=t0k"
        );
        let old = Endpoint {
            token: None,
            ..ep.clone()
        };
        assert_eq!(old.ws_url("/hid"), "ws://127.0.0.1:5555/hid");
    }

    /// The liveness rule, executed against a real file: the current process's
    /// pid is alive, pid 0 never is.
    #[test]
    fn endpoint_at_applies_the_liveness_rule() {
        let dir = tempfile::tempdir().unwrap();
        let me = std::process::id();
        std::fs::write(
            dir.path().join("daemon.json"),
            format!(r#"{{"pid":{me},"port":7,"token":"x"}}"#),
        )
        .unwrap();
        let ep = endpoint_at(dir.path()).unwrap();
        assert_eq!((ep.port, ep.token.as_deref()), (7, Some("x")));
        std::fs::write(
            dir.path().join("daemon.json"),
            r#"{"pid":0,"port":7,"token":"x"}"#,
        )
        .unwrap();
        assert!(endpoint_at(dir.path()).is_none());
    }

    /// The request builders put the bearer header on the wire — checked
    /// against a real loopback listener, not by inspecting a string.
    #[test]
    fn endpoint_requests_carry_the_bearer_header() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap();
            let _ =
                s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        let ep = Endpoint {
            pid: 1,
            port,
            token: Some("t0k".into()),
        };
        let resp = ep
            .get("/status?interface=console")
            .timeout(Duration::from_secs(5))
            .call()
            .unwrap();
        assert_eq!(resp.status(), 200);
        let req = server.join().unwrap();
        assert!(
            req.starts_with("GET /status?interface=console HTTP/1.1"),
            "{req}"
        );
        assert!(
            req.lines()
                .any(|l| l.eq_ignore_ascii_case("authorization: Bearer t0k")),
            "no bearer header in:\n{req}"
        );
    }

    #[test]
    fn runtime_root_honors_env_default_tmp() {
        // With no override, the root is the platform default: the hardcoded
        // /tmp on Unix, the per-user temp dir on Windows. The override is read
        // live, so only assert the shape (the env var is process-global in
        // tests; the lock keeps `with_runtime_root` from flipping it mid-way).
        let _guard = RUNTIME_BASE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if std::env::var_os("PANIOLO_RUNTIME_BASE").is_none() {
            assert_eq!(runtime_root(), crate::platform::default_runtime_root());
            if cfg!(unix) {
                assert_eq!(runtime_root(), PathBuf::from("/tmp"));
            }
        }
    }

    /// `PANIOLO_RUNTIME_BASE` is process-global and the test threads run
    /// concurrently, so every test that points it somewhere holds this.
    static RUNTIME_BASE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Run `f` with `PANIOLO_RUNTIME_BASE` pointed at a fresh scratch root
    /// (so `runtime_base()` is `<root>/paniolo-<uid>`), restoring the
    /// previous value afterwards.
    fn with_runtime_root<R>(f: impl FnOnce(&Path) -> R) -> R {
        let _guard = RUNTIME_BASE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("PANIOLO_RUNTIME_BASE");
        // Safe: serialized by the lock above; restored below.
        unsafe { std::env::set_var("PANIOLO_RUNTIME_BASE", root.path()) };
        let r = f(root.path());
        match prev {
            Some(p) => unsafe { std::env::set_var("PANIOLO_RUNTIME_BASE", p) },
            None => unsafe { std::env::remove_var("PANIOLO_RUNTIME_BASE") },
        }
        r
    }

    fn expected_base(root: &Path) -> PathBuf {
        root.join(format!("paniolo-{}", crate::platform::current_uid()))
    }

    /// A discovery file naming *this* process, so the liveness check passes
    /// and only the base check decides whether the readers trust it.
    fn plant_discovery(base: &Path, rel: &str, port: u16) {
        let dir = base.join(rel);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("daemon.json"),
            format!(r#"{{"pid":{},"port":{port}}}"#, std::process::id()),
        )
        .unwrap();
    }

    /// The readers see a daemon when the base is the private directory the
    /// writer creates — the everyday path, which must keep working with the
    /// trust check in front of it.
    #[test]
    fn readers_trust_a_private_base_we_own() {
        with_runtime_root(|root| {
            ensure_runtime_dir("serialcap", Some("pi5")).unwrap();
            plant_discovery(&expected_base(root), "serialcap/pi5", 4321);

            assert_eq!(daemon_endpoint("serialcap", Some("pi5")).map(|ep| ep.port), Some(4321));
            assert_eq!(
                daemon_pid("serialcap", Some("pi5")),
                Some(std::process::id() as i32)
            );
            let listed = list_discovered();
            assert_eq!(listed.len(), 1, "one live daemon");
            assert_eq!(listed[0].name, "serialcap");
            assert_eq!(listed[0].instance.as_deref(), Some("pi5"));
            assert_eq!(listed[0].port, Some(4321));
        });
    }

    /// A symlink where the base should be — a squatter's, pointing wherever
    /// they like — makes every reader answer "not running", even though the
    /// discovery file behind it names a live pid.
    #[cfg(unix)]
    #[test]
    fn readers_ignore_a_symlinked_base() {
        use std::os::unix::fs::PermissionsExt;
        with_runtime_root(|root| {
            let real = root.join("elsewhere");
            std::fs::create_dir(&real).unwrap();
            std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
            plant_discovery(&real, "serialcap/pi5", 4321);
            std::os::unix::fs::symlink(&real, expected_base(root)).unwrap();

            assert_eq!(daemon_endpoint("serialcap", Some("pi5")).map(|ep| ep.port), None);
            assert_eq!(daemon_pid("serialcap", Some("pi5")), None);
            assert!(list_discovered().is_empty());
        });
    }

    /// A base we own but left group/world-accessible is refused by the
    /// readers: a base we own that was left too open (an older paniolo, or
    /// the ssh ControlMaster path) is tightened to 0700 and then trusted, so
    /// an upgrade never makes a running daemon look stopped.
    #[cfg(unix)]
    #[test]
    fn readers_tighten_an_open_base_we_own() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        with_runtime_root(|root| {
            let base = expected_base(root);
            std::fs::create_dir(&base).unwrap();
            std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
            plant_discovery(&base, "serialcap/pi5", 4321);

            assert_eq!(daemon_endpoint("serialcap", Some("pi5")).map(|ep| ep.port), Some(4321));
            assert_eq!(
                std::fs::metadata(&base).unwrap().mode() & 0o777,
                0o700,
                "reader tightened the base"
            );
            assert!(daemon_pid("serialcap", Some("pi5")).is_some());
        });
    }

    /// The daemon's stderr log is created readable by nobody else.
    #[cfg(unix)]
    #[test]
    fn daemon_log_is_created_private() {
        use std::os::unix::fs::MetadataExt;
        with_runtime_root(|_| {
            let log = create_log("serialcap", Some("pi5")).unwrap();
            assert_eq!(log.metadata().unwrap().mode() & 0o777, 0o600);
            assert!(log_path("serialcap", Some("pi5")).is_file());
        });
    }

    /// Only argv[0] decides whether a process is a stray helper. Rows whose
    /// *arguments* mention the helper dir — the `cargo install --root` that
    /// `make install` runs, an `ls`, an editor — used to match, and
    /// `daemons stop --all` then TERMed them.
    #[test]
    fn strays_match_the_program_path_not_the_arguments() {
        let dirs = vec![PathBuf::from("/opt/paniolo/libexec/bin")];
        let ps = "\
  100     1 /opt/paniolo/libexec/bin/zigplug -d /dev/ttyUSB0 on 1
  101     1 cargo install --path zigplug --root /opt/paniolo/libexec
  102     1 ls -l /opt/paniolo/libexec/bin
  103     1 vim /opt/paniolo/libexec/bin/notes.txt
  104     1 /opt/paniolo/libexec/binx/other
  105     1 /opt/paniolo/libexec/bin/sub/deeper
";
        let pids: Vec<i32> = strays_in_ps(ps, &dirs, 999, &[])
            .into_iter()
            .map(|(pid, _)| pid)
            .collect();
        assert_eq!(pids, vec![100]);
    }

    /// Our own pid, the known daemons, and anything descended from a known
    /// daemon are not strays: on Linux the recorded netboot pid is `sudo`'s
    /// and the real netbootd is its child; hdmicap's OCR helper is likewise
    /// a child of a known daemon. The wedged one-shot is what remains.
    #[test]
    fn strays_exclude_us_known_pids_and_their_descendants() {
        let dirs = vec![PathBuf::from("/usr/libexec/paniolo/bin")];
        let ps = "\
  200     1 sudo env NO_COLOR=1 /usr/libexec/paniolo/bin/netbootd --host-ip 192.168.99.1
  201   200 /usr/libexec/paniolo/bin/netbootd --host-ip 192.168.99.1
  202   201 /usr/libexec/paniolo/bin/bpf-helper
  300     1 /usr/libexec/paniolo/bin/hdmicap daemon --device /dev/video0
  301   300 /usr/libexec/paniolo/bin/linuxocr
  400     1 /usr/libexec/paniolo/bin/zigplug -d /dev/ttyUSB1 on 1
  500     1 /usr/libexec/paniolo/bin/serialcap stop
";
        let strays = strays_in_ps(ps, &dirs, 500, &[200, 300]);
        assert_eq!(
            strays,
            vec![(
                400,
                "/usr/libexec/paniolo/bin/zigplug -d /dev/ttyUSB1 on 1".to_string()
            )]
        );
    }
}
