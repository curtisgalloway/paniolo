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

//! Privilege-separated acquisition of a `/dev/bpf` send descriptor.
//!
//! On macOS, opening `/dev/bpf` requires root (or membership in the `access_bpf`
//! group). To keep `netbootd` itself unprivileged, a tiny **setuid-root** helper
//! — `netbootd-bpf-helper` — does the only privileged work: it opens `/dev/bpfN`,
//! binds it to the netboot interface (`BIOCSETIF`), sets `BIOCSHDRCMPLT` so the
//! caller supplies the full L2 header, and passes the open descriptor back over
//! a `socketpair` via `SCM_RIGHTS`, then exits.
//!
//! Why this works: on BSD/macOS, `/dev/bpf` access is checked at `open()` time.
//! Once the descriptor is open r/w and bound, its send capability travels with
//! the fd — the unprivileged `netbootd` that receives it can `write()` raw
//! frames to it regardless of its own uid. `BIOCSHDRCMPLT` is a per-descriptor
//! flag, so setting it once in the helper persists on the passed fd (and
//! sidesteps the macOS bug where toggling it per-write can break injection).
//!
//! This module is shared by both the helper (the *send* side: [`open_bpf`] +
//! [`send_fd`]) and the daemon (the *receive* side: [`request_bpf_fd`], which
//! spawns the helper and calls [`recv_fd`]).

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// macOS BPF ioctls (64-bit), identical to the constants `_tftp.py` hardcodes.
#[cfg(target_os = "macos")]
const BIOCSETIF: libc::c_ulong = 0x8020_426C; // bind fd to an interface (struct ifreq)
#[cfg(target_os = "macos")]
const BIOCSHDRCMPLT: libc::c_ulong = 0x8004_4275; // we write complete L2 headers

/// Reject interface names that are empty, too long for `struct ifreq`, or
/// contain anything but ASCII alphanumerics. The helper runs setuid-root, so it
/// must not trust this string even though netbootd is the expected caller.
#[cfg(target_os = "macos")]
fn validate_iface(iface: &str) -> io::Result<()> {
    if iface.is_empty() || iface.len() >= 16 || !iface.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid interface name {iface:?}"),
        ));
    }
    Ok(())
}

/// Open a writable `/dev/bpf` descriptor bound to `iface` with `BIOCSHDRCMPLT`
/// set. The privileged operation — only callable by the setuid helper. Tries
/// `/dev/bpf0..256`, skipping devices already in use (`EBUSY`).
#[cfg(target_os = "macos")]
pub fn open_bpf(iface: &str) -> io::Result<OwnedFd> {
    validate_iface(iface)?;

    let mut last_err = io::Error::new(io::ErrorKind::NotFound, "no free /dev/bpf device available");
    for n in 0..256 {
        let path = std::ffi::CString::new(format!("/dev/bpf{n}")).unwrap();
        let raw = unsafe { libc::open(path.as_ptr(), libc::O_RDWR) };
        if raw < 0 {
            last_err = io::Error::last_os_error();
            match last_err.raw_os_error() {
                // Device busy — try the next one.
                Some(libc::EBUSY) => continue,
                // No such device — we have run off the end of the cloning range.
                Some(libc::ENOENT) | Some(libc::ENXIO) => break,
                // Permission denied etc. — surface immediately.
                _ => return Err(last_err),
            }
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };

        // BIOCSETIF: bind to the interface. struct ifreq is 32 bytes; the name
        // goes in the first field, NUL-padded.
        let mut ifreq = [0u8; 32];
        ifreq[..iface.len()].copy_from_slice(iface.as_bytes());
        if unsafe { libc::ioctl(fd.as_raw_fd(), BIOCSETIF, ifreq.as_ptr()) } != 0 {
            // Interface-level failure (e.g. no such interface) — not per-device,
            // so retrying other bpf nodes will not help.
            return Err(io::Error::last_os_error());
        }

        // BIOCSHDRCMPLT(1): we supply the source MAC ourselves.
        let one: u32 = 1;
        if unsafe { libc::ioctl(fd.as_raw_fd(), BIOCSHDRCMPLT, &one as *const u32) } != 0 {
            return Err(io::Error::last_os_error());
        }
        return Ok(fd);
    }
    Err(last_err)
}

