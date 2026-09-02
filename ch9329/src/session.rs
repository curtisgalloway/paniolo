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
const CMD_GET_PARA_CFG: u8 = 0x08;
const CMD_SET_PARA_CFG: u8 = 0x09;
const CMD_RESET: u8 = 0x0F;

/// Vendor extension implemented by Openterface MCUs that *emulate* the CH9329
/// (the KVM-Go's CH32V208), not by a real CH9329: drive or query the USB mux
/// that shares the onboard microSD reader between the host and the target.
/// See `notes/openterface-usb-mux-spec.md`.
const CMD_USB_SWITCH: u8 = 0x17;

/// `CMD_USB_SWITCH` payload length. Load-bearing: only the last of the five
/// bytes selects a direction, but the four leading zeros are part of the
/// declared payload. Trimming them shifts where the checksum lands and the
/// device rejects the frame outright.
const USB_SWITCH_LEN: usize = 5;

const USB_SWITCH_TO_HOST: u8 = 0x00;
const USB_SWITCH_TO_TARGET: u8 = 0x01;
const USB_SWITCH_QUERY: u8 = 0x03;

/// Length of the CH9329 parameter-config block (`docs/ch9329-spec.md` §5).
const PARA_CFG_LEN: usize = 50;
/// Byte offset of the 4-byte big-endian baud field within the config block.
const PARA_CFG_BAUD: usize = 3;

/// CH9329 absolute coordinate full-scale (12-bit, in a 4096×4096 grid).
const ABS_FULL: i64 = 4096;
/// paniolo's `moveabs` logical maximum (`hidrig` ABS_MAX).
const LOGICAL_MAX: i64 = 32_767;

/// Baud rates tried, in order, when none is forced. Openterface units default
/// to 115200; a Sipeed NanoKVM-USB ships at 57600; a factory CH9329 is at 9600
/// (see `docs/ch9329-spec.md` §2).
const BAUD_CANDIDATES: [u32; 3] = [115_200, 57_600, 9_600];

/// The datasheet's supported serial rates for `SET_PARA_CFG`
/// (`docs/ch9329-spec.md` §5). A rate outside this range would be written to
/// flash and leave a chip nothing can talk to.
const BAUD_MIN: u32 = 1_200;
const BAUD_MAX: u32 = 115_200;

/// Ceiling on the text of one `type` command. Each character is a UART round
/// trip plus [`TYPE_GAP`], so an unbounded string holds the wire — and every
/// other client — for as long as the caller likes.
pub const MAX_TYPE_CHARS: usize = 4096;

/// Ceiling on one `move`/`scroll` call's total travel, per axis. The logical
/// space is 0..=32767, so this already crosses the whole screen; each 127-unit
/// report is a round trip plus a 4 ms gap, so an unbounded total would hold
/// the wire for minutes.
const MAX_REL_TOTAL: i32 = 32_767;

/// Quiet period between baud probes — defensive, and host-dependent.
///
/// On some hosts a reopen immediately after a failed probe times out, and only
/// an idle gap lets the next rate answer. Reported (issue #81) on Linux with
/// the CH340 passed through to a VM: an Openterface at the factory 9600 rate
/// (the third candidate, so behind two failed probes) autodetected 1/6, while
/// `-b 9600` answered 6/6 and the same probes as separate processes with a
/// 0.3 s gap answered 4/4.
///
/// This is NOT a property of the CH9329 itself. The same Openterface, same
/// 9600 chip, attached directly to a macOS host autodetects 10/10 *without*
/// any settle — so the delay is compensating for the host's USB-serial path
/// (USB passthrough is the leading suspect), not for the chip.
///
/// Kept because the cost is small and paid once: autodetect runs when the
/// session opens, and one-shots route through a persistent daemon, so this is
/// at most 300 ms per failed candidate once per daemon start — not per
/// command. Measured on macOS against a 9600 chip: 1.14 s → 1.75 s to open.
const PROBE_SETTLE: Duration = Duration::from_millis(300);

