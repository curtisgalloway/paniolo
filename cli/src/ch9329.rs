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

//! CH9329 serial-to-HID keyboard injection — the HID backend of the Openterface
//! Mini-KVM.
//!
//! Implemented from the WCH **CH9329 datasheet (DS1)** and **"CH9329 serial
//! communication protocol"** docs — see docs/ch9329-spec.md. Every frame is
//! `[0x57 0xAB, ADDR=0x00, CMD, LEN, ..data.., SUM]` where SUM is the 8-bit sum
//! of all preceding bytes; the chip's normal response CMD is `request | 0x80`,
//! an error response is `request | 0xC0` carrying a status byte. The chip powers
//! up at 9600 8N1 in protocol mode; 115200 is reached by writing the parameter
//! block (persists to flash, activates after a reset).

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

pub const DEFAULT_BAUD: u32 = 115200;

const HEAD: [u8; 2] = [0x57, 0xAB];
const ADDR: u8 = 0x00;
const MOD_SHIFT: u8 = 0x02;

// Command codes (host → chip).
const CMD_GET_INFO: u8 = 0x01;
const CMD_KEYBOARD: u8 = 0x02;
const CMD_GET_PARA_CFG: u8 = 0x08;
const CMD_SET_PARA_CFG: u8 = 0x09;
const CMD_SET_DEFAULT_CFG: u8 = 0x0C;
const CMD_RESET: u8 = 0x0F;

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u32, |a, &b| a + b as u32) as u8
}

/// Build a CH9329 command frame: HEAD, ADDR, CMD, LEN, DATA, SUM.
pub fn frame(cmd: u8, data: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(6 + data.len());
    f.extend_from_slice(&HEAD);
    f.push(ADDR);
    f.push(cmd);
    f.push(data.len() as u8);
    f.extend_from_slice(data);
    f.push(checksum(&f));
    f
}

/// A standard 8-byte USB boot-keyboard report wrapped as a CH9329 keyboard frame.
pub fn keyboard_frame(modifier: u8, keys: [u8; 6]) -> Vec<u8> {
    let mut report = [0u8; 8];
    report[0] = modifier;
    report[2..8].copy_from_slice(&keys);
    frame(CMD_KEYBOARD, &report)
}

/// Map a US-layout ASCII char to (modifier, HID usage id), or None if unsupported.
pub fn ascii_to_hid(c: char) -> Option<(u8, u8)> {
    let s = MOD_SHIFT;
    Some(match c {
        'a'..='z' => (0, 0x04 + (c as u8 - b'a')),
        'A'..='Z' => (s, 0x04 + (c as u8 - b'A')),
        '1'..='9' => (0, 0x1e + (c as u8 - b'1')),
        '0' => (0, 0x27),
        '\n' => (0, 0x28),
        '\t' => (0, 0x2b),
        ' ' => (0, 0x2c),
        '!' => (s, 0x1e),
        '@' => (s, 0x1f),
        '#' => (s, 0x20),
        '$' => (s, 0x21),
        '%' => (s, 0x22),
        '^' => (s, 0x23),
        '&' => (s, 0x24),
        '*' => (s, 0x25),
        '(' => (s, 0x26),
        ')' => (s, 0x27),
        '-' => (0, 0x2d),
        '_' => (s, 0x2d),
        '=' => (0, 0x2e),
        '+' => (s, 0x2e),
        '[' => (0, 0x2f),
        '{' => (s, 0x2f),
        ']' => (0, 0x30),
        '}' => (s, 0x30),
        '\\' => (0, 0x31),
        '|' => (s, 0x31),
        ';' => (0, 0x33),
        ':' => (s, 0x33),
        '\'' => (0, 0x34),
        '"' => (s, 0x34),
        '`' => (0, 0x35),
        '~' => (s, 0x35),
        ',' => (0, 0x36),
        '<' => (s, 0x36),
        '.' => (0, 0x37),
        '>' => (s, 0x37),
        '/' => (0, 0x38),
        '?' => (s, 0x38),
        _ => return None,
    })
}

