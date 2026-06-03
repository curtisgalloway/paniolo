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

//! paniolo — agent-controlled target machine wrangler.
//!
//! Rust rewrite of the control plane (see docs/config-redesign.md). The lab
//! file is the single, CLI-managed source of truth for hosts, targets, and the
//! channels that connect them. This module is the CLI surface; the model and
//! editor live in [`model`] and [`labfile`].

mod daemons;
mod discover;
mod dispatch;
mod doctor;
mod labfile;
mod model;
mod netboot;
mod netif;
mod power;
mod serial;
mod setup;
mod ssh;
mod state;
mod video;

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use clap::{Parser, Subcommand};

use labfile::LabFile;
use model::{Lab, ResolvedChannel, ResolvedTarget};

#[derive(Parser)]
#[command(
    name = "paniolo",
    version,
    about = "Agent-controlled target machine wrangler."
)]
struct Cli {
    /// Path to the lab config file (default: $PANIOLO_LAB or ~/.config/paniolo/lab.toml).
    #[arg(long, global = true)]
    lab: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create an empty lab file.
    Init {
        /// Where to create it (default: --lab path or ~/.config/paniolo/lab.toml).
        #[arg(long)]
        path: Option<String>,
    },
    /// Inspect and edit the lab configuration file.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Manage control hosts declared in the lab.
    Host {
        #[command(subcommand)]
        cmd: HostCmd,
    },
    /// Manage targets.
    Target {
        #[command(subcommand)]
        cmd: TargetCmd,
    },
    /// Manage a target's serial channels.
    Serial {
        #[command(subcommand)]
        cmd: SerialCmd,
    },
    /// Configure a target's netboot channel and run DHCP+TFTP netboot.
    Netboot {
        #[command(subcommand)]
        cmd: NetbootCmd,
    },
    /// Switch the USB-Ethernet link between netboot and ffx modes.
    Netif {
        #[command(subcommand)]
        cmd: NetifCmd,
    },
    /// Configure a target's power channel.
    Power {
        #[command(subcommand)]
        cmd: PowerCmd,
    },
    /// Configure and drive a target's HDMI-capture (video) channel.
    Video {
        #[command(subcommand)]
        cmd: VideoCmd,
    },
    /// Open the combined video+serial dashboard, starting daemons if needed.
    Console {
        target: Option<String>,
        /// Serial interface name to preselect in the dashboard terminal.
        #[arg(long, short)]
        interface: Option<String>,
    },
    /// Run the target's configured power-cycle command.
    PowerCycle { target: Option<String> },
    /// Show whether the target is powered on (via the serial sense line).
    PowerState { target: Option<String> },
    /// Probe configured channels against reality over SSH (config vs hardware).
    Doctor {
        /// Target to check (default: all).
        target: Option<String>,
        /// Only check channels on this host.
        #[arg(long)]
        host: Option<String>,
    },
    /// List this host's hardware for lab authoring (USB-Ethernet, serial, capture).
    Discover {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Propose a `[targets.<name>]` lab block from hardware discovered on a host.
    Configure {
        /// Target name to propose a block for.
        target: String,
        /// Lab host the target is wired to ('local' or a declared host).
        #[arg(long, short = 'H')]
        host: String,
    },
    /// Build and install paniolo's binaries (daemons + CLI) from a source clone.
    Setup {
        /// Provision this lab host over SSH instead of locally (requires the
        /// paniolo CLI + source already on the host).
        #[arg(long)]
        host: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Show the whole lab: hosts and targets, each channel with its host.
    Show,
    /// Print the active lab file path.
    Path,
    /// Open the raw lab file in $EDITOR.
    Edit,
}

#[derive(Subcommand)]
enum HostCmd {
    /// List control hosts.
    List,
    /// Show a host and the channels that live on it.
    Show { name: String },
    /// Declare a control host.
    Add {
        name: String,
        /// ssh destination: user@host, an ssh_config alias, or 'local'.
        #[arg(long)]
        ssh: String,
        #[arg(long)]
        identity: Option<String>,
        #[arg(long)]
        control_path: Option<String>,
        #[arg(long)]
        paniolo_cmd: Option<String>,
    },
    /// Update an existing host (only the options you pass change).
    Set {
        name: String,
        #[arg(long)]
        ssh: Option<String>,
        #[arg(long)]
        identity: Option<String>,
        #[arg(long)]
        control_path: Option<String>,
        #[arg(long)]
        paniolo_cmd: Option<String>,
    },
    /// Remove a host (refused if any target still binds to it).
    Rm { name: String },
}

#[derive(Subcommand)]
enum TargetCmd {
    /// List targets with their host(s) and channels.
    List,
    /// Show target configuration(s), each channel with its resolved host.
    Show { name: Option<String> },
    /// Create a target.
    Add {
        name: String,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Update a target's default host or note.
    Set {
        name: String,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Remove a target and all its channels.
    Rm { name: String },
}

#[derive(Subcommand)]
enum SerialCmd {
    /// Add a named serial console to a target.
    Add {
        name: String,
        #[arg(long, short)]
        target: String,
        #[arg(long, short)]
        device: String,
        #[arg(long, default_value_t = 115200)]
        baud: i64,
        #[arg(long)]
        sense: Option<String>,
        #[arg(long)]
        host: Option<String>,
    },
    /// Update an existing serial interface (only the options you pass change).
    Set {
        name: String,
        #[arg(long, short)]
        target: String,
        #[arg(long, short)]
        device: Option<String>,
        #[arg(long)]
        baud: Option<i64>,
        #[arg(long)]
        sense: Option<String>,
        #[arg(long)]
        host: Option<String>,
    },
    /// Remove a named serial interface from a target.
    Rm {
        name: String,
        #[arg(long, short)]
        target: String,
    },
    /// Open an interactive serial console (via tio) on the channel's host.
    Connect {
        target: Option<String>,
        /// Interface name (default: the only one).
        #[arg(long, short)]
        interface: Option<String>,
    },
    /// Start the serialcap daemon (owning every configured interface).
    Watch {
        target: Option<String>,
        #[arg(long, default_value_t = serial::DEFAULT_PORT)]
        port: u16,
    },
    /// Stop the running serialcap daemon.
    Stop,
    /// Send a line of input to the console through the running daemon.
    Send {
        /// Text to send.
        text: String,
        #[arg(long, short)]
        target: Option<String>,
        #[arg(long, short)]
        interface: Option<String>,
        /// Per-byte pacing in ms for slow polled consoles (0 = full rate).
        #[arg(long, default_value_t = 0)]
        pace_ms: u32,
        /// Don't append a carriage return after the text.
        #[arg(long)]
        no_newline: bool,
    },
    /// Print captured serial output (reads serialcap's on-disk log).
    Log {
        #[arg(long, short)]
        target: Option<String>,
        #[arg(long, short)]
        interface: Option<String>,
        /// Show only the most recent N lines.
        #[arg(long, short = 'n')]
        tail: Option<u64>,
        /// Lowest line sequence number (inclusive).
        #[arg(long)]
        from: Option<u64>,
        /// Highest line sequence number (inclusive).
        #[arg(long)]
        to: Option<u64>,
        /// Only lines newer than this sequence number.
        #[arg(long)]
        since: Option<u64>,
        /// Keep raw bytes (ANSI/control) instead of cleaning.
        #[arg(long)]
        raw: bool,
        /// Emit JSON Lines instead of formatted text.
        #[arg(long)]
        json: bool,
        /// Exclude the current unterminated line.
        #[arg(long)]
        no_pending: bool,
    },
    /// List available serial devices on this machine.
    Devices,
    /// Show a target's serial interfaces and the daemon status.
    Show { target: Option<String> },
    /// Pulse the DTR line (J2 power-button header) on a serial interface.
    Dtr {
        target: Option<String>,
        /// Pulse duration in ms (≤500 = soft power-button event, ≥3000 = hard off).
        #[arg(long, default_value_t = 200)]
        ms: u64,
        /// Interface name (default: the power channel's serial_interface, or the
        /// only one).
        #[arg(long, short)]
        interface: Option<String>,
    },
    /// Send a soft-reset signal via a brief J2 power-button press.
    Reset {
        target: Option<String>,
        #[arg(long, default_value_t = 200)]
        ms: u64,
        #[arg(long, short)]
        interface: Option<String>,
    },
}

#[derive(Subcommand)]
enum NetbootCmd {
    /// Configure the target's netboot channel (one per target).
    Set {
        #[arg(long, short)]
        target: String,
        #[arg(long, short)]
        interface: Option<String>,
        #[arg(long)]
        host_ip: Option<String>,
        #[arg(long, short = 'r')]
        tftp_root: Option<String>,
        #[arg(long)]
        host: Option<String>,
    },
    /// Remove the target's netboot channel.
    Rm {
        #[arg(long, short)]
        target: String,
    },
    /// Start DHCP+TFTP netboot (the netbootd daemon) for a target.
    Start { target: Option<String> },
    /// Stop netboot and restore the interface.
    Stop { target: Option<String> },
    /// Show netboot daemon status.
    Status { target: Option<String> },
    /// Show the netboot log (combined DHCP+TFTP).
    Logs {
        target: Option<String>,
        /// Show only the most recent N lines.
        #[arg(long, short = 'n', default_value_t = 50)]
        tail: usize,
        /// Keep printing new lines as they arrive.
        #[arg(long, short)]
        follow: bool,
    },
    /// Print the target's TFTP root path.
    TftpRoot { target: Option<String> },
    /// List candidate USB-Ethernet interfaces on this machine.
    Devices,
    /// Bring the USB-Ethernet link up with the host IP assigned.
    LinkUp { target: Option<String> },
    /// Take the link down and release the host IP.
    LinkDown { target: Option<String> },
    /// Show the current state of the USB-Ethernet link.
    LinkStatus { target: Option<String> },
}

#[derive(Subcommand)]
enum NetifCmd {
    /// Switch the link mode: netboot | ffx | off (idempotent).
    Mode {
        /// netboot | ffx | off.
        mode: String,
        target: Option<String>,
    },
    /// Show which mode the link is in and its addresses.
    Status { target: Option<String> },
}

#[derive(Subcommand)]
enum PowerCmd {
    /// Configure the target's power channel (one per target).
    Set {
        #[arg(long, short)]
        target: String,
        #[arg(long)]
        cycle_cmd: Option<String>,
        #[arg(long)]
        serial_interface: Option<String>,
        #[arg(long)]
        host: Option<String>,
    },
    /// Remove the target's power channel.
    Rm {
        #[arg(long, short)]
        target: String,
    },
}

#[derive(Subcommand)]
enum VideoCmd {
    /// Configure the target's video channel (one per target).
    Set {
        #[arg(long, short)]
        target: String,
        /// Capture device: an hdmicap device name substring, index, or /dev path.
        #[arg(long, short)]
        device: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Remove the target's video channel.
    Rm {
        #[arg(long, short)]
        target: String,
    },
    /// Start the hdmicap warm-stream daemon for the target's capture device.
    Watch {
        target: Option<String>,
        #[arg(long, default_value_t = video::DEFAULT_PORT)]
        port: u16,
        /// Force-restart a running (possibly stalled) daemon.
        #[arg(long)]
        restart: bool,
    },
    /// Stop the running hdmicap daemon.
    Stop,
    /// Fetch one PNG screenshot from the running daemon.
    Shot {
        target: Option<String>,
        /// Wait until the signal is stable before capturing.
        #[arg(long)]
        stable: bool,
        /// Only return once the frame differs from this hex hash.
        #[arg(long)]
        changed_since: Option<String>,
        /// Timeout in ms.
        #[arg(long, default_value_t = 2000)]
        timeout: u64,
        /// Output path; "-" for stdout.
        #[arg(long, short, default_value = "-")]
        out: String,
    },
    /// Print the live-preview URL of the running daemon.
    Preview,
    /// List available capture devices.
    Devices,
    /// Show the target's video channel and daemon status.
    Show { target: Option<String> },
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let lab_flag = cli.lab.as_deref();
    match cli.command {
        Command::Init { path } => cmd_init(lab_flag, path.as_deref()),
        Command::Config { cmd } => match cmd {
            ConfigCmd::Show => config_show(lab_flag),
            ConfigCmd::Path => config_path(lab_flag),
            ConfigCmd::Edit => config_edit(lab_flag),
        },
        Command::Host { cmd } => host_cmd(lab_flag, cmd),
        Command::Target { cmd } => target_cmd(lab_flag, cmd),
        Command::Serial { cmd } => serial_cmd(lab_flag, cmd),
        Command::Netboot { cmd } => netboot_cmd(lab_flag, cmd),
        Command::Netif { cmd } => netif_cmd(lab_flag, cmd),
        Command::Power { cmd } => power_cmd(lab_flag, cmd),
        Command::Video { cmd } => video_cmd(lab_flag, cmd),
        Command::Console { target, interface } => {
            cmd_console(lab_flag, target.as_deref(), interface.as_deref())
        }
        Command::PowerCycle { target } => cmd_power_cycle(lab_flag, target.as_deref()),
        Command::PowerState { target } => cmd_power_state(lab_flag, target.as_deref()),
        Command::Doctor { target, host } => {
            cmd_doctor(lab_flag, target.as_deref(), host.as_deref())
        }
        Command::Discover { json } => cmd_discover(json),
        Command::Configure { target, host } => cmd_configure(lab_flag, &target, &host),
        Command::Setup { host } => cmd_setup(lab_flag, host.as_deref()),
    }
}

// ── discover / configure / setup ────────────────────────────────────────────

fn cmd_discover(json: bool) -> Result<()> {
    let inv = discover::local_inventory();
    if json {
        println!("{}", serde_json::to_string(&inv)?);
        return Ok(());
    }
    let eths: Vec<String> = inv["ethernet"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|e| {
                    let dev = e["device"].as_str().unwrap_or("");
                    let star = if e["active"].as_bool().unwrap_or(false) {
                        "*"
                    } else {
                        ""
                    };
                    format!("{dev}{star}")
                })
                .collect()
        })
        .unwrap_or_default();
    println!(
        "usb-ethernet\t{}",
        if eths.is_empty() {
            "(none)".to_string()
        } else {
            eths.join(" ")
        }
    );
    let serials: Vec<&str> = inv["serial"]
        .as_array()
        .map(|a| a.iter().filter_map(|s| s.as_str()).collect())
        .unwrap_or_default();
    println!(
        "serial\t{}",
        if serials.is_empty() {
            "(none)".to_string()
        } else {
            serials.join("\n\t")
        }
    );
    let captures: Vec<String> = inv["video"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|d| {
                    let id = d["id"].as_str().unwrap_or("");
                    let id_note = if id.is_empty() {
                        String::new()
                    } else {
                        format!("  id={id}")
                    };
                    format!(
                        "{}: {}{id_note}",
                        d["index"],
                        d["name"].as_str().unwrap_or("")
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    println!(
        "capture\t{}",
        if captures.is_empty() {
            "(none)".to_string()
        } else {
            captures.join("\n\t")
        }
    );
    println!("(* = carrier up)");
    Ok(())
}

/// Look up a control host for host-scoped commands ('local' = the dev machine).
fn resolve_host(lab_flag: Option<&str>, name: &str) -> Result<model::Host> {
    if name == model::LOCAL {
        return Ok(model::Host {
            ssh: model::LOCAL.to_string(),
            ..Default::default()
        });
    }
    let lab = load_for_read(lab_flag)?;
    lab.hosts.get(name).cloned().ok_or_else(|| {
        let have: Vec<&str> = lab.hosts.keys().map(String::as_str).collect();
        anyhow!(
            "host '{name}' not in lab (hosts: {})",
            if have.is_empty() {
                "(none)".to_string()
            } else {
                have.join(", ")
            }
        )
    })
}

fn cmd_configure(lab_flag: Option<&str>, target: &str, host: &str) -> Result<()> {
    let resolved = resolve_host(lab_flag, host)?;
    let inv = if resolved.is_local(host) {
        discover::local_inventory()
    } else {
        eprintln!("Discovering hardware on {host} ({})…", resolved.ssh);
        let out = ssh::run(
            &resolved,
            &[
                resolved.paniolo(),
                "discover".to_string(),
                "--json".to_string(),
            ],
            None,
            &[],
        )?;
        if out.status != 0 {
            bail!(
                "discover failed on {host}: {}",
                if out.stderr.trim().is_empty() {
                    out.stdout.trim()
                } else {
                    out.stderr.trim()
                }
            );
        }
        serde_json::from_str(out.stdout.trim())
            .map_err(|e| anyhow!("unparseable discover output from {host}: {e}"))?
    };

    let block = discover::propose_target_block(target, host, &inv);
    println!("# Proposed lab block — review, add to your lab file, and commit.");
    if let Ok(lab) = load_for_read(lab_flag) {
        if lab.targets.contains_key(target) {
            println!("# NOTE: target '{target}' already exists in the lab — reconcile by hand.");
        }
    }
    println!("{block}");
    Ok(())
}

fn cmd_setup(lab_flag: Option<&str>, host: Option<&str>) -> Result<()> {
    if let Some(name) = host {
        let resolved = resolve_host(lab_flag, name)?;
        if !resolved.is_local(name) {
            eprintln!("Running paniolo setup on {name} ({})…", resolved.ssh);
            let code =
                ssh::run_interactive(&resolved, &[resolved.paniolo(), "setup".to_string()], &[])?;
            std::process::exit(code);
        }
        eprintln!("'{name}' is the local machine; setting up here.");
    }
    let repo = setup::find_repo_root().ok_or_else(|| {
        anyhow!(
            "paniolo source checkout not found. `paniolo setup` rebuilds the daemons \
             and OCR helper from source, so it must run from inside a clone \
             (e.g. `make install`). cd into the paniolo repo and try again."
        )
    })?;
    setup::run(&repo)
}

/// Resolve a runtime command's target: the given name, or the sole target.
fn resolve_single_target(lab: &Lab, name: Option<&str>) -> Result<String> {
    if let Some(n) = name {
        return Ok(n.to_string());
    }
    let names = lab.target_names();
    match names.len() {
        1 => Ok(names[0].to_string()),
        0 => bail!("No targets configured."),
        _ => bail!(
            "Multiple targets ({}) — specify one with -t.",
            names.join(", ")
        ),
    }
}

fn cmd_doctor(lab_flag: Option<&str>, target: Option<&str>, host: Option<&str>) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    let problems = doctor::run(&lab, target, host);
    if problems > 0 {
        eprintln!("{problems} problem(s) found.");
        std::process::exit(1);
    }
    Ok(())
}

// ── lab open helpers ────────────────────────────────────────────────────────

fn load_for_read(lab_flag: Option<&str>) -> Result<Lab> {
    let path = model::resolve_lab_path(lab_flag).ok_or_else(|| {
        anyhow!(
            "No lab configured. Create one with `paniolo init`, or point at one \
             with --lab / PANIOLO_LAB."
        )
    })?;
    Ok(model::load(&path)?)
}

/// Open the lab for editing (creating it if absent), apply `mutate`, save.
/// Config writes are always local and pure — they never touch a target or SSH.
fn edit_lab<F>(lab_flag: Option<&str>, mutate: F) -> Result<()>
where
    F: FnOnce(&mut LabFile) -> Result<(), model::LabError>,
{
    let path = model::resolve_lab_path(lab_flag).unwrap_or_else(model::default_lab_path);
    let created = !path.exists();
    let mut lf = if created {
        LabFile::create(&path)
    } else {
        LabFile::load(&path)?
    };
    mutate(&mut lf)?;
    lf.save()?;
    if created {
        eprintln!("Created new lab: {}", path.display());
    }
    Ok(())
}

// ── init / config ───────────────────────────────────────────────────────────

fn cmd_init(lab_flag: Option<&str>, path_opt: Option<&str>) -> Result<()> {
    let path: PathBuf = path_opt
        .map(model::expand_tilde)
        .or_else(|| model::resolve_lab_path(lab_flag))
        .unwrap_or_else(model::default_lab_path);
    if path.exists() {
        bail!("Lab file already exists: {}", path.display());
    }
    LabFile::create(&path).save()?;
    println!("Created lab: {}", path.display());
    println!("  Add a host:   paniolo host add <name> --ssh user@host");
    println!("  Add a target: paniolo target add <name>");
    Ok(())
}

fn config_path(lab_flag: Option<&str>) -> Result<()> {
    match model::resolve_lab_path(lab_flag) {
        Some(p) => println!("{}", p.display()),
        None => {
            println!("{}", model::default_lab_path().display());
            eprintln!("(does not exist yet — create it with `paniolo init`)");
        }
    }
    Ok(())
}

fn config_edit(lab_flag: Option<&str>) -> Result<()> {
    let path = model::resolve_lab_path(lab_flag)
        .ok_or_else(|| anyhow!("No lab to edit. Create one with `paniolo init`."))?;
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let prog = parts.next().unwrap_or("vi");
    let status = std::process::Command::new(prog)
        .args(parts)
        .arg(&path)
        .status()?;
    std::process::exit(status.code().unwrap_or(1));
}

fn config_show(lab_flag: Option<&str>) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    let path = model::resolve_lab_path(lab_flag).unwrap_or_default();
    println!("Lab {}", path.display());

    println!("  Hosts");
    if lab.hosts.is_empty() {
        println!("    (none — everything runs on the local dev machine)");
    } else {
        for (name, h) in &lab.hosts {
            let id = h
                .identity
                .as_deref()
                .map(|i| format!("  identity={i}"))
                .unwrap_or_default();
            println!("    {name}  {}{id}", h.ssh);
        }
    }

    println!("  Targets");
    if lab.targets.is_empty() {
        println!("    (none)");
    } else {
        for name in lab.targets.keys() {
            let rt = lab.resolved_target(name).unwrap();
            println!("    {}", target_headline(&rt));
            if let Some(note) = &rt.note {
                println!("      note: {note}");
            }
            for ch in &rt.channels {
                println!("      {}", channel_label(ch));
            }
            if rt.channels.is_empty() {
                println!("      (no channels)");
            }
        }
    }
    Ok(())
}

// ── host ────────────────────────────────────────────────────────────────────

fn host_cmd(lab_flag: Option<&str>, cmd: HostCmd) -> Result<()> {
    match cmd {
        HostCmd::List => host_list(lab_flag),
        HostCmd::Show { name } => host_show(lab_flag, &name),
        HostCmd::Add {
            name,
            ssh,
            identity,
            control_path,
            paniolo_cmd,
        } => {
            edit_lab(lab_flag, |lf| {
                lf.add_host(
                    &name,
                    &ssh,
                    identity.as_deref(),
                    control_path.as_deref(),
                    paniolo_cmd.as_deref(),
                )
            })?;
            println!("Host '{name}' added. ({ssh})");
            Ok(())
        }
        HostCmd::Set {
            name,
            ssh,
            identity,
            control_path,
            paniolo_cmd,
        } => {
            edit_lab(lab_flag, |lf| {
                lf.update_host(
                    &name,
                    ssh.as_deref(),
                    identity.as_deref(),
                    control_path.as_deref(),
                    paniolo_cmd.as_deref(),
                )
            })?;
            println!("Host '{name}' updated.");
            Ok(())
        }
        HostCmd::Rm { name } => {
            edit_lab(lab_flag, |lf| lf.remove_host(&name))?;
            println!("Host '{name}' removed.");
            Ok(())
        }
    }
}

fn host_list(lab_flag: Option<&str>) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    if lab.hosts.is_empty() {
        println!("No hosts declared. (everything runs locally)");
        return Ok(());
    }
    for (name, h) in &lab.hosts {
        println!("{name}\t{}\t{}", h.ssh, h.identity.as_deref().unwrap_or(""));
    }
    Ok(())
}

fn host_show(lab_flag: Option<&str>, name: &str) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    if name != model::LOCAL && !lab.hosts.contains_key(name) {
        let have: Vec<&str> = lab.hosts.keys().map(String::as_str).collect();
        bail!(
            "Host '{name}' not in lab. Hosts: {}",
            if have.is_empty() {
                "(none)".into()
            } else {
                have.join(", ")
            }
        );
    }
    match lab.hosts.get(name) {
        Some(h) => {
            println!("Host: {name}");
            println!("  ssh           {}", h.ssh);
            if let Some(v) = &h.identity {
                println!("  identity      {v}");
            }
            if let Some(v) = &h.control_path {
                println!("  control_path  {v}");
            }
            if let Some(v) = &h.paniolo_cmd {
                println!("  paniolo_cmd   {v}");
            }
        }
        None => println!("Host: local  (the dev machine)"),
    }
    let pairs = lab.channels_on_host(name);
    if pairs.is_empty() {
        println!("  (no channels bound to this host)");
        return Ok(());
    }
    for (tname, ch) in pairs {
        println!(
            "  {tname}\t{}\t{}",
            channel_name(&ch),
            fields_str(&ch.fields)
        );
    }
    Ok(())
}

// ── target ────────────────────────────────────────────────────────────────────

fn target_cmd(lab_flag: Option<&str>, cmd: TargetCmd) -> Result<()> {
    match cmd {
        TargetCmd::List => target_list(lab_flag),
        TargetCmd::Show { name } => target_show(lab_flag, name.as_deref()),
        TargetCmd::Add { name, host, note } => {
            edit_lab(lab_flag, |lf| {
                lf.add_target(&name, host.as_deref(), note.as_deref())
            })?;
            println!("Target '{name}' added.");
            Ok(())
        }
        TargetCmd::Set { name, host, note } => {
            edit_lab(lab_flag, |lf| {
                lf.update_target(&name, host.as_deref(), note.as_deref())
            })?;
            println!("Target '{name}' updated.");
            Ok(())
        }
        TargetCmd::Rm { name } => {
            edit_lab(lab_flag, |lf| lf.remove_target(&name))?;
            println!("Target '{name}' removed.");
            Ok(())
        }
    }
}

fn target_list(lab_flag: Option<&str>) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    if lab.targets.is_empty() {
        println!("No targets configured. (paniolo target add <name>)");
        return Ok(());
    }
    for name in lab.targets.keys() {
        let rt = lab.resolved_target(name).unwrap();
        let chans: Vec<String> = rt
            .channels
            .iter()
            .map(|c| {
                if c.name == c.kind.as_str() {
                    c.kind.as_str().to_string()
                } else {
                    format!("{}:{}", c.kind.as_str(), c.name)
                }
            })
            .collect();
        println!("{name}\t{}\t{}", rt.hosts().join(", "), chans.join(", "));
    }
    Ok(())
}

fn target_show(lab_flag: Option<&str>, name: Option<&str>) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    let names: Vec<String> = match name {
        Some(n) => vec![n.to_string()],
        None => lab.targets.keys().cloned().collect(),
    };
    if names.is_empty() {
        println!("No targets configured.");
        return Ok(());
    }
    for tname in names {
        match lab.resolved_target(&tname) {
            Some(rt) => print_resolved_target(&rt),
            None => eprintln!("Target '{tname}' not found in lab."),
        }
    }
    Ok(())
}

