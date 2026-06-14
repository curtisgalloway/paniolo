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

//! A pseudo-terminal the hid daemon uses to re-export the DUT serial console.
//!
//! paniolo's `serial` channel opens a *device path* (`serialcap` hands it
//! straight to `tokio_serial`), so to feed the multiplexed `0x03` console
//! through it unchanged we hand the daemon a PTY: the owner thread reads/writes
//! the master end, and paniolo opens the slave path (via a stable symlink the
//! daemon publishes). A PTY — not a socket — is what also satisfies
//! `paniolo serial connect`, which `exec`s `tio` against a real terminal.

use std::ffi::CStr;
use std::fs::File;
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};

use anyhow::{anyhow, Result};

/// An allocated PTY: the master end (owned by the daemon) and the slave device
/// path that paniolo's `serial` channel points its `device =` at.
pub struct Pty {
    /// Master end, in non-blocking mode (so it never stalls the owner loop).
    pub master: File,
    /// The slave device node, e.g. `/dev/pts/7` (Linux) or `/dev/ttys003`
    /// (macOS). Dynamic per allocation — the daemon symlinks a stable path to it.
    pub slave_path: String,
}

/// Allocate a PTY master and return it with the slave device path.
pub fn open() -> Result<Pty> {
    // Safe: posix_openpt only allocates a master fd; we own it in `master`
    // immediately so any early return below closes it.
    let master_fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
    if master_fd < 0 {
        return Err(anyhow!("posix_openpt: {}", io::Error::last_os_error()));
    }
    let master = unsafe { File::from_raw_fd(master_fd) };
    if unsafe { libc::grantpt(master_fd) } != 0 {
        return Err(anyhow!("grantpt: {}", io::Error::last_os_error()));
    }
    if unsafe { libc::unlockpt(master_fd) } != 0 {
        return Err(anyhow!("unlockpt: {}", io::Error::last_os_error()));
    }
    // ptsname is not reentrant, but the daemon calls open() once at startup
    // before any thread touches the master, so the static buffer is safe here.
    let name_ptr = unsafe { libc::ptsname(master_fd) };
    if name_ptr.is_null() {
        return Err(anyhow!("ptsname: {}", io::Error::last_os_error()));
    }
    let slave_path = unsafe { CStr::from_ptr(name_ptr) }
        .to_string_lossy()
        .into_owned();
    set_nonblocking(master.as_raw_fd())?;
    Ok(Pty { master, slave_path })
}

/// Put `fd` in non-blocking mode so the owner loop's reads/writes return
/// promptly (`WouldBlock`) instead of stalling the HID relay.
fn set_nonblocking(fd: RawFd) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(anyhow!("fcntl F_GETFL: {}", io::Error::last_os_error()));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(anyhow!("fcntl F_SETFL: {}", io::Error::last_os_error()));
    }
    Ok(())
}