/// Parse the first complete, checksum-valid frame in `buf`.
/// Returns (bytes_consumed, response_cmd, data).
fn parse_frame(buf: &[u8]) -> Option<(usize, u8, Vec<u8>)> {
    let mut i = 0;
    while i + 1 < buf.len() && !(buf[i] == HEAD[0] && buf[i + 1] == HEAD[1]) {
        i += 1;
    }
    if i + 5 > buf.len() {
        return None; // not enough for HEAD+ADDR+CMD+LEN
    }
    let len = buf[i + 4] as usize;
    let end = i + 6 + len; // through SUM
    if end > buf.len() {
        return None;
    }
    let sum = checksum(&buf[i..i + 5 + len]);
    if sum != buf[i + 5 + len] {
        return None;
    }
    Some((end, buf[i + 3], buf[i + 5..i + 5 + len].to_vec()))
}

fn status_name(code: u8) -> &'static str {
    match code {
        0x00 => "success",
        0xE1 => "serial timeout",
        0xE2 => "bad header",
        0xE3 => "unknown command",
        0xE4 => "checksum error",
        0xE5 => "parameter error",
        0xE6 => "operation failed",
        _ => "unknown status",
    }
}

type Port = Box<dyn serialport::SerialPort>;

fn open(device: &str, baud: u32) -> Result<Port> {
    serialport::new(device, baud)
        .timeout(Duration::from_millis(250))
        .open()
        .with_context(|| format!("opening CH9329 device {device} at {baud} baud"))
}

/// Max resends when the chip reports a checksum error or doesn't answer — a
/// marginal serial link (e.g. 115200 over the CH340) corrupts/drops the
/// occasional byte, and the cure for a garbled frame is simply to send it again.
const MAX_RESENDS: u32 = 4;

/// Send `cmd`+`data` and read the matching response (`cmd | 0x80`), returning its
/// data. Resends on a `0xE4` checksum-error response or a timeout (up to
/// `MAX_RESENDS`); any other error response (`cmd | 0xC0`) is surfaced.
fn transact(port: &mut Port, cmd: u8, data: &[u8], timeout_ms: u64) -> Result<Vec<u8>> {
    let req = frame(cmd, data);
    let mut attempts = 0u32;
    loop {
        port.write_all(&req)?;
        port.flush()?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut buf: Vec<u8> = Vec::new();
        let mut tmp = [0u8; 96];
        let outcome = loop {
            match port.read(&mut tmp) {
                Ok(n) if n > 0 => buf.extend_from_slice(&tmp[..n]),
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
                Err(e) => return Err(e.into()),
            }
            let mut retry = false;
            while let Some((end, rcmd, rdata)) = parse_frame(&buf) {
                if rcmd == cmd | 0x80 {
                    return Ok(rdata);
                }
                if rcmd == cmd | 0xC0 {
                    let code = rdata.first().copied().unwrap_or(0);
                    if code == 0xE4 {
                        retry = true; // checksum error — our frame was garbled; resend
                        break;
                    }
                    bail!(
                        "CH9329 cmd 0x{cmd:02x} error: status 0x{code:02x} ({})",
                        status_name(code)
                    );
                }
                buf.drain(..end); // unrelated/spontaneous frame — skip it
            }
            if retry {
                break true;
            }
            if Instant::now() > deadline {
                break true; // no answer — resend
            }
        };
        if outcome {
            attempts += 1;
            if attempts > MAX_RESENDS {
                bail!("CH9329: no valid response to cmd 0x{cmd:02x} after {attempts} attempts");
            }
        }
    }
}

/// Chip status from GET_INFO.
#[derive(Debug, Clone, Copy)]
pub struct Info {
    pub version: u8,
    pub target_connected: bool,
}

fn query_info(port: &mut Port) -> Result<Info> {
    let d = transact(port, CMD_GET_INFO, &[], 600)?;
    if d.len() < 2 {
        bail!("CH9329 GET_INFO response too short ({} bytes)", d.len());
    }
    Ok(Info {
        version: d[0],
        target_connected: d[1] == 0x01,
    })
}

