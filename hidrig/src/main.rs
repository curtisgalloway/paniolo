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

//! hidrig — drive the dual-board KB2040 USB-HID injector.
//!
//! hidrig owns HID *composition*: each subcommand (type, key, combo, down, up,
//! releaseall, move, moveabs, click, mdown, mup, scroll, ping, version) is
//! turned into HID report bytes (see [`compose`]) and wrapped in the binary
//! frames the control board relays over I2C to the target board, which injects
//! them as USB-HID into the DUT — the "dumb pipe" rig in `firmware/dual/`. The
//! control board is a USB-CDC device; hidrig writes frames to its data endpoint.
//!
//! `run` executes a command file. `serve` runs a daemon that owns the control
//! link, holds the composition state (held keys, virtual cursor), and re-exposes
//! the command protocol over a localhost WebSocket so the web console intermixes
//! with CLI injections; when a daemon is running for the same device, one-shots
//! route through it automatically.

mod compose;
mod daemon;
mod proto;
mod pty;
mod server;
mod uart;

use std::io::Read;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};

use compose::Composer;
use proto::{open_port, parse_sequence, run_command, Step};

#[derive(Parser)]
#[command(
    name = "hidrig",
    version,
    about = "Dual-board KB2040 USB-HID injector: keyboard/mouse via the control board's CDC link"
)]
struct Cli {
    /// Control board data CDC endpoint (e.g. /dev/cu.usbmodemXXXX). Required.
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
    /// Absolute mouse move in a 0..32767 logical space (requires the
    /// `moveabs` capability; the host OS maps the range across the screen).
    Moveabs {
        /// X in 0..32767 (clamped).
        x: i32,
        /// Y in 0..32767 (clamped).
        y: i32,
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
    /// Print the board's protocol version, implementation id, and capabilities.
    Version,
    /// Power the DUT off/on, or power-cycle it, via the control board's relay.
    Power {
        /// off | on | cycle
        action: String,
        /// For `cycle`: seconds to stay off before powering back on (the
        /// firmware default is used if omitted).
        secs: Option<u8>,
    },
    /// Run a command file: one protocol command per line; blank lines and
    /// `# comments` are skipped; `delay <ms>` / `sleep <seconds>` pause.
    Run {
        /// Path to the command file, or `-` for stdin.
        file: String,
        /// Extra delay in milliseconds after every command.
        #[arg(long, default_value_t = 0)]
        delay_ms: u64,
    },
    /// Run the daemon: own the UART and re-expose the protocol over a localhost
    /// WebSocket (the KVM path). Blocks until stopped. One-shot invocations for
    /// the same device route through this daemon automatically.
    Serve {
        /// TCP port to listen on (0 = OS-assigned; the port is published in the
        /// discovery file the paniolo console reads).
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    /// Stop a running hid daemon (SIGTERM to the recorded pid).
    Stop,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Commands that don't open the UART directly.
    match &cli.cmd {
        Cmd::Serve { port } => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "info".into()),
                )
                .init();
            let device = require_device(&cli)?;
            return daemon::run(device.to_string(), *port);
        }
        Cmd::Stop => return cmd_stop(),
        _ => {}
    }

    let device = require_device(&cli)?;
    let mut tx = Sender::open(device)?;

    match cli.cmd {
        Cmd::Type { text } => one(&mut tx, &format!("type {}", text.join(" "))),
        Cmd::Key { name } => one(&mut tx, &format!("key {name}")),
        Cmd::Combo { names } => one(&mut tx, &format!("combo {}", names.join(" "))),
        Cmd::Down { name } => one(&mut tx, &format!("down {name}")),
        Cmd::Up { name } => one(&mut tx, &format!("up {name}")),
        Cmd::Releaseall => one(&mut tx, "releaseall"),
        Cmd::Move { dx, dy } => one(&mut tx, &format!("move {dx} {dy}")),
        Cmd::Moveabs { x, y } => one(&mut tx, &format!("moveabs {x} {y}")),
        Cmd::Click { button } => one(&mut tx, &format!("click {button}")),
        Cmd::Mdown { button } => one(&mut tx, &format!("mdown {button}")),
        Cmd::Mup { button } => one(&mut tx, &format!("mup {button}")),
        Cmd::Scroll { amount } => one(&mut tx, &format!("scroll {amount}")),
        Cmd::Ping => one(&mut tx, "ping"),
        Cmd::Version => {
            let reply = tx.send("version")?;
            println!("{reply}");
            Ok(())
        }
        Cmd::Power { action, secs } => {
            let line = match secs {
                Some(s) => format!("power {action} {s}"),
                None => format!("power {action}"),
            };
            one(&mut tx, &line)
        }
        Cmd::Run { file, delay_ms } => cmd_run(&mut tx, &file, delay_ms),
        Cmd::Serve { .. } | Cmd::Stop => unreachable!("handled above"),
    }
}

