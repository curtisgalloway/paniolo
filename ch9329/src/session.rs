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

//! CH9329 serial-to-HID protocol session.
//!
//! The WCH CH9329 bridges framed UART commands to a USB HID keyboard + mouse
//! presented to the *target*. Frames are
//! `HEAD(57 AB) ADDR(00) CMD LEN DATA SUM`, `SUM = Σ(all preceding) & 0xFF`;
//! the chip replies `CMD|0x80` (ok) or `CMD|0xC0` (error). See
//! `docs/ch9329-spec.md` for the clean-room protocol reference, restated from
//! the WCH datasheet. The framing, checksum, and GET_INFO paths here are
//! verified against real hardware (chip version 0x38 over a CH340 adapter).
//!
//! A [`Session`] also tracks the *held* report state (modifiers, key slots,
//! mouse buttons, last absolute position) so `combo`/`down`/`mdown` compose
//! within one process — the chip itself only remembers the last report it was
//! given.

use std::io::Write;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serialport::SerialPort;

use crate::keys::Key;

const HEAD: [u8; 2] = [0x57, 0xAB];
const ADDR: u8 = 0x00;

const CMD_GET_INFO: u8 = 0x01;
const CMD_KB_GENERAL: u8 = 0x02;
const CMD_MS_ABS: u8 = 0x04;
const CMD_MS_REL: u8 = 0x05;

/// CH9329 absolute coordinate full-scale (12-bit, in a 4096×4096 grid).
const ABS_FULL: i64 = 4096;
/// paniolo's `moveabs` logical maximum (`hidrig` ABS_MAX).
const LOGICAL_MAX: i64 = 32_767;

/// Baud rates tried, in order, when none is forced. Openterface units default
/// to 115200; a factory CH9329 is at 9600 (see `docs/ch9329-spec.md` §2).
const BAUD_CANDIDATES: [u32; 2] = [115_200, 9_600];

/// How long a key is held before release on a `tap`/`combo`.
const HOLD: Duration = Duration::from_millis(30);
/// How long a mouse button is held during a `click`. Much longer than a
/// keypress: the target's input layer must sample the button-down and
/// button-up as distinct events across the serial→USB→OS chain, and 12 ms was
/// too brief to register a click on a Raspberry Pi OS desktop.
const CLICK_HOLD: Duration = Duration::from_millis(80);
/// Settle delay after positioning the cursor before pressing a button. Lets
/// the target process the absolute-pointer motion first, so the button event
/// is attributed to the new location (without it, motion+press arrive together
/// and libinput drops the click).
const CLICK_SETTLE: Duration = Duration::from_millis(60);
/// Per-character hold/pacing for `type`.
const TYPE_GAP: Duration = Duration::from_millis(15);

fn button_mask(name: &str) -> Result<u8> {
    match name.to_ascii_lowercase().as_str() {
        "left" => Ok(0x01),
        "right" => Ok(0x02),
        "middle" => Ok(0x04),
        other => Err(anyhow!("unknown mouse button: {other}")),
    }
}

/// GET_INFO reply (`docs/ch9329-spec.md` §3).
#[derive(Debug, Clone, Copy)]
pub struct Info {
    pub chip_version: u8,
    pub target_connected: bool,
    pub num_lock: bool,
    pub caps_lock: bool,
    pub scroll_lock: bool,
}

pub struct Session {
    port: Box<dyn SerialPort>,
    baud: u32,
    // Held report state (the device only remembers its last report). Pointer
    // position is *not* tracked here: clicks go through the relative report
    // and land wherever the OS pointer currently is, set by the last move_abs.
    mods: u8,
    keys: Vec<u8>,
    buttons: u8,
}

impl Session {
    /// Open `device` and confirm the CH9329 answers. When `baud` is `None`,
    /// probe [`BAUD_CANDIDATES`] in order; otherwise use the given rate.
    pub fn open(device: &str, baud: Option<u32>) -> Result<Session> {
        let candidates: Vec<u32> = match baud {
            Some(b) => vec![b],
            None => BAUD_CANDIDATES.to_vec(),
        };
        let mut last_err: Option<anyhow::Error> = None;
        for rate in candidates {
            let port = serialport::new(device, rate)
                .data_bits(serialport::DataBits::Eight)
                .parity(serialport::Parity::None)
                .stop_bits(serialport::StopBits::One)
                .timeout(Duration::from_millis(500))
                .open_native()
                .with_context(|| format!("cannot open {device}"))?;
            #[cfg(target_os = "macos")]
            {
                use std::os::unix::io::AsRawFd;
                set_low_read_latency(port.as_raw_fd());
            }
            let mut s = Session {
                port: Box::new(port),
                baud: rate,
                mods: 0,
                keys: Vec::new(),
                buttons: 0,
            };
            match s.get_info() {
                Ok(_) => return Ok(s),
                Err(e) => last_err = Some(e),
            }
        }
        Err(anyhow!(
            "CH9329 did not respond on {device} at {} baud: {}",
            BAUD_CANDIDATES
                .iter()
                .map(|b| b.to_string())
                .collect::<Vec<_>>()
                .join("/"),
            last_err.map(|e| e.to_string()).unwrap_or_default(),
        ))
    }

