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
//! `click`, `mdown`, `mup`, `scroll`, `ping`, `version`, `run`, plus `serve`/
//! `stop` for the KVM daemon — so it drops into a paniolo `hid` channel
//! (`paniolo hid set --cmd "ch9329 -d <uart>"`). Unlike hidrig there is no
//! microcontroller running firmware: the CH9329 chip is itself the USB HID
//! device, so this client speaks the chip's binary frame protocol directly (see
//! `session.rs`). `serve` runs a daemon that owns the UART and re-exposes the
//! protocol over a WebSocket so the web console can stream events that intermix
//! with CLI injections; when a daemon is running for the same device, one-shots
//! route through it automatically.

mod daemon;
mod keys;
mod proto;
mod server;
mod session;
mod uart;

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
    /// Run the KVM daemon: own the UART and re-expose the protocol over a
    /// localhost WebSocket (the `paniolo console` path). Blocks until stopped.
    /// One-shots for the same device route through this daemon automatically.
    Serve {
        /// TCP port to listen on (0 = OS-assigned; the port is published in the
        /// discovery file paniolo reads).
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    /// Stop a running hid daemon (SIGTERM to the recorded pid).
    Stop,
    /// (Not yet implemented) Renegotiate the serial link baud.
    Baud { rate: u32 },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Commands that don't take the one-shot UART path.
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
        Cmd::Baud { .. } => {
            return Err(anyhow!(
                "baud renegotiation is not implemented for this CH9329 helper"
            ));
        }
        _ => {}
    }

    let device = require_device(&cli)?;
    let mut tx = Sender::open(device, cli.baud)?;

    match cli.cmd {
        Cmd::Type { text } => one(&mut tx, &format!("type {}", text.join(" "))),
        Cmd::Key { name } => one(&mut tx, &format!("key {name}")),
        Cmd::Combo { names } => one(&mut tx, &format!("combo {}", names.join(" "))),
        Cmd::Down { name } => one(&mut tx, &format!("down {name}")),
        Cmd::Up { name } => one(&mut tx, &format!("up {name}")),
        Cmd::Releaseall => one(&mut tx, "releaseall"),
        Cmd::Move { dx, dy } => one(&mut tx, &format!("move {dx} {dy}")),
        Cmd::Moveabs { x, y } => one(
            &mut tx,
            &format!("moveabs {} {}", proto::clamp_abs(x), proto::clamp_abs(y)),
        ),
        Cmd::Click { button } => one(&mut tx, &format!("click {button}")),
        Cmd::Mdown { button } => one(&mut tx, &format!("mdown {button}")),
        Cmd::Mup { button } => one(&mut tx, &format!("mup {button}")),
        Cmd::Scroll { amount } => one(&mut tx, &format!("scroll {amount}")),
        Cmd::Ping => one(&mut tx, "ping"),
        Cmd::Version => {
            println!("{}", tx.run_line("version")?);
            Ok(())
        }
        Cmd::Info => {
            println!("{}", tx.run_line("info")?);
            Ok(())
        }
        Cmd::Run { file, delay_ms } => cmd_run(&mut tx, &file, delay_ms),
        Cmd::Serve { .. } | Cmd::Stop | Cmd::Baud { .. } => unreachable!("handled above"),
    }
}

fn require_device(cli: &Cli) -> Result<&str> {
    cli.device
        .as_deref()
        .ok_or_else(|| anyhow!("required argument '--device <DEVICE>' (-d) was not provided"))
}

/// One command line, executed either through a running daemon or directly on
/// the UART, depending on what owns the device.
enum Sender {
    /// A hid daemon owns this device; route commands through its HTTP API.
    Daemon { base: String },
    /// No daemon for this device; we hold the UART ourselves.
    Direct { session: Session },
}

impl Sender {
    /// Choose the transport: if a hid daemon is running for `device`, route
    /// through it (it holds the port, so a direct open would fail anyway);
    /// otherwise open the UART directly.
    fn open(device: &str, baud: Option<u32>) -> Result<Sender> {
        if let Some(d) = daemon::discover() {
            if d.device == device {
                return Ok(Sender::Daemon {
                    base: format!("http://127.0.0.1:{}", d.port),
                });
            }
        }
        Ok(Sender::Direct {
            session: Session::open(device, baud)?,
        })
    }

    /// Execute one command line, returning the `OK` reply data (empty for a
    /// bare `OK`, the capability/info string for `version`/`info`).
    fn run_line(&mut self, line: &str) -> Result<String> {
        match self {
            Sender::Daemon { base } => post_send(base, line),
            Sender::Direct { session } => execute_line(session, line),
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

/// Execute a single command and acknowledge with `OK` on stdout.
fn one(tx: &mut Sender, line: &str) -> Result<()> {
    tx.run_line(line)?;
    println!("OK");
    Ok(())
}

fn cmd_run(tx: &mut Sender, file: &str, delay_ms: u64) -> Result<()> {
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
                tx.run_line(&cmd)?;
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