fn require_device(cli: &Cli) -> Result<&str> {
    cli.device
        .as_deref()
        .ok_or_else(|| anyhow!("required argument '--device <DEVICE>' (-d) was not provided"))
}

/// One command line, sent either through a running daemon or directly to the
/// UART, depending on what owns the device.
enum Sender {
    /// A hid daemon owns this device; route commands through its HTTP API.
    Daemon { base: String },
    /// No daemon for this device; we hold the control link ourselves and
    /// compose frames in-process (held-key / cursor state lasts one process).
    Direct {
        composer: Composer,
        port: Box<dyn serialport::SerialPort>,
    },
}

impl Sender {
    /// Choose the transport: if a hid daemon is running for `device`, route
    /// through it (it holds the port, so a direct open would fail anyway);
    /// otherwise open the UART directly.
    fn open(device: &str) -> Result<Sender> {
        if let Some(d) = daemon::discover() {
            if d.device == device {
                return Ok(Sender::Daemon {
                    base: format!("http://127.0.0.1:{}", d.port),
                });
            }
        }
        Ok(Sender::Direct {
            composer: Composer::new(),
            port: open_port(device)?,
        })
    }

    /// Send one command line, returning the `OK` reply data (empty for a bare
    /// `OK`). Errors carry the board's `ERR` message or a transport failure.
    fn send(&mut self, line: &str) -> Result<String> {
        match self {
            Sender::Daemon { base } => post_send(base, line),
            Sender::Direct { composer, port } => run_command(composer, port, line),
        }
    }
}

/// POST one command line to a running daemon's `/send`; return the reply body.
fn post_send(base: &str, line: &str) -> Result<String> {
    match ureq::post(&format!("{base}/send"))
        .timeout(Duration::from_secs(15))
        .send_string(line)
    {
        Ok(resp) => Ok(resp.into_string().unwrap_or_default().trim().to_string()),
        // A 503 carries the board's ERR / transport message in the body.
        Err(ureq::Error::Status(_, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(anyhow!("{}", body.trim()))
        }
        Err(e) => Err(anyhow!("hid daemon /send failed: {e}")),
    }
}

/// Send a single command and acknowledge with `OK` on stdout.
fn one(tx: &mut Sender, cmd: &str) -> Result<()> {
    tx.send(cmd)?;
    println!("OK");
    Ok(())
}

fn cmd_run(tx: &mut Sender, file: &str, delay_ms: u64) -> Result<()> {
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
                tx.send(&cmd)?;
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

/// Stop a running hid daemon by sending SIGTERM to its recorded pid.
fn cmd_stop() -> Result<()> {
    match daemon::discover() {
        Some(d) => {
            // Safe: kill with SIGTERM to a pid we just confirmed alive.
            let rc = unsafe { libc::kill(d.pid as i32, libc::SIGTERM) };
            if rc == 0 {
                println!("hid daemon (pid {}) stopped", d.pid);
                Ok(())
            } else {
                Err(anyhow!("failed to signal hid daemon pid {}", d.pid))
            }
        }
        None => {
            println!("no hid daemon running");
            Ok(())
        }
    }
}
