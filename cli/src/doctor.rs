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

//! `paniolo doctor` — probe configured channels against reality.
//!
//! Read-only health check: for each channel it SSHes to the control host the
//! channel lives on (or runs locally) and tests that the configured device or
//! interface actually exists. This is the probing that used to be baked into the
//! Python `setup` commands, separated so config edits stay local and offline.

use std::process::Command;

use crate::model::{ChannelKind, Lab, ResolvedChannel, ResolvedTarget};
use crate::ssh;

#[derive(PartialEq, Eq)]
enum Status {
    Ok,
    Missing,
    Unreachable,
    Incomplete,
}

impl Status {
    fn label(&self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Missing => "MISSING",
            Status::Unreachable => "unreachable",
            Status::Incomplete => "incomplete",
        }
    }
    fn is_problem(&self) -> bool {
        matches!(self, Status::Missing | Status::Unreachable)
    }
}

/// Run a POSIX-sh probe on a host (locally or over SSH); None == unreachable.
fn probe(lab: &Lab, host_name: &str, script: &str) -> Option<i32> {
    let host = lab.host(host_name);
    if host.is_local(host_name) {
        return Command::new("sh")
            .arg("-c")
            .arg(script)
            .status()
            .ok()
            .map(|s| s.code().unwrap_or(-1));
    }
    match ssh::run(
        &host,
        &["sh".to_string(), "-c".to_string(), script.to_string()],
        None,
        &[],
    ) {
        Ok(o) => Some(o.status),
        Err(_) => None,
    }
}

fn interpret(rc: Option<i32>, what: &str) -> (Status, String) {
    match rc {
        None | Some(255) => (Status::Unreachable, "host unreachable".to_string()),
        Some(0) => (Status::Ok, what.to_string()),
        Some(_) => (Status::Missing, what.to_string()),
    }
}

fn field<'a>(ch: &'a ResolvedChannel, key: &str) -> Option<&'a str> {
    ch.fields
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.as_str())
}

/// Shell statement giving hook commands the same resolution the hooks get at
/// runtime (`daemons::hook_path()`): the per-user libexec dir, then the system
/// package dir, then PATH. The literal paths must match
/// `daemons::libexec_dir()` / `daemons::system_libexec_dir()`; `$HOME` is
/// expanded by the probed host's own shell so that host's home applies. A
/// standalone assignment (not a `VAR=… cmd` prefix) so the `command -v`
/// lookup that follows honors it in every /bin/sh, dash included. Exe-relative
/// helper dirs (a Homebrew keg) can't be derived for a remote host, so a
/// keg-only install on a *remote* control host may still probe MISSING.
const HOOK_PATH_PREFIX: &str =
    "PATH=\"$HOME/.local/libexec/paniolo/bin:/usr/libexec/paniolo/bin:$PATH\";";

/// Probe script for a video channel. The device is usually a capture-device
/// NAME (e.g. "USB Video" on macOS), not a path, so `test -e` alone is wrong:
/// ask `hdmicap devices` on the channel host whether it enumerates. Path-style
/// devices (`/dev/video0`) still short-circuit via `test -e`. hdmicap resolves
/// like `daemons::find_binary()`: the helper dirs (via [`HOOK_PATH_PREFIX`]),
/// then PATH, then the legacy ~/.cargo/bin. Exit 3 = hdmicap itself is
/// missing, a distinct failure from a missing device.
fn video_probe_script(device: &str) -> String {
    let q = ssh::shell_quote(device);
    format!(
        "test -e {q} && exit 0; \
         {HOOK_PATH_PREFIX} PATH=\"$PATH:$HOME/.cargo/bin\"; \
         command -v hdmicap >/dev/null || exit 3; \
         hdmicap devices 2>/dev/null | grep -F -q -- {q}"
    )
}

