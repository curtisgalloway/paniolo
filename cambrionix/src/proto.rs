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

//! Cambrionix serial protocol: command framing, response parsing, and the
//! `state` table parser.

use anyhow::{anyhow, Result};
use serialport::SerialPort;
use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

/// A single row from the hub's `state` output.
#[derive(Debug, Clone, PartialEq)]
pub struct PortRow {
    /// Port number as reported by the hub (0 = host/system port).
    pub port: u8,
    /// Voltage in millivolts (hub reports volts×100; we store as-is: raw×10 mV).
    pub volts_raw: u16,
    /// Current in milliamps.
    pub milliamps: u16,
    /// Attach-state letter: 'A' attached, 'D' disconnected, 'P' host port.
    pub attach: char,
    /// Mode letter: 'C' charge, 'S' sync, 'O' off, 'I' idle, 'F' host-port flag.
    pub mode: char,
    /// Remaining raw columns (exactly as received).
    pub rest: String,
}

impl PortRow {
    /// Returns `true` when the port is considered **on** (mode is not `O`).
    pub fn is_on(&self) -> bool {
        !self.mode.eq_ignore_ascii_case(&'O')
    }
}

/// Parse the multi-line text returned by the `state` command into a Vec of
/// `PortRow`. Lines that do not match the expected format are silently skipped
/// (echo lines, prompts, blanks, etc.).
pub fn parse_state(output: &str) -> Vec<PortRow> {
    let mut rows = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(">>") || trimmed.starts_with("state") {
            continue;
        }
        if let Some(row) = parse_state_line(trimmed) {
            rows.push(row);
        }
    }
    rows
}

/// Parse a single state line of the form:
/// ` 0, 0531, 0000, P F -, 121, 6795, 0.00`
///
/// Returns `None` if the line doesn't match the expected shape.
fn parse_state_line(line: &str) -> Option<PortRow> {
    let mut cols = line.splitn(7, ',');

    let port_str = cols.next()?.trim();
    let volts_str = cols.next()?.trim();
    let ma_str = cols.next()?.trim();
    let flags_str = cols.next()?.trim();
    let rest_cols: Vec<&str> = cols.collect();

    let port: u8 = port_str.parse().ok()?;
    let volts_raw: u16 = volts_str.parse().ok()?;
    let milliamps: u16 = ma_str.parse().ok()?;

    // flags_str is three space-separated letters, e.g. "P F -" or "A C -"
    let mut flags = flags_str.split_whitespace();
    let attach = flags.next()?.chars().next()?;
    let mode = flags.next()?.chars().next()?;

    let rest = rest_cols.join(",");

    Some(PortRow {
        port,
        volts_raw,
        milliamps,
        attach,
        mode,
        rest,
    })
}

/// Open a serial port to the hub control UART.
///
/// Returns the port ready for command/response exchanges.  The caller should
/// sleep ~200 ms after calling this to let the hardware settle.
pub fn open_port(device: &str) -> Result<Box<dyn SerialPort>> {
    let port = serialport::new(device, 115_200)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .timeout(Duration::from_millis(1000))
        .open()
        .map_err(|e| anyhow!("cannot open {device}: {e}"))?;
    Ok(port)
}

/// Send a command to the hub and collect the response lines up to (but not
/// including) the `>>` prompt line.
///
/// The caller is responsible for clearing any stale input before calling here.
pub fn run_command(port: &mut Box<dyn SerialPort>, cmd: &str) -> Result<String> {
    // Send the command terminated with CR LF.
    let msg = format!("{cmd}\r\n");
    port.write_all(msg.as_bytes())
        .map_err(|e| anyhow!("write error: {e}"))?;

    // Read lines until we see the `>>` prompt or we time out.
    let mut reader = BufReader::new(&mut **port);
    let mut output = String::new();
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF (shouldn't happen on serial, but handle it)
            Ok(_) => {
                let trimmed = line.trim_end_matches(['\r', '\n']);
                if trimmed.starts_with(">>") {
                    break;
                }
                output.push_str(trimmed);
                output.push('\n');
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => break,
            Err(e) => return Err(anyhow!("read error: {e}")),
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
 0, 0531, 0000, P F -, 121, 6795, 0.00
 1, 0509, 0245, A C -, 489675, x, 168.67
 2, 0000, 0000, D I -, 0, x, 0.00
 3, 0000, 0000, D I -, 0, x, 0.00
 4, 0509, 0538, A C -, 85285, x, 54.47
";

    #[test]
    fn parse_all_five_rows() {
        let rows = parse_state(SAMPLE);
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn port0_is_host_row() {
        let rows = parse_state(SAMPLE);
        let r = &rows[0];
        assert_eq!(r.port, 0);
        assert_eq!(r.attach, 'P');
        assert_eq!(r.mode, 'F');
        assert_eq!(r.volts_raw, 531);
        assert_eq!(r.milliamps, 0);
    }

    #[test]
    fn port4_is_on() {
        let rows = parse_state(SAMPLE);
        let r = rows.iter().find(|r| r.port == 4).unwrap();
        assert_eq!(r.attach, 'A');
        assert_eq!(r.mode, 'C');
        assert!(r.is_on(), "port 4 mode C should be on");
        assert_eq!(r.volts_raw, 509);
        assert_eq!(r.milliamps, 538);
    }

    #[test]
    fn port2_idle_is_on() {
        // Idle ('I') is NOT off — only 'O' means off.
        let rows = parse_state(SAMPLE);
        let r = rows.iter().find(|r| r.port == 2).unwrap();
        assert_eq!(r.mode, 'I');
        assert!(r.is_on(), "port 2 mode I (idle) should be considered on");
    }

    #[test]
    fn port3_idle_is_on() {
        let rows = parse_state(SAMPLE);
        let r = rows.iter().find(|r| r.port == 3).unwrap();
        assert_eq!(r.mode, 'I');
        assert!(r.is_on());
    }

    #[test]
    fn off_mode_letter() {
        // A synthetic line with mode 'O' → is_on() == false.
        let row = parse_state_line(" 5, 0000, 0000, D O -, 0, x, 0.00").unwrap();
        assert_eq!(row.mode, 'O');
        assert!(!row.is_on(), "mode O should be off");
    }

    #[test]
    fn tolerates_echo_and_prompt_lines() {
        let noisy = "state\n\
 0, 0531, 0000, P F -, 121, 6795, 0.00\n\
>> \n\
\n\
some garbage line\n\
 1, 0509, 0245, A C -, 489675, x, 168.67\n";
        let rows = parse_state(noisy);
        // Only rows 0 and 1 should parse; "state", ">>", blank, and garbage skip.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].port, 0);
        assert_eq!(rows[1].port, 1);
    }

    #[test]
    fn port1_volts_and_current() {
        let rows = parse_state(SAMPLE);
        let r = rows.iter().find(|r| r.port == 1).unwrap();
        assert_eq!(r.volts_raw, 509);
        assert_eq!(r.milliamps, 245);
        assert!(r.is_on());
    }
}