/// Candidate baud order: the hint (if any) first, then 115200, then 9600.
fn candidate_bauds(prefer: Option<u32>) -> Vec<u32> {
    let mut bauds = Vec::new();
    if let Some(b) = prefer {
        bauds.push(b);
    }
    for b in [DEFAULT_BAUD, 9600] {
        if !bauds.contains(&b) {
            bauds.push(b);
        }
    }
    bauds
}

/// Find the chip: open at each candidate baud and GET_INFO until one answers.
fn detect(device: &str, prefer: Option<u32>) -> Result<(Port, u32, Info)> {
    let bauds = candidate_bauds(prefer);
    for &baud in &bauds {
        let Ok(mut port) = open(device, baud) else {
            continue;
        };
        std::thread::sleep(Duration::from_millis(150));
        let _ = port.clear(serialport::ClearBuffer::Input);
        if let Ok(info) = query_info(&mut port) {
            return Ok((port, baud, info));
        }
    }
    bail!(
        "CH9329 on {device} did not respond at {bauds:?}. Initialize it \
         (`paniolo hid init`), replug the device, or run the vendor app once."
    )
}

/// Type `text` into the target, gating on the chip ACK for every key frame.
pub fn type_string(device: &str, prefer_baud: Option<u32>, text: &str) -> Result<()> {
    let mut taps = Vec::new();
    for c in text.chars() {
        let (m, u) = ascii_to_hid(c)
            .ok_or_else(|| anyhow::anyhow!("unsupported character {c:?} in input"))?;
        taps.push((m, u));
    }
    let (mut port, baud, info) = detect(device, prefer_baud)?;
    if !info.target_connected {
        eprintln!(
            "warning: CH9329 reports the target USB is not enumerated — \
             key presses may not register"
        );
    }
    for (modifier, usage) in taps {
        send_key(&mut port, modifier, usage)?; // press
        send_key(&mut port, 0, 0)?; // release all
    }
    let _ = baud;
    Ok(())
}

fn send_key(port: &mut Port, modifier: u8, usage: u8) -> Result<()> {
    let report = [modifier, 0, usage, 0, 0, 0, 0, 0];
    let st = transact(port, CMD_KEYBOARD, &report, 400)?;
    match st.first().copied() {
        Some(0x00) => Ok(()),
        Some(code) => bail!(
            "CH9329 rejected key frame: 0x{code:02x} ({})",
            status_name(code)
        ),
        None => bail!("CH9329 keyboard ACK had no status byte"),
    }
}

/// Encode a baud rate into the parameter block's 4-byte big-endian field.
fn baud_be(baud: u32) -> [u8; 4] {
    baud.to_be_bytes()
}

/// Configure the chip to `target_baud` (default 115200) by editing its parameter
/// block, then reset and re-open at the new rate. Persists to chip flash.
pub fn configure_baud(device: &str, target_baud: u32) -> Result<()> {
    let (mut port, baud, _info) = detect(device, None)?;
    if baud == target_baud {
        println!("CH9329 already at {target_baud} baud.");
        return Ok(());
    }
    let mut cfg = transact(&mut port, CMD_GET_PARA_CFG, &[], 700)
        .context("reading CH9329 parameter block")?;
    if cfg.len() < 7 {
        bail!("CH9329 parameter block too short ({} bytes)", cfg.len());
    }
    cfg[0] = 0x00; // working mode: keyboard + mouse + custom HID
    cfg[1] = 0x00; // serial comm mode: protocol (required)
    cfg[3..7].copy_from_slice(&baud_be(target_baud));
    let st = transact(&mut port, CMD_SET_PARA_CFG, &cfg, 800)
        .context("writing CH9329 parameter block")?;
    if st.first().copied() != Some(0x00) {
        bail!(
            "CH9329 SET_PARA_CFG failed: 0x{:02x}",
            st.first().copied().unwrap_or(0xFF)
        );
    }
    // New config activates only after reset; the chip may reset before ACKing.
    let _ = transact(&mut port, CMD_RESET, &[], 400);
    drop(port);
    std::thread::sleep(Duration::from_millis(600));

    let (_p, got, info) = detect(device, Some(target_baud))?;
    if got != target_baud {
        bail!("after config, CH9329 is at {got} baud (expected {target_baud})");
    }
    println!(
        "CH9329 configured to {target_baud} baud (chip v0x{:02x}, target {}).",
        info.version,
        if info.target_connected {
            "connected"
        } else {
            "not connected"
        }
    );
    Ok(())
}

