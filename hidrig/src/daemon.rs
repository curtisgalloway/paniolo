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

//! Daemon lifecycle for `hidrig serve`: advisory lock, discovery file, tokio
//! runtime, graceful shutdown. Mirrors serialcap/hdmicap so the three read the
//! same way and paniolo discovers them identically.
//!
//! The discovery directory is the **channel** name `hid` (not `hidrig`), under
//! `/tmp/paniolo-<uid>/hid/daemon.json`, so paniolo finds the daemon without
//! knowing which helper implements the channel. The file records the owned
//! `device` so a CLI one-shot can tell whether the running daemon owns *its*
//! UART before routing through it.

use std::fs::{self, File};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::server::{self, AppState};
use crate::uart::HidHandle;

/// Discovery subdir = the paniolo channel name, not the binary name.
pub const DISCOVERY_NAME: &str = "hid";

/// How long shutdown waits for the release of held keys and buttons before
/// exiting anyway.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Serialize, Deserialize)]
pub struct Discovery {
    pub pid: u32,
    pub port: u16,
    /// The bearer token every request to this daemon must carry (see
    /// auth.rs). Optional on read so a file written by an older daemon still
    /// parses; always written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The control UART this daemon owns (so a CLI one-shot can match its -d).
    pub device: String,
    /// The path paniolo's `serial` channel points its `device =` at when the
    /// console bridge is up: the stable symlink `<runtime>/hid/console` when
    /// it could be made, else the slave device itself. Absent if PTY
    /// allocation failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console: Option<String>,
    /// The PTY slave device node behind `console` (`/dev/pts/7`, or
    /// `/dev/ttys003` on macOS). This file is the source of truth for the
    /// console's identity; the symlink is a convenience for a lab file that
    /// wants a stable path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console_device: Option<String>,
}

/// The daemon's runtime dir. Paniolo passes the canonical location as
/// `PANIOLO_RUNTIME_DIR` (named for the hid *channel*, not this binary —
/// any conforming injector helper serves the same discovery dir); the
/// literal fallback below is for standalone invocations and matches it:
/// `/tmp/paniolo-<uid>/hid` (deliberately not `$TMPDIR`/`$XDG_RUNTIME_DIR`
/// — see the paniolo CLI's daemons.rs for why).
pub fn runtime_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("PANIOLO_RUNTIME_DIR") {
        let dir = PathBuf::from(dir);
        fs::create_dir_all(&dir)?;
        return Ok(dir);
    }
    let uid = crate::platform::current_uid();
    let base = crate::platform::runtime_root().join(format!("paniolo-{uid}"));
    crate::platform::ensure_private_dir(&base)?;
    let dir = base.join(DISCOVERY_NAME);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn lock_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("daemon.lock"))
}

fn discovery_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("daemon.json"))
}

/// Read the discovery file, or None if no daemon is recorded / it's dead.
pub fn discover() -> Option<Discovery> {
    let s = fs::read_to_string(discovery_path().ok()?).ok()?;
    let d: Discovery = serde_json::from_str(&s).ok()?;
    // Liveness: the recorded pid still exists.
    if !crate::platform::pid_alive(d.pid as i32) {
        return None;
    }
    Some(d)
}

/// Blocking entry point for `hidrig serve`.
pub fn run(device: String, port: u16) -> Result<()> {
    let lock_file = File::create(lock_path()?)?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| anyhow!("another hid daemon is already running"))?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move {
        // Bring up the DUT serial-console PTY and publish a stable symlink that
        // paniolo's `serial` channel points its `device =` at. Best-effort: if
        // PTY allocation fails the daemon still serves HID/control, just without
        // a console. `link` is set only when we created a symlink, so shutdown
        // removes ours and never the /dev/pts node itself. The slave handle is
        // bound here, in this block, so it stays open until the process exits
        // (pty.rs explains why it must).
        let ConsoleBridge {
            master: console_master,
            slave: _console_slave,
            published: console_path,
            device: console_device,
            link: console_link,
        } = open_console_bridge(&runtime_dir()?);

        let hid = HidHandle::spawn(device.clone(), console_master);

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let bound = listener.local_addr()?;

        // Every request must present this token (auth.rs); it reaches clients
        // only through the owner-only discovery file.
        let token = crate::auth::generate_token()?;
        let disc = Discovery {
            pid: std::process::id(),
            port: bound.port(),
            token: Some(token.clone()),
            device: device.clone(),
            console: console_path,
            console_device,
        };
        crate::auth::write_private_file(
            &discovery_path()?,
            serde_json::to_string(&disc)?.as_bytes(),
        )
        .context("writing discovery file")?;
        info!("hid daemon listening on http://{bound} (device {device})");

        let shutdown_hid = hid.clone();
        let app = server::router(AppState { hid }, crate::auth::Auth::new(token, &[]));

        // The /hid WebSocket is long-lived, so plain graceful shutdown would
        // block forever. Release whatever is held, remove discovery + lock,
        // brief grace, then hard-exit (the OS releases the UART).
        let disc_p = discovery_path()?;
        let lock_p = lock_path()?;
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_signal().await;
                // This daemon is the only thing that remembers what it pressed;
                // leave the target with nothing held.
                if let Err(e) = shutdown_hid.release_for_shutdown(RELEASE_TIMEOUT).await {
                    warn!("shutdown: {e}");
                }
                let _ = fs::remove_file(&disc_p);
                let _ = fs::remove_file(&lock_p);
                if let Some(link) = &console_link {
                    let _ = fs::remove_file(link);
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
                info!("hid daemon shut down");
                std::process::exit(0);
            })
            .await?;

        Ok::<(), anyhow::Error>(())
    })?;

    drop(lock_file);
    Ok(())
}