    pub fn baud(&self) -> u32 {
        self.baud
    }

    // -- framing -------------------------------------------------------------

    fn frame(cmd: u8, data: &[u8]) -> Vec<u8> {
        let mut body = vec![HEAD[0], HEAD[1], ADDR, cmd, data.len() as u8];
        body.extend_from_slice(data);
        let sum = body.iter().fold(0u32, |a, &b| a + b as u32) as u8;
        body.push(sum);
        body
    }

    /// Send a framed command and return the reply payload (data bytes only).
    fn send(&mut self, cmd: u8, data: &[u8]) -> Result<Vec<u8>> {
        self.port.clear(serialport::ClearBuffer::Input).ok();
        let pkt = Self::frame(cmd, data);
        self.port
            .write_all(&pkt)
            .map_err(|e| anyhow!("serial write failed: {e}"))?;
        self.read_reply(cmd)
    }

    fn read_exact(&mut self, n: usize) -> Result<Vec<u8>> {
        use std::io::Read;
        let mut buf = vec![0u8; n];
        let mut filled = 0;
        while filled < n {
            match self.port.read(&mut buf[filled..]) {
                Ok(0) => bail!("serial port closed mid-reply"),
                Ok(k) => filled += k,
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    bail!("timed out waiting for CH9329 reply (check device/baud, target on)")
                }
                Err(e) => bail!("serial read failed: {e}"),
            }
        }
        Ok(buf)
    }

    fn read_reply(&mut self, cmd: u8) -> Result<Vec<u8>> {
        let head = self.read_exact(2)?;
        if head != HEAD {
            bail!("bad reply header {head:02x?} (expected 57 ab)");
        }
        let rest = self.read_exact(3)?;
        let (raddr, rcmd, len) = (rest[0], rest[1], rest[2] as usize);
        let payload = self.read_exact(len)?;
        let sum = self.read_exact(1)?[0];
        let expected = (HEAD[0] as u32
            + HEAD[1] as u32
            + raddr as u32
            + rcmd as u32
            + len as u32
            + payload.iter().map(|&b| b as u32).sum::<u32>()) as u8;
        if sum != expected {
            bail!("reply checksum mismatch (got {sum:#04x}, want {expected:#04x})");
        }
        if rcmd == cmd | 0xC0 {
            let status = payload.first().copied().unwrap_or(0xFF);
            bail!("CH9329 rejected cmd {cmd:#04x}: {}", status_name(status));
        }
        if rcmd != cmd | 0x80 {
            bail!("unexpected reply cmd {rcmd:#04x} to {cmd:#04x}");
        }
        // For non-GET_INFO commands the first payload byte is a status code.
        if cmd != CMD_GET_INFO {
            if let Some(&status) = payload.first() {
                if status != 0x00 {
                    bail!("cmd {cmd:#04x} failed: {}", status_name(status));
                }
            }
        }
        Ok(payload)
    }

    // -- status --------------------------------------------------------------

    pub fn get_info(&mut self) -> Result<Info> {
        let p = self.send(CMD_GET_INFO, &[])?;
        if p.len() < 3 {
            bail!("short GET_INFO reply: {p:02x?}");
        }
        Ok(Info {
            chip_version: p[0],
            target_connected: p[1] != 0,
            num_lock: p[2] & 0x01 != 0,
            caps_lock: p[2] & 0x02 != 0,
            scroll_lock: p[2] & 0x04 != 0,
        })
    }

    // -- keyboard ------------------------------------------------------------

    /// Push the current held report (`self.mods` + `self.keys`) to the chip.
    fn push_keyboard(&mut self) -> Result<()> {
        let mut data = vec![self.mods, 0x00];
        let mut slots = self.keys.clone();
        slots.resize(6, 0x00);
        data.extend_from_slice(&slots[..6]);
        self.send(CMD_KB_GENERAL, &data)?;
        Ok(())
    }

    /// Tap a key: add it to the held set, push, hold briefly, then restore the
    /// previously-held report. A modifier taps as a held bit (e.g. the GUI key).
    pub fn tap(&mut self, key: Key) -> Result<()> {
        self.apply_down(key);
        self.push_keyboard()?;
        sleep(HOLD);
        self.apply_up(key);
        self.push_keyboard()
    }

    /// Chord: press every key together, hold, then release back to held state.
    pub fn combo(&mut self, chord: &[Key]) -> Result<()> {
        for &k in chord {
            self.apply_down(k);
        }
        self.push_keyboard()?;
        sleep(HOLD);
        for &k in chord {
            self.apply_up(k);
        }
        self.push_keyboard()
    }

    pub fn key_down(&mut self, key: Key) -> Result<()> {
        self.apply_down(key);
        self.push_keyboard()
    }

    pub fn key_up(&mut self, key: Key) -> Result<()> {
        self.apply_up(key);
        self.push_keyboard()
    }

    pub fn release_all(&mut self) -> Result<()> {
        self.mods = 0;
        self.keys.clear();
        self.push_keyboard()
    }

    fn apply_down(&mut self, key: Key) {
        match key {
            Key::Modifier(bit) => self.mods |= bit,
            Key::Usage(u) => {
                if !self.keys.contains(&u) && self.keys.len() < 6 {
                    self.keys.push(u);
                }
            }
        }
    }

    fn apply_up(&mut self, key: Key) {
        match key {
            Key::Modifier(bit) => self.mods &= !bit,
            Key::Usage(u) => self.keys.retain(|&k| k != u),
        }
    }

    /// Type literal text (US layout) on top of any held modifiers.
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        let mut prev: u8 = 0;
        for c in text.chars() {
            let (usage, shift) = crate::keys::char_to_usage(c)?;
            if usage == prev {
                // Same key twice needs the release between presses to register.
                sleep(TYPE_GAP);
            }
            let mods = self.mods
                | if shift {
                    crate::keys::MOD_LEFT_SHIFT
                } else {
                    0
                };
            let mut data = vec![mods, 0x00, usage, 0, 0, 0, 0, 0];
            // Keep any already-held keys alongside the typed one.
            for (i, &k) in self.keys.iter().take(5).enumerate() {
                data[3 + i] = k;
            }
            self.send(CMD_KB_GENERAL, &data)?;
            sleep(TYPE_GAP);
            self.push_keyboard()?; // release the typed key, restore held state
            prev = usage;
        }
        Ok(())
    }

    // -- mouse ---------------------------------------------------------------

    fn push_mouse_abs(&mut self, x: u16, y: u16, buttons: u8) -> Result<()> {
        let data = [
            0x02,
            buttons,
            (x & 0xFF) as u8,
            (x >> 8) as u8,
            (y & 0xFF) as u8,
            (y >> 8) as u8,
            0x00,
        ];
        self.send(CMD_MS_ABS, &data)?;
        Ok(())
    }

    /// Absolute move; `x`/`y` are paniolo logical coords in `0..=32767`.
    ///
    /// The CH9329 absolute device coalesces a report whose coordinates equal
    /// its previous one. If the pointer was since moved by a relative report
    /// (a click, a `move`), re-sending the same absolute coordinate would be a
    /// no-op and the cursor would never snap to the target. So nudge one unit
    /// first, then send the exact target — the second report always differs
    /// from the first and lands the cursor precisely on `(x, y)`.
    pub fn move_abs(&mut self, x: i32, y: i32) -> Result<()> {
        let tx = scale_abs(x);
        let ty = scale_abs(y);
        let ny = if ty >= 1 { ty - 1 } else { ty + 1 };
        self.push_mouse_abs(tx, ny, self.buttons)?;
        self.push_mouse_abs(tx, ty, self.buttons)
    }

    pub fn click(&mut self, button: &str) -> Result<()> {
        let mask = button_mask(button)?;
        // Let any just-issued positioning move land first.
        sleep(CLICK_SETTLE);
        // Press/release via the RELATIVE report (zero motion). The absolute
        // report reliably positions the pointer, but a same-coordinate abs
        // button transition gets coalesced by libinput and never registers as
        // a click. A relative BTN report always processes, and clicks wherever
        // the pointer currently is — so a prior `moveabs` (even in a separate
        // process invocation) sets the spot.
        self.push_mouse_rel(self.buttons | mask, 0, 0, 0)?;
        sleep(CLICK_HOLD);
        self.push_mouse_rel(self.buttons, 0, 0, 0)
    }

    pub fn mouse_down(&mut self, button: &str) -> Result<()> {
        self.buttons |= button_mask(button)?;
        self.push_mouse_rel(self.buttons, 0, 0, 0)
    }

    pub fn mouse_up(&mut self, button: &str) -> Result<()> {
        self.buttons &= !button_mask(button)?;
        self.push_mouse_rel(self.buttons, 0, 0, 0)
    }

    fn push_mouse_rel(&mut self, buttons: u8, dx: i8, dy: i8, wheel: i8) -> Result<()> {
        let data = [0x01, buttons, dx as u8, dy as u8, wheel as u8];
        self.send(CMD_MS_REL, &data)?;
        Ok(())
    }

    /// Relative move, split into per-report int8 deltas.
    pub fn move_rel(&mut self, mut dx: i32, mut dy: i32) -> Result<()> {
        loop {
            let sx = dx.clamp(-127, 127);
            let sy = dy.clamp(-127, 127);
            if sx == 0 && sy == 0 {
                break;
            }
            self.push_mouse_rel(self.buttons, sx as i8, sy as i8, 0)?;
            dx -= sx;
            dy -= sy;
            if dx == 0 && dy == 0 {
                break;
            }
            sleep(Duration::from_millis(4));
        }
        Ok(())
    }

    /// Scroll the wheel; positive is up. Split into per-report int8 steps.
    pub fn scroll(&mut self, mut amount: i32) -> Result<()> {
        while amount != 0 {
            let step = amount.clamp(-127, 127);
            self.push_mouse_rel(self.buttons, 0, 0, step as i8)?;
            amount -= step;
            sleep(Duration::from_millis(4));
        }
        Ok(())
    }
}

