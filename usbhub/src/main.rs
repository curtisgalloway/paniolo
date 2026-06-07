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

//! usbhub — per-port USB hub power control via hub-class requests.
//!
//! Switches VBUS on individual ports of off-the-shelf USB hubs (the same
//! mechanism as uhubctl), addressed by *physical* silkscreen port numbers
//! through a per-model profile that a human builds and verifies with the
//! `learn` workflow. Switching is refused on any port that no human has
//! verified — see profile.rs for the assertion contract.
//!
//! Hook-facing subcommands follow the paniolo helper conventions
//! (docs/adding-power-helpers.md):
//!   state <port>   prints exactly `on` or `off`
//!   on/off <port>  switch + read-back confirm
//!   cycle <port>   off → delay → on → confirm

mod act;
mod hub;
mod learn;
mod profile;
mod topo;
mod tty;

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use act::DeviceTable;
use learn::{Session, Stage, VerifyResult, WalkOutcome};
use profile::{parse_at, AtSpec, Instance, PortEntry, Profile};
use topo::{DevRecord, Side};

/// How long to wait after clearing a port's power before re-enumerating to see
/// whether the probe device dropped off the bus. Devices (phones especially)
/// can take most of a second to disconnect.
pub(crate) const VERIFY_SETTLE: Duration = Duration::from_millis(800);

#[derive(Parser)]
#[command(name = "usbhub", version, about = "Per-port USB hub power control")]
struct Cli {
    /// Hub model name (a profile in the profiles dir). Required for
    /// status/state/on/off/cycle.
    #[arg(short = 'm', long = "model", value_name = "MODEL", global = true)]
    model: Option<String>,

    /// Instance pin when several hubs of the same model are attached:
    /// `usb3=BUS:CHAIN[,usb2=BUS:CHAIN]` as printed by the ambiguity error
    /// or `usbhub probe`.
    #[arg(long, value_name = "ANCHORS", global = true)]
    at: Option<String>,

