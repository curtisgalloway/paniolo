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
//!   cycle <id>    off → confirm → delay → on → confirm

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
    /// Power-cycle: off → confirm → delay → on → confirm (hook: cycle_cmd).
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

/// The two operations the hook sequencing needs from a device: command the
/// relay, and read its state back. Abstracted so the on/off/cycle logic can
/// be exercised against a fake, which is the only way to prove the
/// confirmation steps actually run without a plug on the bench.
trait Switch {
    fn set(&self, id: u32, on: bool) -> Result<()>;
    fn is_on(&self, id: u32) -> Result<bool>;
}

impl Switch for Client {
    fn set(&self, id: u32, on: bool) -> Result<()> {
        self.switch_set(id, on)
    }

    fn is_on(&self, id: u32) -> Result<bool> {
        Ok(self.switch_status(id)?.output)
    }
}

fn cmd_state(client: &Client, id: u32) -> Result<()> {
    let st = client.switch_status(id)?;
    println!("{}", onoff(st.output));
    Ok(())
}

/// Command the relay and confirm by read-back, failing on a mismatch.
fn set_confirmed(sw: &impl Switch, id: u32, on: bool) -> Result<()> {
    sw.set(id, on)?;
    thread::sleep(SETTLE);
    let now = sw.is_on(id)?;
    if now != on {
        bail!(
            "switch {id}: commanded {} but device reports {}",
            onoff(on),
            onoff(now)
        );
    }
    Ok(())
}

fn cmd_switch(sw: &impl Switch, id: u32, on: bool) -> Result<()> {
    set_confirmed(sw, id, on)?;
    println!("switch {id}: {}", onoff(on));
    Ok(())
}

/// off → confirm → delay → on → confirm. The off phase is confirmed before
/// the hold starts: a relay that ignored the off command would otherwise
/// produce a "cycle" that never removed power, with the final read-back
/// happily reporting on.
fn cmd_cycle(sw: &impl Switch, id: u32, delay_ms: u64) -> Result<()> {
    set_confirmed(sw, id, false).map_err(|e| anyhow!("{e} (cycle aborted before the off-hold)"))?;
    thread::sleep(Duration::from_millis(delay_ms));
    set_confirmed(sw, id, true).map_err(|e| anyhow!("{e} (after the off-hold)"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// A relay that records every call and can be told to ignore commands,
    /// standing in for a plug whose `Switch.Set` returned OK without acting.
    struct Fake {
        on: Cell<bool>,
        ignore_off: bool,
        ignore_on: bool,
        log: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(on: bool) -> Self {
            Fake {
                on: Cell::new(on),
                ignore_off: false,
                ignore_on: false,
                log: RefCell::new(Vec::new()),
            }
        }
    }

    impl Switch for Fake {
        fn set(&self, id: u32, on: bool) -> Result<()> {
            self.log
                .borrow_mut()
                .push(format!("set {id} {}", onoff(on)));
            let ignored = if on { self.ignore_on } else { self.ignore_off };
            if !ignored {
                self.on.set(on);
            }
            Ok(())
        }

        fn is_on(&self, id: u32) -> Result<bool> {
            self.log.borrow_mut().push(format!("status {id}"));
            Ok(self.on.get())
        }
    }

    #[test]
    fn cycle_confirms_off_then_on() {
        let plug = Fake::new(true);
        cmd_cycle(&plug, 0, 0).unwrap();
        assert!(plug.on.get());
        assert_eq!(
            *plug.log.borrow(),
            ["set 0 off", "status 0", "set 0 on", "status 0"]
        );
    }

    #[test]
    fn cycle_aborts_before_the_hold_when_off_did_not_take() {
        let plug = Fake {
            ignore_off: true,
            ..Fake::new(true)
        };
        let err = cmd_cycle(&plug, 3, 0).expect_err("a relay stuck on must fail the cycle");
        assert!(err.to_string().contains("commanded off"), "{err}");
        // It must stop right there: no power-on was ever sent.
        assert_eq!(*plug.log.borrow(), ["set 3 off", "status 3"]);
    }

    #[test]
    fn cycle_fails_when_on_did_not_take() {
        let plug = Fake {
            ignore_on: true,
            ..Fake::new(true)
        };
        let err = cmd_cycle(&plug, 0, 0).expect_err("a relay stuck off must fail the cycle");
        assert!(err.to_string().contains("commanded on"), "{err}");
        assert!(!plug.on.get());
    }

    #[test]
    fn on_and_off_confirm_by_read_back() {
        let plug = Fake::new(false);
        cmd_switch(&plug, 1, true).unwrap();
        assert!(plug.on.get());
        cmd_switch(&plug, 1, false).unwrap();
        assert!(!plug.on.get());

        let stuck = Fake {
            ignore_on: true,
            ..Fake::new(false)
        };
        let err = cmd_switch(&stuck, 1, true).unwrap_err();
        assert!(err.to_string().contains("device reports off"), "{err}");
    }
}