// ── serial / netboot / power (config) ───────────────────────────────────────

fn serial_cmd(lab_flag: Option<&str>, cmd: SerialCmd) -> Result<()> {
    match cmd {
        SerialCmd::Add {
            name,
            target,
            device,
            baud,
            sense,
            host,
        } => {
            let sense = normalize_sense(sense.as_deref())?;
            edit_lab(lab_flag, |lf| {
                lf.add_serial(
                    &target,
                    &name,
                    &device,
                    baud,
                    sense.as_deref(),
                    host.as_deref(),
                )
            })?;
            println!("Serial '{name}' added to '{target}': {device} @ {baud}");
            Ok(())
        }
        SerialCmd::Set {
            name,
            target,
            device,
            baud,
            sense,
            host,
        } => {
            let sense = match sense.as_deref() {
                None => None,
                Some(s) => match normalize_sense(Some(s))? {
                    Some(v) => Some(v),
                    None => bail!(
                        "`set` can't clear a sense signal. Remove and re-add the \
                         interface to clear it."
                    ),
                },
            };
            edit_lab(lab_flag, |lf| {
                lf.update_serial(
                    &target,
                    &name,
                    device.as_deref(),
                    baud,
                    sense.as_deref(),
                    host.as_deref(),
                )
            })?;
            println!("Serial '{name}' updated on '{target}'.");
            Ok(())
        }
        SerialCmd::Rm { name, target } => {
            edit_lab(lab_flag, |lf| lf.remove_serial(&target, &name))?;
            println!("Serial '{name}' removed from '{target}'.");
            Ok(())
        }
        SerialCmd::Connect { target, interface } => {
            cmd_serial_connect(lab_flag, target.as_deref(), interface.as_deref())
        }
        SerialCmd::Watch { target, port } => cmd_serial_watch(lab_flag, target.as_deref(), port),
        SerialCmd::Stop => {
            let code = serial::stop_daemon()?;
            if code == 0 {
                println!("Serial daemon stopped.");
                Ok(())
            } else {
                std::process::exit(code);
            }
        }
        SerialCmd::Send {
            text,
            target,
            interface,
            pace_ms,
            no_newline,
        } => cmd_serial_send(
            lab_flag,
            target.as_deref(),
            interface.as_deref(),
            &text,
            pace_ms,
            !no_newline,
        ),
        SerialCmd::Log {
            target,
            interface,
            tail,
            from,
            to,
            since,
            raw,
            json,
            no_pending,
        } => cmd_serial_log(
            lab_flag,
            target.as_deref(),
            interface.as_deref(),
            tail,
            from,
            to,
            since,
            raw,
            json,
            no_pending,
        ),
        SerialCmd::Devices => {
            let devices = serial::list_devices();
            if devices.is_empty() {
                println!("No serial devices found.");
            }
            for d in devices {
                println!("  {d}");
            }
            Ok(())
        }
        SerialCmd::Show { target } => cmd_serial_show(lab_flag, target.as_deref()),
        SerialCmd::Dtr {
            target,
            ms,
            interface,
        } => cmd_serial_dtr(
            lab_flag,
            target.as_deref(),
            ms,
            interface.as_deref(),
            "DTR pulse",
        ),
        SerialCmd::Reset {
            target,
            ms,
            interface,
        } => cmd_serial_dtr(
            lab_flag,
            target.as_deref(),
            ms,
            interface.as_deref(),
            "Soft reset",
        ),
    }
}