/// Map a paniolo logical coordinate (`0..=32767`) to a CH9329 12-bit
/// coordinate (`0..=4095`), rounded.
fn scale_abs(v: i32) -> u16 {
    let v = v.clamp(0, LOGICAL_MAX as i32) as i64;
    let scaled = (v * ABS_FULL + LOGICAL_MAX / 2) / LOGICAL_MAX;
    scaled.min(ABS_FULL - 1) as u16
}

fn status_name(code: u8) -> String {
    match code {
        0x00 => "success".into(),
        0xE1 => "serial receive timeout (0xE1)".into(),
        0xE2 => "bad frame header (0xE2)".into(),
        0xE3 => "unknown command (0xE3)".into(),
        0xE4 => "checksum mismatch (0xE4)".into(),
        0xE5 => "bad parameter (0xE5)".into(),
        0xE6 => "execution failed (0xE6)".into(),
        other => format!("status {other:#04x}"),
    }
}

/// macOS buffers serial reads behind a data-latency timer (`IOSSDATALAT`)
/// whose default adds well over 100 ms per round trip; drop it to the floor
/// so per-report HID commands stay responsive. No-op elsewhere.
#[cfg(target_os = "macos")]
fn set_low_read_latency(fd: std::os::unix::io::RawFd) {
    const IOSSDATALAT: libc::c_ulong = 0x8008_5400;
    let latency: libc::c_ulong = 1; // microseconds
    unsafe { libc::ioctl(fd, IOSSDATALAT, &latency) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_checksum_matches_spec_example() {
        // docs/ch9329-spec.md §1: press 'A' (usage 0x04).
        // 57 AB 00 02 08 00 00 04 00 00 00 00 00 10
        let data = [0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00];
        let f = Session::frame(CMD_KB_GENERAL, &data);
        assert_eq!(
            f,
            vec![
                0x57, 0xAB, 0x00, 0x02, 0x08, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10
            ]
        );
    }

    #[test]
    fn get_info_frame() {
        // docs/ch9329-spec.md §3: request 57 AB 00 01 00 03.
        assert_eq!(
            Session::frame(CMD_GET_INFO, &[]),
            vec![0x57, 0xAB, 0x00, 0x01, 0x00, 0x03]
        );
    }

    #[test]
    fn abs_scaling_endpoints() {
        assert_eq!(scale_abs(0), 0);
        assert_eq!(scale_abs(32_767), 4095);
        assert_eq!(scale_abs(16_384), 2048); // midpoint
        assert_eq!(scale_abs(-5), 0); // clamped
        assert_eq!(scale_abs(40_000), 4095); // clamped
    }
}
