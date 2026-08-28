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

//! The POSIX facilities paniolo depends on, behind a portable surface.
//!
//! Paniolo grew up on macOS and Linux, so process and filesystem primitives
//! were reached for directly (`libc::getuid`, `libc::kill`, `CommandExt::exec`,
//! `DirBuilderExt::mode`). Windows has none of those. Rather than sprinkle
//! `#[cfg]` through the call sites, every such primitive lives here with one
//! implementation per platform and a single documented contract.
//!
//! The same module is duplicated (deliberately, matching how `daemon.rs` is
//! duplicated) in the helper crates that run daemons — hdmicap, serialcap,
//! ch9329, hidrig. Keep the four in sync.

use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Result};

/// Which signal [`signal_pid`] should deliver.
///
/// Windows has no signals: both variants land on `TerminateProcess`, which is
/// unconditional. A Windows daemon therefore never runs its graceful-shutdown
/// path (discovery-file removal, the 300 ms grace) — it is killed outright, and
/// a stale discovery file may be left behind for the next `pid_alive` probe to
/// reap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Polite stop: SIGTERM on Unix.
    Term,
    /// Unconditional kill: SIGKILL on Unix.
    Kill,
}

// ── identity ────────────────────────────────────────────────────────────────

/// A stable per-user id used to namespace the runtime base (`paniolo-<id>`).
///
/// On Unix this is the real uid. Windows has no uid, so we hash the username
/// (FNV-1a, truncated) — the value is meaningless on its own but is stable for
/// a given user across sessions and reboots, which is all the runtime path
/// needs. It is never used for an authorization decision.
#[cfg(unix)]
pub fn current_uid() -> u32 {
    // Safe: getuid() has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(windows)]
pub fn current_uid() -> u32 {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "unknown".to_string());
    let mut h: u32 = 0x811c_9dc5;
    for b in user.to_ascii_lowercase().bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

// ── runtime directories ─────────────────────────────────────────────────────

/// Default root beneath which the per-user runtime base is created, when
/// `$PANIOLO_RUNTIME_BASE` is unset.
///
/// Unix uses a hardcoded `/tmp` rather than `$TMPDIR`, because macOS hands each
/// environment a different TMPDIR (GUI terminal vs SSH vs a sandboxed agent
/// shell) and a daemon started in one would be invisible from the others.
/// Windows has no such problem: `std::env::temp_dir()` resolves to
/// `%LOCALAPPDATA%\Temp`, which is per-user, stable across sessions, and
/// already ACL'd to that user — so it is both the right location and the
/// safe one.
#[cfg(unix)]
pub fn default_runtime_root() -> std::path::PathBuf {
    std::path::PathBuf::from("/tmp")
}

#[cfg(windows)]
pub fn default_runtime_root() -> std::path::PathBuf {
    std::env::temp_dir()
}

/// Create `base` as a private, user-owned directory, or validate it if it
/// already exists.
///
/// On Unix this is a 0700 create plus an ownership check, guarding against a
/// squatter pre-creating the well-known `/tmp` path. On Windows the parent is
/// inside the user's own profile, whose inherited ACL already denies other
/// non-administrative users, so the check reduces to "exists and is a
/// directory". (An administrator can write anywhere on either platform; that
/// is out of scope for both.)
#[cfg(unix)]
pub fn ensure_private_dir(base: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    match std::fs::DirBuilder::new().mode(0o700).create(base) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let uid = current_uid();
            let md = std::fs::symlink_metadata(base)?;
            if !md.is_dir() || md.uid() != uid {
                return Err(anyhow!(
                    "{} exists but is not a directory owned by uid {uid}",
                    base.display()
                ));
            }
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(windows)]
pub fn ensure_private_dir(base: &Path) -> Result<()> {
    match std::fs::create_dir(base) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let md = std::fs::symlink_metadata(base)?;
            if !md.is_dir() {
                return Err(anyhow!("{} exists but is not a directory", base.display()));
            }
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

/// Mark `path` executable. A no-op on Windows, where executability comes from
/// the file extension rather than a permission bit.
#[cfg(unix)]
pub fn make_executable(path: &Path) -> std::io::Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(path, perms)
}

#[cfg(windows)]
pub fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

// ── process lifecycle ───────────────────────────────────────────────────────

/// True if a process with this pid exists.
///
/// Unix uses the signal-0 probe, treating `EPERM` as alive (the process exists
/// but belongs to another user). Windows opens the process for a
/// limited-information query and asks whether it has an exit code yet; a
/// successful open that reports `STILL_ACTIVE` is the equivalent answer, and
/// `ERROR_ACCESS_DENIED` likewise means the pid is live but not ours.
#[cfg(unix)]
pub fn pid_alive(pid: i32) -> bool {
    // Safe: kill(pid, 0) only probes for existence.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
pub fn pid_alive(pid: i32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    if pid <= 0 {
        return false;
    }
    // Safe: OpenProcess validates the pid itself and returns null on failure.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
        if h.is_null() {
            // Access-denied means the process exists, we just can't open it.
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code: u32 = 0;
        let ok = windows_sys::Win32::System::Threading::GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

/// True if this process runs with superuser rights.
///
/// Used only to decide whether a privileged-port helper needs a `sudo` prefix.
/// Windows has no `sudo` and no caller that needs this, so it answers `false`
/// and the caller takes the unprivileged path.
#[cfg(unix)]
pub fn is_superuser() -> bool {
    current_uid() == 0
}

#[cfg(windows)]
pub fn is_superuser() -> bool {
    false
}

/// Like [`signal_pid`], but reports whether the signal was delivered, so the
/// caller can escalate (e.g. re-send under `sudo`) when it was not.
#[cfg(unix)]
pub fn try_signal_pid(pid: i32, signal: Signal) -> bool {
    let sig = match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    // Safe: kill() on an arbitrary pid is defined; we only read the result.
    unsafe { libc::kill(pid, sig) == 0 }
}

#[cfg(windows)]
pub fn try_signal_pid(pid: i32, _signal: Signal) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    if pid <= 0 {
        return false;
    }
    // Safe: a null handle is checked before use.
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid as u32);
        if h.is_null() {
            return false;
        }
        let ok = TerminateProcess(h, 1) != 0;
        CloseHandle(h);
        ok
    }
}

/// Deliver `signal` to `pid`, best-effort. Failure (already exited, not ours)
/// is silently ignored on both platforms.
#[cfg(unix)]
pub fn signal_pid(pid: i32, signal: Signal) {
    let sig = match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    // Safe: sending a signal to a pid we just enumerated; failure is fine.
    unsafe {
        libc::kill(pid, sig);
    }
}

#[cfg(windows)]
pub fn signal_pid(pid: i32, _signal: Signal) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    if pid <= 0 {
        return;
    }
    // Safe: a null handle is checked; TerminateProcess on a live handle is
    // defined, and failure is ignored by contract.
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid as u32);
        if h.is_null() {
            return;
        }
        TerminateProcess(h, 1);
        CloseHandle(h);
    }
}