/// How long a key is held before release on a `tap`/`combo`.
const HOLD: Duration = Duration::from_millis(30);
/// Hold used instead of [`HOLD`] when a tap/combo involves a lock key
/// (Caps/Num/Scroll Lock). macOS debounces Caps Lock — a tap shorter than its
/// threshold is discarded by the host before it changes state, and the chip
/// still ACKs, so the drop is silent. Measured against a macOS 15 target over
/// an Openterface: 30 ms dropped, 60 ms registered; 200 ms toggled 10/10 and
/// keeps margin for slower hosts.
const LOCK_HOLD: Duration = Duration::from_millis(200);
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

/// Hold duration for a tap/chord: [`LOCK_HOLD`] if any key is a lock key,
/// [`HOLD`] otherwise.
/// Whether a reply's payload begins with real data rather than an ACK status
/// byte.
///
/// Most ACKs lead with a status byte that is non-zero on failure. Three
/// commands do not: GET_INFO and GET_PARA_CFG return data blocks, and
/// USB_SWITCH returns the *resulting* mux position — where `0x01` means "now
/// on the target side", not "error 1". Reading that as a status would make
/// every successful switch to the target report a failure.
fn reply_payload_is_data(cmd: u8) -> bool {
    matches!(cmd, CMD_GET_INFO | CMD_GET_PARA_CFG | CMD_USB_SWITCH)
}

fn hold_for(keys: &[Key]) -> Duration {
    if keys.iter().any(|k| k.is_lock()) {
        LOCK_HOLD
    } else {
        HOLD
    }
}

/// Refuse a rate the chip cannot store, before anything touches it.
pub fn validate_baud(rate: u32) -> Result<()> {
    if !(BAUD_MIN..=BAUD_MAX).contains(&rate) {
        bail!("baud {rate} is outside the CH9329's {BAUD_MIN}..={BAUD_MAX} range");
    }
    Ok(())
}

/// Bound a relative `move`/`scroll` total to [`MAX_REL_TOTAL`] per call.
fn clamp_rel_total(v: i32) -> i32 {
    v.clamp(-MAX_REL_TOTAL, MAX_REL_TOTAL)
}

/// Refuse `type` text longer than [`MAX_TYPE_CHARS`] rather than truncate it.
fn check_type_len(text: &str) -> Result<()> {
    let n = text.chars().count();
    if n > MAX_TYPE_CHARS {
        bail!("type text is {n} characters; the limit is {MAX_TYPE_CHARS}");
    }
    Ok(())
}

/// The probe order for a reopen: `first` (the rate the previous session ran
/// at), then the defaults it is not already among.
fn probe_order(first: Option<u32>) -> Vec<u32> {
    let mut order: Vec<u32> = first.into_iter().collect();
    order.extend(
        BAUD_CANDIDATES
            .iter()
            .copied()
            .filter(|b| Some(*b) != first),
    );
    order
}

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

/// Which side of the USB mux the shared device is attached to. The mux is
/// exclusive: the microSD reader is visible to the host or the target, never
/// both, and the position does not survive the device losing power (it returns
/// to [`MuxSide::Host`]), so it must be read rather than remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuxSide {
    Host,
    Target,
}

