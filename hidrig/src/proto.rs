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

//! HID rig line protocol: one text command per line over the injector's
//! control UART; the board replies `OK` or `ERR <message>`. The board owns
//! the protocol — `hidrig/firmware/code.py` is the source of truth. This
//! module also parses host-side command files (the firmware stays dumb;
//! sequencing and timing live here).

use anyhow::{anyhow, Result};
use serialport::SerialPort;
use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

/// The firmware's boot default (`BAUD` in firmware/code.py). A naive connection
/// always works at this rate; the daemon may negotiate up from here.
pub const BAUD: u32 = 115_200;

/// The rate the daemon negotiates up to for KVM-streaming throughput (the
/// device must advertise the `baud` capability). Boots at [`BAUD`], so a
/// power-cycle re-syncs.
pub const FAST_BAUD: u32 = 460_800;

/// The absolute-pointer logical maximum (`moveabs` axis range is `0..=ABS_MAX`).
pub const ABS_MAX: i32 = 32_767;

/// Clamp a value into the `moveabs` logical range.
///
/// (Pixel→logical scaling for click-where-you-point lives in the dashboard,
/// which knows the rendered video rectangle; the firmware and this tool only
/// deal in the already-scaled logical range.)
pub fn clamp_abs(v: i32) -> i32 {
    v.clamp(0, ABS_MAX)
}

/// Generous read timeout for normal operation: a long `type` or `move`
/// executes a HID report per step before the board replies.
const READ_TIMEOUT: Duration = Duration::from_millis(10_000);
/// Short timeout for liveness probes during baud auto-detection.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// macOS buffers serial reads behind a data-latency timer (`IOSSDATALAT`) whose
/// default adds well over 100 ms to every request/reply round trip on FTDI
/// adapters — measured ~230 ms cmd→reply, the dominant HID-path latency. The
/// injector protocol sends one report per command for KVM streaming, so drop
/// the timer to its floor. No-op off macOS (the `serialport` crate does not set
/// this; Linux's ftdi_sio uses a low default and a `latency_timer` sysfs knob).
#[cfg(target_os = "macos")]
fn set_low_read_latency(fd: std::os::unix::io::RawFd) {
    // _IOW('T', 0, c_ulong), per <IOKit/serial/ioss.h>.
    const IOSSDATALAT: libc::c_ulong = 0x8008_5400;
    let latency: libc::c_ulong = 1; // microseconds
                                    // Best-effort: a failure just leaves the default (higher) latency in place.
    unsafe { libc::ioctl(fd, IOSSDATALAT, &latency) };
}

fn open_at(device: &str, baud: u32, timeout: Duration) -> Result<Box<dyn SerialPort>> {
    let port = serialport::new(device, baud)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .timeout(timeout)
        .open_native()
        .map_err(|e| anyhow!("cannot open {device}: {e}"))?;
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        set_low_read_latency(port.as_raw_fd());
    }
    Ok(Box::new(port))
}

/// Open the injector's control UART at the boot default [`BAUD`].
pub fn open_port(device: &str) -> Result<Box<dyn SerialPort>> {
    open_at(device, BAUD, READ_TIMEOUT)
}

/// Tell the device to switch to `rate`, then switch our port and confirm.
///
/// The device replies `OK` at the *current* rate and then switches; we wait for
/// it to switch, change our port, and `ping` to confirm. On any failure the
/// caller should treat the link as still at the previous rate.
pub fn negotiate_baud(port: &mut Box<dyn SerialPort>, rate: u32) -> Result<()> {
    send_command(port, &format!("baud {rate}"))?; // acked at the current rate
    std::thread::sleep(Duration::from_millis(120)); // let the device switch
    port.set_baud_rate(rate)
        .map_err(|e| anyhow!("set_baud_rate {rate}: {e}"))?;
    std::thread::sleep(Duration::from_millis(40));
    send_command(port, "ping").map(|_| ())
}

/// Open the UART and end up speaking the device's actual rate, negotiating up
/// to `fast` when possible. Tries the boot default first; if nothing answers
/// there, probes `fast` (a prior session may have left the device elevated and
/// not power-cycled). Returns the open port and the rate now in effect.
pub fn open_synced(device: &str, fast: u32) -> Result<(Box<dyn SerialPort>, u32)> {
    let mut port = open_at(device, BAUD, PROBE_TIMEOUT)?;
    if send_command(&mut port, "ping").is_ok() {
        port.set_timeout(READ_TIMEOUT).ok();
        if fast != BAUD && negotiate_baud(&mut port, fast).is_ok() {
            return Ok((port, fast));
        }
        port.set_baud_rate(BAUD).ok(); // negotiation reverts to the boot default
        return Ok((port, BAUD));
    }
    // Nothing at the boot default — maybe the device is already elevated.
    if fast != BAUD && port.set_baud_rate(fast).is_ok() && send_command(&mut port, "ping").is_ok() {
        port.set_timeout(READ_TIMEOUT).ok();
        return Ok((port, fast));
    }
    // Give up syncing; hand back a boot-default port and let the first real
    // command surface the error.
    port.set_baud_rate(BAUD).ok();
    port.set_timeout(READ_TIMEOUT).ok();
    Ok((port, BAUD))
}