/// Pulse DTR on a target's serial interface — via the serialcap daemon when it
/// is running (it owns the port), else directly.
fn cmd_serial_dtr(
    lab_flag: Option<&str>,
    target: Option<&str>,
    ms: u64,
    interface: Option<&str>,
    label: &str,
) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    let target = resolve_single_target(&lab, target)?;
    // Default interface: the power channel's serial_interface (if configured).
    let default_iface = lab
        .targets
        .get(&target)
        .and_then(|t| t.power.as_ref())
        .and_then(|p| p.serial_interface.clone());
    let iface = interface.map(String::from).or(default_iface);
    if let Some(code) = dispatch::maybe_dispatch(
        &lab,
        &target,
        model::ChannelKind::Serial,
        iface.as_deref(),
        dispatch::Mode::Reexec,
    )? {
        std::process::exit(code);
    }
    let serials = local_serials(&lab, &target)?;
    let ch = pick_serial(&serials, iface.as_deref())?;
    if let Some(url) = serial::daemon_url() {
        eprintln!("{label} on '{target}' ({ms} ms via serialcap daemon)");
        power::dtr_press_daemon(&url, &ch.name, ms)?;
    } else {
        eprintln!("{label} on '{target}' ({ms} ms via {} directly)", ch.device);
        power::dtr_press_direct(&ch.device, ms)?;
    }
    println!("Done.");
    Ok(())
}