impl MuxSide {
    pub fn as_str(self) -> &'static str {
        match self {
            MuxSide::Host => "host",
            MuxSide::Target => "target",
        }
    }

    fn selector(self) -> u8 {
        match self {
            MuxSide::Host => USB_SWITCH_TO_HOST,
            MuxSide::Target => USB_SWITCH_TO_TARGET,
        }
    }

    fn from_status(b: u8) -> Option<MuxSide> {
        match b {
            USB_SWITCH_TO_HOST => Some(MuxSide::Host),
            USB_SWITCH_TO_TARGET => Some(MuxSide::Target),
            _ => None,
        }
    }
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
        Self::open_candidates(device, &candidates)
    }

    /// [`open`](Self::open) with no forced rate, but probing `first` — the
    /// rate a previous session ran at — before the defaults. After an adapter
    /// replug or a target power cycle the chip is almost always still at it,
    /// so the daemon's reopen skips the failed probes (and their settle
    /// delays) that autodetect from scratch would pay.
    pub fn open_preferring(device: &str, first: Option<u32>) -> Result<Session> {
        Self::open_candidates(device, &probe_order(first))
    }

    fn open_candidates(device: &str, candidates: &[u32]) -> Result<Session> {
        let mut last_err: Option<anyhow::Error> = None;
        let last_index = candidates.len() - 1;
        for (i, &rate) in candidates.iter().enumerate() {
            let mut port = serialport::new(device, rate)
                .data_bits(serialport::DataBits::Eight)
                .parity(serialport::Parity::None)
                .stop_bits(serialport::StopBits::One)
                .timeout(Duration::from_millis(500))
                .dtr_on_open(false)
                .open_native()
                .with_context(|| format!("cannot open {device}"))?;
            #[cfg(target_os = "macos")]
            {
                use std::os::unix::io::AsRawFd;
                set_low_read_latency(port.as_raw_fd());
            }
            // The modem lines are not cosmetic on the devices this helper
            // drives. On the KVM-Go's CH32V208, RTS is a hardware reset of the
            // MCU — the vendor's own reset path asserts it for four seconds —
            // and on the Mini-KVM, DTR floats the switchable port's ground
            // return, so an asserted line unplugs whatever is in that port.
            // Deassert both. A brief assertion during open itself cannot be
            // prevented on Linux without kernel changes; it has not been
            // observed to disturb either device.
            port.write_request_to_send(false).ok();
            port.write_data_terminal_ready(false).ok();
            let mut s = Session {
                port: Box::new(port),
                baud: rate,
                mods: 0,
                keys: Vec::new(),
                buttons: 0,
            };
            match s.get_info() {
                Ok(_) => return Ok(s),
                Err(e) => {
                    last_err = Some(e);
                    // Close the port before the settle window so the chip sees
                    // an idle line rather than a held-open handle. Skipped
                    // after the final candidate: nothing follows it, so the
                    // wait would only delay the error.
                    drop(s);
                    if i != last_index {
                        sleep(PROBE_SETTLE);
                    }
                }
            }
        }
        Err(anyhow!(
            "CH9329 did not respond on {device} at {} baud: {}",
            candidates
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
        if !reply_payload_is_data(cmd) {
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

    // -- USB mux -------------------------------------------------------------

    /// Which side the USB mux is on, without changing it.
    pub fn usb_query(&mut self) -> Result<MuxSide> {
        self.usb_cmd(USB_SWITCH_QUERY)
    }

    /// Drive the USB mux to `side`, and confirm it actually landed there.
    ///
    /// The reply carries the *resulting* position rather than a success code,
    /// so a device that ignored the request still answers with a well-formed
    /// frame stating the old position. Comparing the two is the only success
    /// criterion the protocol offers. The operation is idempotent — switching
    /// to the side it is already on is harmless — so callers may retry.
    pub fn usb_set(&mut self, side: MuxSide) -> Result<MuxSide> {
        let got = self.usb_cmd(side.selector())?;
        if got != side {
            bail!(
                "USB mux did not move: asked for {}, device reports {}",
                side.as_str(),
                got.as_str()
            );
        }
        Ok(got)
    }

    fn usb_cmd(&mut self, selector: u8) -> Result<MuxSide> {
        let mut data = [0u8; USB_SWITCH_LEN];
        data[USB_SWITCH_LEN - 1] = selector;
        let p = self.send(CMD_USB_SWITCH, &data).map_err(|e| {
            anyhow!(
                "{e}\nA device with no switchable USB mux does not answer this \
                 command at all — the protocol has no negative ack for an \
                 unknown opcode — so a timeout here means unsupported rather \
                 than broken."
            )
        })?;
        match (p.len(), p.first().copied().and_then(MuxSide::from_status)) {
            (1, Some(side)) => Ok(side),
            _ => bail!("this device does not support USB mux switching (reply {p:02x?})"),
        }
    }

    /// Persistently change the CH9329's serial baud (`docs/ch9329-spec.md` §5).
    ///
    /// Unlike the HID serial protocol's *transient* `baud` renegotiation, the
    /// CH9329 stores its rate in flash: read the 50-byte parameter block, rewrite
    /// only the 4-byte big-endian baud field, `SET_PARA_CFG` (persist), `RESET`
    /// (activate), then reopen the host port at the new rate and confirm with
    /// `GET_INFO`. The reset clears the chip's HID state, so held keys/buttons
    /// are dropped. The datasheet supported range is 1200..=115200 (the
    /// Openterface default is already 115200; a factory chip is 9600); a rate
    /// outside it is refused before the chip is touched.
    ///
    /// Once `SET_PARA_CFG` has succeeded the new rate is in flash, so the
    /// `RESET` acknowledgement is best-effort: the chip may reboot before the
    /// ack leaves the UART, and a lost ack says nothing about the stored rate.
    /// Returns `Ok(None)` when the chip then answers at the new rate,
    /// `Ok(Some(note))` when it does but the ack was lost, and an error only
    /// when the chip does not answer afterwards (the message says the rate is
    /// persisted and how to reconnect).
    pub fn set_baud(&mut self, rate: u32) -> Result<Option<String>> {
        validate_baud(rate)?;
        let mut cfg = self.send(CMD_GET_PARA_CFG, &[])?;
        if cfg.len() != PARA_CFG_LEN {
            bail!(
                "GET_PARA_CFG returned {} bytes, expected {PARA_CFG_LEN}",
                cfg.len()
            );
        }
        // Rewrite only the baud field; preserve working mode, USB IDs, etc.
        cfg[PARA_CFG_BAUD..PARA_CFG_BAUD + 4].copy_from_slice(&rate.to_be_bytes());
        self.send(CMD_SET_PARA_CFG, &cfg)?; // persist to flash (expect 0x89/0x00)
                                            // From here the rate is in flash: record it so a reopen (the daemon's
                                            // transport-error path) probes the right rate first.
        self.baud = rate;
        let reset_ack = self.send(CMD_RESET, &[]); // activate (expect 0x8F/0x00), best-effort

        // The chip reboots at the new rate with its HID state cleared.
        self.mods = 0;
        self.keys.clear();
        self.buttons = 0;
        sleep(Duration::from_millis(400));

        self.port
            .set_baud_rate(rate)
            .map_err(|e| anyhow!("set host port to {rate} baud: {e}"))?;
        sleep(Duration::from_millis(80));

        // First frames after a reset can be lost; retry GET_INFO a few times.
        let mut last: Option<anyhow::Error> = None;
        for _ in 0..3 {
            match self.get_info() {
                Ok(_) => {
                    return Ok(reset_ack.err().map(|e| {
                        format!("the reset ack was lost ({e}) but the chip answers at {rate} baud")
                    }));
                }
                Err(e) => {
                    last = Some(e);
                    sleep(Duration::from_millis(80));
                }
            }
        }
        Err(anyhow!(
            "CH9329 did not respond at {rate} baud after reset (the rate is \
             persisted; reconnect with -b {rate}, or factory-reset via the DEF \
             pin): {}",
            last.map(|e| e.to_string()).unwrap_or_default()
        ))
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
        sleep(hold_for(&[key]));
        self.apply_up(key);
        self.push_keyboard()
    }

    /// Chord: press every key together, hold, then release back to held state.
    pub fn combo(&mut self, chord: &[Key]) -> Result<()> {
        for &k in chord {
            self.apply_down(k);
        }
        self.push_keyboard()?;
        sleep(hold_for(chord));
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

    /// Type literal text (US layout) on top of any held modifiers. Text longer
    /// than [`MAX_TYPE_CHARS`] is refused rather than truncated.
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        check_type_len(text)?;
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

    /// Relative move, split into per-report int8 deltas. Each axis's total is
    /// clamped to [`MAX_REL_TOTAL`] per call.
    pub fn move_rel(&mut self, dx: i32, dy: i32) -> Result<()> {
        let (mut dx, mut dy) = (clamp_rel_total(dx), clamp_rel_total(dy));
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

    /// Scroll the wheel; positive is up. Split into per-report int8 steps; the
    /// total is clamped to [`MAX_REL_TOTAL`] per call.
    pub fn scroll(&mut self, amount: i32) -> Result<()> {
        let mut amount = clamp_rel_total(amount);
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
    fn baud_field_encoding_matches_spec() {
        // docs/ch9329-spec.md §5: the 4-byte big-endian baud field is
        // 115200 = 00 01 C2 00, 9600 = 00 00 25 80.
        assert_eq!(115_200u32.to_be_bytes(), [0x00, 0x01, 0xC2, 0x00]);
        assert_eq!(9_600u32.to_be_bytes(), [0x00, 0x00, 0x25, 0x80]);
    }

    #[test]
    fn usb_switch_frames_match_hardware() {
        // Exactly the bytes an Openterface KVM-Go accepted on the bench
        // (notes/openterface-usb-mux-spec.md). The four leading zero payload
        // bytes are load-bearing: trimming them moves the checksum and the
        // device rejects the frame.
        let f = |sel| Session::frame(CMD_USB_SWITCH, &[0, 0, 0, 0, sel]);
        assert_eq!(
            f(USB_SWITCH_TO_HOST),
            vec![0x57, 0xAB, 0x00, 0x17, 0x05, 0, 0, 0, 0, 0x00, 0x1E]
        );
        assert_eq!(
            f(USB_SWITCH_TO_TARGET),
            vec![0x57, 0xAB, 0x00, 0x17, 0x05, 0, 0, 0, 0, 0x01, 0x1F]
        );
        assert_eq!(
            f(USB_SWITCH_QUERY),
            vec![0x57, 0xAB, 0x00, 0x17, 0x05, 0, 0, 0, 0, 0x03, 0x21]
        );
    }

    #[test]
    fn usb_switch_reply_payload_is_data_not_status() {
        // The regression this guards: the mux-switch reply's single byte is
        // the resulting position, so a successful switch to the target answers
        // 0x01. Classified as an ACK status byte, that reads as "error 1" and
        // every switch to the target would be reported as a failure.
        assert!(reply_payload_is_data(CMD_USB_SWITCH));
        assert!(reply_payload_is_data(CMD_GET_INFO));
        assert!(reply_payload_is_data(CMD_GET_PARA_CFG));
        assert!(!reply_payload_is_data(CMD_KB_GENERAL));
        assert!(!reply_payload_is_data(CMD_SET_PARA_CFG));
    }

    #[test]
    fn usb_switch_status_decoding() {
        assert_eq!(MuxSide::from_status(0x00), Some(MuxSide::Host));
        assert_eq!(MuxSide::from_status(0x01), Some(MuxSide::Target));
        // 0x03 selects "query" in a *request*; it is never a reply status,
        // and 0x02 is undefined in both directions. Either means the device
        // does not really implement the command.
        assert_eq!(MuxSide::from_status(0x02), None);
        assert_eq!(MuxSide::from_status(0x03), None);
        assert_eq!(MuxSide::from_status(0xFF), None);
    }

    #[test]
    fn usb_switch_selectors_round_trip() {
        assert_eq!(
            MuxSide::from_status(MuxSide::Host.selector()),
            Some(MuxSide::Host)
        );
        assert_eq!(
            MuxSide::from_status(MuxSide::Target.selector()),
            Some(MuxSide::Target)
        );
        assert_eq!(MuxSide::Host.as_str(), "host");
        assert_eq!(MuxSide::Target.as_str(), "target");
    }

    #[test]
    fn baud_validation_matches_the_datasheet_range() {
        assert!(validate_baud(1_200).is_ok());
        assert!(validate_baud(9_600).is_ok());
        assert!(validate_baud(115_200).is_ok());
        assert!(validate_baud(0).is_err());
        assert!(validate_baud(1_199).is_err());
        assert!(validate_baud(115_201).is_err());
        assert!(validate_baud(230_400).is_err());
    }

    #[test]
    fn preferred_rate_is_probed_first_without_duplicates() {
        assert_eq!(probe_order(None), BAUD_CANDIDATES.to_vec());
        assert_eq!(probe_order(Some(9_600)), vec![9_600, 115_200, 57_600]);
        assert_eq!(
            probe_order(Some(38_400)),
            vec![38_400, 115_200, 57_600, 9_600]
        );
    }

    #[test]
    fn relative_totals_are_clamped_per_call() {
        assert_eq!(clamp_rel_total(100), 100);
        assert_eq!(clamp_rel_total(-100), -100);
        assert_eq!(clamp_rel_total(i32::MAX), MAX_REL_TOTAL);
        assert_eq!(clamp_rel_total(i32::MIN), -MAX_REL_TOTAL);
    }

    #[test]
    fn type_text_has_a_ceiling() {
        assert!(check_type_len(&"a".repeat(MAX_TYPE_CHARS)).is_ok());
        assert!(check_type_len(&"a".repeat(MAX_TYPE_CHARS + 1)).is_err());
        // Characters, not bytes: multi-byte text is measured the same way.
        assert!(check_type_len(&"é".repeat(MAX_TYPE_CHARS)).is_ok());
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
