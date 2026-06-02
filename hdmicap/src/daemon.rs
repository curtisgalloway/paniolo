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

use std::fs::{self, File};
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::capture::DeviceSpec;
use crate::capture_thread;
use crate::server::{self, AppState};

#[derive(Serialize, Deserialize)]
pub struct Discovery {
    pub pid: u32,
    pub port: u16,
}

fn runtime_dir() -> Result<PathBuf> {
    // XDG_RUNTIME_DIR on Linux; a per-user tmp path on macOS via `directories`.
    let dirs = directories::BaseDirs::new().context("no base dirs")?;
    let base = dirs
        .runtime_dir()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("hdmicap");
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

/// Blocking entry point for `hdmicap daemon`. Builds the tokio runtime itself
/// so the capture thread can stay a plain std::thread alongside it.
pub fn run(device: DeviceSpec, port: u16) -> Result<()> {
    // 1. Acquire the advisory lock. Held for the lifetime of the process.
    let lock_file = File::create(lock_path()?)?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| anyhow!("another hdmicap daemon is already running"))?;

    // 2. Spawn the capture thread BEFORE the runtime. It owns the device and
    //    publishes into the watch channel.
    let (frames, _capture_handle) = capture_thread::spawn(device);

    // 3. Build a multi-thread runtime for axum and run the server.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let bound = listener.local_addr()?;

        // 4. Publish discovery info now that we have the real port.
        let disc = Discovery {
            pid: std::process::id(),
            port: bound.port(),
        };
        let mut f = File::create(discovery_path()?)?;
        f.write_all(serde_json::to_string(&disc)?.as_bytes())?;
        info!("hdmicap daemon listening on http://{bound}");

        let app = server::router(AppState { frames });

        // 5. Serve until SIGTERM/SIGINT. The /preview MJPEG stream is an
        //    infinite response, so a plain graceful shutdown would block on it
        //    forever. Remove the discovery file, give short in-flight requests a
        //    brief grace period, then hard-exit (the OS releases the device).
        let disc = discovery_path()?;
        let lock = lock_path()?;
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_signal().await;
                let _ = fs::remove_file(&disc);
                let _ = fs::remove_file(&lock);
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