/// Detach `cmd` so the spawned daemon outlives this CLI process.
///
/// Unix puts it in its own process group, so a Ctrl-C to the CLI's group is not
/// forwarded. Windows has no process groups in that sense; `DETACHED_PROCESS`
/// plus `CREATE_NEW_PROCESS_GROUP` is the closest equivalent — the child gets
/// no console and does not receive the parent's Ctrl-C/Ctrl-Break.
#[cfg(unix)]
pub fn detach(cmd: &mut Command) {
    std::os::unix::process::CommandExt::process_group(cmd, 0);
}

#[cfg(windows)]
pub fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

/// Hand this process over to `cmd`, never returning on success.
///
/// Unix `exec`s, so the child inherits the pid and the terminal outright —
/// which is what an interactive console (`tio`, `adb shell`) wants. Windows has
/// no `exec`, so we spawn, wait, and exit with the child's status. The
/// observable difference is that this process stays alive as a parent while the
/// child runs; stdio is inherited either way, so an interactive session behaves
/// the same.
#[cfg(unix)]
pub fn exec_replace(cmd: &mut Command) -> std::io::Error {
    std::os::unix::process::CommandExt::exec(cmd)
}

#[cfg(windows)]
pub fn exec_replace(cmd: &mut Command) -> std::io::Error {
    match cmd.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => e,
    }
}
