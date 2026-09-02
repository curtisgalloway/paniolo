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

use crate::daemons;
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

/// One health check, expressed structurally rather than as shell text.
///
/// Locally a probe runs natively, so `paniolo doctor` needs no shell at all.
/// It used to build a POSIX script and run it through `sh -c`, which does not
/// exist on Windows — and nothing caught that, because the tests asserted on
/// the *text* of the generated script instead of running it. Anything that
/// only inspects a command string is blind to whether the command can run.
///
/// Remotely a probe still renders to POSIX sh via [`Probe::to_posix`], because
/// the far side of the SSH hop is a Unix control host. Keeping both views of
/// each probe in one type is what stops them drifting apart.
enum Probe {
    /// The path exists.
    Exists(String),
    /// The program resolves the way a hook would: libexec dirs, then PATH.
    OnHookPath(String),
    /// A capture device: either a path that exists, or a name `hdmicap
    /// devices` lists. Exit 3 means hdmicap itself is missing — a different
    /// failure from a missing device.
    Video(String),
    /// The network interface is present on this host.
    NetInterface(String),
    /// adb reaches the device: the binary resolves (else exit 3), then
    /// `get-state` succeeds.
    Adb { bin: String, serial: Option<String> },
}

impl Probe {
    /// Render as a POSIX-sh script, for probing a remote Unix control host.
    fn to_posix(&self) -> String {
        match self {
            Probe::Exists(p) => format!("test -e {}", ssh::shell_quote(p)),
            Probe::OnHookPath(prog) => format!(
                "{HOOK_PATH_PREFIX} command -v {} >/dev/null",
                ssh::shell_quote(prog)
            ),
            Probe::Video(dev) => video_probe_script(dev),
            Probe::NetInterface(iface) => {
                let q = ssh::shell_quote(iface);
                format!("test -e /sys/class/net/{q} || ifconfig {q} >/dev/null 2>&1")
            }
            Probe::Adb { bin, serial } => {
                let q_bin = ssh::shell_quote(bin);
                let find = if bin.starts_with('/') {
                    format!("test -x {q_bin}")
                } else {
                    format!("command -v {q_bin} >/dev/null")
                };
                let sel = match serial {
                    Some(s) => format!("-s {} ", ssh::shell_quote(s)),
                    None => String::new(),
                };
                format!("{find} || exit 3; {q_bin} {sel}get-state >/dev/null 2>&1")
            }
        }
    }

    /// Run the probe on this host, natively. Exit codes match [`to_posix`], so
    /// a local and a remote answer mean the same thing.
    fn run_local(&self) -> Option<i32> {
        let ok = |b: bool| Some(if b { 0 } else { 1 });
        match self {
            Probe::Exists(p) => ok(std::path::Path::new(p).exists()),
            Probe::OnHookPath(prog) => ok(crate::daemons::find_binary(prog).is_some()),
            Probe::Video(dev) => {
                if std::path::Path::new(dev).exists() {
                    return Some(0);
                }
                let Some(hdmicap) = crate::daemons::find_binary("hdmicap") else {
                    return Some(3);
                };
                let out = Command::new(hdmicap).arg("devices").output().ok()?;
                ok(String::from_utf8_lossy(&out.stdout).contains(dev.as_str()))
            }
            Probe::NetInterface(iface) => local_interface_exists(iface),
            Probe::Adb { bin, serial } => {
                let found = if bin.starts_with('/') {
                    is_executable(std::path::Path::new(bin)).then(|| bin.into())
                } else {
                    crate::daemons::find_binary(bin)
                };
                let Some(adb) = found else {
                    return Some(3);
                };
                let mut cmd = Command::new(adb);
                if let Some(s) = serial {
                    cmd.arg("-s").arg(s);
                }
                let status = cmd
                    .arg("get-state")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .ok()?;
                ok(status.success())
            }
        }
    }
}

