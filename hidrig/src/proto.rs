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

//! Transport to the dual-board rig's control board: a serial link carrying the
//! binary HID frames composed in [`crate::compose`]. The control board is a
//! USB-CDC device, so the "baud" is nominal (USB sets the real rate); there is
//! no baud negotiation and no ASCII reply protocol — HID frames are
//! fire-and-forget, and only `0x02` control frames (ping/version) draw a reply.
//!
//! This module also parses host-side command files (sequencing/timing lives
//! here; the firmware stays a dumb relay).

use anyhow::{anyhow, Result};
use serialport::SerialPort;
use std::io::{Read, Write};
use std::time::Duration;

use crate::compose::{Composer, F_CTRL};

/// Nominal open rate — a USB-CDC endpoint ignores it, but `serialport` requires
/// a value.
const NOMINAL_BAUD: u32 = 115_200;
/// How long to wait for a control-frame reply (ping/version).
const READ_TIMEOUT: Duration = Duration::from_millis(1_500);

/// macOS buffers serial reads behind a data-latency timer (`IOSSDATALAT`); drop
/// it to its floor so control-frame round trips are prompt. No-op off macOS.
#[cfg(target_os = "macos")]
fn set_low_read_latency(fd: std::os::unix::io::RawFd) {
    // _IOW('T', 0, c_ulong), per <IOKit/serial/ioss.h>.
    const IOSSDATALAT: libc::c_ulong = 0x8008_5400;
    let latency: libc::c_ulong = 1; // microseconds
    unsafe { libc::ioctl(fd, IOSSDATALAT, &latency) };
}

/// Open the control board's data CDC endpoint.
pub fn open_port(device: &str) -> Result<Box<dyn SerialPort>> {
    let port = serialport::new(device, NOMINAL_BAUD)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .timeout(READ_TIMEOUT)
        .open_native()
        .map_err(|e| anyhow!("cannot open {device}: {e}"))?;
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        set_low_read_latency(port.as_raw_fd());
    }
    Ok(Box::new(port))
}

/// Compose `line` into frames, write them to the control board, and return the
/// reply. HID frames are fire-and-forget so a clean write is the "OK" (empty
/// string); control frames (ping/version) draw a reply we read back.
pub fn run_command(
    composer: &mut Composer,
    port: &mut Box<dyn SerialPort>,
    line: &str,
) -> Result<String> {
    let frames = composer.dispatch(line)?;
    let mut wants_reply = false;
    for f in &frames {
        port.write_all(f).map_err(|e| anyhow!("write error: {e}"))?;
        if f.first() == Some(&F_CTRL) {
            wants_reply = true;
        }
    }
    port.flush().map_err(|e| anyhow!("write error: {e}"))?;
    if wants_reply {
        read_control_reply(port)
    } else {
        Ok(String::new())
    }
}

/// Read one `[0x02][cmd][len][payload]` control reply; return the payload text.
fn read_control_reply(port: &mut Box<dyn SerialPort>) -> Result<String> {
    let mut header = [0u8; 3];
    read_exact(port, &mut header)?;
    if header[0] != F_CTRL {
        return Err(anyhow!("unexpected reply frame type 0x{:02x}", header[0]));
    }
    let len = header[2] as usize;
    let mut payload = vec![0u8; len];
    if len > 0 {
        read_exact(port, &mut payload)?;
    }
    Ok(String::from_utf8_lossy(&payload).into_owned())
}

/// Fill `buf` from the port, treating a timeout or EOF as a failed reply.
fn read_exact(port: &mut Box<dyn SerialPort>, buf: &mut [u8]) -> Result<()> {
    let mut got = 0;
    while got < buf.len() {
        match port.read(&mut buf[got..]) {
            Ok(0) => return Err(anyhow!("no reply from the control board (port closed?)")),
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                return Err(anyhow!(
                    "timed out waiting for a control reply — is the control board powered \
                     and the data CDC endpoint wired?"
                ));
            }
            Err(e) => return Err(anyhow!("read error: {e}")),
        }
    }
    Ok(())
}

/// One step of a command file.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A protocol command line, composed and sent.
    Cmd(String),
    /// A pause, in seconds.
    Delay(f64),
}

/// Parse a command file into steps.
///
/// Each non-blank, non-`#`-comment line is either a command or a timing
/// directive: `delay <ms>` or `sleep <seconds>`. Trailing text after the
/// directive's value is ignored for directives; command lines pass through
/// verbatim (composition happens when they run).
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
}
