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
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::server::{self, AppState};
use crate::uart::HidHandle;

/// Discovery subdir = the paniolo channel name, not the binary name.
pub const DISCOVERY_NAME: &str = "hid";

#[derive(Serialize, Deserialize)]
pub struct Discovery {
    pub pid: u32,
    pub port: u16,
    /// The control UART this daemon owns (so a CLI one-shot can match its -d).
    pub device: String,
}

/// The daemon's runtime dir. Paniolo passes the canonical location as
/// `PANIOLO_RUNTIME_DIR` (named for the hid *channel*, not this binary —
/// any conforming injector helper serves the same discovery dir); the
/// literal fallback below is for standalone invocations and matches it:
/// `/tmp/paniolo-<uid>/hid` (deliberately not `$TMPDIR`/`$XDG_RUNTIME_DIR`
/// — see the paniolo CLI's daemons.rs for why).
pub fn runtime_dir() -> Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    if let Some(dir) = std::env::var_os("PANIOLO_RUNTIME_DIR") {
        let dir = PathBuf::from(dir);
        fs::create_dir_all(&dir)?;
        return Ok(dir);
    }
    // Safe: getuid always succeeds.
    let uid = unsafe { libc::getuid() };
    let base = PathBuf::from(format!("/tmp/paniolo-{uid}"));
    match fs::DirBuilder::new().mode(0o700).create(&base) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let md = fs::symlink_metadata(&base)?;
            if !md.is_dir() || md.uid() != uid {
                return Err(anyhow!(
                    "{} exists but is not a directory owned by uid {uid}",
                    base.display()
                ));
            }
        }
        Err(e) => return Err(e.into()),
    }
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
    if unsafe { libc::kill(d.pid as i32, 0) } != 0 {
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
        let hid = HidHandle::spawn(device.clone());

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let bound = listener.local_addr()?;

        let disc = Discovery {
            pid: std::process::id(),
            port: bound.port(),
            device: device.clone(),
        };
        let mut f = File::create(discovery_path()?).context("writing discovery file")?;
        f.write_all(serde_json::to_string(&disc)?.as_bytes())?;
        info!("hid daemon listening on http://{bound} (device {device})");

        let app = server::router(AppState { hid });

        // The /hid WebSocket is long-lived, so plain graceful shutdown would
        // block forever. Remove discovery + lock, brief grace, then hard-exit
        // (the OS releases the UART).
        let disc_p = discovery_path()?;
        let lock_p = lock_path()?;
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_signal().await;
                let _ = fs::remove_file(&disc_p);
                let _ = fs::remove_file(&lock_p);
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                info!("hid daemon shut down");
                std::process::exit(0);
            })
            .await?;

        Ok::<(), anyhow::Error>(())
    })?;

    drop(lock_file);
    Ok(())
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
