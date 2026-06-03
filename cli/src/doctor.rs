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

fn check_channel(lab: &Lab, ch: &ResolvedChannel, rt: &ResolvedTarget) -> (Status, String) {
    match ch.kind {
        ChannelKind::Serial | ChannelKind::Video | ChannelKind::Hid => match field(ch, "device") {
            None => (Status::Incomplete, "no device set".to_string()),
            Some(dev) => interpret(
                probe(lab, &ch.host, &format!("test -e {}", ssh::shell_quote(dev))),
                dev,
            ),
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
            if let Some(cmd) = field(ch, "cycle_cmd") {
                let prog = cmd.split_whitespace().next().unwrap_or("");
                if prog.starts_with('/') {
                    return interpret(
                        probe(
                            lab,
                            &ch.host,
                            &format!("test -e {}", ssh::shell_quote(prog)),
                        ),
                        prog,
                    );
                }
                return (Status::Ok, format!("cmd={cmd}"));
            }
            (Status::Ok, "configured".to_string())
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
