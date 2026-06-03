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

//! SSH transport for driving a target on a remote control host.
//!
//! Ported from the Python `_ssh.py`. The dev machine is the hub; every command
//! against a remote target reaches its control host over SSH. There is no agent
//! or RPC server — `ssh` is the whole transport. A per-host **ControlMaster**
//! connection means only the first call to a host pays the handshake.
//!
//! A [`Host`] whose `ssh` destination is `"local"` is the dev machine itself;
//! the SSH functions must not be called for it (callers run those commands
//! directly).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::model::{expand_tilde, Host, LOCAL};

const CONTROL_PERSIST: &str = "300";
const CONNECT_TIMEOUT: &str = "10";

fn uid() -> u32 {
    // Safe: getuid() has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// Short directory holding paniolo's default ControlMaster sockets. Kept short
/// because a Unix-domain socket path is length-limited and ssh appends a 40-char
/// `%C` hash; `$XDG_RUNTIME_DIR` is short on Linux, else `/tmp`.
fn control_dir() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let d = PathBuf::from(base).join(format!("paniolo-{}", uid()));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn control_args(host: &Host) -> Vec<String> {
    let cp = match &host.control_path {
        Some(p) => expand_tilde(p).to_string_lossy().into_owned(),
        None => control_dir().join("cm-%C").to_string_lossy().into_owned(),
    };
    vec![
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        format!("ControlPath={cp}"),
        "-o".into(),
        format!("ControlPersist={CONTROL_PERSIST}"),
    ]
}

/// Base `ssh` argv (program + options) for a non-local host. `multiplex=false`
/// gives a standalone connection — a port forward must own its channel: an
/// `ssh -N -L` attached to a ControlMaster hands the forward to the master and
/// exits, so the process no longer represents (or can tear down) the tunnel.
fn base_args(host: &Host, interactive: bool, multiplex: bool) -> Vec<String> {
    debug_assert!(host.ssh != LOCAL, "ssh called for the local host");
    let mut a = vec!["ssh".to_string()];
    if !interactive {
        // Fail rather than block on a password prompt for non-interactive use.
        a.push("-o".into());
        a.push("BatchMode=yes".into());
    }
    a.push("-o".into());
    a.push(format!("ConnectTimeout={CONNECT_TIMEOUT}"));
    if let Some(id) = &host.identity {
        a.push("-i".into());
        a.push(expand_tilde(id).to_string_lossy().into_owned());
        a.push("-o".into());
        a.push("IdentitiesOnly=yes".into());
    }
    if multiplex {
        a.extend(control_args(host));
    } else {
        a.extend(
            ["-o", "ControlMaster=no", "-o", "ControlPath=none"]
                .iter()
                .map(|s| s.to_string()),
        );
    }
    a
}

/// Quote a single token for a POSIX shell.
pub fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@%_+=:,./-".contains(&b));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Quote argv (and optional `KEY=val` env assignments) into one remote command,
/// preserving argument boundaries through the remote shell.
pub fn remote_command(argv: &[String], env: &[(String, String)]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in env {
        parts.push(format!("{k}={}", shell_quote(v)));
    }
    parts.extend(argv.iter().map(|a| shell_quote(a)));
    parts.join(" ")
}

/// Result of a captured remote command.
pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run `argv` on `host` and capture its output (never errors on non-zero exit).
pub fn run(
    host: &Host,
    argv: &[String],
    stdin: Option<&str>,
    env: &[(String, String)],
) -> std::io::Result<Output> {
    let mut cmd = Command::new("ssh");
    cmd.args(&base_args(host, false, true)[1..]);
    cmd.arg(&host.ssh);
    cmd.arg(remote_command(argv, env));
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    if let Some(data) = stdin {
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(data.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    Ok(Output {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Run `argv` on `host` with the local terminal's stdio passed through (no PTY).
/// This is the transparent re-exec path. Returns the exit code.
pub fn run_passthrough(
    host: &Host,
    argv: &[String],
    env: &[(String, String)],
) -> std::io::Result<i32> {
    let status = Command::new("ssh")
        .args(&base_args(host, false, true)[1..])
        .arg(&host.ssh)
        .arg(remote_command(argv, env))
        .status()?;
    Ok(status.code().unwrap_or(-1))
}

/// Run `argv` on `host` over an `ssh -t` PTY (for interactive tools like tio).
pub fn run_interactive(
    host: &Host,
    argv: &[String],
    env: &[(String, String)],
) -> std::io::Result<i32> {
    let status = Command::new("ssh")
        .args(&base_args(host, true, true)[1..])
        .arg("-t")
        .arg(&host.ssh)
        .arg(remote_command(argv, env))
        .status()?;
    Ok(status.code().unwrap_or(-1))
}

/// A held `ssh -L` tunnel to a port on `host`; killed on drop.
pub struct Forward {
    pub local_port: u16,
    child: std::process::Child,
}

impl Drop for Forward {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_local_port() -> std::io::Result<u16> {
    // Small TOCTOU window is acceptable.
    let l = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(l.local_addr()?.port())
}

/// Open an `ssh -L` tunnel to `127.0.0.1:remote_port` on `host`, returning once
/// the local end accepts connections. The forwarder is standalone (not
/// multiplexed) so killing it reliably tears the tunnel down.
pub fn forward(host: &Host, remote_port: u16) -> anyhow::Result<Forward> {
    use anyhow::bail;
    let local_port = free_local_port()?;
    let spec = format!("{local_port}:127.0.0.1:{remote_port}");
    let mut child = Command::new("ssh")
        .args(&base_args(host, false, false)[1..])
        .arg("-N")
        .arg("-L")
        .arg(&spec)
        .arg(&host.ssh)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            bail!(
                "ssh forward to {}:{remote_port} exited early ({status})",
                host.ssh
            );
        }
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], local_port));
        if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(500))
            .is_ok()
        {
            return Ok(Forward { local_port, child });
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            bail!("timed out waiting for forwarded port {local_port}");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_leaves_safe_tokens_bare() {
        assert_eq!(shell_quote("/dev/ttyUSB0"), "/dev/ttyUSB0");
        assert_eq!(shell_quote("user@host"), "user@host");
    }

    #[test]
    fn shell_quote_wraps_specials() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn remote_command_prepends_env() {
        let argv = vec![
            "paniolo".to_string(),
            "netboot".to_string(),
            "start".to_string(),
        ];
        let env = vec![("PANIOLO_LAB".to_string(), "/tmp/s lice".to_string())];
        assert_eq!(
            remote_command(&argv, &env),
            "PANIOLO_LAB='/tmp/s lice' paniolo netboot start"
        );
    }

    #[test]
    fn base_args_include_batch_and_control_master() {
        let host = Host {
            ssh: "u@bench1".into(),
            identity: Some("~/.ssh/id".into()),
            ..Default::default()
        };
        let a = base_args(&host, false, true).join(" ");
        assert!(a.contains("BatchMode=yes"), "{a}");
        assert!(a.contains("ControlMaster=auto"), "{a}");
        assert!(a.contains("IdentitiesOnly=yes"), "{a}");
        // Interactive variant drops BatchMode (so a PTY/password can work).
        assert!(!base_args(&host, true, true).join(" ").contains("BatchMode"));
    }
}