/// Send one command line and return the board's reply with the `OK` prefix
/// stripped. Errors on `ERR <message>` replies and on timeout.
pub fn send_command(port: &mut Box<dyn SerialPort>, cmd: &str) -> Result<String> {
    if cmd.contains('\n') || cmd.contains('\r') {
        return Err(anyhow!("command contains a newline: {cmd:?}"));
    }
    let msg = format!("{cmd}\n");
    port.write_all(msg.as_bytes())
        .map_err(|e| anyhow!("write error: {e}"))?;

    let mut reader = BufReader::new(&mut **port);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => Err(anyhow!("no reply from the board (port closed?)")),
        Ok(_) => {
            let reply = line.trim_end_matches(['\r', '\n']);
            if let Some(rest) = reply.strip_prefix("OK") {
                Ok(rest.trim().to_string())
            } else if reply.starts_with("ERR") {
                Err(anyhow!("board rejected '{cmd}': {reply}"))
            } else {
                Err(anyhow!("unexpected reply to '{cmd}': {reply:?}"))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Err(anyhow!(
            "timed out waiting for a reply to '{cmd}' — is the injector powered \
             (target on) and the adapter wired TX<->RX?"
        )),
        Err(e) => Err(anyhow!("read error: {e}")),
    }
}

/// One step of a command file.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A protocol command line, sent verbatim.
    Cmd(String),
    /// A pause, in seconds.
    Delay(f64),
}

/// Parse a command file into steps.
///
/// Each non-blank, non-`#`-comment line is either a command or a timing
/// directive: `delay <ms>` or `sleep <seconds>`. Trailing text after the
/// directive's value (e.g. an inline comment) is ignored for directives;
/// command lines pass through verbatim.
pub fn parse_sequence(text: &str) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (head, rest) = line.split_once(' ').unwrap_or((line, ""));
        let value = rest.split_whitespace().next().unwrap_or("");
        match head.to_ascii_lowercase().as_str() {
            "delay" => {
                let ms: f64 = value
                    .parse()
                    .map_err(|_| anyhow!("invalid delay value: {rest:?}"))?;
                steps.push(Step::Delay(ms / 1000.0));
            }
            "sleep" => {
                let secs: f64 = value
                    .parse()
                    .map_err(|_| anyhow!("invalid sleep value: {rest:?}"))?;
                steps.push(Step::Delay(secs));
            }
            _ => steps.push(Step::Cmd(line.to_string())),
        }
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands_and_directives() {
        let steps = parse_sequence(
            "# boot sequence\n\
             type root\n\
             key ENTER\n\
             delay 500\n\
             \n\
             sleep 1.5\n\
             move 300 -50\n",
        )
        .unwrap();
        assert_eq!(
            steps,
            vec![
                Step::Cmd("type root".into()),
                Step::Cmd("key ENTER".into()),
                Step::Delay(0.5),
                Step::Delay(1.5),
                Step::Cmd("move 300 -50".into()),
            ]
        );
    }

    #[test]
    fn directive_values_tolerate_inline_comments() {
        let steps = parse_sequence("delay 250   # settle\n").unwrap();
        assert_eq!(steps, vec![Step::Delay(0.25)]);
    }

    #[test]
    fn command_lines_pass_through_verbatim() {
        // No inline-comment stripping on commands: `type` text may contain '#'.
        let steps = parse_sequence("type issue #42\n").unwrap();
        assert_eq!(steps, vec![Step::Cmd("type issue #42".into())]);
    }

    #[test]
    fn rejects_bad_delay() {
        assert!(parse_sequence("delay soon\n").is_err());
        assert!(parse_sequence("sleep\n").is_err());
    }

    #[test]
    fn directive_case_insensitive() {
        let steps = parse_sequence("DELAY 1000\nSleep 2\n").unwrap();
        assert_eq!(steps, vec![Step::Delay(1.0), Step::Delay(2.0)]);
    }

    #[test]
    fn moveabs_passes_through_as_a_command() {
        let steps = parse_sequence("moveabs 16000 8000\n").unwrap();
        assert_eq!(steps, vec![Step::Cmd("moveabs 16000 8000".into())]);
    }

    #[test]
    fn clamp_abs_bounds() {
        assert_eq!(clamp_abs(-1), 0);
        assert_eq!(clamp_abs(0), 0);
        assert_eq!(clamp_abs(ABS_MAX), ABS_MAX);
        assert_eq!(clamp_abs(ABS_MAX + 100), ABS_MAX);
    }
}