// ── console (composite: video + serial dashboard) ───────────────────────────

fn dashboard_url(video_base: &str, serial_ws: Option<&str>, interface: Option<&str>) -> String {
    let mut params: Vec<String> = Vec::new();
    if let Some(ws) = serial_ws {
        params.push(format!("serialws={ws}"));
    }
    if let Some(i) = interface {
        params.push(format!("interface={i}"));
    }
    if params.is_empty() {
        video_base.to_string()
    } else {
        format!("{video_base}/?{}", params.join("&"))
    }
}

fn open_in_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// The combined dashboard is a composite command: it needs the serial and video
/// channels co-located on one host. Locally it ensures both daemons; remotely
/// it starts them over SSH and holds tunnels to both.
fn cmd_console(
    lab_flag: Option<&str>,
    target: Option<&str>,
    interface: Option<&str>,
) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    let target = resolve_single_target(&lab, target)?;
    let rt = lab
        .resolved_target(&target)
        .ok_or_else(|| anyhow!("target '{target}' not found in lab"))?;
    let serial_host = model::channel_host(&rt, model::ChannelKind::Serial, interface)?;
    let video_host = model::channel_host(&rt, model::ChannelKind::Video, None)?;
    if serial_host != video_host {
        bail!(
            "console needs the serial and video channels on one host; \
             serial is on '{serial_host}', video on '{video_host}'"
        );
    }
    let host = lab.host(&serial_host);
    if !host.is_local(&serial_host) {
        return remote_console(&lab, &target, &serial_host, interface);
    }

    // Local: ensure both daemons (OS-assigned ports; discovery finds them).
    let video_url = match video::daemon_url() {
        Some(u) => u,
        None => {
            let v = local_video(&lab, &target)?;
            let device = v
                .device
                .ok_or_else(|| anyhow!("video channel for '{target}' has no device set"))?;
            eprintln!("Starting video daemon…");
            video::start_daemon(&device, 0, Some(&target))?;
            daemons::wait_for_daemon(video::DAEMON, std::time::Duration::from_secs(5))
                .ok_or_else(|| anyhow!("video daemon did not start within 5 s"))?
        }
    };
    if serial::daemon_url().is_none() {
        let serials = local_serials(&lab, &target)?;
        if serials.is_empty() {
            bail!("no serial interfaces configured for '{target}' (paniolo serial add ...)");
        }
        eprintln!("Starting serial daemon…");
        serial::start_daemon(&serials, 0)?;
        daemons::wait_for_daemon(serial::DAEMON, std::time::Duration::from_secs(5))
            .ok_or_else(|| anyhow!("serial daemon did not start within 5 s"))?;
    }
    let url = dashboard_url(&video_url, None, interface);
    open_in_browser(&url);
    println!("Opened {url}");
    Ok(())
}

