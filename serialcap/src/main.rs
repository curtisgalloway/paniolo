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

//! serialcap — serial console daemon + thin client CLI.
//!
//! Subcommands:
//!   daemon   own a serial port and serve it over a localhost WebSocket
//!   log      print captured serial output (timestamped, by line range)
//!   devices  list serial devices
//!   stop     ask the running daemon to exit (SIGTERM)

mod capture;
mod daemon;
mod serial_io;
mod server;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::serial_io::InterfaceSpec;

#[derive(Parser)]
#[command(name = "serialcap", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Parse a `NAME=DEVICE[@BAUD][:SENSE]` interface spec.
///
/// - `BAUD` defaults to 115200 when omitted.
/// - `SENSE` is optional; valid values are `cts`, `dsr`, `dcd`, `ri`.
///   It denotes which FTDI modem-control input is wired to the target's 3.3 V
///   rail for power-state sensing.
///
/// Example: `console=/dev/ttyUSB0@115200:cts`
fn parse_interface(s: &str) -> Result<InterfaceSpec, String> {
    let (name, rest) = s
        .split_once('=')
        .ok_or("expected NAME=DEVICE[@BAUD][:SENSE], e.g. console=/dev/ttyUSB0@115200:cts")?;
    if name.is_empty() {
        return Err("interface name is empty".into());
    }

    // Peel off optional :SENSE suffix (must be a known signal name).
    let (dev_baud, power_sense_signal) = if let Some((prefix, maybe_sense)) = rest.rsplit_once(':')
    {
        match maybe_sense {
            "cts" | "dsr" | "dcd" | "ri" => (prefix, Some(maybe_sense.to_string())),
            _ => {
                return Err(format!(
                    "unknown sense signal '{maybe_sense}'; valid values: cts, dsr, dcd, ri"
                ))
            }
        }
    } else {
        (rest, None)
    };

    let (device, baud) = match dev_baud.rsplit_once('@') {
        Some((dev, b)) => (
            dev,
            b.parse::<u32>()
                .map_err(|_| format!("invalid baud '{b}'"))?,
        ),
        None => (dev_baud, 115_200_u32),
    };
    if device.is_empty() {
        return Err("device path is empty".into());
    }
    Ok(InterfaceSpec {
        name: name.to_string(),
        device: device.to_string(),
        baud,
        power_sense_signal,
    })
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the serial console daemon (foreground; controller manages it).
    ///
    /// Owns one or more named interfaces; repeat --interface for each.
    Daemon {
        /// A serial interface as NAME=DEVICE[@BAUD] (repeatable), e.g.
        /// console=/dev/ttyUSB0@115200.
        #[arg(long = "interface", value_name = "NAME=DEVICE[@BAUD]", value_parser = parse_interface, required = true)]
        interfaces: Vec<InterfaceSpec>,
        /// Port to bind on localhost. 0 = OS-assigned.
        #[arg(long, default_value_t = 8724)]
        port: u16,
        /// Approximate number of recent lines retained on disk, per interface.
        #[arg(long, default_value_t = capture::DEFAULT_BUFFER_LINES)]
        buffer_lines: u64,
    },
    /// Print captured serial output. Reads the daemon's on-disk log directly, so
    /// it works whether or not the daemon is currently running.
    Log {
        /// Interface name. Optional when only one interface has been captured.
        #[arg(long, short = 'i')]
        interface: Option<String>,
        /// Show only the most recent N lines.
        #[arg(long, short = 'n')]
        tail: Option<u64>,
        /// Lowest line sequence number to include (inclusive).
        #[arg(long)]
        from: Option<u64>,
        /// Highest line sequence number to include (inclusive).
        #[arg(long)]
        to: Option<u64>,
        /// Show only lines newer than this sequence number (for polling).
        #[arg(long)]
        since: Option<u64>,
        /// Keep raw bytes (ANSI escapes, control chars) instead of cleaning them.
        #[arg(long)]
        raw: bool,
        /// Emit JSON Lines (seq, ts_ms, text) instead of formatted text.
        #[arg(long)]
        json: bool,
        /// Exclude the current unterminated line.
        #[arg(long)]
        no_pending: bool,
    },
    /// List available serial devices and exit (no daemon needed).
    Devices,
    /// Tell the running daemon to shut down.
    Stop,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "serialcap=info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Daemon {
            interfaces,
            port,
            buffer_lines,
        } => daemon::run(interfaces, port, buffer_lines),
        Cmd::Log {
            interface,
            tail,
            from,
            to,
            since,
            raw,
            json,
            no_pending,
        } => capture::cmd_log(capture::LogArgs {
            interface,
            tail,
            from,
            to,
            since,
            raw,
            json,
            no_pending,
        }),
        Cmd::Devices => cmd_devices(),
        Cmd::Stop => cmd_stop(),
    }
}

fn cmd_devices() -> Result<()> {
    for (path, misc) in serial_io::list_ports()? {
        println!("{path}  [{misc}]");
    }
    Ok(())
}

fn cmd_stop() -> Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let d = daemon::discover().context("is the daemon running?")?;
    kill(Pid::from_raw(d.pid as i32), Signal::SIGTERM)
        .context("failed to send SIGTERM to daemon")?;
    println!("daemon (pid {}) stopping", d.pid);
    Ok(())
}
