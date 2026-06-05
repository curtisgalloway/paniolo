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
    /// Inject USB HID keyboard/mouse input via the target's configured helper.
    Hid {
        #[command(subcommand)]
        cmd: HidCmd,
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
        /// Only build + install the Rust crates (skip the OCR, setuid, and
        /// zigplug steps) — the fast path for iterating on the Rust code.
        #[arg(long)]
        rust_only: bool,
    },
    /// Run a helper binary from paniolo's private libexec dir (omit NAME to
    /// list the installed helpers).
    Helper {
        /// Helper name (e.g. hdmicap, hidrig, zigplug).
        name: Option<String>,
        /// Arguments passed through to the helper.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// List or stop paniolo's background daemons on this host.
    Daemons {
        #[command(subcommand)]
        cmd: Option<DaemonsCmd>,
    },
}

#[derive(Subcommand)]
enum DaemonsCmd {
    /// List running daemons and stray helper processes (the default).
    List,
    /// Stop daemons — named ones, or every one with --all.
    Stop {
        /// Daemon names from `paniolo daemons` (e.g. serialcap, zigplug,
        /// netbootd).
        names: Vec<String>,
        /// Stop every daemon, and TERM stray helper processes too.
        #[arg(long)]
        all: bool,
        /// SIGKILL anything still alive after the 3 s grace period.
        #[arg(long)]
        force: bool,
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
    Stop {
        /// Target whose serial host's daemon to stop (optional when local).
        target: Option<String>,
    },
    /// Send a line of input to the console through the running daemon.
    Send {
        /// With two positionals the first is the target (`serial send pi5
        /// "text"`); with one, it is the text itself (`serial send "text"`).
        #[arg(value_name = "TARGET|TEXT")]
        first: String,
        /// Text to send (when the first positional is the target).
        #[arg(value_name = "TEXT")]
        second: Option<String>,
        #[arg(long, short, conflicts_with = "second")]
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
        /// Target (optional when the lab has one); `-t` also accepted.
        #[arg(value_name = "TARGET", conflicts_with = "target")]
        target_pos: Option<String>,
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
        /// Shell command to power the target on.
        #[arg(long)]
        on_cmd: Option<String>,
        /// Shell command to power the target off.
        #[arg(long)]
        off_cmd: Option<String>,
        /// Shell command to query power state (stdout must begin with 'on' or 'off').
        #[arg(long)]
        state_cmd: Option<String>,
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
    /// Power on the target via the configured on_cmd.
    On { target: Option<String> },
    /// Power off the target via the configured off_cmd.
    Off { target: Option<String> },
}

#[derive(Subcommand)]
enum HidCmd {
    /// Configure the target's hid channel (one per target).
    Set {
        #[arg(long, short)]
        target: String,
        /// Injection helper command; `hid send` arguments are appended to it
        /// (e.g. "hidrig -d /dev/cu.usbserial-XXXX").
        #[arg(long)]
        cmd: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Remove the target's hid channel.
    Rm {
        #[arg(long, short)]
        target: String,
    },
    /// Run the configured helper with the given arguments appended,
    /// e.g. `paniolo hid send -t pi5 type hello`.
    Send {
        #[arg(long, short)]
        target: Option<String>,
        /// Arguments appended to the configured cmd (the helper's CLI).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        args: Vec<String>,
    },
    /// Start the injection daemon (the KVM path): the helper owns the UART and
    /// re-exposes it over a WebSocket. `paniolo console` starts it on demand;
    /// run this to warm it ahead of time. Idempotent.
    Serve { target: Option<String> },
    /// Stop the running injection daemon.
    Stop { target: Option<String> },
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
    Stop {
        /// Target whose video host's daemon to stop (optional when local).
        target: Option<String>,
    },
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
    /// OCR the current frame via the running daemon, printing the text.
    Read {
        target: Option<String>,
        /// Wait until the signal is stable before reading.
        #[arg(long)]
        stable: bool,
        /// Timeout in ms for the stable wait.
        #[arg(long, default_value_t = 2000)]
        timeout: u64,
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
        Command::Hid { cmd } => hid_cmd(lab_flag, cmd),
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
        Command::Setup { host, rust_only } => cmd_setup(lab_flag, host.as_deref(), rust_only),
        Command::Helper { name, args } => cmd_helper(name.as_deref(), &args),
        Command::Daemons { cmd } => match cmd.unwrap_or(DaemonsCmd::List) {
            DaemonsCmd::List => cmd_daemons_list(),
            DaemonsCmd::Stop { names, all, force } => cmd_daemons_stop(&names, all, force),
        },
    }
}

// ── daemon inventory ────────────────────────────────────────────────────────

/// Running netboot engines, found via their per-target state files.
fn running_netboots() -> Vec<(String, state::NetbootState)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(state::state_dir()) {
        for e in entries.flatten() {
            let target = e.file_name().to_string_lossy().into_owned();
            if let Some(st) = state::load_netboot_state(&target) {
                if state::is_netboot_running(&target) {
                    out.push((target, st));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// One unified view of every paniolo background process on this host: the
/// discovery-file daemons, netbootd (state files), and stray helper
/// processes running out of the libexec dir.
fn cmd_daemons_list() -> Result<()> {
    let discovered = daemons::list_discovered();
    let netboots = running_netboots();
    let mut known: Vec<i32> = discovered.iter().map(|d| d.pid).collect();
    known.extend(netboots.iter().map(|(_, st)| st.dhcp_pid));
    let strays = daemons::list_stray_helpers(&known);

    if discovered.is_empty() && netboots.is_empty() && strays.is_empty() {
        println!("No paniolo daemons running.");
        return Ok(());
    }
    for d in &discovered {
        let port = d.port.map_or("-".to_string(), |p| p.to_string());
        println!("{}\tpid {}\tport {}\t{}", d.name, d.pid, port, d.detail);
    }
    for (target, st) in &netboots {
        println!(
            "netbootd\tpid {}\tport -\ttarget {target} ({})",
            st.dhcp_pid, st.interface
        );
    }
    if !strays.is_empty() {
        println!("\nStray helper processes (not daemons — wedged one-shots?):");
        for (pid, args) in &strays {
            println!("  pid {pid}\t{args}");
        }
        println!("Stop everything with `paniolo daemons stop --all [--force]`.");
    }
    Ok(())
}

/// Stop daemons by name or wholesale: netbootd via its proper teardown
/// (interface cleanup), everything else via SIGTERM, with an optional
/// SIGKILL escalation after a grace period.
fn cmd_daemons_stop(names: &[String], all: bool, force: bool) -> Result<()> {
    if !all && names.is_empty() {
        bail!("name one or more daemons (see `paniolo daemons`), or pass --all");
    }
    let wanted = |n: &str| all || names.iter().any(|w| w == n);

    // netbootd first — its stop also restores the interface.
    for (target, _) in running_netboots() {
        if wanted("netbootd") {
            netboot::stop(&target)?;
            println!("netbootd stopped (target {target}).");
        }
    }

    let mut victims: Vec<(String, i32)> = daemons::list_discovered()
        .into_iter()
        .filter(|d| wanted(&d.name))
        .map(|d| (d.name, d.pid))
        .collect();
    let known: Vec<i32> = victims.iter().map(|(_, p)| p).copied().collect();
    if all {
        for (pid, args) in daemons::list_stray_helpers(&known) {
            let short = args
                .split_whitespace()
                .take(4)
                .collect::<Vec<_>>()
                .join(" ");
            victims.push((format!("stray: {short}"), pid));
        }
    }
    for name in names {
        if name != "netbootd" && !victims.iter().any(|(n, _)| n == name) {
            eprintln!("warning: no running daemon named '{name}'");
        }
    }
    if victims.is_empty() {
        println!("Nothing to stop.");
        return Ok(());
    }

    for (name, pid) in &victims {
        daemons::signal_pid(*pid, libc::SIGTERM);
        println!("TERM {name} (pid {pid})");
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        victims.retain(|(_, pid)| state::is_pid_alive(*pid));
        if victims.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    victims.retain(|(_, pid)| state::is_pid_alive(*pid));
    for (name, pid) in &victims {
        if force {
            daemons::signal_pid(*pid, libc::SIGKILL);
            println!("KILL {name} (pid {pid})");
        } else {
            eprintln!("still alive: {name} (pid {pid}) — re-run with --force to SIGKILL");
        }
    }
    if !force && !victims.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

// ── helper passthrough ──────────────────────────────────────────────────────

/// Run a libexec helper with stdio passed through, propagating its exit code;
/// with no name, list the installed helpers.
fn cmd_helper(name: Option<&str>, args: &[String]) -> Result<()> {
    let Some(name) = name else {
        let dir = daemons::libexec_dir()
            .ok_or_else(|| anyhow!("could not determine the home directory"))?;
        let mut names: Vec<String> = match std::fs::read_dir(&dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect(),
            Err(_) => Vec::new(),
        };
        if names.is_empty() {
            println!(
                "No helpers installed in {} — run `paniolo setup`.",
                dir.display()
            );
            return Ok(());
        }
        names.sort();
        for n in names {
            println!("{n}");
        }
        return Ok(());
    };
    let binary = daemons::find_binary(name)
        .ok_or_else(|| anyhow!("helper '{name}' not found — run `paniolo setup`"))?;
    let status = std::process::Command::new(binary).args(args).status()?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
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

fn cmd_setup(lab_flag: Option<&str>, host: Option<&str>, rust_only: bool) -> Result<()> {
    if let Some(name) = host {
        let resolved = resolve_host(lab_flag, name)?;
        if !resolved.is_local(name) {
            eprintln!("Running paniolo setup on {name} ({})…", resolved.ssh);
            let mut argv = vec![resolved.paniolo(), "setup".to_string()];
            if rust_only {
                argv.push("--rust-only".to_string());
            }
            let code = ssh::run_interactive(&resolved, &argv, &[])?;
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
    setup::run(&repo, rust_only)
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
        SerialCmd::Stop { target } => {
            // With a target, route to its serial channel's host (no-op locally).
            if let Some(t) = target.as_deref() {
                let _ = serial_runtime(lab_flag, Some(t), None, dispatch::Mode::Reexec)?;
            }
            let code = serial::stop_daemon()?;
            if code == 0 {
                println!("Serial daemon stopped.");
                Ok(())
            } else {
                std::process::exit(code);
            }
        }
        SerialCmd::Send {
            first,
            second,
            target,
            interface,
            pace_ms,
            no_newline,
        } => {
            // Two positionals = target + text; one = text (clap rejects -t
            // alongside a second positional).
            let (target, text) = match second {
                Some(text) => (Some(first), text),
                None => (target, first),
            };
            cmd_serial_send(
                lab_flag,
                target.as_deref(),
                interface.as_deref(),
                &text,
                pace_ms,
                !no_newline,
            )
        }
        SerialCmd::Log {
            target_pos,
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
            target_pos.or(target).as_deref(),
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

/// Daemon endpoints the dashboard needs, as URL query parameters. Each daemon
/// passes either a cross-port `*ws` URL (the remote/tunnel path) or a bare
/// `port` (the local same-host path); the page builds the WebSocket URL.
#[derive(Default)]
struct DashboardLinks<'a> {
    serial_ws: Option<&'a str>,
    serial_port: Option<u16>,
    interface: Option<&'a str>,
    hid_ws: Option<&'a str>,
    hid_port: Option<u16>,
}

fn dashboard_url(video_base: &str, links: &DashboardLinks) -> String {
    let mut params: Vec<String> = Vec::new();
    if let Some(ws) = links.serial_ws {
        params.push(format!("serialws={ws}"));
    } else if let Some(port) = links.serial_port {
        params.push(format!("serial={port}"));
    }
    if let Some(i) = links.interface {
        params.push(format!("interface={i}"));
    }
    if let Some(ws) = links.hid_ws {
        params.push(format!("hidws={ws}"));
    } else if let Some(port) = links.hid_port {
        params.push(format!("hid={port}"));
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
            daemons::wait_for_daemon(video::DAEMON, std::time::Duration::from_secs(5)).ok_or_else(
                || daemons::start_failure(video::DAEMON, std::time::Duration::from_secs(5)),
            )?
        }
    };
    if serial::daemon_url().is_none() {
        let serials = local_serials(&lab, &target)?;
        if serials.is_empty() {
            bail!("no serial interfaces configured for '{target}' (paniolo serial add ...)");
        }
        eprintln!("Starting serial daemon…");
        serial::start_daemon(&serials, 0)?;
        daemons::wait_for_daemon(serial::DAEMON, std::time::Duration::from_secs(5)).ok_or_else(
            || daemons::start_failure(serial::DAEMON, std::time::Duration::from_secs(5)),
        )?;
    }
    // Optional KVM leg: if the target has a local hid channel, ensure its
    // daemon and hand the dashboard its port (?hid=PORT). Absent/remote hid
    // channels just leave the console without input injection.
    let hid_port = ensure_hid_daemon_local(&lab, &target).unwrap_or_else(|e| {
        eprintln!("hid (KVM) disabled: {e}");
        None
    });

    // The dashboard's panes can't discover the daemons' OS-assigned ports
    // themselves — hand them over as ?serial=PORT / ?hid=PORT.
    let serial_port = daemons::daemon_port(serial::DAEMON);
    let url = dashboard_url(
        &video_url,
        &DashboardLinks {
            serial_port,
            interface,
            hid_port,
            ..Default::default()
        },
    );
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

    // Optional KVM leg: start the hid daemon on the host if its channel lives
    // there too, then tunnel its port. A KVM-less console still works.
    let hid_on_host = lab
        .resolved_target(target)
        .map(|rt| model::channel_host(&rt, model::ChannelKind::Hid, None).ok())
        .unwrap_or(None)
        .as_deref()
        == Some(host_name)
        && lab
            .targets
            .get(target)
            .and_then(|t| t.hid.as_ref())
            .is_some();
    let mut _fwd_hid = None;
    let hid_ws: Option<String> = if hid_on_host {
        match dispatch::run_subcommand(lab, target, host_name, &["hid", "serve", target]) {
            Ok(out) if out.status == 0 => dispatch::remote_daemon_port(&host, HID_DAEMON)
                .and_then(|p| ssh::forward(&host, p).ok())
                .map(|fwd| {
                    let url = format!("ws://127.0.0.1:{}/hid", fwd.local_port);
                    _fwd_hid = Some(fwd);
                    url
                }),
            _ => {
                eprintln!("hid (KVM) disabled: could not start the hid daemon on {host_name}");
                None
            }
        }
    } else {
        None
    };

    let url = dashboard_url(
        &format!("http://127.0.0.1:{}", fwd_video.local_port),
        &DashboardLinks {
            serial_ws: Some(&format!("ws://127.0.0.1:{}/stream", fwd_serial.local_port)),
            interface,
            hid_ws: hid_ws.as_deref(),
            ..Default::default()
        },
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

/// Run an opaque shell hook via `sh -c`, propagating its exit code. The
/// libexec dir is prepended to PATH so lab files can name helpers bare.
/// `label` is a human-readable description shown in the progress message.
fn run_power_hook(cmd: &str, label: &str, target: &str) -> Result<()> {
    eprintln!("{label} '{target}' via {cmd}");
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .env("PATH", daemons::hook_path())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        eprintln!(
            "{label} script exited with code {}",
            status.code().unwrap_or(1)
        );
        std::process::exit(status.code().unwrap_or(1));
    }
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
    run_power_hook(&cmd, "Power cycling", &target)?;
    println!("Power cycle complete.");
    Ok(())
}

fn cmd_power_on(lab_flag: Option<&str>, target: Option<&str>) -> Result<()> {
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
    let cmd = p.on_cmd.ok_or_else(|| {
        anyhow!(
            "no on_cmd configured for '{target}' \
             (paniolo power set -t {target} --on-cmd /path/to/script)"
        )
    })?;
    run_power_hook(&cmd, "Powering on", &target)?;
    println!("Power on complete.");
    Ok(())
}

fn cmd_power_off(lab_flag: Option<&str>, target: Option<&str>) -> Result<()> {
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
    let cmd = p.off_cmd.ok_or_else(|| {
        anyhow!(
            "no off_cmd configured for '{target}' \
             (paniolo power set -t {target} --off-cmd /path/to/script)"
        )
    })?;
    run_power_hook(&cmd, "Powering off", &target)?;
    println!("Power off complete.");
    Ok(())
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

    // Prefer state_cmd when configured; fall back to serial sense.
    if let Some(cmd) = p.state_cmd {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .env("PATH", daemons::hook_path())
            .output()?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            bail!(
                "state_cmd '{cmd}' exited with code {} — stdout: {stdout} stderr: {stderr}",
                out.status.code().unwrap_or(1)
            );
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let token = text
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match token.as_str() {
            "on" => {
                println!("Power ON  ({target})");
                Ok(())
            }
            "off" => {
                println!("Power OFF  ({target})");
                Ok(())
            }
            _ => bail!("state_cmd '{cmd}' output did not begin with 'on' or 'off' — got: {text}"),
        }
    } else {
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
        None => Err(daemons::start_failure(
            serial::DAEMON,
            std::time::Duration::from_secs(5),
        )),
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
                None => Err(daemons::start_failure(
                    video::DAEMON,
                    std::time::Duration::from_secs(5),
                )),
            }
        }
        VideoCmd::Stop { target } => {
            // With a target, route to its video channel's host (no-op locally).
            if let Some(t) = target.as_deref() {
                let _ = video_runtime(lab_flag, Some(t))?;
            }
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
        VideoCmd::Read {
            target,
            stable,
            timeout,
        } => {
            let _ = video_runtime(lab_flag, target.as_deref())?;
            let text = video::ocr(stable, timeout)?;
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
            Ok(())
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
            on_cmd,
            off_cmd,
            state_cmd,
            serial_interface,
            host,
        } => {
            edit_lab(lab_flag, |lf| {
                lf.set_power(
                    &target,
                    cycle_cmd.as_deref(),
                    on_cmd.as_deref(),
                    off_cmd.as_deref(),
                    state_cmd.as_deref(),
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
        PowerCmd::On { target } => cmd_power_on(lab_flag, target.as_deref()),
        PowerCmd::Off { target } => cmd_power_off(lab_flag, target.as_deref()),
    }
}

fn hid_cmd(lab_flag: Option<&str>, cmd: HidCmd) -> Result<()> {
    match cmd {
        HidCmd::Set { target, cmd, host } => {
            edit_lab(lab_flag, |lf| {
                lf.set_hid(&target, Some(&cmd), host.as_deref())
            })?;
            println!("hid channel set for '{target}'.");
            Ok(())
        }
        HidCmd::Rm { target } => {
            edit_lab(lab_flag, |lf| lf.remove_hid(&target))?;
            println!("hid channel removed from '{target}'.");
            Ok(())
        }
        HidCmd::Send { target, args } => cmd_hid_send(lab_flag, target.as_deref(), &args),
        HidCmd::Serve { target } => cmd_hid_serve(lab_flag, target.as_deref()),
        HidCmd::Stop { target } => cmd_hid_stop(lab_flag, target.as_deref()),
    }
}

/// The `hid` daemon discovery name (must match hidrig's DISCOVERY_NAME).
const HID_DAEMON: &str = "hid";

/// Ensure the hid injection daemon is running locally for `target`, returning
/// its port, or None when the target has no local hid channel. The helper's
/// `cmd` is run as `<cmd> serve --port 0` via `sh -c`; the contract is that it
/// daemonizes and publishes `/tmp/paniolo-<uid>/hid/daemon.json`.
fn ensure_hid_daemon_local(lab: &Lab, target: &str) -> Result<Option<u16>> {
    let t = lab
        .targets
        .get(target)
        .ok_or_else(|| anyhow!("target '{target}' not found in lab"))?;
    let dh = t.default_host().to_string();
    let h = match &t.hid {
        Some(h) => h,
        None => return Ok(None),
    };
    if h.host.as_deref().unwrap_or(&dh) != model::LOCAL {
        return Ok(None);
    }
    if let Some(port) = daemons::daemon_port(HID_DAEMON) {
        return Ok(Some(port));
    }
    let cmd = h.cmd.clone().ok_or_else(|| {
        anyhow!("hid channel for '{target}' has no cmd (paniolo hid set -t {target} --cmd ...)")
    })?;
    eprintln!("Starting hid daemon…");
    let log = std::fs::File::create(daemons::ensure_runtime_dir(HID_DAEMON)?.join("daemon.log"))?;
    let mut command = std::process::Command::new("sh");
    command
        .arg("-c")
        .arg(format!("exec {cmd} serve --port 0"))
        .env("PATH", daemons::hook_path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(log);
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.spawn()?;
    Ok(Some(
        daemons::wait_for_daemon(HID_DAEMON, std::time::Duration::from_secs(5))
            .and_then(|_| daemons::daemon_port(HID_DAEMON))
            .ok_or_else(|| daemons::start_failure(HID_DAEMON, std::time::Duration::from_secs(5)))?,
    ))
}

fn cmd_hid_serve(lab_flag: Option<&str>, target: Option<&str>) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    let target = resolve_single_target(&lab, target)?;
    if let Some(code) = dispatch::maybe_dispatch(
        &lab,
        &target,
        model::ChannelKind::Hid,
        None,
        dispatch::Mode::Reexec,
    )? {
        std::process::exit(code);
    }
    match ensure_hid_daemon_local(&lab, &target)? {
        Some(port) => {
            println!("hid daemon running for '{target}' (port {port}).");
            Ok(())
        }
        None => bail!("target '{target}' has no hid channel on this host"),
    }
}

fn cmd_hid_stop(lab_flag: Option<&str>, target: Option<&str>) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    let target = resolve_single_target(&lab, target)?;
    if let Some(code) = dispatch::maybe_dispatch(
        &lab,
        &target,
        model::ChannelKind::Hid,
        None,
        dispatch::Mode::Reexec,
    )? {
        std::process::exit(code);
    }
    let t = lab
        .targets
        .get(&target)
        .ok_or_else(|| anyhow!("target '{target}' not found in lab"))?;
    let cmd = t
        .hid
        .as_ref()
        .and_then(|h| h.cmd.clone())
        .ok_or_else(|| anyhow!("target '{target}' has no hid channel"))?;
    // The helper owns its own stop (e.g. `hidrig stop`); strip any trailing
    // device args isn't needed — `<cmd> stop` ignores extra args it doesn't use.
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{cmd} stop"))
        .env("PATH", daemons::hook_path())
        .status()?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Run the target's hid helper with `args` appended, propagating its exit code.
/// Paniolo is agnostic to the helper's CLI — the configured cmd owns it (see
/// docs/hid.md), exactly like the power hooks.
fn cmd_hid_send(lab_flag: Option<&str>, target: Option<&str>, args: &[String]) -> Result<()> {
    let lab = load_for_read(lab_flag)?;
    let target = resolve_single_target(&lab, target)?;
    if let Some(code) = dispatch::maybe_dispatch(
        &lab,
        &target,
        model::ChannelKind::Hid,
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
    let h = t.hid.clone().ok_or_else(|| {
        anyhow!("target '{target}' has no hid channel (paniolo hid set -t {target} --cmd ...)")
    })?;
    if h.host.as_deref().unwrap_or(&dh) != model::LOCAL {
        bail!("hid channel for '{target}' is not on this host");
    }
    let cmd = h.cmd.ok_or_else(|| {
        anyhow!(
            "no hid cmd configured for '{target}' \
             (paniolo hid set -t {target} --cmd 'hidrig -d /dev/...')"
        )
    })?;
    let quoted: Vec<String> = args.iter().map(|a| ssh::shell_quote(a)).collect();
    let full = format!("{cmd} {}", quoted.join(" "));
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&full)
        .env("PATH", daemons::hook_path())
        .status()?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
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