fn remote_console(lab: &Lab, target: &str, host_name: &str, interface: Option<&str>) -> Result<()> {
    let host = lab.host(host_name);
    eprintln!("Starting daemons on {host_name}…");
    for sub in [["video", "watch"], ["serial", "watch"]] {
        let out = dispatch::run_subcommand(lab, target, host_name, &[sub[0], sub[1], target])?;
        if out.status != 0 {
            let msg = if out.stderr.trim().is_empty() {
                out.stdout.trim().to_string()
            } else {
                out.stderr.trim().to_string()
            };
            bail!(
                "failed to start '{} {}' on {host_name}: {msg}",
                sub[0],
                sub[1]
            );
        }
    }
    let video_port = dispatch::remote_daemon_port(&host, "hdmicap")
        .ok_or_else(|| anyhow!("could not read the hdmicap daemon port on {host_name}"))?;
    let serial_port = dispatch::remote_daemon_port(&host, "serialcap")
        .ok_or_else(|| anyhow!("could not read the serialcap daemon port on {host_name}"))?;

    let fwd_video = ssh::forward(&host, video_port)?;
    let fwd_serial = ssh::forward(&host, serial_port)?;
    let url = dashboard_url(
        &format!("http://127.0.0.1:{}", fwd_video.local_port),
        Some(&format!("ws://127.0.0.1:{}/stream", fwd_serial.local_port)),
        interface,
    );
    open_in_browser(&url);
    println!("Opened {url}");
    println!("Tunnels to {host_name} open. Press Ctrl-C to close.");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

// ── power runtime bodies ────────────────────────────────────────────────────

/// The target's power channel as visible on *this* host.
fn local_power(lab: &Lab, target: &str) -> Result<model::PowerChannel> {
    let t = lab
        .targets
        .get(target)
        .ok_or_else(|| anyhow!("target '{target}' not found in lab"))?;
    let dh = t.default_host().to_string();
    let p = t.power.clone().ok_or_else(|| {
        anyhow!("target '{target}' has no power channel (paniolo power set -t {target} ...)")
    })?;
    if p.host.as_deref().unwrap_or(&dh) != model::LOCAL {
        bail!("power channel for '{target}' is not on this host");
    }
    Ok(p)
}

fn cmd_power_cycle(lab_flag: Option<&str>, target: Option<&str>) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    let target = resolve_single_target(&lab, target)?;
    if let Some(code) = dispatch::maybe_dispatch(
        &lab,
        &target,
        model::ChannelKind::Power,
        None,
        dispatch::Mode::Reexec,
    )? {
        std::process::exit(code);
    }
    let p = local_power(&lab, &target)?;
    let cmd = p.cycle_cmd.ok_or_else(|| {
        anyhow!(
            "no cycle_cmd configured for '{target}' \
             (paniolo power set -t {target} --cycle-cmd /path/to/script)"
        )
    })?;
    eprintln!("Power cycling '{target}' via {cmd}");
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .status()?;
    if status.success() {
        println!("Power cycle complete.");
        Ok(())
    } else {
        eprintln!(
            "power-cycle script exited with code {}",
            status.code().unwrap_or(1)
        );
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn cmd_power_state(lab_flag: Option<&str>, target: Option<&str>) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    let target = resolve_single_target(&lab, target)?;
    if let Some(code) = dispatch::maybe_dispatch(
        &lab,
        &target,
        model::ChannelKind::Power,
        None,
        dispatch::Mode::Reexec,
    )? {
        std::process::exit(code);
    }
    let p = local_power(&lab, &target)?;
    let si = p.serial_interface.ok_or_else(|| {
        anyhow!(
            "no power serial_interface configured for '{target}' \
             (paniolo power set -t {target} --serial-interface <name>)"
        )
    })?;
    let url = serial::daemon_url().ok_or_else(|| {
        anyhow!("serialcap daemon not running — start it with `paniolo serial watch`")
    })?;
    match power::read_power_state(&url, &si) {
        Some(true) => {
            println!("Power ON  ({target})");
            Ok(())
        }
        Some(false) => {
            println!("Power OFF  ({target})");
            Ok(())
        }
        None => bail!(
            "power state unknown — the sense signal may not be configured on '{si}' \
             (paniolo serial set {si} -t {target} --sense <cts|dsr|dcd|ri>)"
        ),
    }
}

// ── serial runtime bodies ───────────────────────────────────────────────────

/// The target's serial channels visible on *this* host (all of them when
/// running against a shipped slice, the local ones otherwise).
fn local_serials(lab: &Lab, target: &str) -> Result<Vec<model::SerialChannel>> {
    let t = lab
        .targets
        .get(target)
        .ok_or_else(|| anyhow!("target '{target}' not found in lab"))?;
    let dh = t.default_host().to_string();
    Ok(t.serial
        .iter()
        .filter(|s| s.host.as_deref().unwrap_or(&dh) == model::LOCAL)
        .cloned()
        .collect())
}

fn pick_serial<'a>(
    serials: &'a [model::SerialChannel],
    name: Option<&str>,
) -> Result<&'a model::SerialChannel> {
    let have = || {
        serials
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match name {
        Some(n) => serials
            .iter()
            .find(|s| s.name == n)
            .ok_or_else(|| anyhow!("no serial interface '{n}' (have: {})", have())),
        None => match serials.len() {
            1 => Ok(&serials[0]),
            0 => bail!("no serial interfaces configured (paniolo serial add ...)"),
            _ => bail!(
                "multiple serial interfaces ({}); specify one with -i",
                have()
            ),
        },
    }
}

