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

//! Pin a socket to one interface — the netboot link.
//!
//! netbootd answers every DHCP DISCOVER it hears with a lease and hands out
//! whatever is in its root directory to anyone who asks. Its listen sockets
//! bind the wildcard address (so they work through the link flaps where the
//! interface IP is momentarily gone), which without a pin would make the same
//! sockets reachable on the host's primary NIC: a rogue DHCP server on the
//! office LAN and a file server anyone there can read. So every listen socket
//! — DHCP, TFTP and HTTP — is pinned to the netboot interface before it is
//! bound, and a pin that cannot be applied is fatal: the daemon refuses to
//! start rather than serve unpinned. Reply sockets are pinned the same way so
//! a limited-broadcast DHCP reply or a TFTP DATA block leaves via the netboot
//! link and not the default-route interface.
//!
//! * **macOS:** `IP_BOUND_IF`, which applies to IPv4 sockets (the HTTP
//!   listener stays IPv4 for that reason). No privilege needed.
//! * **Linux:** `SO_BINDTODEVICE`. Kernels before 5.7 require `CAP_NET_RAW`
//!   to set it, so the listen sockets are pinned while the daemon still holds
//!   root — before the privilege drop (see `privdrop`).
//! * **Elsewhere:** there is no per-socket interface binding, so the pin is a
//!   loudly logged no-op; netboot is not a supported feature on those hosts.

use socket2::Socket;

/// Pin `sock` so it only receives on, and only sends via, `iface`.
#[cfg(target_os = "macos")]
pub fn pin_socket_to_interface(sock: &Socket, iface: &str) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let cname = std::ffi::CString::new(iface)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "iface has NUL"))?;
    let idx = unsafe { libc::if_nametoindex(cname.as_ptr()) };
    if idx == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let idx: libc::c_uint = idx;
    let rc = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::IPPROTO_IP,
            libc::IP_BOUND_IF,
            &idx as *const libc::c_uint as *const libc::c_void,
            std::mem::size_of::<libc::c_uint>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Pin `sock` so it only receives on, and only sends via, `iface`.
#[cfg(target_os = "linux")]
pub fn pin_socket_to_interface(sock: &Socket, iface: &str) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let rc = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            iface.as_ptr() as *const libc::c_void,
            iface.len() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// No per-socket interface binding exists on this platform: log it and carry
/// on unpinned. Netboot is not a supported feature here, so this is a
/// development convenience, not a silent hole on a bench host.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn pin_socket_to_interface(_sock: &Socket, iface: &str) -> std::io::Result<()> {
    tracing::warn!(
        "this platform cannot pin a socket to {iface}; the socket is reachable on every interface"
    );
    Ok(())
}
