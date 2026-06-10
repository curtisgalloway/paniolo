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

//! ch9329 — drive a WCH CH9329 USB-HID bridge (as in the Openterface Mini-KVM)
//! over its control UART, speaking paniolo's HID serial protocol.
//!
//! This is a sibling of `hidrig` (the KB2040 injector client): same CLI surface
//! — `type`, `key`, `combo`, `down`, `up`, `releaseall`, `move`, `moveabs`,
//! `click`, `mdown`, `mup`, `scroll`, `ping`, `version`, `run` — so it drops
//! into a paniolo `hid` channel (`paniolo hid set --cmd "ch9329 -d <uart>"`).
//! Unlike hidrig there is no microcontroller running firmware: the CH9329 chip
//! is itself the USB HID device, so this client speaks the chip's binary frame
//! protocol directly (see `session.rs`). The `serve`/`stop` daemon path (the
//! `paniolo console` KVM) is a later milestone.

mod keys;
mod proto;
mod session;

use std::io::Read;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};

use proto::{execute_line, parse_sequence, Step};
use session::Session;

#[derive(Parser)]
#[command(
    name = "ch9329",
    version,
    about = "WCH CH9329 USB-HID bridge (Openterface Mini-KVM): keyboard/mouse injection over a control UART"
)]
struct Cli {
    /// CH9329 control UART (the CH340 USB-serial adapter, e.g.
    /// /dev/cu.usbserial-XXXX). Required.
    #[arg(short = 'd', long = "device", value_name = "DEVICE", global = true)]
    device: Option<String>,

    /// Force the serial baud rate (default: autodetect 115200 then 9600).
    #[arg(short = 'b', long = "baud", global = true)]
    baud: Option<u32>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Type a string of text (US layout).
    Type {
        #[arg(required = true)]
        text: Vec<String>,
    },
    /// Tap (press then release) a key, e.g. `key ENTER`.
    Key {
        /// adafruit_hid Keycode name (A-Z, ENTER, TAB, LEFT_CONTROL, F1..F12, ...).
        name: String,
    },
    /// Chord: press all named keys, then release all, e.g. `combo LEFT_CONTROL C`.
    Combo {
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Press and hold a key.
    Down { name: String },
    /// Release a held key.
    Up { name: String },
    /// Release all held keys.
    Releaseall,
    /// Relative mouse move (negative values allowed).
    Move {
        #[arg(allow_hyphen_values = true)]
        dx: i32,
        #[arg(allow_hyphen_values = true)]
        dy: i32,
    },
    /// Absolute mouse move in a 0..32767 logical space (the host OS maps the
    /// range across the screen).
    Moveabs { x: i32, y: i32 },
    /// Click a mouse button.
    Click {
        #[arg(default_value = "left")]
        button: String,
    },
    /// Press and hold a mouse button.
    Mdown {
        #[arg(default_value = "left")]
        button: String,
    },
    /// Release a held mouse button.
    Mup {
        #[arg(default_value = "left")]
        button: String,
    },
    /// Scroll the wheel (positive = up, negative = down).
    Scroll {
        #[arg(allow_hyphen_values = true)]
        amount: i32,
    },
    /// No-op health check: confirms the chip is powered and replying (GET_INFO).
    Ping,
    /// Print protocol version, implementation id, and capabilities.
    Version,
    /// Print CH9329 chip status (GET_INFO): firmware version, whether the
    /// target has enumerated the emulated HID, lock-LED state, and link baud.
    /// CH9329-specific (not part of the HID serial protocol).
    Info,
    /// Run a command file: one protocol command per line; blank lines and
    /// `# comments` are skipped; `delay <ms>` / `sleep <seconds>` pause.
    Run {
        /// Path to the command file, or `-` for stdin.
        file: String,
        /// Extra delay in milliseconds after every command.
        #[arg(long, default_value_t = 0)]
        delay_ms: u64,
    },
    /// (Not yet implemented) Run the KVM daemon for `paniolo console`.
    Serve {
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    /// (Not yet implemented) Stop a running daemon.
    Stop,
    /// (Not yet implemented) Renegotiate the serial link baud.
    Baud { rate: u32 },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Daemon/baud paths are a later milestone; fail clearly rather than silently.
    if let Cmd::Serve { .. } | Cmd::Stop | Cmd::Baud { .. } = cli.cmd {
        return Err(anyhow!(
            "this CH9329 helper implements one-shot injection only; the \
             serve/stop/baud (KVM daemon) path is not yet built"
        ));
    }

    let device = require_device(&cli)?;
    let mut s = Session::open(device, cli.baud)?;

    match cli.cmd {
        Cmd::Type { text } => one(&mut s, &format!("type {}", text.join(" "))),
        Cmd::Key { name } => one(&mut s, &format!("key {name}")),
        Cmd::Combo { names } => one(&mut s, &format!("combo {}", names.join(" "))),
        Cmd::Down { name } => one(&mut s, &format!("down {name}")),
        Cmd::Up { name } => one(&mut s, &format!("up {name}")),
        Cmd::Releaseall => one(&mut s, "releaseall"),
        Cmd::Move { dx, dy } => one(&mut s, &format!("move {dx} {dy}")),
        Cmd::Moveabs { x, y } => one(
            &mut s,
            &format!("moveabs {} {}", proto::clamp_abs(x), proto::clamp_abs(y)),
        ),
        Cmd::Click { button } => one(&mut s, &format!("click {button}")),
        Cmd::Mdown { button } => one(&mut s, &format!("mdown {button}")),
        Cmd::Mup { button } => one(&mut s, &format!("mup {button}")),
        Cmd::Scroll { amount } => one(&mut s, &format!("scroll {amount}")),
        Cmd::Ping => one(&mut s, "ping"),
        Cmd::Version => {
            let reply = execute_line(&mut s, "version")?;
            println!("{reply}");
            Ok(())
        }
        Cmd::Info => {
            let info = s.get_info()?;
            println!(
                "chip_version={:#04x} target_connected={} num_lock={} caps_lock={} \
                 scroll_lock={} baud={}",
                info.chip_version,
                info.target_connected,
                info.num_lock,
                info.caps_lock,
                info.scroll_lock,
                s.baud(),
            );
            Ok(())
        }
        Cmd::Run { file, delay_ms } => cmd_run(&mut s, &file, delay_ms),
        Cmd::Serve { .. } | Cmd::Stop | Cmd::Baud { .. } => unreachable!("handled above"),
    }
}

fn require_device(cli: &Cli) -> Result<&str> {
    cli.device
        .as_deref()
        .ok_or_else(|| anyhow!("required argument '--device <DEVICE>' (-d) was not provided"))
}

/// Execute one command line and acknowledge with `OK` on stdout.
fn one(s: &mut Session, line: &str) -> Result<()> {
    execute_line(s, line)?;
    println!("OK");
    Ok(())
}

fn cmd_run(s: &mut Session, file: &str, delay_ms: u64) -> Result<()> {
    let text = if file == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| anyhow!("reading stdin: {e}"))?;
        buf
    } else {
        std::fs::read_to_string(file).map_err(|e| anyhow!("{file}: {e}"))?
    };
    let steps = parse_sequence(&text)?;
    let mut sent = 0usize;
    for step in steps {
        match step {
            Step::Delay(secs) => sleep(Duration::from_secs_f64(secs)),
            Step::Cmd(cmd) => {
                execute_line(s, &cmd)?;
                sent += 1;
                if delay_ms > 0 {
                    sleep(Duration::from_millis(delay_ms));
                }
            }
        }
    }
    println!("OK ({sent} commands)");
    Ok(())
}
