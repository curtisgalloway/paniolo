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
    lf.add_target(target, None, t.description.as_deref())?;
    if let Some(nb) = &t.netboot {
        if on(&nb.host) {
            lf.set_netboot(
                target,
                nb.interface.as_deref(),
                nb.host_ip.as_deref(),
                nb.tftp_root.as_deref(),
                nb.boot_file.as_deref(),
                nb.http_port.as_deref(),
                nb.content_type.as_deref(),
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
                s.power_button,
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
            lf.set_video(target, v.device.as_deref(), v.ocr_mode.as_deref(), None)?;
        }
    }
    if let Some(h) = &t.hid {
        if on(&h.host) {
            lf.set_hid(target, h.cmd.as_deref(), None)?;
        }
    }
    if let Some(a) = &t.adb {
        if on(&a.host) {
            lf.set_adb(target, a.serial.as_deref(), a.adb.as_deref(), None)?;
        }
    }
    if let Some(u) = &t.usb {
        if on(&u.host) {
            lf.set_usb(target, u.cmd.as_deref(), None)?;
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
///
/// `args_os` rather than `args`: a non-UTF-8 argument (a device path or file
/// name from the shell's locale, say) must not panic the whole dispatch —
/// [`std::env::args`] does exactly that on invalid Unicode. Losing fidelity
/// on the rare non-UTF-8 byte is an acceptable trade for not crashing.
pub fn subcommand_args() -> Vec<String> {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    strip_lab_option(&args)
}

/// Write `slice_toml` to a file on `host` over SFTP and return its remote name.
///
/// This used to run a POSIX shell script (`f=$(mktemp …) && cat > "$f"`) to
/// create the file and echo its path. That works on a Unix control host and is
/// not a command at all on a Windows one, whose OpenSSH answers with PowerShell
/// — which is what stopped paniolo dispatching to a Windows host. SFTP is a
/// protocol rather than a shell, so it behaves the same on both.
///
/// The returned name is **relative**, and the remote `--lab` depends on that:
/// see [`ssh::sftp_put`] for why an absolute path is not usable on Windows.
pub fn ship_slice(host: &crate::model::Host, slice_toml: &str) -> std::io::Result<String> {
    // Unique per invocation: several dispatches to one host can overlap, and a
    // shared name would let one clobber another's lab slice mid-run.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let remote = format!(".paniolo-lab-{}-{stamp}.toml", std::process::id());

    // The local copy only has to survive long enough for the SFTP put below;
    // `tempfile` picks a private, collision-free name in the system temp dir
    // and removes it on drop, so a failed put (or this process being killed)
    // never leaves a stray lab slice behind the way a hand-rolled name could.
    let mut local = tempfile::NamedTempFile::new()?;
    use std::io::Write;
    local.write_all(slice_toml.as_bytes())?;
    let sent = ssh::sftp_put(host, local.path(), &remote);
    sent.map_err(|e| {
        std::io::Error::other(format!("failed to ship lab slice to {}: {e}", host.ssh))
    })?;
    Ok(remote)
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
        // No environment crosses here: the remote's stdin *is* the terminal
        // the user is typing into (see `ssh::run_interactive`'s doc comment
        // for why that rules out a forwarding channel).
        Mode::Interactive => ssh::run_interactive(&host, &argv),
        Mode::Reexec => ssh::run_passthrough(&host, &argv, &ssh::forwarded_env()?),
    }?;

    // Best-effort cleanup of the shipped slice.
    let _ = ssh::sftp_rm(&host, &remote_path);
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
    let out = ssh::run(&host, &argv, None, &ssh::forwarded_env()?);
    let _ = ssh::sftp_rm(&host, &remote_path);
    Ok(out?)
}

/// Run `write_body` with a writable sink prepared for `out_path` — a sibling
/// temp file in the same directory, never `out_path` itself — and persist
/// that sink onto `out_path` only when `write_body` returns exit code 0.
///
/// Shared by every "stream a remote binary payload into a local file" path
/// (`video shot`, `adb screencap` — see [`dispatch_stdout_to_file`]) so the
/// atomicity guarantee lives in one place, testable without SSH: a failed or
/// partial capture leaves whatever was already at `out_path` untouched
/// instead of destroying it (the old code truncated `out_path` up front,
/// before the transfer even started), and a reader of `out_path` never sees
/// a partially-written file mid-transfer. On a non-zero exit the temp file
/// is simply dropped, which removes it.
fn capture_to_file(
    out_path: &str,
    write_body: impl FnOnce(std::fs::File) -> anyhow::Result<i32>,
) -> anyhow::Result<i32> {
    let dir = Path::new(out_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let tmp = tempfile::Builder::new()
        .prefix(".paniolo-capture-")
        .suffix(".tmp")
        .tempfile_in(dir)
        .map_err(|e| anyhow::anyhow!("creating a temp file next to {out_path}: {e}"))?;
    let sink = tmp
        .as_file()
        .try_clone()
        .map_err(|e| anyhow::anyhow!("preparing {out_path}: {e}"))?;
    let code = write_body(sink)?;
    if code == 0 {
        tmp.persist(out_path)
            .map_err(|e| anyhow::anyhow!("saving {out_path}: {}", e.error))?;
    }
    Ok(code)
}

/// Like [`dispatch`], but the remote command's stdout streams into the local
/// file at `out_path` instead of the terminal — for remote commands producing
/// a binary payload (`video shot`, `adb screencap`), where `--out <path>` must
/// mean the invoking machine's filesystem, not the control host's. See
/// [`capture_to_file`] for the write-then-rename guarantee.
pub fn dispatch_stdout_to_file(
    lab: &Lab,
    target: &str,
    host_name: &str,
    sub_argv: &[String],
    out_path: &str,
) -> anyhow::Result<i32> {
    let host = lab.host(host_name);
    let slice = build_slice(lab, target, host_name)?;
    let remote_path = ship_slice(&host, &slice)?;

    let mut argv = vec![host.paniolo(), "--lab".to_string(), remote_path.clone()];
    argv.extend(sub_argv.iter().cloned());

    let env = ssh::forwarded_env()?;
    let code = capture_to_file(out_path, |sink| {
        Ok(ssh::run_stdout_to(&host, &argv, &env, sink)?)
    });

    let _ = ssh::sftp_rm(&host, &remote_path);
    code
}

/// Read a daemon's discovery record — port and token — from its `daemon.json`
/// on `host`, or None. The path is resolved by a remote shell so the host's
/// own uid applies; must match `runtime_base()` in daemons.rs (and the daemon
/// crates). `subdir` is `<name>` for a host-singleton daemon or
/// `<name>/<target>` for a per-target one — build it with
/// [`crate::daemons::runtime_rel`]. The token crosses only the SSH session.
pub fn remote_daemon_endpoint(
    host: &crate::model::Host,
    subdir: &str,
) -> Option<crate::daemons::Endpoint> {
    let script = format!(
        "cat \"${{PANIOLO_RUNTIME_BASE:-/tmp}}/paniolo-$(id -u)/{subdir}/daemon.json\" 2>/dev/null"
    );
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
    crate::daemons::Endpoint::from_json(out.stdout.trim())
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

    /// A zero exit persists the sink's bytes onto `out_path`, replacing
    /// whatever was already there. Proves the rename actually happens — not
    /// just that the function returns without error.
    #[test]
    fn capture_to_file_persists_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("shot.png");
        std::fs::write(&out, b"stale png").unwrap();

        let code = capture_to_file(out.to_str().unwrap(), |mut sink| {
            use std::io::Write;
            sink.write_all(b"fresh png bytes")?;
            Ok(0)
        })
        .unwrap();

        assert_eq!(code, 0);
        assert_eq!(std::fs::read(&out).unwrap(), b"fresh png bytes");
        // No leftover temp file beside it.
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["shot.png"], "{names:?}");
    }

    /// A non-zero exit must not touch `out_path` at all — the old code
    /// (`File::create(out_path)` before the transfer even started) truncated
    /// it regardless of the outcome, destroying whatever was there on a
    /// failed capture. This is the case that regresses without the
    /// sibling-temp-file rewrite (Review "adb screencap on a remote channel").
    #[test]
    fn capture_to_file_leaves_the_original_untouched_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("shot.png");
        std::fs::write(&out, b"the only good copy").unwrap();

        let code = capture_to_file(out.to_str().unwrap(), |mut sink| {
            use std::io::Write;
            // The remote wrote a partial payload before failing.
            sink.write_all(b"partial garbage")?;
            Ok(1)
        })
        .unwrap();

        assert_eq!(code, 1);
        assert_eq!(std::fs::read(&out).unwrap(), b"the only good copy");
        // The temp file was cleaned up too, not left beside it.
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["shot.png"], "{names:?}");
    }

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
            ocr_mode = "gui"
            host = "bench2"
            [targets.fortune.usb]
            cmd = "ch9329 -d /dev/ttyUSB1"
            [targets.fortune.hid]
            cmd = "ch9329 -d /dev/ttyUSB1"
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
        // bench1 has netboot + the console serial + usb (inherited default
        // host); video and hid (bench2) are excluded.
        assert!(t.netboot.is_some());
        assert_eq!(t.serial.len(), 1);
        assert!(t.video.is_none());
        assert!(t.hid.is_none());
        let usb = t.usb.as_ref().expect("usb channel shipped in the slice");
        assert_eq!(usb.cmd.as_deref(), Some("ch9329 -d /dev/ttyUSB1"));
        // Host fields are stripped so the remote resolves them as local.
        assert!(t.host.is_none());
        assert!(t.serial[0].host.is_none());
        assert!(usb.host.is_none());
    }

    #[test]
    fn slice_for_the_other_host_carries_only_its_channels() {
        let s = build_slice(&lab(), "fortune", "bench2").unwrap();
        let reparsed = model::parse(&s).unwrap();
        let t = &reparsed.targets["fortune"];
        assert!(t.video.is_some());
        assert!(t.hid.is_some());
        assert!(t.usb.is_none());
        assert!(t.netboot.is_none());
        assert!(t.serial.is_empty());
    }

    /// `video.ocr_mode` used to be dropped by `build_slice` (it called
    /// `set_video` with a hardcoded `None`), so a remote target's GUI OCR
    /// mode silently reverted to the default engine after re-exec (Review
    /// M14a).
    #[test]
    fn slice_carries_the_videos_ocr_mode() {
        let s = build_slice(&lab(), "fortune", "bench2").unwrap();
        let reparsed = model::parse(&s).unwrap();
        let v = reparsed.targets["fortune"].video.as_ref().unwrap();
        assert_eq!(v.device.as_deref(), Some("/dev/video0"));
        assert_eq!(v.ocr_mode.as_deref(), Some("gui"));
        assert!(v.host.is_none());
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