/// Common preamble for serial runtime commands: resolve the target, dispatch
/// to the channel's host if remote, and return the local serial channels.
fn serial_runtime(
    lab_flag: Option<&str>,
    target: Option<&str>,
    interface: Option<&str>,
    mode: dispatch::Mode,
) -> Result<Vec<model::SerialChannel>> {
    let lab = load_for_read(lab_flag)?;
    let target = resolve_single_target(&lab, target)?;
    if let Some(code) =
        dispatch::maybe_dispatch(&lab, &target, model::ChannelKind::Serial, interface, mode)?
    {
        std::process::exit(code);
    }
    local_serials(&lab, &target)
}

fn cmd_serial_connect(
    lab_flag: Option<&str>,
    target: Option<&str>,
    interface: Option<&str>,
) -> Result<()> {
    let serials = serial_runtime(lab_flag, target, interface, dispatch::Mode::Interactive)?;
    let ch = pick_serial(&serials, interface)?;
    serial::exec_tio(&ch.device, ch.baud)
}

fn cmd_serial_watch(lab_flag: Option<&str>, target: Option<&str>, port: u16) -> Result<()> {
    let serials = serial_runtime(lab_flag, target, None, dispatch::Mode::Reexec)?;
    if serials.is_empty() {
        bail!("no serial interfaces configured (paniolo serial add ...)");
    }
    if let Some(url) = serial::daemon_url() {
        println!("Serial daemon already running at {url}");
        return Ok(());
    }
    serial::start_daemon(&serials, port)?;
    let names: Vec<&str> = serials.iter().map(|s| s.name.as_str()).collect();
    eprintln!(
        "Starting serial daemon for {} interface(s): {}…",
        serials.len(),
        names.join(", ")
    );
    match daemons::wait_for_daemon(serial::DAEMON, std::time::Duration::from_secs(5)) {
        Some(url) => {
            println!("Serial daemon started. {url}");
            Ok(())
        }
        None => bail!("serial daemon did not start within 5 s"),
    }
}

