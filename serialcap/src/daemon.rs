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

//! Daemon lifecycle: advisory lock, discovery file, runtime wiring, shutdown.
//! Mirrors hdmicap's daemon so the two read the same way.

use std::fs::{self, File};
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::serial_io::{InterfaceSpec, Serials};
use crate::server::{self, AppState};

#[derive(Serialize, Deserialize)]
pub struct DiscoveryInterface {
    pub name: String,
    pub device: String,
    pub baud: u32,
}

#[derive(Serialize, Deserialize)]
pub struct Discovery {
    pub pid: u32,
    pub port: u16,
    pub interfaces: Vec<DiscoveryInterface>,
}

pub fn runtime_dir() -> Result<PathBuf> {
    let dirs = directories::BaseDirs::new().context("no base dirs")?;
    let base = dirs
        .runtime_dir()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("serialcap");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn lock_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("daemon.lock"))
}

fn discovery_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("daemon.json"))
}

/// Read the discovery file so the CLI knows which port to hit.
pub fn discover() -> Result<Discovery> {
    let p = discovery_path()?;
    let s = fs::read_to_string(&p).with_context(|| format!("daemon not running? {p:?}"))?;
    Ok(serde_json::from_str(&s)?)
}

/// Blocking entry point for `serialcap daemon`.
pub fn run(interfaces: Vec<InterfaceSpec>, port: u16, buffer_lines: u64) -> Result<()> {
    if interfaces.is_empty() {
        return Err(anyhow!("no serial interfaces specified"));
    }

    let lock_file = File::create(lock_path()?)?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| anyhow!("another serialcap daemon is already running"))?;

    let capture_dir = crate::capture::capture_dir()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move {
        // The serial supervisors use tokio::spawn, so start them inside the runtime.
        let serials = Serials::spawn_all(&interfaces, &capture_dir, buffer_lines);

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let bound = listener.local_addr()?;

        let disc = Discovery {
            pid: std::process::id(),
            port: bound.port(),
            interfaces: interfaces
                .iter()
                .map(|i| DiscoveryInterface {
                    name: i.name.clone(),
                    device: i.device.clone(),
                    baud: i.baud,
                })
                .collect(),
        };
        let mut f = File::create(discovery_path()?)?;
        f.write_all(serde_json::to_string(&disc)?.as_bytes())?;
        info!(
            "serialcap daemon listening on http://{bound} ({} interface(s))",
            interfaces.len()
        );

        let app = server::router(AppState { serials });

        // The /stream WebSocket is long-lived, so a plain graceful shutdown
        // would block on it forever. Remove the discovery file, give short
        // in-flight requests a brief grace period, then hard-exit (the OS
        // releases the serial port).
        let disc = discovery_path()?;
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_signal().await;
                let _ = fs::remove_file(&disc);
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                info!("daemon shut down");
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
