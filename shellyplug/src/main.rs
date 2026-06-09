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

//! shellyplug — power control for Shelly Gen2+ smart plugs and relays.
//!
//! A one-shot paniolo power helper: each invocation makes a single stateless
//! HTTP RPC call to the device and exits. Hook-facing subcommands follow the
//! paniolo helper conventions (docs/adding-power-helpers.md):
//!   state <id>    prints exactly `on` or `off`
//!   on/off <id>   switch + read-back confirm
//!   cycle <id>    off → delay → on → confirm

mod rpc;

use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use clap::{Parser, Subcommand};

use rpc::Client;

/// Settle time between commanding the relay and reading state back. The relay
/// switches synchronously, but a brief pause lets the metering subsystem catch
/// up so the read-back reflects the command.
const SETTLE: Duration = Duration::from_millis(150);

#[derive(Parser)]
#[command(
    name = "shellyplug",
    version,
    about = "Power control for Shelly Gen2+ smart plugs/relays over the local HTTP RPC API",
    long_about = "Power control for Shelly Gen2+ smart plugs and relays (Plus / Pro / Gen3 / \
Gen4) over the device's local HTTP RPC API — no cloud, no Home Assistant, no Matter.

MENTAL MODEL
  - A device is addressed by its address on your network: pass --device with an
    IP or hostname (10.0.0.5, shelly.local), optionally with a scheme or port.
    A DHCP reservation or the device's .local mDNS name keeps the hook stable
    across reboots.
  - A switch is addressed by its component id (the positional [ID], default 0).
    Single-outlet plugs only have id 0; multi-channel devices (e.g. a Pro 4PM)
    use 0..N.
  - on/off/cycle confirm by reading the relay state back, so a hook that
    silently failed surfaces as a non-zero exit.

TYPICAL USE
  shellyplug -d 10.0.0.5 status        device info + switch state and power
  shellyplug -d 10.0.0.5 state         prints exactly `on` or `off`
  shellyplug -d 10.0.0.5 on|off [id]
  shellyplug -d 10.0.0.5 cycle [id] [--delay-ms 3000]

Only devices with authentication disabled are supported for now; an
auth-enabled device answers HTTP 401 with a clear message."
)]
struct Cli {
    /// Device address: IP or hostname, optionally `http://host` or `host:port`
    /// (e.g. 10.0.0.5, shelly.local, http://10.0.0.5:8080).
    #[arg(short = 'd', long = "device", value_name = "HOST", global = true)]
    device: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print exactly `on` or `off` for a switch (hook: state_cmd).
    State {
        /// Switch component id (default 0).
        #[arg(default_value_t = 0)]
        id: u32,
    },
    /// Switch on and confirm by read-back (hook: on_cmd).
    On {
        /// Switch component id (default 0).
        #[arg(default_value_t = 0)]
        id: u32,
    },
    /// Switch off and confirm by read-back (hook: off_cmd).
    Off {
        /// Switch component id (default 0).
        #[arg(default_value_t = 0)]
        id: u32,
    },
    /// Power-cycle: off → delay → on → confirm (hook: cycle_cmd).
    Cycle {
        /// Switch component id (default 0).
        #[arg(default_value_t = 0)]
        id: u32,
        /// Milliseconds to hold the switch off before restoring power.
        #[arg(long, default_value_t = 3000)]
        delay_ms: u64,
    },
    /// Human-readable device info plus a switch's state and power metering.
    Status {
        /// Switch component id (default 0).
        #[arg(default_value_t = 0)]
        id: u32,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let device = cli
        .device
        .as_deref()
        .ok_or_else(|| anyhow!("required option '--device <HOST>' (-d) was not provided"))?;
    let client = Client::new(device);
    match cli.cmd {
        Cmd::State { id } => cmd_state(&client, id),
        Cmd::On { id } => cmd_switch(&client, id, true),
        Cmd::Off { id } => cmd_switch(&client, id, false),
        Cmd::Cycle { id, delay_ms } => cmd_cycle(&client, id, delay_ms),
        Cmd::Status { id } => cmd_status(&client, id),
    }
}

fn onoff(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

fn cmd_state(client: &Client, id: u32) -> Result<()> {
    let st = client.switch_status(id)?;
    println!("{}", onoff(st.output));
    Ok(())
}

fn cmd_switch(client: &Client, id: u32, on: bool) -> Result<()> {
    client.switch_set(id, on)?;
    thread::sleep(SETTLE);
    let now = client.switch_status(id)?.output;
    if now != on {
        bail!(
            "switch {id}: commanded {} but device reports {}",
            onoff(on),
            onoff(now)
        );
    }
    println!("switch {id}: {}", onoff(now));
    Ok(())
}

fn cmd_cycle(client: &Client, id: u32, delay_ms: u64) -> Result<()> {
    client.switch_set(id, false)?;
    thread::sleep(Duration::from_millis(delay_ms));
    client.switch_set(id, true)?;
    thread::sleep(SETTLE);
    if !client.switch_status(id)?.output {
        bail!("switch {id}: cycled but device reports off after restore");
    }
    println!("switch {id}: cycled ({delay_ms} ms off)");
    Ok(())
}

fn cmd_status(client: &Client, id: u32) -> Result<()> {
    let info = client.device_info()?;
    let name = info.name.as_deref().unwrap_or("(unnamed)");
    let app = info
        .app
        .as_deref()
        .map(|a| format!(" [{a}]"))
        .unwrap_or_default();
    let ver = info.ver.as_deref().unwrap_or("?");
    println!("device   {} — {}{}", info.id, info.model, app);
    println!("name     {name}");
    println!("gen/fw   gen {}, {ver}", info.generation);
    println!(
        "auth     {}",
        if info.auth_en { "enabled" } else { "disabled" }
    );

    let st = client.switch_status(id)?;
    println!("switch {id}  {}", onoff(st.output));
    if let Some(w) = st.apower {
        println!("  power    {w:.1} W");
    }
    if let Some(v) = st.voltage {
        println!("  voltage  {v:.1} V");
    }
    if let Some(a) = st.current {
        println!("  current  {a:.3} A");
    }
    if let Some(t) = st.temperature.as_ref().and_then(|t| t.t_c) {
        println!("  temp     {t:.1} °C");
    }
    if let Some(e) = st.aenergy.as_ref().and_then(|e| e.total) {
        println!("  energy   {e:.1} Wh total");
    }
    Ok(())
}
