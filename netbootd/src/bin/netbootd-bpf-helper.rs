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

//! `netbootd-bpf-helper` — the *only* privileged component of netboot.
//!
//! Installed setuid-root by `paniolo setup` (the one-time `sudo`). Its entire
//! job: open `/dev/bpf`, bind it to the requested interface, set
//! `BIOCSHDRCMPLT`, and hand the descriptor back to its (unprivileged) caller
//! over the inherited socketpair fd via `SCM_RIGHTS`, then exit. It never reads
//! from the network, never writes frames, and holds root for only microseconds.
//!
//! Because it is setuid-root, every input is treated as hostile: the interface
//! name is validated in [`netbootd::handoff::open_bpf`] and the handoff fd is
//! the only descriptor it writes to.
//!
//! Usage: `netbootd-bpf-helper --interface <name> --handoff-fd <n>`

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    use std::os::fd::AsRawFd;
    use std::process::ExitCode;

    let mut iface: Option<String> = None;
    let mut handoff_fd: Option<i32> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--interface" => iface = args.next(),
            "--handoff-fd" => handoff_fd = args.next().and_then(|s| s.parse().ok()),
            other => {
                eprintln!("netbootd-bpf-helper: unexpected argument {other:?}");
                return ExitCode::from(2);
            }
        }
    }

    let (Some(iface), Some(handoff_fd)) = (iface, handoff_fd) else {
        eprintln!("usage: netbootd-bpf-helper --interface <name> --handoff-fd <n>");
        return ExitCode::from(2);
    };

    let bpf = match netbootd::handoff::open_bpf(&iface) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("netbootd-bpf-helper: open_bpf({iface}): {e}");
            return ExitCode::from(1);
        }
    };

    if let Err(e) = netbootd::handoff::send_fd(handoff_fd, bpf.as_raw_fd()) {
        eprintln!("netbootd-bpf-helper: send_fd: {e}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

#[cfg(not(target_os = "macos"))]
fn main() -> std::process::ExitCode {
    eprintln!("netbootd-bpf-helper is macOS-only (Linux uses the kernel send path)");
    std::process::ExitCode::from(1)
}
