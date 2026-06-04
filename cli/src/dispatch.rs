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

//! Per-channel transparent dispatch (see docs/config-redesign.md "Dispatch
//! design"). A location-transparent command resolves the host of the channel it
//! touches; if that's the dev machine it runs locally, otherwise paniolo re-execs
//! the same command on the control host over SSH against a shipped single-host
//! **slice** of the lab. Because the slice's channels carry no `host`, the remote
//! resolves them as local and never re-dispatches.

use std::path::Path;

use crate::labfile::LabFile;
use crate::model::{ChannelKind, Lab, LabError};
use crate::ssh;

/// Re-exec transport mode.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Non-interactive: stdio passed through.
    Reexec,
    /// PTY over `ssh -t` (e.g. tio serial console).
    Interactive,
}

/// Build a one-target lab containing only `target`'s channels that live on
/// `host`, with their `host` fields stripped so the remote treats them as local.
/// This is the slice shipped for remote re-exec (and the shape a host sees).
pub fn build_slice(lab: &Lab, target: &str, host: &str) -> Result<String, LabError> {
    let t = lab
        .targets
        .get(target)
        .ok_or_else(|| LabError(format!("no target '{target}'")))?;
    let default_host = t.default_host();
    let on = |h: &Option<String>| h.as_deref().unwrap_or(default_host) == host;

    let mut lf = LabFile::create(Path::new("slice.toml"));
    lf.add_target(target, None, t.note.as_deref())?;
    if let Some(nb) = &t.netboot {
        if on(&nb.host) {
            lf.set_netboot(
                target,
                nb.interface.as_deref(),
                nb.host_ip.as_deref(),
                nb.tftp_root.as_deref(),
                None,
            )?;
        }
    }
    for s in &t.serial {
        if on(&s.host) {
            lf.add_serial(
                target,
                &s.name,
                &s.device,
                s.baud,
                s.power_sense_signal.as_deref(),
                None,
            )?;
        }
    }
    if let Some(p) = &t.power {
        if on(&p.host) {
            lf.set_power(
                target,
                p.cycle_cmd.as_deref(),
                p.on_cmd.as_deref(),
                p.off_cmd.as_deref(),
                p.state_cmd.as_deref(),
                p.serial_interface.as_deref(),
                None,
            )?;
        }
    }
    if let Some(v) = &t.video {
        if on(&v.host) {
            lf.set_video(target, v.device.as_deref(), None)?;
        }
    }
    Ok(lf.doc.to_string())
}

/// Drop the global `--lab PATH` / `--lab=PATH` option from an argv tail; the
/// dev machine's lab path is meaningless on the control host (it gets a slice).
pub fn strip_lab_option(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if a == "--lab" {
            skip = true;
            continue;
        }
        if a.starts_with("--lab=") {
            continue;
        }
        out.push(a.clone());
    }
    out
}

/// The subcommand argv to re-exec on the remote: this process's args minus the
/// program name and the global `--lab` option.
pub fn subcommand_args() -> Vec<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    strip_lab_option(&args)
}

const SHIP_SCRIPT: &str =
    r#"f=$(mktemp "${TMPDIR:-/tmp}/paniolo-lab.XXXXXX") && cat > "$f" && printf %s "$f""#;

/// Write `slice_toml` to a temp file on `host` over SSH and return its path.
pub fn ship_slice(host: &crate::model::Host, slice_toml: &str) -> std::io::Result<String> {
    let argv = vec!["sh".to_string(), "-c".to_string(), SHIP_SCRIPT.to_string()];
    let out = ssh::run(host, &argv, Some(slice_toml), &[])?;
    let path = out.stdout.trim().to_string();
    if out.status != 0 || path.is_empty() {
        return Err(std::io::Error::other(format!(
            "failed to ship lab slice to {}: {}",
            host.ssh,
            out.stderr.trim()
        )));
    }
    Ok(path)
}

/// Re-exec `sub_argv` on `host` against a shipped slice; return the remote exit
/// code. Cleans up the slice file afterward.
pub fn dispatch(
    lab: &Lab,
    target: &str,
    host_name: &str,
    mode: Mode,
    sub_argv: &[String],
) -> anyhow::Result<i32> {
    let host = lab.host(host_name);
    let slice = build_slice(lab, target, host_name)?;
    let remote_path = ship_slice(&host, &slice)?;

    let mut argv = vec![host.paniolo(), "--lab".to_string(), remote_path.clone()];
    argv.extend(sub_argv.iter().cloned());

    let code = match mode {
        Mode::Interactive => ssh::run_interactive(&host, &argv, &[]),
        Mode::Reexec => ssh::run_passthrough(&host, &argv, &[]),
    }?;

    // Best-effort cleanup of the shipped slice.
    let _ = ssh::run(
        &host,
        &["rm".to_string(), "-f".to_string(), remote_path],
        None,
        &[],
    );
    Ok(code)
}