/// Restore factory defaults over serial (returns the chip to 9600), then reset.
pub fn restore_defaults(device: &str) -> Result<()> {
    let (mut port, _baud, _info) = detect(device, None)?;
    let st = transact(&mut port, CMD_SET_DEFAULT_CFG, &[], 800)?;
    if st.first().copied() != Some(0x00) {
        bail!(
            "CH9329 restore-defaults failed: 0x{:02x}",
            st.first().copied().unwrap_or(0xFF)
        );
    }
    let _ = transact(&mut port, CMD_RESET, &[], 400);
    println!("CH9329 restored to factory defaults (9600 baud).");
    Ok(())
}

/// Probe for `doctor`: report the chip's baud and target-connection, or an error.
pub fn probe(device: &str, prefer_baud: Option<u32>) -> Result<(u32, Info)> {
    let (_port, baud, info) = detect(device, prefer_baud)?;
    Ok((baud, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_info_frame_is_known_bytes() {
        assert_eq!(
            frame(CMD_GET_INFO, &[]),
            vec![0x57, 0xAB, 0x00, 0x01, 0x00, 0x03]
        );
    }

    #[test]
    fn keyboard_frame_matches_known_bytes() {
        let f = keyboard_frame(0, [0x04, 0, 0, 0, 0, 0]);
        assert_eq!(
            f,
            vec![0x57, 0xAB, 0x00, 0x02, 0x08, 0x00, 0x00, 0x04, 0, 0, 0, 0, 0, 0x10]
        );
    }

    #[test]
    fn ascii_maps_letters_digits_shifted() {
        assert_eq!(ascii_to_hid('a'), Some((0, 0x04)));
        assert_eq!(ascii_to_hid('A'), Some((MOD_SHIFT, 0x04)));
        assert_eq!(ascii_to_hid('1'), Some((0, 0x1e)));
        assert_eq!(ascii_to_hid('!'), Some((MOD_SHIFT, 0x1e)));
        assert_eq!(ascii_to_hid(' '), Some((0, 0x2c)));
        assert_eq!(ascii_to_hid('€'), None);
    }

    #[test]
    fn parse_frame_reads_get_info_response() {
        // 0x81 response: version 0x38, target_connected 0x01, then reserved.
        let raw = vec![
            0x57, 0xAB, 0x00, 0x81, 0x08, 0x38, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let sum = checksum(&raw);
        let mut full = raw.clone();
        full.push(sum);
        let (end, cmd, data) = parse_frame(&full).expect("valid frame");
        assert_eq!(end, full.len());
        assert_eq!(cmd, 0x81);
        assert_eq!(data[0], 0x38);
        assert_eq!(data[1], 0x01);
    }

    #[test]
    fn parse_frame_rejects_bad_checksum() {
        let bad = vec![0x57, 0xAB, 0x00, 0x82, 0x01, 0x00, 0xFF];
        assert!(parse_frame(&bad).is_none());
    }

    #[test]
    fn parse_frame_skips_leading_garbage() {
        let mut buf = vec![0x11, 0x22];
        buf.extend(frame(0x82, &[0x00]));
        let (_end, cmd, data) = parse_frame(&buf).expect("frame after garbage");
        assert_eq!(cmd, 0x82);
        assert_eq!(data, vec![0x00]);
    }

    #[test]
    fn baud_be_115200() {
        assert_eq!(baud_be(115200), [0x00, 0x01, 0xC2, 0x00]);
        assert_eq!(baud_be(9600), [0x00, 0x00, 0x25, 0x80]);
    }
}
