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

//! cambrionix — drive a Cambrionix USB hub's serial command-line API.
//!
//! Subcommands:
//!   state [port]         Print a table of all ports, or `on`/`off` for one port.
//!   on <port>            Set a port to charge mode (mode c).
//!   off <port>           Set a port off (mode o).
//!   cycle <port>         Power-cycle a port: off → delay → restore → confirm.

mod proto;

use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};

use proto::{open_port, parse_state, run_command, PortRow};

#[derive(Parser)]
#[command(
    name = "cambrionix",
    version,
    about = "Cambrionix USB hub port control"
)]
struct Cli {
    /// Hub control UART device path (e.g. /dev/cu.usbserial-XXXX).  Required.
    // Option<String> because clap does not permit global required args; main()
    // validates presence before dispatch.
    #[arg(short = 'd', long = "device", value_name = "DEVICE", global = true)]
    device: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print port state. Without <port>, prints a table of all ports.
    /// With <port>, prints exactly `on` or `off` (machine-readable).
    State {
        /// Port number to query (1–15). Omit to print the full table.
        #[arg(value_parser = clap::value_parser!(u8).range(1..=15))]
        port: Option<u8>,
    },
    /// Set a port to charge mode (mode c <port>).
    On {
        /// Port number (1–15).
        #[arg(value_parser = clap::value_parser!(u8).range(1..=15))]
        port: u8,
    },
    /// Set a port off (mode o <port>).
    Off {
        /// Port number (1–15).
        #[arg(value_parser = clap::value_parser!(u8).range(1..=15))]
        port: u8,
    },
    /// Power-cycle a port: turn it off, wait, then restore its previous mode.
    Cycle {
        /// Port number (1–15).
        #[arg(value_parser = clap::value_parser!(u8).range(1..=15))]
        port: u8,
        /// Milliseconds to hold the port off before restoring.
        #[arg(long, default_value_t = 3000)]
        delay_ms: u64,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let device = cli
        .device
        .as_deref()
        .ok_or_else(|| anyhow!("required argument '--device <DEVICE>' (-d) was not provided"))?;
    match cli.cmd {
        Cmd::State { port } => cmd_state(device, port),
        Cmd::On { port } => cmd_on(device, port),
        Cmd::Off { port } => cmd_off(device, port),
        Cmd::Cycle { port, delay_ms } => cmd_cycle(device, port, delay_ms),
    }
}

/// Open the hub serial port and pause briefly to let it settle.
fn connect(device: &str) -> Result<Box<dyn serialport::SerialPort>> {
    let port = open_port(device)?;
    thread::sleep(Duration::from_millis(200));
    Ok(port)
}

/// Flush any pending input by reading until the port times out.
fn flush_input(port: &mut Box<dyn serialport::SerialPort>) {
    let _ = port.clear(serialport::ClearBuffer::Input);
}

/// Issue the `state` command and return the parsed rows.
fn fetch_state(port: &mut Box<dyn serialport::SerialPort>) -> Result<Vec<PortRow>> {
    flush_input(port);
    let raw = run_command(port, "state")?;
    Ok(parse_state(&raw))
}

/// Find a specific port row, returning a descriptive error if absent.
fn find_port(rows: &[PortRow], port_num: u8) -> Result<&PortRow> {
    rows.iter()
        .find(|r| r.port == port_num)
        .ok_or_else(|| anyhow!("port {port_num} not found in hub response"))
}

fn cmd_state(device: &str, port_filter: Option<u8>) -> Result<()> {
    let mut port = connect(device)?;
    let rows = fetch_state(&mut port)?;

    if rows.is_empty() {
        return Err(anyhow!("hub returned no state rows — check device path"));
    }

    match port_filter {
        None => {
            // Print a human-readable table.
            println!(
                "{:<5} {:<9} {:<8} {:<6} {:<6} Rest",
                "Port", "Volts×100", "mA", "Attach", "Mode"
            );
            println!("{}", "-".repeat(60));
            for r in &rows {
                println!(
                    "{:<5} {:<9} {:<8} {:<6} {:<6} {}",
                    r.port,
                    r.volts_raw,
                    r.milliamps,
                    r.attach,
                    r.mode,
                    r.rest.trim()
                );
            }
        }
        Some(p) => {
            let r = find_port(&rows, p)?;
            if r.is_on() {
                println!("on");
            } else {
                println!("off");
            }
        }
    }
    Ok(())
}

fn cmd_on(device: &str, port_num: u8) -> Result<()> {
    let mut port = connect(device)?;
    flush_input(&mut port);
    run_command(&mut port, &format!("mode c {port_num}"))?;
    println!("port {port_num}: charge mode enabled");
    Ok(())
}

fn cmd_off(device: &str, port_num: u8) -> Result<()> {
    let mut port = connect(device)?;
    flush_input(&mut port);
    run_command(&mut port, &format!("mode o {port_num}"))?;
    println!("port {port_num}: off");
    Ok(())
}

fn cmd_cycle(device: &str, port_num: u8, delay_ms: u64) -> Result<()> {
    let mut port = connect(device)?;

    // Read the current mode so we can restore it afterwards.
    let rows = fetch_state(&mut port)?;
    let row = find_port(&rows, port_num)?;
    let prev_mode = row.mode;

    // Turn the port off.
    flush_input(&mut port);
    run_command(&mut port, &format!("mode o {port_num}"))?;

    thread::sleep(Duration::from_millis(delay_ms));

    // Restore: Sync→ mode s, anything else → mode c.
    let restore_cmd = if prev_mode.eq_ignore_ascii_case(&'S') {
        format!("mode s {port_num}")
    } else {
        format!("mode c {port_num}")
    };
    flush_input(&mut port);
    run_command(&mut port, &restore_cmd)?;

    // Confirm the port is back on.
    let rows_after = fetch_state(&mut port)?;
    let after = find_port(&rows_after, port_num)?;
    if after.is_on() {
        println!(
            "port {port_num}: cycled (was mode {prev_mode}, restored to mode {}, now on)",
            after.mode
        );
    } else {
        return Err(anyhow!(
            "port {port_num}: cycled but port reports mode {} (off) after restore",
            after.mode
        ));
    }
    Ok(())
}