fn cmd_serial_send(
    lab_flag: Option<&str>,
    target: Option<&str>,
    interface: Option<&str>,
    text: &str,
    pace_ms: u32,
    newline: bool,
) -> Result<()> {
    let serials = serial_runtime(lab_flag, target, interface, dispatch::Mode::Reexec)?;
    let ch = pick_serial(&serials, interface)?;
    let url = serial::daemon_url().ok_or_else(|| {
        anyhow!("serialcap daemon not running — start it with `paniolo serial watch`")
    })?;
    let mut payload = text.as_bytes().to_vec();
    if newline {
        payload.push(b'\r');
    }
    let pace_note = if pace_ms > 0 {
        format!(" paced {pace_ms} ms/byte")
    } else {
        String::new()
    };
    eprintln!(
        "Sending {} bytes to '{}'{pace_note}",
        payload.len(),
        ch.name
    );
    serial::send_input(&url, &ch.name, &payload, pace_ms)?;
    println!("Sent.");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_serial_log(
    lab_flag: Option<&str>,
    target: Option<&str>,
    interface: Option<&str>,
    tail: Option<u64>,
    from: Option<u64>,
    to: Option<u64>,
    since: Option<u64>,
    raw: bool,
    json: bool,
    no_pending: bool,
) -> Result<()> {
    // Dispatch to the channel's host; the capture log lives where the daemon ran.
    let serials = serial_runtime(lab_flag, target, interface, dispatch::Mode::Reexec)?;
    // serialcap reads its own on-disk log, so this works daemon-up or -down.
    let binary = daemons::find_binary(serial::DAEMON)
        .ok_or_else(|| anyhow!("serialcap not found — run `paniolo setup`"))?;
    let mut cmd = std::process::Command::new(binary);
    cmd.arg("log");
    // Name the interface explicitly when we can (sole or selected).
    if let Some(name) = interface.or_else(|| {
        if serials.len() == 1 {
            Some(serials[0].name.as_str())
        } else {
            None
        }
    }) {
        cmd.arg("--interface").arg(name);
    }
    if let Some(n) = tail {
        cmd.arg("--tail").arg(n.to_string());
    }
    if let Some(n) = from {
        cmd.arg("--from").arg(n.to_string());
    }
    if let Some(n) = to {
        cmd.arg("--to").arg(n.to_string());
    }
    if let Some(n) = since {
        cmd.arg("--since").arg(n.to_string());
    }
    if raw {
        cmd.arg("--raw");
    }
    if json {
        cmd.arg("--json");
    }
    if no_pending {
        cmd.arg("--no-pending");
    }
    let status = cmd.status()?;
    if status.success() {
        Ok(())
    } else {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn cmd_serial_show(lab_flag: Option<&str>, target: Option<&str>) -> Result<()> {
    let serials = serial_runtime(lab_flag, target, None, dispatch::Mode::Reexec)?;
    if serials.is_empty() {
        println!("No serial interfaces configured. (paniolo serial add ...)");
        return Ok(());
    }
    for ch in &serials {
        let sense = ch
            .power_sense_signal
            .as_deref()
            .map(|s| format!("  (sense: {s})"))
            .unwrap_or_default();
        println!("{}\t{} @ {}{sense}", ch.name, ch.device, ch.baud);
    }
    match serial::daemon_url() {
        Some(url) => println!("daemon\trunning at {url}"),
        None => println!("daemon\tstopped"),
    }
    Ok(())
}

// ── video runtime bodies ────────────────────────────────────────────────────

/// The target's video channel as visible on *this* host.
fn local_video(lab: &Lab, target: &str) -> Result<model::VideoChannel> {
    let t = lab
        .targets
        .get(target)
        .ok_or_else(|| anyhow!("target '{target}' not found in lab"))?;
    let dh = t.default_host().to_string();
    let v = t.video.clone().ok_or_else(|| {
        anyhow!(
            "target '{target}' has no video channel (paniolo video set -t {target} --device ...)"
        )
    })?;
    if v.host.as_deref().unwrap_or(&dh) != model::LOCAL {
        bail!("video channel for '{target}' is not on this host");
    }
    Ok(v)
}

fn video_runtime(
    lab_flag: Option<&str>,
    target: Option<&str>,
) -> Result<(String, model::VideoChannel)> {
    let lab = load_for_read(lab_flag)?;
    let target = resolve_single_target(&lab, target)?;
    if let Some(code) = dispatch::maybe_dispatch(
        &lab,
        &target,
        model::ChannelKind::Video,
        None,
        dispatch::Mode::Reexec,
    )? {
        std::process::exit(code);
    }
    let v = local_video(&lab, &target)?;
    Ok((target, v))
}

fn video_cmd(lab_flag: Option<&str>, cmd: VideoCmd) -> Result<()> {
    match cmd {
        VideoCmd::Set {
            target,
            device,
            host,
        } => {
            edit_lab(lab_flag, |lf| {
                lf.set_video(&target, Some(&device), host.as_deref())
            })?;
            println!("video channel set for '{target}'.");
            Ok(())
        }
        VideoCmd::Rm { target } => {
            edit_lab(lab_flag, |lf| lf.remove_video(&target))?;
            println!("video channel removed from '{target}'.");
            Ok(())
        }
        VideoCmd::Watch {
            target,
            port,
            restart,
        } => {
            let (target, v) = video_runtime(lab_flag, target.as_deref())?;
            let device = v
                .device
                .ok_or_else(|| anyhow!("video channel for '{target}' has no device set"))?;
            if let Some(url) = video::daemon_url() {
                if !restart {
                    println!("Video daemon already running at {url}");
                    return Ok(());
                }
                let _ = video::stop_daemon();
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            eprintln!("Starting video daemon for '{device}'…");
            video::start_daemon(&device, port, Some(&target))?;
            match daemons::wait_for_daemon(video::DAEMON, std::time::Duration::from_secs(5)) {
                Some(url) => {
                    println!("Video daemon started. Preview at {url}");
                    Ok(())
                }
                None => bail!("video daemon did not start within 5 s"),
            }
        }
        VideoCmd::Stop => {
            let code = video::stop_daemon()?;
            if code == 0 {
                println!("Video daemon stopped.");
                Ok(())
            } else {
                std::process::exit(code);
            }
        }
        VideoCmd::Shot {
            target,
            stable,
            changed_since,
            timeout,
            out,
        } => {
            let _ = video_runtime(lab_flag, target.as_deref())?;
            let mut args = vec![
                "shot".to_string(),
                "--timeout".to_string(),
                timeout.to_string(),
                "--out".to_string(),
                out,
            ];
            if stable {
                args.push("--stable".to_string());
            }
            if let Some(h) = changed_since {
                args.push("--changed-since".to_string());
                args.push(h);
            }
            std::process::exit(video::passthrough(&args)?);
        }
        VideoCmd::Preview => match video::daemon_url() {
            Some(url) => {
                println!("{url}");
                Ok(())
            }
            None => bail!("no video daemon running — start one with `paniolo video watch`"),
        },
        VideoCmd::Devices => {
            std::process::exit(video::passthrough(&["devices".to_string()])?);
        }
        VideoCmd::Show { target } => {
            let (_target, v) = video_runtime(lab_flag, target.as_deref())?;
            println!("device\t{}", v.device.as_deref().unwrap_or("(not set)"));
            match video::daemon_url() {
                Some(url) => println!("daemon\trunning at {url}"),
                None => println!("daemon\tstopped"),
            }
            Ok(())
        }
    }
}

fn netboot_cmd(lab_flag: Option<&str>, cmd: NetbootCmd) -> Result<()> {
    match cmd {
        NetbootCmd::Set {
            target,
            interface,
            host_ip,
            tftp_root,
            host,
        } => {
            edit_lab(lab_flag, |lf| {
                lf.set_netboot(
                    &target,
                    interface.as_deref(),
                    host_ip.as_deref(),
                    tftp_root.as_deref(),
                    host.as_deref(),
                )
            })?;
            println!("netboot channel set for '{target}'.");
            Ok(())
        }
        NetbootCmd::Rm { target } => {
            edit_lab(lab_flag, |lf| lf.remove_netboot(&target))?;
            println!("netboot channel removed from '{target}'.");
            Ok(())
        }
        NetbootCmd::Start { target } => {
            let (target, iface, host_ip, tftp_root) = netboot_runtime(lab_flag, target.as_deref())?;
            let root = tftp_root.ok_or_else(|| {
                anyhow!(
                    "no tftp_root configured \
                     (paniolo netboot set -t {target} --tftp-root <path>)"
                )
            })?;
            netboot::start(&target, &iface, &host_ip, &root)?;
            println!("netboot started for '{target}' on {iface} ({host_ip}, tftp {root}).");
            Ok(())
        }
        NetbootCmd::Stop { target } => {
            let (target, ..) = netboot_runtime(lab_flag, target.as_deref())?;
            netboot::stop(&target)?;
            println!("netboot stopped for '{target}'.");
            Ok(())
        }
        NetbootCmd::Status { target } => {
            let (target, ..) = netboot_runtime(lab_flag, target.as_deref())?;
            let st = netboot::status(&target);
            match st.state {
                None => println!("netboot\tnot running (no state)"),
                Some(s) => {
                    println!(
                        "netboot\t{}",
                        if st.running {
                            "running"
                        } else {
                            "NOT running (stale state)"
                        }
                    );
                    println!("pid\t{}", s.dhcp_pid);
                    println!("interface\t{}", s.interface);
                    println!("tftp_root\t{}", s.tftp_root);
                    if let Some(up) = st.uptime_seconds {
                        println!("uptime\t{:.0}s", up);
                    }
                }
            }
            Ok(())
        }
        NetbootCmd::Logs {
            target,
            tail,
            follow,
        } => {
            let (target, ..) = netboot_runtime(lab_flag, target.as_deref())?;
            cmd_netboot_logs(&target, tail, follow)
        }
        NetbootCmd::TftpRoot { target } => {
            let (_t, _i, _ip, tftp_root) = netboot_runtime(lab_flag, target.as_deref())?;
            match tftp_root {
                Some(r) => {
                    println!("{r}");
                    Ok(())
                }
                None => bail!("no tftp_root configured"),
            }
        }
        NetbootCmd::Devices => {
            let ifaces = netif::list_usb_ethernet_interfaces();
            if ifaces.is_empty() {
                println!("No candidate Ethernet interfaces found.");
            }
            for i in ifaces {
                println!(
                    "{}\t{}\t{}",
                    i.device,
                    i.port,
                    if i.active { "active" } else { "inactive" }
                );
            }
            Ok(())
        }
        NetbootCmd::LinkUp { target } => {
            let (_t, iface, host_ip, _r) = netboot_runtime(lab_flag, target.as_deref())?;
            netif::configure_interface(&iface, &host_ip)?;
            let active = netif::is_interface_active(&iface);
            println!(
                "Link {}  {iface}  {host_ip}",
                if active { "up" } else { "not yet up" }
            );
            Ok(())
        }
        NetbootCmd::LinkDown { target } => {
            let (_t, iface, ..) = netboot_runtime(lab_flag, target.as_deref())?;
            netif::restore_interface(&iface);
            println!("Link down  {iface}");
            Ok(())
        }
        NetbootCmd::LinkStatus { target } => {
            let (_t, iface, ..) = netboot_runtime(lab_flag, target.as_deref())?;
            let (inet, inet6) = netif::iface_addresses(&iface);
            println!("interface\t{iface}");
            println!(
                "carrier\t{}",
                if netif::is_interface_active(&iface) {
                    "yes"
                } else {
                    "no"
                }
            );
            let mut addrs = inet;
            addrs.extend(inet6);
            println!(
                "addresses\t{}",
                if addrs.is_empty() {
                    "(none)".to_string()
                } else {
                    addrs.join(" ")
                }
            );
            Ok(())
        }
    }
}

/// Common preamble for netboot/netif runtime commands: resolve, dispatch to the
/// netboot channel's host if remote, and return (target, interface, host_ip,
/// tftp_root) from the local channel.
fn netboot_runtime(
    lab_flag: Option<&str>,
    target: Option<&str>,
) -> Result<(String, String, String, Option<String>)> {
    let lab = load_for_read(lab_flag)?;
    let target = resolve_single_target(&lab, target)?;
    if let Some(code) = dispatch::maybe_dispatch(
        &lab,
        &target,
        model::ChannelKind::Netboot,
        None,
        dispatch::Mode::Reexec,
    )? {
        std::process::exit(code);
    }
    let t = lab
        .targets
        .get(&target)
        .ok_or_else(|| anyhow!("target '{target}' not found in lab"))?;
    let dh = t.default_host().to_string();
    let nb = t.netboot.clone().ok_or_else(|| {
        anyhow!("target '{target}' has no netboot channel (paniolo netboot set -t {target} ...)")
    })?;
    if nb.host.as_deref().unwrap_or(&dh) != model::LOCAL {
        bail!("netboot channel for '{target}' is not on this host");
    }
    let iface = nb
        .interface
        .ok_or_else(|| anyhow!("netboot channel for '{target}' has no interface set"))?;
    let host_ip = nb
        .host_ip
        .unwrap_or_else(|| model::DEFAULT_HOST_IP.to_string());
    Ok((target, iface, host_ip, nb.tftp_root))
}

fn cmd_netboot_logs(target: &str, tail: usize, follow: bool) -> Result<()> {
    let path = state::netboot_log_path(target);
    if !path.exists() {
        bail!("no netboot log for '{target}' yet ({})", path.display());
    }
    let text = std::fs::read_to_string(&path)?;
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(tail);
    for line in &lines[start..] {
        println!("{line}");
    }
    if !follow {
        return Ok(());
    }
    // Follow: poll for appended bytes (Ctrl-C to stop).
    use std::io::{Read, Seek};
    let mut f = std::fs::File::open(&path)?;
    f.seek(std::io::SeekFrom::End(0))?;
    let mut buf = String::new();
    loop {
        buf.clear();
        f.read_to_string(&mut buf)?;
        if !buf.is_empty() {
            print!("{buf}");
            use std::io::Write;
            std::io::stdout().flush()?;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

fn netif_cmd(lab_flag: Option<&str>, cmd: NetifCmd) -> Result<()> {
    match cmd {
        NetifCmd::Mode { mode, target } => {
            if !netif::MODES.contains(&mode.as_str()) {
                bail!(
                    "unknown mode '{mode}' (use one of: {})",
                    netif::MODES.join(", ")
                );
            }
            let (target, iface, host_ip, tftp_root) = netboot_runtime(lab_flag, target.as_deref())?;
            match mode.as_str() {
                "netboot" => {
                    let root = tftp_root.clone().ok_or_else(|| {
                        anyhow!("no tftp_root configured — netboot mode needs one")
                    })?;
                    netif::mode_netboot(&target, &iface, &host_ip, &root)?;
                }
                "ffx" => netif::mode_ffx(&target, &iface)?,
                _ => netif::mode_off(&target, &iface, &host_ip)?,
            }
            print_netif_status(&target, &iface, &host_ip);
            Ok(())
        }
        NetifCmd::Status { target } => {
            let (target, iface, host_ip, _r) = netboot_runtime(lab_flag, target.as_deref())?;
            print_netif_status(&target, &iface, &host_ip);
            Ok(())
        }
    }
}

fn print_netif_status(target: &str, iface: &str, host_ip: &str) {
    let s = netif::get_status(target, iface);
    println!("target\t{target}");
    println!("interface\t{iface}");
    println!("mode\t{}", s.mode);
    println!(
        "inet\t{}",
        if s.inet.is_empty() {
            "(none)".to_string()
        } else {
            s.inet.join(" ")
        }
    );
    println!(
        "inet6\t{}",
        if s.inet6.is_empty() {
            "(none)".to_string()
        } else {
            s.inet6.join(" ")
        }
    );
    if s.mode == "netboot" {
        println!("dhcp+tftp\tserving on {host_ip}/24");
    }
    if s.mode == "ffx" {
        if s.peers.is_empty() {
            println!("peer\t(none discovered yet — power-cycle the target and wait for SLAAC)");
        }
        for peer in &s.peers {
            println!("peer\t{peer}%{iface}  (try: ffx target add {peer}%{iface})");
        }
    }
}

fn power_cmd(lab_flag: Option<&str>, cmd: PowerCmd) -> Result<()> {
    match cmd {
        PowerCmd::Set {
            target,
            cycle_cmd,
            serial_interface,
            host,
        } => {
            edit_lab(lab_flag, |lf| {
                lf.set_power(
                    &target,
                    cycle_cmd.as_deref(),
                    serial_interface.as_deref(),
                    host.as_deref(),
                )
            })?;
            println!("power channel set for '{target}'.");
            Ok(())
        }
        PowerCmd::Rm { target } => {
            edit_lab(lab_flag, |lf| lf.remove_power(&target))?;
            println!("power channel removed from '{target}'.");
            Ok(())
        }
    }
}

// ── rendering helpers ───────────────────────────────────────────────────────

fn normalize_sense(sense: Option<&str>) -> Result<Option<String>> {
    match sense {
        None => Ok(None),
        Some(s) if s.eq_ignore_ascii_case("none") => Ok(None),
        Some(s) => {
            let v = s.to_ascii_lowercase();
            if !model::VALID_SENSE_SIGNALS.contains(&v.as_str()) {
                bail!(
                    "Unknown sense signal '{s}'. Valid: {}, none",
                    model::VALID_SENSE_SIGNALS.join(", ")
                );
            }
            Ok(Some(v))
        }
    }
}

fn fields_str(fields: &[(&'static str, String)]) -> String {
    fields
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("  ")
}

fn channel_name(ch: &ResolvedChannel) -> String {
    if ch.name == ch.kind.as_str() {
        ch.kind.as_str().to_string()
    } else {
        format!("{} {}", ch.kind.as_str(), ch.name)
    }
}

fn channel_label(ch: &ResolvedChannel) -> String {
    let fields = fields_str(&ch.fields);
    let tail = if fields.is_empty() {
        String::new()
    } else {
        format!("  {fields}")
    };
    format!("{} @{}{tail}", channel_name(ch), ch.host)
}

fn target_headline(rt: &ResolvedTarget) -> String {
    let hosts = rt.hosts();
    if hosts.len() > 1 {
        format!("{}  (spans {})", rt.name, hosts.join(", "))
    } else if let Some(h) = hosts.first() {
        format!("{}  @{h}", rt.name)
    } else {
        rt.name.clone()
    }
}

fn print_resolved_target(rt: &ResolvedTarget) {
    println!("Target: {}", target_headline(rt));
    println!("  default host  {}", rt.default_host);
    if let Some(note) = &rt.note {
        println!("  note          {note}");
    }
    if rt.channels.is_empty() {
        println!("  channels      (none)");
    } else {
        for ch in &rt.channels {
            println!("  channel       {}", channel_label(ch));
        }
    }
}
