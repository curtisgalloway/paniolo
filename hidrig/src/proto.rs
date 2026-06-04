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

/// Fixed by the firmware (`BAUD` in firmware/code.py).
pub const BAUD: u32 = 115_200;

/// Open the injector's control UART (via the USB-serial adapter).
///
/// The read timeout is generous because a long `type` or `move` executes a
/// HID report per step before the board replies.
pub fn open_port(device: &str) -> Result<Box<dyn SerialPort>> {
    let port = serialport::new(device, BAUD)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .timeout(Duration::from_millis(10_000))
        .open()
        .map_err(|e| anyhow!("cannot open {device}: {e}"))?;
    Ok(port)
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
}