/// Is this a file we could execute? On Unix that is an execute bit; Windows
/// has no such bit, so existing and being a file is the whole test.
fn is_executable(p: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Is `iface` present on this host?
///
/// Linux exposes every interface under sysfs. macOS has no such tree, so ask
/// `ifconfig`. Windows has neither, and `netif` has no Windows implementation
/// to configure one with — so the honest answer there is "cannot tell", which
/// [`interpret`] renders as unreachable rather than a confident "missing".
fn local_interface_exists(iface: &str) -> Option<i32> {
    if cfg!(target_os = "linux") {
        return Some(i32::from(
            !std::path::Path::new(&format!("/sys/class/net/{iface}")).exists(),
        ));
    }
    if cfg!(target_os = "macos") {
        return Command::new("ifconfig")
            .arg(iface)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()
            .map(|s| s.code().unwrap_or(-1));
    }
    None
}

/// Run a probe on a host: natively when it is this machine, over SSH as a
/// POSIX script otherwise. None == unreachable.
fn probe(lab: &Lab, host_name: &str, p: &Probe) -> Option<i32> {
    let host = lab.host(host_name);
    if host.is_local(host_name) {
        return p.run_local();
    }
    match ssh::run(
        &host,
        &["sh".to_string(), "-c".to_string(), p.to_posix()],
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

/// How a hook program (a power hook, or the hid `cmd`) is probed: an absolute
/// path just has to exist; a bare name has to resolve the way the hook itself
/// will resolve it — libexec dirs, then PATH.
fn hook_probe(prog: &str) -> Probe {
    if prog.starts_with('/') {
        Probe::Exists(prog.to_string())
    } else {
        Probe::OnHookPath(prog.to_string())
    }
}

fn check_channel(lab: &Lab, ch: &ResolvedChannel, rt: &ResolvedTarget) -> (Status, String) {
    match ch.kind {
        ChannelKind::Serial => match field(ch, "device") {
            None => (Status::Incomplete, "no device set".to_string()),
            Some(dev) => interpret(probe(lab, &ch.host, &Probe::Exists(dev.to_string())), dev),
        },
        ChannelKind::Video => match field(ch, "device") {
            None => (Status::Incomplete, "no device set".to_string()),
            Some(dev) => match probe(lab, &ch.host, &Probe::Video(dev.to_string())) {
                Some(3) => (Status::Missing, format!("{dev} (hdmicap not installed)")),
                rc => interpret(rc, dev),
            },
        },
        ChannelKind::Netboot => match field(ch, "interface") {
            None => (Status::Incomplete, "no interface set".to_string()),
            Some(iface) => interpret(
                probe(lab, &ch.host, &Probe::NetInterface(iface.to_string())),
                iface,
            ),
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
            // then PATH (see daemons::hook_path()). A hook may be prefixed
            // with `VAR=value` credential assignments (docs/power.md
            // "Credentials"); the program to probe is the token after them.
            let hook_keys = ["cycle_cmd", "on_cmd", "off_cmd", "state_cmd"];
            let mut configured: Vec<&str> = Vec::new();
            for key in hook_keys {
                if let Some(cmd) = field(ch, key) {
                    configured.push(key);
                    let prog = daemons::first_program_token(cmd).unwrap_or("");
                    let rc = probe(lab, &ch.host, &hook_probe(prog));
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
        ChannelKind::Hid | ChannelKind::Usb => match field(ch, "cmd") {
            None => (Status::Incomplete, "no cmd set".to_string()),
            // Like the power hooks: absolute-path helpers are probed for
            // existence; bare names are probed under libexec-then-PATH; a
            // leading `VAR=value` is skipped the same way.
            Some(cmd) => {
                let prog = daemons::first_program_token(cmd).unwrap_or("");
                interpret(probe(lab, &ch.host, &hook_probe(prog)), prog)
            }
        },
        ChannelKind::Adb => {
            // adb is a system tool (PATH), not a paniolo libexec helper. Exit 3
            // distinguishes "adb not installed" from "device not reachable".
            let adb_bin = field(ch, "adb").unwrap_or("adb");
            let serial = field(ch, "serial");
            let p = Probe::Adb {
                bin: adb_bin.to_string(),
                serial: serial.map(str::to_string),
            };
            match probe(lab, &ch.host, &p) {
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

    // ── behavioural tests ───────────────────────────────────────────────
    //
    // These RUN the probes. The string-shape tests below cover the POSIX
    // rendering used over SSH, but they pass identically on every platform —
    // which is exactly how `paniolo doctor` shipped depending on `sh -c`, a
    // binary that does not exist on Windows. A probe that cannot execute has
    // to fail a test somewhere, so it fails here.

    #[test]
    fn exists_probe_runs_natively() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("device");
        std::fs::write(&present, b"").unwrap();

        assert_eq!(
            Probe::Exists(present.to_string_lossy().into_owned()).run_local(),
            Some(0),
            "a file that exists must probe ok"
        );
        assert_eq!(
            Probe::Exists(
                dir.path()
                    .join("no-such-device")
                    .to_string_lossy()
                    .into_owned()
            )
            .run_local(),
            Some(1),
            "a missing file must probe missing, not error"
        );
    }

    #[test]
    fn hook_path_probe_runs_natively() {
        // A name nothing could resolve. The point is that the lookup runs to
        // completion and reports missing, rather than failing to launch.
        assert_eq!(
            Probe::OnHookPath("paniolo-no-such-helper-xyzzy".to_string()).run_local(),
            Some(1)
        );
    }

    #[test]
    fn hook_probe_picks_path_vs_name_lookup() {
        assert!(matches!(hook_probe("/opt/bin/zigplug"), Probe::Exists(_)));
        assert!(matches!(hook_probe("zigplug"), Probe::OnHookPath(_)));
    }

    /// `check_channel`'s power and hid/usb arms both build their probe from
    /// `daemons::first_program_token(cmd)`, exactly as here — a hook written
    /// `AMT_PASSWORD=... amt-tool cycle` (docs/power.md "Credentials") must
    /// probe `amt-tool`, not the literal string `AMT_PASSWORD=...` (which the
    /// old `cmd.split_whitespace().next()` treated as the program, and
    /// `hook_probe` would then send down the `OnHookPath` branch as a bare
    /// name doctor could never find).
    #[test]
    fn hook_probe_skips_a_leading_env_assignment() {
        let cmd = "AMT_PASSWORD=hunter2 /opt/bin/amt-tool cycle";
        let prog = daemons::first_program_token(cmd).unwrap_or("");
        assert_eq!(prog, "/opt/bin/amt-tool");
        assert!(matches!(hook_probe(prog), Probe::Exists(p) if p == "/opt/bin/amt-tool"));
    }

    #[test]
    fn adb_probe_reports_missing_binary_distinctly() {
        // Exit 3 is what separates "adb is not installed" from "the device is
        // not reachable"; callers branch on it.
        let p = Probe::Adb {
            bin: "paniolo-no-such-adb-xyzzy".to_string(),
            serial: None,
        };
        assert_eq!(p.run_local(), Some(3));
    }

    #[test]
    fn video_probe_reports_missing_hdmicap_distinctly() {
        // A device path that cannot exist, so the probe falls through to the
        // hdmicap lookup. If hdmicap IS installed on this machine the fallback
        // runs it instead, which is also a valid answer — assert on the pair.
        let p = Probe::Video("/paniolo-no-such-capture-device-xyzzy".to_string());
        let rc = p.run_local();
        if crate::daemons::find_binary("hdmicap").is_some() {
            assert_eq!(rc, Some(1), "hdmicap present: device simply not listed");
        } else {
            assert_eq!(rc, Some(3), "hdmicap absent: exit 3, not a device failure");
        }
    }

    #[test]
    fn every_probe_renders_posix_for_the_remote_hop() {
        // The SSH path still ships shell text, so each variant must render.
        for p in [
            Probe::Exists("/dev/ttyUSB0".to_string()),
            Probe::OnHookPath("zigplug".to_string()),
            Probe::Video("/dev/video0".to_string()),
            Probe::NetInterface("eth0".to_string()),
            Probe::Adb {
                bin: "adb".to_string(),
                serial: Some("XYZ".to_string()),
            },
        ] {
            assert!(!p.to_posix().is_empty());
        }
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