/// The DUT console bridge as brought up at start. Every field is `None` when
/// no console could be made — the daemon then serves HID and control without
/// one, which is the documented best-effort behaviour.
#[derive(Default)]
struct ConsoleBridge {
    /// PTY master, handed to the control-link owner thread.
    master: Option<File>,
    /// The daemon's own raw-mode handle on the slave, held open for the
    /// daemon's lifetime (see pty.rs).
    slave: Option<File>,
    /// What the discovery file's `console` names: the stable symlink, or the
    /// device itself when the link could not be made.
    published: Option<String>,
    /// The slave device node (`console_device` in the discovery file).
    device: Option<String>,
    /// The symlink we created, removed at shutdown.
    link: Option<PathBuf>,
}

/// Allocate the DUT console PTY and publish a stable symlink to its slave node.
#[cfg(unix)]
fn open_console_bridge(dir: &Path) -> ConsoleBridge {
    let p = match crate::pty::open() {
        Ok(p) => p,
        Err(e) => {
            warn!("DUT console bridge unavailable: {e}");
            return ConsoleBridge::default();
        }
    };
    let link = dir.join("console");
    remove_stale_console_link(&link);
    let (published, owned_link) = match std::os::unix::fs::symlink(&p.slave_path, &link) {
        Ok(()) => (link.to_string_lossy().into_owned(), Some(link)),
        Err(e) => {
            warn!(
                "console symlink failed ({e}); exposing {} directly",
                p.slave_path
            );
            (p.slave_path.clone(), None)
        }
    };
    info!(
        "DUT console bridge at {published} (device {})",
        p.slave_path
    );
    ConsoleBridge {
        master: Some(p.master),
        slave: Some(p.slave),
        published: Some(published),
        device: Some(p.slave_path),
        link: owned_link,
    }
}

/// A `console` link left behind by a previous daemon — pointing at a PTY that
/// no longer exists, or at one that now belongs to some other process — is
/// removed so ours can take the name. Anything there that is not a symlink is
/// left alone; publication then falls back to the device path.
#[cfg(unix)]
fn remove_stale_console_link(link: &Path) {
    match fs::symlink_metadata(link) {
        Ok(md) if md.file_type().is_symlink() => {
            match fs::read_link(link) {
                Ok(target) => info!(
                    "removing stale console link {} -> {}",
                    link.display(),
                    target.display()
                ),
                Err(_) => info!("removing stale console link {}", link.display()),
            }
            if let Err(e) = fs::remove_file(link) {
                warn!("cannot remove stale console link {}: {e}", link.display());
            }
        }
        Ok(_) => warn!(
            "{} exists and is not a symlink; leaving it alone",
            link.display()
        ),
        Err(_) => {}
    }
}

/// Windows has no PTY layer to hand a slave device path to paniolo's `serial`
/// channel, so the console bridge is simply absent. (A ConPTY pseudoconsole is
/// not a substitute: it has no filesystem node another process can open, which
/// is the whole point of the published `console` path.) HID and control are
/// unaffected.
#[cfg(windows)]
fn open_console_bridge(_dir: &Path) -> ConsoleBridge {
    warn!("DUT console bridge unavailable: no PTY support on Windows");
    ConsoleBridge::default()
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let term = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    info!("shutdown signal received");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The discovery file now carries the token. One written by an older
    /// daemon (no token) must still parse, so a one-shot can still route
    /// through it and the CLI can tell the operator to restart it.
    #[test]
    fn discovery_token_is_optional_on_read_and_written_when_present() {
        let old: Discovery =
            serde_json::from_str(r#"{"pid":1,"port":2,"device":"/dev/x"}"#).unwrap();
        assert_eq!(old.token, None);
        assert_eq!(old.console_device, None);
        let new = Discovery {
            pid: 1,
            port: 2,
            token: Some("ab".into()),
            device: "/dev/x".into(),
            console: Some("/run/hid/console".into()),
            console_device: Some("/dev/pts/7".into()),
        };
        let text = serde_json::to_string(&new).unwrap();
        assert!(text.contains(r#""token":"ab""#), "{text}");
        assert!(text.contains(r#""console_device":"/dev/pts/7""#), "{text}");
        let back: Discovery = serde_json::from_str(&text).unwrap();
        assert_eq!(back.token.as_deref(), Some("ab"));
        assert_eq!(back.console.as_deref(), Some("/run/hid/console"));
        assert_eq!(back.console_device.as_deref(), Some("/dev/pts/7"));
    }

    /// A `console` link left by a previous daemon (here: dangling) is replaced
    /// by one to our own PTY, and the published path and device agree with it.
    #[cfg(unix)]
    #[test]
    fn stale_console_link_is_replaced_on_start() {
        let dir = std::env::temp_dir().join(format!("paniolo-hid-console-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let link = dir.join("console");
        std::os::unix::fs::symlink("/dev/pts/paniolo-does-not-exist", &link).unwrap();
        assert!(fs::metadata(&link).is_err(), "the stale link dangles");

        let bridge = open_console_bridge(&dir);
        let device = bridge.device.clone().expect("a pty was allocated");
        assert_eq!(bridge.published.as_deref(), link.to_str());
        assert_eq!(bridge.link.as_deref(), Some(link.as_path()));
        assert_eq!(fs::read_link(&link).unwrap(), PathBuf::from(&device));
        assert!(
            fs::metadata(&link).is_ok(),
            "the link resolves to a live pty"
        );
        assert!(bridge.master.is_some() && bridge.slave.is_some());

        drop(bridge);
        let _ = fs::remove_dir_all(&dir);
    }
}