/// Send `fd` to the peer of the connected unix socket `sock` via `SCM_RIGHTS`,
/// alongside a single dummy data byte (some platforms drop ancillary data on a
/// zero-length payload).
pub fn send_fd(sock: RawFd, fd: RawFd) -> io::Result<()> {
    let mut dummy = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: dummy.as_mut_ptr() as *mut libc::c_void,
        iov_len: 1,
    };
    let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
    let mut cmsg_buf = vec![0u8; space];

    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = space as _;

    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
        std::ptr::copy_nonoverlapping(
            &fd as *const RawFd as *const u8,
            libc::CMSG_DATA(cmsg),
            std::mem::size_of::<RawFd>(),
        );
    }

    if unsafe { libc::sendmsg(sock, &msg, 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Receive a single descriptor sent with [`send_fd`] from the connected unix
/// socket `sock`.
pub fn recv_fd(sock: RawFd) -> io::Result<OwnedFd> {
    let mut dummy = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: dummy.as_mut_ptr() as *mut libc::c_void,
        iov_len: 1,
    };
    let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
    let mut cmsg_buf = vec![0u8; space];

    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = space as _;

    let n = unsafe { libc::recvmsg(sock, &mut msg, 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }

    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        if cmsg.is_null()
            || (*cmsg).cmsg_level != libc::SOL_SOCKET
            || (*cmsg).cmsg_type != libc::SCM_RIGHTS
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "no SCM_RIGHTS control message received",
            ));
        }
        let mut fd: RawFd = -1;
        std::ptr::copy_nonoverlapping(
            libc::CMSG_DATA(cmsg),
            &mut fd as *mut RawFd as *mut u8,
            std::mem::size_of::<RawFd>(),
        );
        if fd < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "received invalid fd",
            ));
        }
        Ok(OwnedFd::from_raw_fd(fd))
    }
}

/// Locate the `netbootd-bpf-helper` binary: prefer the copy installed next to
/// the running `netbootd` executable, otherwise fall back to `PATH`.
#[cfg(target_os = "macos")]
fn locate_helper() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join("netbootd-bpf-helper");
            if cand.exists() {
                return cand;
            }
        }
    }
    std::path::PathBuf::from("netbootd-bpf-helper")
}

/// Spawn the setuid helper and receive a bound, write-ready `/dev/bpf`
/// descriptor over a `socketpair`. The daemon side of the handoff.
///
/// The helper inherits the child end of the socketpair as fd 3, writes the bpf
/// fd back via `SCM_RIGHTS`, and exits. Failure here is non-fatal to the caller:
/// netbootd logs it and falls back to the kernel `send_to` path.
#[cfg(target_os = "macos")]
pub fn request_bpf_fd(iface: &str) -> io::Result<OwnedFd> {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let mut sv = [0 as RawFd; 2];
    if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, sv.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let parent = unsafe { OwnedFd::from_raw_fd(sv[0]) };
    let child = unsafe { OwnedFd::from_raw_fd(sv[1]) };
    // Don't leak the parent end into the child.
    unsafe { libc::fcntl(parent.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };

    let helper = locate_helper();
    let child_raw = child.as_raw_fd();

    let mut cmd = Command::new(&helper);
    cmd.arg("--interface")
        .arg(iface)
        .arg("--handoff-fd")
        .arg("3");
    unsafe {
        cmd.pre_exec(move || {
            // Move the child socketpair end to fd 3 (dup2 clears FD_CLOEXEC).
            if libc::dup2(child_raw, 3) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut proc = cmd
        .spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("spawn {}: {e}", helper.display())))?;
    // Parent no longer needs the child end.
    drop(child);

    let fd = recv_fd(parent.as_raw_fd());
    let status = proc.wait()?;
    let fd = fd?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "netbootd-bpf-helper exited with {status}"
        )));
    }
    Ok(fd)
}
