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

//! hidrig — drive the KB2040 USB HID injector over its control UART.
//!
//! The injector's built-in USB port is a device-mode HID keyboard + mouse
//! plugged into the target machine; this tool talks the board's line-based
//! text protocol over the TX/RX UART (via a USB-serial adapter).
//!
//! Subcommands mirror the firmware protocol one-to-one (type, key, combo,
//! down, up, releaseall, move, click, mdown, mup, scroll, ping), plus `run`
//! for host-side command files with `delay`/`sleep` directives.

mod proto;

use std::io::Read;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};

use proto::{open_port, parse_sequence, send_command, Step};

#[derive(Parser)]
#[command(
    name = "hidrig",
    version,
    about = "KB2040 USB HID injector: keyboard/mouse injection over a control UART"
)]
struct Cli {
    /// Injector control UART (the USB-serial adapter, e.g. /dev/cu.usbserial-XXXX). Required.
    // Option<String> because clap does not permit global required args; main()
    // validates presence before dispatch.
    #[arg(short = 'd', long = "device", value_name = "DEVICE", global = true)]
    device: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Type a string of text.
    Type {
        /// Text to type (multiple words are joined with single spaces).
        #[arg(required = true)]
        text: Vec<String>,
    },
    /// Tap (press then release) a key, e.g. `key ENTER`.
    Key {
        /// adafruit_hid Keycode name (A-Z, ENTER, TAB, LEFT_CONTROL, F1..F12, ...).
        name: String,
    },
    /// Chord: press all the named keys, then release all, e.g. `combo LEFT_CONTROL C`.
    Combo {
        /// Keycode names, pressed together.
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Press and hold a key.
    Down { name: String },
    /// Release a held key.
    Up { name: String },
    /// Release all held keys.
    Releaseall,
    /// Relative mouse move in pixels (negative values allowed).
    Move {
        #[arg(allow_hyphen_values = true)]
        dx: i32,
        #[arg(allow_hyphen_values = true)]
        dy: i32,
    },
    /// Click a mouse button.
    Click {
        /// left | right | middle.
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
    /// No-op health check: confirms the board is powered and replying.
    Ping,
    /// Print the board's protocol version and implementation id.
    Version,
    /// Run a command file: one protocol command per line; blank lines and
    /// `# comments` are skipped; `delay <ms>` / `sleep <seconds>` pause.
    Run {
        /// Path to the command file, or `-` for stdin.
        file: String,
        /// Extra delay in milliseconds after every command.
        #[arg(long, default_value_t = 0)]
        delay_ms: u64,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let device = cli
        .device
        .as_deref()
        .ok_or_else(|| anyhow!("required argument '--device <DEVICE>' (-d) was not provided"))?;
    let mut port = open_port(device)?;

    match cli.cmd {
        Cmd::Type { text } => one(&mut port, &format!("type {}", text.join(" "))),
        Cmd::Key { name } => one(&mut port, &format!("key {name}")),
        Cmd::Combo { names } => one(&mut port, &format!("combo {}", names.join(" "))),
        Cmd::Down { name } => one(&mut port, &format!("down {name}")),
        Cmd::Up { name } => one(&mut port, &format!("up {name}")),
        Cmd::Releaseall => one(&mut port, "releaseall"),
        Cmd::Move { dx, dy } => one(&mut port, &format!("move {dx} {dy}")),
        Cmd::Click { button } => one(&mut port, &format!("click {button}")),
        Cmd::Mdown { button } => one(&mut port, &format!("mdown {button}")),
        Cmd::Mup { button } => one(&mut port, &format!("mup {button}")),
        Cmd::Scroll { amount } => one(&mut port, &format!("scroll {amount}")),
        Cmd::Ping => one(&mut port, "ping"),
        Cmd::Version => {
            let reply = send_command(&mut port, "version")?;
            println!("{reply}");
            Ok(())
        }
        Cmd::Run { file, delay_ms } => cmd_run(&mut port, &file, delay_ms),
    }
}

/// Send a single command and acknowledge with `OK` on stdout.
fn one(port: &mut Box<dyn serialport::SerialPort>, cmd: &str) -> Result<()> {
    send_command(port, cmd)?;
    println!("OK");
    Ok(())
}

fn cmd_run(port: &mut Box<dyn serialport::SerialPort>, file: &str, delay_ms: u64) -> Result<()> {
    let text = if file == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| anyhow!("reading stdin: {e}"))?;
        s
    } else {
        std::fs::read_to_string(file).map_err(|e| anyhow!("{file}: {e}"))?
    };
    let steps = parse_sequence(&text)?;
    let mut sent = 0usize;
    for step in steps {
        match step {
            Step::Delay(secs) => thread::sleep(Duration::from_secs_f64(secs)),
            Step::Cmd(cmd) => {
                send_command(port, &cmd)?;
                sent += 1;
                if delay_ms > 0 {
                    thread::sleep(Duration::from_millis(delay_ms));
                }
            }
        }
    }
    println!("OK ({sent} commands)");
    Ok(())
}