fn check_channel(lab: &Lab, ch: &ResolvedChannel, rt: &ResolvedTarget) -> (Status, String) {
    match ch.kind {
        ChannelKind::Serial => match field(ch, "device") {
            None => (Status::Incomplete, "no device set".to_string()),
            Some(dev) => interpret(
                probe(lab, &ch.host, &format!("test -e {}", ssh::shell_quote(dev))),
                dev,
            ),
        },
        ChannelKind::Video => match field(ch, "device") {
            None => (Status::Incomplete, "no device set".to_string()),
            Some(dev) => match probe(lab, &ch.host, &video_probe_script(dev)) {
                Some(3) => (Status::Missing, format!("{dev} (hdmicap not installed)")),
                rc => interpret(rc, dev),
            },
        },
        ChannelKind::Netboot => match field(ch, "interface") {
            None => (Status::Incomplete, "no interface set".to_string()),
            Some(iface) => {
                let q = ssh::shell_quote(iface);
                let script = format!("test -e /sys/class/net/{q} || ifconfig {q} >/dev/null 2>&1");
                interpret(probe(lab, &ch.host, &script), iface)
            }
        },
        ChannelKind::Power => {
            if let Some(si) = field(ch, "serial_interface") {
                let have = rt
                    .channels
                    .iter()
                    .any(|c| c.kind == ChannelKind::Serial && c.name == si);
                if !have {
                    return (
                        Status::Missing,
                        format!("serial_interface '{si}' has no matching serial"),
                    );
                }
            }
            // Probe all four hook fields; report the first missing program.
            // Bare names resolve like the hooks themselves do: libexec first,
            // then PATH (see daemons::hook_path()).
            let hook_keys = ["cycle_cmd", "on_cmd", "off_cmd", "state_cmd"];
            let mut configured: Vec<&str> = Vec::new();
            for key in hook_keys {
                if let Some(cmd) = field(ch, key) {
                    configured.push(key);
                    let prog = cmd.split_whitespace().next().unwrap_or("");
                    let script = if prog.starts_with('/') {
                        format!("test -e {}", ssh::shell_quote(prog))
                    } else {
                        format!(
                            "{HOOK_PATH_PREFIX} command -v {} >/dev/null",
                            ssh::shell_quote(prog)
                        )
                    };
                    let rc = probe(lab, &ch.host, &script);
                    if rc != Some(0) {
                        return interpret(rc, prog);
                    }
                }
            }
            if configured.is_empty() {
                (Status::Ok, "configured".to_string())
            } else {
                (Status::Ok, configured.join(","))
            }
        }
        ChannelKind::Hid => match field(ch, "cmd") {
            None => (Status::Incomplete, "no cmd set".to_string()),
            // Like the power hooks: absolute-path helpers are probed for
            // existence; bare names are probed under libexec-then-PATH.
            Some(cmd) => {
                let prog = cmd.split_whitespace().next().unwrap_or("");
                let script = if prog.starts_with('/') {
                    format!("test -e {}", ssh::shell_quote(prog))
                } else {
                    format!(
                        "{HOOK_PATH_PREFIX} command -v {} >/dev/null",
                        ssh::shell_quote(prog)
                    )
                };
                interpret(probe(lab, &ch.host, &script), prog)
            }
        },
        ChannelKind::Adb => {
            // adb is a system tool (PATH), not a paniolo libexec helper. Exit 3
            // distinguishes "adb not installed" from "device not reachable".
            let adb_bin = field(ch, "adb").unwrap_or("adb");
            let serial = field(ch, "serial");
            let q_bin = ssh::shell_quote(adb_bin);
            let find = if adb_bin.starts_with('/') {
                format!("test -x {q_bin}")
            } else {
                format!("command -v {q_bin} >/dev/null")
            };
            let sel = match serial {
                Some(s) => format!("-s {} ", ssh::shell_quote(s)),
                None => String::new(),
            };
            let script = format!("{find} || exit 3; {q_bin} {sel}get-state >/dev/null 2>&1");
            match probe(lab, &ch.host, &script) {
                Some(3) => (Status::Missing, format!("{adb_bin} (adb not installed)")),
                rc => interpret(rc, serial.unwrap_or("device")),
            }
        }
    }
}

fn channel_name(ch: &ResolvedChannel) -> String {
    if ch.name == ch.kind.as_str() {
        ch.kind.as_str().to_string()
    } else {
        format!("{} {}", ch.kind.as_str(), ch.name)
    }
}

/// Probe `target` (or all targets), optionally limited to `host_filter`.
/// Prints a report and returns the number of problems found.
pub fn run(lab: &Lab, target: Option<&str>, host_filter: Option<&str>) -> i32 {
    let names: Vec<String> = match target {
        Some(n) => vec![n.to_string()],
        None => lab.targets.keys().cloned().collect(),
    };
    if names.is_empty() {
        println!("No targets configured.");
        return 0;
    }
    let mut problems = 0;
    for tname in names {
        let Some(rt) = lab.resolved_target(&tname) else {
            eprintln!("Target '{tname}' not found in lab.");
            problems += 1;
            continue;
        };
        for ch in &rt.channels {
            if let Some(h) = host_filter {
                if ch.host != h {
                    continue;
                }
            }
            let (status, detail) = check_channel(lab, ch, &rt);
            if status.is_problem() {
                problems += 1;
            }
            println!(
                "{tname}\t{}\t@{}\t{}\t{}",
                channel_name(ch),
                ch.host,
                status.label(),
                detail
            );
        }
    }
    if problems == 0 {
        println!("All configured channels present.");
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_path_prefix_covers_user_and_system_libexec() {
        // The probe must search everywhere the hooks themselves resolve
        // helpers (daemons::helper_dirs()): a .deb install's helpers live only
        // in /usr/libexec/paniolo/bin, off PATH.
        assert!(HOOK_PATH_PREFIX.contains("$HOME/.local/libexec/paniolo/bin"));
        assert!(HOOK_PATH_PREFIX.contains("/usr/libexec/paniolo/bin"));
        // A standalone statement, so a following `command -v` honors it in
        // dash as well as bash.
        assert!(HOOK_PATH_PREFIX.ends_with(';'));
    }

    #[test]
    fn video_probe_searches_helper_dirs_then_path_then_cargo() {
        let s = video_probe_script("/dev/video0");
        assert!(s.contains("/usr/libexec/paniolo/bin"), "{s}");
        assert!(s.contains("$HOME/.cargo/bin"), "{s}");
        assert!(s.contains("command -v hdmicap"), "{s}");
        assert!(s.starts_with("test -e /dev/video0 && exit 0"), "{s}");
    }
}