    /// Override the profiles directory (default: $PANIOLO_STATE_DIR/profiles).
    #[arg(long, value_name = "DIR", global = true)]
    profile_dir: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the live USB hub topology (read-only). Shows what each hub
    /// *claims* about power switching — a hint, not a verification.
    Probe,
    /// List known model profiles.
    Models,
    /// Show the model's ports: mappings, assertions, and live power bits.
    Status,
    /// Print exactly `on` or `off` for a physical port (hook: state_cmd).
    State { physical: u16 },
    /// Power a physical port on (hook: on_cmd). Refused without a verified
    /// `controllable = true` assertion in the profile.
    On {
        physical: u16,
        #[arg(long, value_enum, default_value_t = SideArg::Both)]
        side: SideArg,
    },
    /// Power a physical port off (hook: off_cmd). Same refusal rule.
    Off {
        physical: u16,
        #[arg(long, value_enum, default_value_t = SideArg::Both)]
        side: SideArg,
    },
    /// Power-cycle a physical port: off → delay → on → confirm (hook:
    /// cycle_cmd). Same refusal rule.
    Cycle {
        physical: u16,
        /// Milliseconds to hold the port off before restoring.
        #[arg(long, default_value_t = 3000)]
        delay_ms: u64,
        #[arg(long, value_enum, default_value_t = SideArg::Both)]
        side: SideArg,
    },
    /// Build a model profile from physical actions at the bench: discrete,
    /// resumable steps an agent can drive, or `learn run` for a TTY loop.
    Learn {
        #[command(subcommand)]
        cmd: LearnCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum LearnCmd {
    /// Open a session to build (or rebuild) a profile: snapshot the bus as-is.
    Edit {
        /// Discard an existing unsaved session.
        #[arg(long)]
        force: bool,
    },
    /// Record the snapshot after the hub has been unplugged.
    Unplugged,
    /// Record the snapshot after the hub was plugged back in; derives the
    /// chip cascade.
    Plugged,
    /// Map a physical port: run this, then plug the probe device into that
    /// port; blocks until the probe is seen (or times out).
    Port {
        physical: u16,
        #[arg(long, default_value_t = 120)]
        timeout_secs: u64,
    },
    /// Without --result: power the port off so a human can look at the
    /// probe. With --result: record the human's verdict and restore power.
    Verify {
        physical: u16,
        /// What the human observed: did the probe actually lose power?
        #[arg(long, value_enum)]
        result: Option<ResultArg>,
        /// Why the port is not controllable (recorded with --result alive).
        #[arg(long)]
        reason: Option<String>,
    },
    /// Show session progress and the suggested next step.
    Status,
    /// Abandon the session (restores power if a verify was pending).
    Abort,
    /// Save the session as a profile (writes it to the profiles dir).
    Save {
        #[arg(long)]
        model: String,
        #[arg(long)]
        description: Option<String>,
    },
    /// Interactive TTY harness over the same steps.
    Run,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum SideArg {
    Both,
    Usb3,
    Usb2,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum ResultArg {
    Dead,
    Alive,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let profile_dir = profile::profiles_dir(cli.profile_dir.as_deref());
    match cli.cmd {
        Cmd::Probe => cmd_probe(),
        Cmd::Models => {
            let models = profile::list_models(&profile_dir);
            if models.is_empty() {
                println!("no profiles in {}", profile_dir.display());
            } else {
                for m in models {
                    println!("{m}");
                }
            }
            Ok(())
        }
        Cmd::Status => cmd_status(&load_ctx(&cli, &profile_dir)?),
        Cmd::State { physical } => cmd_state(&mut load_ctx(&cli, &profile_dir)?, physical),
        Cmd::On { physical, side } => {
            cmd_switch(&mut load_ctx(&cli, &profile_dir)?, physical, side, true)
        }
        Cmd::Off { physical, side } => {
            cmd_switch(&mut load_ctx(&cli, &profile_dir)?, physical, side, false)
        }
        Cmd::Cycle {
            physical,
            delay_ms,
            side,
        } => cmd_cycle(&mut load_ctx(&cli, &profile_dir)?, physical, delay_ms, side),
        Cmd::Learn { cmd } => cmd_learn(cmd, &profile_dir),
    }
}

// ---------------------------------------------------------------------------
// Resolution context for the profile-driven commands
// ---------------------------------------------------------------------------

struct Ctx {
    profile: Profile,
    instance: Instance,
    table: DeviceTable,
}

fn load_ctx(cli: &Cli, profile_dir: &std::path::Path) -> Result<Ctx> {
    let model = cli
        .model
        .as_deref()
        .ok_or_else(|| anyhow!("required argument '--model <MODEL>' (-m) was not provided"))?;
    let profile = profile::load_profile(profile_dir, model)?;
    let at: AtSpec = match &cli.at {
        Some(s) => parse_at(s)?,
        None => AtSpec::new(),
    };
    let (snapshot, table) = DeviceTable::snapshot()?;
    let instance = profile::resolve(&profile, &snapshot, &at);
    Ok(Ctx {
        profile,
        instance,
        table,
    })
}

/// The sides this operation acts on: the entry's mapped sides, narrowed by
/// --side. The controllable assertion was verified against the mapped set,
/// so acting on all mapped sides is the verified behavior.
fn sides_for(entry: &PortEntry, filter: SideArg) -> Result<Vec<Side>> {
    let sides: Vec<Side> = [Side::Usb3, Side::Usb2]
        .into_iter()
        .filter(|s| entry.side(*s).is_some())
        .filter(|s| match filter {
            SideArg::Both => true,
            SideArg::Usb3 => *s == Side::Usb3,
            SideArg::Usb2 => *s == Side::Usb2,
        })
        .collect();
    if sides.is_empty() {
        bail!(
            "physical port {} has no mapping on the requested side(s)",
            entry.physical
        );
    }
    Ok(sides)
}

/// Locate the live chip record serving `entry` on `side`.
fn chip_for<'a>(ctx: &'a Ctx, entry: &PortEntry, side: Side) -> Result<(&'a DevRecord, u8)> {
    let sp = entry
        .side(side)
        .ok_or_else(|| anyhow!("port {} unmapped on {side}", entry.physical))?;
    let si = ctx.instance.side(side)?;
    let chip = si
        .chips
        .get(&sp.path)
        .ok_or_else(|| anyhow!("resolved {side} instance has no chip at path {:?}", sp.path))?;
    Ok((chip, sp.port))
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_probe() -> Result<()> {
    let (mut snapshot, mut table) = DeviceTable::snapshot()?;
    snapshot.sort_by(|a, b| {
        a.bus_id
            .cmp(&b.bus_id)
            .then(a.port_chain.cmp(&b.port_chain))
    });
    let mut bus = String::new();
    for rec in &snapshot {
        if rec.bus_id != bus {
            bus = rec.bus_id.clone();
            println!("bus {bus}");
        }
        let indent = "  ".repeat(rec.port_chain.len().max(1));
        if rec.is_hub() {
            let desc = match table
                .device(rec)
                .and_then(|d| rec.side().context("speed unknown").map(|s| (d, s)))
                .and_then(|(d, s)| hub::hub_descriptor(d, s))
            {
                Ok(info) => format!(
                    "{} ports, power switching: {}",
                    info.nbr_ports, info.power_switching
                ),
                Err(e) => format!("(descriptor unavailable: {e:#})"),
            };
            println!("{indent}{} — {desc}", rec.describe());
        } else {
            println!("{indent}{}", rec.describe());
        }
    }
    println!(
        "\nNote: \"per-port (claimed)\" is the chip's claim — chips routinely \
         claim switching they cannot do. Only `usbhub learn` verification \
         makes a port switchable here."
    );
    Ok(())
}

fn cmd_status(ctx: &Ctx) -> Result<()> {
    println!("model {} — resolved instance:", ctx.profile.model.name);
    for side in [Side::Usb3, Side::Usb2] {
        match ctx.instance.side(side) {
            Ok(si) => println!("  [{side}] root {}", si.root().describe()),
            Err(e) => println!("  [{side}] unresolved: {e:#}"),
        }
    }
    println!(
        "{:<9} {:<12} {:<12} {:<14} live power",
        "physical", "usb3", "usb2", "assertion"
    );
    println!("{}", "-".repeat(64));
    // Re-snapshot once for live bits via a fresh table (ctx.table is not
    // mutable here; status is read-only and tolerant of failures).
    let (_, mut table) = DeviceTable::snapshot()?;
    for entry in &ctx.profile.ports {
        let map_str = |sp: Option<&profile::SidePort>| {
            sp.map(|s| {
                format!(
                    "{}@{}",
                    if s.path.is_empty() { "root" } else { &s.path },
                    s.port
                )
            })
            .unwrap_or_else(|| "-".to_string())
        };
        let assertion = match (entry.controllable, &entry.reason) {
            (Some(true), _) => "controllable".to_string(),
            (Some(false), Some(r)) => format!("NO ({r})"),
            (Some(false), None) => "NO".to_string(),
            (None, _) => "unverified".to_string(),
        };
        let mut live = Vec::new();
        for side in [Side::Usb3, Side::Usb2] {
            if entry.side(side).is_none() {
                continue;
            }
            let state = chip_for(ctx, entry, side)
                .and_then(|(chip, port)| {
                    table
                        .device(chip)
                        .and_then(|d| hub::port_power_is_on(d, side, port))
                })
                .map(|on| if on { "on" } else { "off" })
                .unwrap_or("?");
            live.push(format!("{side}:{state}"));
        }
        println!(
            "{:<9} {:<12} {:<12} {:<14} {}",
            entry.physical,
            map_str(entry.usb3.as_ref()),
            map_str(entry.usb2.as_ref()),
            assertion,
            live.join(" ")
        );
    }
    Ok(())
}

fn cmd_state(ctx: &mut Ctx, physical: u16) -> Result<()> {
    let entry = ctx
        .profile
        .port_entry(physical)
        .ok_or_else(|| anyhow!("physical port {physical} has no entry in the profile"))?
        .clone();
    let sides = sides_for(&entry, SideArg::Both)?;
    let mut states = Vec::new();
    for side in sides {
        let (chip, port) = chip_for(ctx, &entry, side)?;
        let chip = chip.clone();
        let dev = ctx.table.device(&chip)?;
        states.push((side, hub::port_power_is_on(dev, side, port)?));
    }
    let ons = states.iter().filter(|(_, on)| *on).count();
    if ons == states.len() {
        println!("on");
    } else if ons == 0 {
        println!("off");
    } else {
        bail!(
            "port {physical} is in a mixed state: {} — fix with `usbhub on/off {physical}`",
            states
                .iter()
                .map(|(s, on)| format!("{s}={}", if *on { "on" } else { "off" }))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    Ok(())
}

fn cmd_switch(ctx: &mut Ctx, physical: u16, side: SideArg, on: bool) -> Result<()> {
    let entry = ctx.profile.check_switchable(physical)?.clone();
    let sides = sides_for(&entry, side)?;
    for s in &sides {
        let (chip, port) = chip_for(ctx, &entry, *s)?;
        let chip = chip.clone();
        let dev = ctx.table.device(&chip)?;
        hub::set_port_power(dev, port, on)?;
    }
    // Read back: the status bit must reflect the command.
    thread::sleep(Duration::from_millis(150));
    for s in &sides {
        let (chip, port) = chip_for(ctx, &entry, *s)?;
        let chip = chip.clone();
        let dev = ctx.table.device(&chip)?;
        let now = hub::port_power_is_on(dev, *s, port)?;
        if now != on {
            bail!(
                "port {physical} [{s}]: commanded {} but PORT_POWER reads {}",
                if on { "on" } else { "off" },
                if now { "on" } else { "off" }
            );
        }
    }
    println!(
        "port {physical}: {} ({})",
        if on { "on" } else { "off" },
        sides
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join("+")
    );
    Ok(())
}

fn cmd_cycle(ctx: &mut Ctx, physical: u16, delay_ms: u64, side: SideArg) -> Result<()> {
    let entry = ctx.profile.check_switchable(physical)?.clone();
    let sides = sides_for(&entry, side)?;
    for s in &sides {
        let (chip, port) = chip_for(ctx, &entry, *s)?;
        let chip = chip.clone();
        let dev = ctx.table.device(&chip)?;
        hub::set_port_power(dev, port, false)?;
    }
    thread::sleep(Duration::from_millis(delay_ms));
    for s in &sides {
        let (chip, port) = chip_for(ctx, &entry, *s)?;
        let chip = chip.clone();
        let dev = ctx.table.device(&chip)?;
        hub::set_port_power(dev, port, true)?;
    }
    thread::sleep(Duration::from_millis(150));
    for s in &sides {
        let (chip, port) = chip_for(ctx, &entry, *s)?;
        let chip = chip.clone();
        let dev = ctx.table.device(&chip)?;
        if !hub::port_power_is_on(dev, *s, port)? {
            bail!("port {physical} [{s}]: cycled but PORT_POWER reads off after restore");
        }
    }
    println!("port {physical}: cycled ({delay_ms} ms off)");
    Ok(())
}

// ---------------------------------------------------------------------------
// Learn dispatch (the IO edges around learn.rs's state machine)
// ---------------------------------------------------------------------------

fn cmd_learn(cmd: LearnCmd, profile_dir: &std::path::Path) -> Result<()> {
    let sd = profile::state_dir();
    match cmd {
        LearnCmd::Edit { force } => {
            if let Ok(existing) = learn::load_session(&sd) {
                if existing.stage != Stage::Finished && !force {
                    bail!(
                        "an unsaved learn session exists (stage {:?}) — resume it \
                         (`usbhub learn status`), or discard with `usbhub learn edit --force`",
                        existing.stage
                    );
                }
            }
            let session = Session::start(topo::snapshot()?);
            learn::save_session(&sd, &session)?;
            println!(
                "Editing a new profile: {} device(s) on the bus.\n\
                 Next: unplug the hub from the host, then run `usbhub learn unplugged`",
                session.snap_start.len()
            );
            Ok(())
        }
        LearnCmd::Unplugged => {
            let mut session = learn::load_session(&sd)?;
            let msg = session.unplugged(topo::snapshot()?)?;
            learn::save_session(&sd, &session)?;
            println!("{msg}");
            Ok(())
        }
        LearnCmd::Plugged => {
            let mut session = learn::load_session(&sd)?;
            let msg = session.plugged(topo::snapshot()?)?;
            learn::save_session(&sd, &session)?;
            println!("{msg}");
            Ok(())
        }
        LearnCmd::Port {
            physical,
            timeout_secs,
        } => {
            let mut session = learn::load_session(&sd)?;
            let msg = walk_port(&mut session, physical, timeout_secs)?;
            learn::save_session(&sd, &session)?;
            println!("{msg}");
            Ok(())
        }
        LearnCmd::Verify {
            physical,
            result,
            reason,
        } => {
            let mut session = learn::load_session(&sd)?;
            let (_, mut table) = DeviceTable::snapshot()?;
            let msg = match result {
                None => session.begin_verify(physical, &mut table, VERIFY_SETTLE)?,
                Some(r) => {
                    let vr = match r {
                        ResultArg::Dead => VerifyResult::Dead,
                        ResultArg::Alive => VerifyResult::Alive,
                    };
                    session.record_verify(physical, vr, reason, &mut table)?
                }
            };
            learn::save_session(&sd, &session)?;
            println!("{msg}");
            Ok(())
        }
        LearnCmd::Status => {
            let session = learn::load_session(&sd)?;
            println!("{}", session.status_text());
            Ok(())
        }
        LearnCmd::Abort => {
            if let Ok(mut session) = learn::load_session(&sd) {
                let mut table = DeviceTable::snapshot().map(|(_, t)| t);
                if let Ok(t) = table.as_mut() {
                    if let Some(msg) = session.abort_restore(t) {
                        println!("{msg}");
                    }
                }
            }
            learn::delete_session(&sd)?;
            println!("Session discarded.");
            Ok(())
        }
        LearnCmd::Save { model, description } => {
            let mut session = learn::load_session(&sd)?;
            let msg = finish_session(&mut session, &model, description, profile_dir)?;
            learn::save_session(&sd, &session)?;
            println!("{msg}");
            Ok(())
        }
        LearnCmd::Run => tty::run(profile_dir),
    }
}

/// The port-walk polling loop: baseline now, watch for the probe to arrive
/// under the captured chips. Shared by `learn port` and the TTY harness.
pub(crate) fn walk_port(session: &mut Session, physical: u16, timeout_secs: u64) -> Result<String> {
    let baseline = topo::snapshot()?;
    println!(
        "Watching for the probe — plug it into physical port {physical} now \
         (up to {timeout_secs}s)..."
    );
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        thread::sleep(Duration::from_millis(300));
        let now = topo::snapshot()?;
        match session.walk_check(&baseline, &now) {
            WalkOutcome::Nothing => {
                if Instant::now() >= deadline {
                    bail!(
                        "timed out waiting for the probe on port {physical}. If it \
                         was already plugged in, unplug it, wait 2 s, and re-run \
                         `usbhub learn port {physical}`."
                    );
                }
            }
            WalkOutcome::Found(_) => {
                // Settle, then re-diff against the same baseline so a
                // compound probe's other-side companion (which enumerates a
                // beat later) is captured too.
                thread::sleep(Duration::from_millis(1500));
                let now = topo::snapshot()?;
                let WalkOutcome::Found(findings) = session.walk_check(&baseline, &now) else {
                    bail!(
                        "probe disappeared while settling — re-run `usbhub learn port {physical}`"
                    );
                };
                return session.record_port(physical, &findings);
            }
            WalkOutcome::Elsewhere(devs) => {
                bail!(
                    "something arrived, but not under the captured hub:\n{}\n\
                     Wrong port (or wrong hub)? Unplug it and re-run \
                     `usbhub learn port {physical}`.",
                    devs.iter()
                        .map(|d| format!("  {}", d.describe()))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
    }
}

/// Compile and write the profile; shared by `learn save` and the wizard.
pub(crate) fn finish_session(
    session: &mut Session,
    model: &str,
    description: Option<String>,
    profile_dir: &std::path::Path,
) -> Result<String> {
    let anchors = session.anchors();
    let prof = session.finish(model, description)?;
    let path = profile::save_profile(profile_dir, &prof)?;
    let toml_text = profile::to_toml(&prof)?;
    Ok(format!(
        "Profile written to {}:\n\n{}\n\
         This instance is currently at: --at {}\n\
         (only needed if several {} hubs share one host)\n\n\
         Lab-file wiring:\n  paniolo power set -t <target> \\\n    \
         --cycle-cmd \"usbhub --model {} cycle <port>\" \\\n    \
         --on-cmd    \"usbhub --model {} on <port>\" \\\n    \
         --off-cmd   \"usbhub --model {} off <port>\" \\\n    \
         --state-cmd \"usbhub --model {} state <port>\"",
        path.display(),
        toml_text,
        anchors,
        model,
        model,
        model,
        model,
        model
    ))
}
