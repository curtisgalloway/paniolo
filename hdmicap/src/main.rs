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

//! hdmicap — warm-stream HDMI capture daemon + thin client CLI.
//!
//! Subcommands:
//!   daemon   run the capture daemon (the controller starts this)
//!   devices  list capture devices
//!   shot     fetch one PNG from a running daemon (--stable, --out)
//!   watch    block until the screen changes, then print the new hash
//!   stop     ask the running daemon to exit
//!   preview  print the URL to open in a browser

mod capture;
mod capture_thread;
mod daemon;
mod frame;
// Pixel formats and conversions exist to serve a capture backend. On a
// platform with none compiled in (Windows), nothing constructs them — that is
// the design, not dead weight.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
mod pixel;
// Not every helper needs every primitive in here (hdmicap and serialcap have
// no pid-liveness probe of their own), and the file is kept byte-identical
// across the crates that copy it, so unused items are expected rather than a
// smell.
#[allow(dead_code)]
mod platform;
mod server;

use std::io::{Read, Write};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use capture::DeviceSpec;

#[derive(Parser)]
#[command(name = "hdmicap", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the capture daemon (foreground; controller manages the process).
    Daemon {
        /// Device: "auto" (default), an index, or a name substring.
        #[arg(long, default_value = "auto")]
        device: String,
        /// Port to bind on localhost. 0 = OS-assigned.
        #[arg(long, default_value_t = 8723)]
        port: u16,
    },
    /// List available capture devices and exit (no daemon needed).
    Devices {
        /// Emit machine-readable JSON instead of the human table.
        #[arg(long)]
        json: bool,
        /// Include internal platform video nodes (SoC pipeline stages, codecs)
        /// normally hidden from the listing.
        #[arg(long)]
        all: bool,
    },
    /// Fetch one screenshot from the running daemon.
    Shot {
        /// Wait until the signal is Stable before capturing.
        #[arg(long)]
        stable: bool,
        /// Only return once the frame differs from this hex hash.
        #[arg(long)]
        changed_since: Option<String>,
        /// Timeout in ms.
        #[arg(long, default_value_t = 2000)]
        timeout: u64,
        /// Output path; "-" for stdout.
        #[arg(long, default_value = "-")]
        out: String,
    },
    /// Block until the screen changes vs the current frame; print the new hash.
    Watch {
        #[arg(long, default_value_t = 30000)]
        timeout: u64,
    },
    /// Print the preview URL (open in a browser).
    Preview,
    /// Tell the running daemon to shut down.
    Stop,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hdmicap=info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Daemon { device, port } => daemon::run(DeviceSpec::parse(&device), port),
        Cmd::Devices { json, all } => cmd_devices(json, all),
        Cmd::Shot {
            stable,
            changed_since,
            timeout,
            out,
        } => cmd_shot(stable, changed_since, timeout, &out),
        Cmd::Watch { timeout } => cmd_watch(timeout),
        Cmd::Preview => cmd_preview(),
        Cmd::Stop => cmd_stop(),
    }
}

fn base_url() -> Result<String> {
    let d = daemon::discover()?;
    Ok(format!("http://127.0.0.1:{}", d.port))
}

fn cmd_devices(json: bool, all: bool) -> Result<()> {
    // The default listing hides nodes that can't capture (Linux: SoC pipeline
    // stages, codecs, UVC metadata nodes — filtered by V4L2 device caps in
    // capture::enumerate_capture); --all shows the raw enumeration.
    let devices = if all {
        capture::enumerate()?
    } else {
        capture::enumerate_capture()?
    };
    if json {
        let list: Vec<serde_json::Value> = devices
            .iter()
            .map(|d| {
                serde_json::json!({"index": d.index, "name": d.name, "misc": d.misc, "id": d.id})
            })
            .collect();
        println!("{}", serde_json::to_string(&list)?);
        return Ok(());
    }
    for d in devices {
        println!("{:>3}  {}  [{}]  id={}", d.index, d.name, d.misc, d.id);
    }
    Ok(())
}

fn cmd_shot(stable: bool, changed_since: Option<String>, timeout: u64, out: &str) -> Result<()> {
    let url = base_url().context("is the daemon running? try `hdmicap daemon`")?;
    let mut snap_url = format!("{url}/snapshot?timeout={timeout}");
    if stable {
        snap_url.push_str("&wait=stable");
    }
    if let Some(ref hash) = changed_since {
        snap_url.push_str(&format!("&changed_since={hash}"));
    }

    let resp = ureq::get(&snap_url)
        .call()
        .context("GET /snapshot failed")?;

    let timed_out = resp.header("x-timeout") == Some("1");
    let signal = resp.header("x-signal").unwrap_or("unknown").to_string();
    let hash = resp.header("x-frame-hash").unwrap_or("").to_string();
    eprintln!(
        "signal={}  hash={}{}",
        signal,
        hash,
        if timed_out { "  (timeout)" } else { "" }
    );

    let mut body = Vec::new();
    resp.into_reader().read_to_end(&mut body)?;

    if out == "-" {
        std::io::stdout().write_all(&body)?;
    } else {
        std::fs::write(out, &body).with_context(|| format!("writing {out}"))?;
        eprintln!("wrote {} bytes to {out}", body.len());
    }
    Ok(())
}

fn cmd_watch(timeout: u64) -> Result<()> {
    let url = base_url().context("is the daemon running? try `hdmicap daemon`")?;

    // Read the current hash so we can long-poll for a change.
    let body = ureq::get(&format!("{url}/status"))
        .call()
        .context("GET /status failed")?
        .into_string()
        .context("reading /status body")?;
    let status: serde_json::Value = serde_json::from_str(&body).context("parsing /status JSON")?;
    let hash = status["hash"]
        .as_str()
        .unwrap_or("0000000000000000")
        .to_string();

    // Block until the frame changes or we time out.
    let resp = ureq::get(&format!(
        "{url}/snapshot?changed_since={hash}&timeout={timeout}"
    ))
    .call()
    .context("GET /snapshot (changed_since) failed")?;

    let new_hash = resp.header("x-frame-hash").unwrap_or("").to_string();
    let timed_out = resp.header("x-timeout") == Some("1");
    if timed_out {
        anyhow::bail!("timed out waiting for screen change after {timeout}ms");
    }
    println!("{new_hash}");
    Ok(())
}

fn cmd_preview() -> Result<()> {
    let url = base_url().context("is the daemon running? try `hdmicap daemon`")?;
    println!("Open {url}/preview in a browser to watch the screen.");
    Ok(())
}

fn cmd_stop() -> Result<()> {
    let d = daemon::discover().context("is the daemon running?")?;
    crate::platform::terminate_pid(d.pid as i32).context("failed to send SIGTERM to daemon")?;
    println!("daemon (pid {}) stopping", d.pid);
    Ok(())
}