/// Run a paniolo subcommand on `host_name` against a shipped slice, captured.
/// Used by composite commands (e.g. `console`) to drive helper commands on the
/// host before tunnelling to its daemons.
pub fn run_subcommand(
    lab: &Lab,
    target: &str,
    host_name: &str,
    subargs: &[&str],
) -> anyhow::Result<ssh::Output> {
    let host = lab.host(host_name);
    let slice = build_slice(lab, target, host_name)?;
    let remote_path = ship_slice(&host, &slice)?;
    let mut argv = vec![host.paniolo(), "--lab".to_string(), remote_path.clone()];
    argv.extend(subargs.iter().map(|s| s.to_string()));
    let out = ssh::run(&host, &argv, None, &[]);
    let _ = ssh::run(
        &host,
        &["rm".to_string(), "-f".to_string(), remote_path],
        None,
        &[],
    );
    Ok(out?)
}

/// Read the TCP port from a daemon's discovery file on `host`, or None.
/// The path is resolved by a remote shell so the host's own uid applies;
/// must match `runtime_base()` in daemons.rs (and the daemon crates).
pub fn remote_daemon_port(host: &crate::model::Host, subdir: &str) -> Option<u16> {
    let script = format!("cat \"/tmp/paniolo-$(id -u)/{subdir}/daemon.json\" 2>/dev/null");
    let out = ssh::run(
        host,
        &["sh".to_string(), "-c".to_string(), script],
        None,
        &[],
    )
    .ok()?;
    if out.status != 0 || out.stdout.trim().is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(out.stdout.trim()).ok()?;
    v.get("port")?.as_u64().map(|p| p as u16)
}

/// Resolve where a command should run and dispatch if remote.
///
/// Returns `Some(exit_code)` when the command was dispatched to a control host
/// (the caller should exit with it), or `None` when it should run locally.
pub fn maybe_dispatch(
    lab: &Lab,
    target: &str,
    kind: ChannelKind,
    serial_name: Option<&str>,
    mode: Mode,
) -> anyhow::Result<Option<i32>> {
    let rt = lab
        .resolved_target(target)
        .ok_or_else(|| LabError(format!("target '{target}' not found in lab")))?;
    let host_name = crate::model::channel_host(&rt, kind, serial_name)?;
    let host = lab.host(&host_name);
    if host.is_local(&host_name) {
        return Ok(None);
    }
    let code = dispatch(lab, target, &host_name, mode, &subcommand_args())?;
    Ok(Some(code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model;

    fn lab() -> Lab {
        model::parse(
            r#"
            [hosts.bench1]
            ssh = "u@bench1"
            [hosts.bench2]
            ssh = "u@bench2"
            [targets.fortune]
            host = "bench1"
            [targets.fortune.netboot]
            interface = "en0"
            [[targets.fortune.serial]]
            name = "console"
            device = "/dev/ttyUSB0"
            [targets.fortune.video]
            device = "/dev/video0"
            host = "bench2"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn slice_keeps_only_that_hosts_channels_host_stripped() {
        let s = build_slice(&lab(), "fortune", "bench1").unwrap();
        let reparsed = model::parse(&s).unwrap();
        let t = &reparsed.targets["fortune"];
        // bench1 has netboot + the console serial; video (bench2) is excluded.
        assert!(t.netboot.is_some());
        assert_eq!(t.serial.len(), 1);
        assert!(t.video.is_none());
        // Host fields are stripped so the remote resolves them as local.
        assert!(t.host.is_none());
        assert!(t.serial[0].host.is_none());
    }

    #[test]
    fn strip_lab_handles_both_forms() {
        let a: Vec<String> = ["--lab", "/x", "serial", "connect", "fortune"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(strip_lab_option(&a), vec!["serial", "connect", "fortune"]);
        let b: Vec<String> = ["--lab=/x", "netboot", "start"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(strip_lab_option(&b), vec!["netboot", "start"]);
    }
}
