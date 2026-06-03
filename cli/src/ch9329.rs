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
//! Protocol (verified against TechxArtisanStudio/Openterface_MacOS): every frame
//! is `[0x57 0xAB 0x00, CMD, LEN, ..data.., CHECKSUM]` where the checksum is the
//! 8-bit sum of all preceding bytes. A keyboard report is CMD `0x02`, LEN `0x08`,
//! data = a standard 8-byte USB HID keyboard report `[modifier, 0, k1..k6]`.
//! Default device baud is 115200.

use std::time::Duration;

pub const DEFAULT_BAUD: u32 = 115200;

const HEADER: [u8; 3] = [0x57, 0xAB, 0x00];
const CMD_KEYBOARD: u8 = 0x02;
const MOD_SHIFT: u8 = 0x02;

/// Build one CH9329 keyboard frame from a HID report (modifier + up to 6 keys).
pub fn keyboard_frame(modifier: u8, keys: [u8; 6]) -> Vec<u8> {
    let mut f = Vec::with_capacity(14);
    f.extend_from_slice(&HEADER);
    f.push(CMD_KEYBOARD);
    f.push(0x08); // LEN
    f.push(modifier);
    f.push(0x00); // reserved
    f.extend_from_slice(&keys);
    let checksum = f.iter().fold(0u32, |a, &b| a + b as u32) as u8;
    f.push(checksum);
    f
}

/// A single-key press frame followed by an all-keys-up frame.
fn tap(modifier: u8, usage: u8) -> [Vec<u8>; 2] {
    [
        keyboard_frame(modifier, [usage, 0, 0, 0, 0, 0]),
        keyboard_frame(0, [0; 6]),
    ]
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

/// The ordered frames to type `text`: press+release per character. Returns the
/// first unsupported character as an error.
pub fn type_frames(text: &str) -> Result<Vec<Vec<u8>>, char> {
    let mut frames = Vec::new();
    for c in text.chars() {
        let (modifier, usage) = ascii_to_hid(c).ok_or(c)?;
        frames.extend(tap(modifier, usage));
    }
    Ok(frames)
}

/// Open the CH9329 serial device and type `text` into the target. The
/// inter-frame gap gives the target's USB host time to register each report.
pub fn type_string(device: &str, baud: u32, text: &str) -> anyhow::Result<()> {
    let frames =
        type_frames(text).map_err(|c| anyhow::anyhow!("unsupported character {c:?} in input"))?;
    let mut port = serialport::new(device, baud)
        .timeout(Duration::from_millis(500))
        .open()
        .map_err(|e| anyhow::anyhow!("opening CH9329 device {device}: {e}"))?;
    use std::io::Write;
    for frame in &frames {
        port.write_all(frame)?;
        port.flush()?;
        std::thread::sleep(Duration::from_millis(8));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_frame_matches_known_bytes() {
        // 'a' (usage 0x04, no modifier): header+cmd+len+report, checksum = sum&0xFF.
        let f = keyboard_frame(0, [0x04, 0, 0, 0, 0, 0]);
        assert_eq!(
            f,
            vec![0x57, 0xAB, 0x00, 0x02, 0x08, 0x00, 0x00, 0x04, 0, 0, 0, 0, 0, 0x10]
        );
        // Verify checksum independently.
        let sum: u32 = f[..f.len() - 1].iter().map(|&b| b as u32).sum();
        assert_eq!(*f.last().unwrap(), (sum & 0xFF) as u8);
    }

    #[test]
    fn ascii_maps_letters_digits_and_shifted() {
        assert_eq!(ascii_to_hid('a'), Some((0, 0x04)));
        assert_eq!(ascii_to_hid('z'), Some((0, 0x1d)));
        assert_eq!(ascii_to_hid('A'), Some((MOD_SHIFT, 0x04)));
        assert_eq!(ascii_to_hid('1'), Some((0, 0x1e)));
        assert_eq!(ascii_to_hid('0'), Some((0, 0x27)));
        assert_eq!(ascii_to_hid('!'), Some((MOD_SHIFT, 0x1e)));
        assert_eq!(ascii_to_hid(' '), Some((0, 0x2c)));
        assert_eq!(ascii_to_hid('\n'), Some((0, 0x28)));
        assert_eq!(ascii_to_hid('€'), None);
    }

    #[test]
    fn type_frames_emits_press_and_release_per_char() {
        let frames = type_frames("Hi").unwrap();
        assert_eq!(frames.len(), 4); // 2 chars × (press + release)
                                     // 'H' press carries the shift modifier; the following release is zeroed.
        assert_eq!(frames[0][5], MOD_SHIFT);
        assert_eq!(frames[1][5], 0);
        assert_eq!(frames[1][7], 0);
    }

    #[test]
    fn type_frames_reports_unsupported_char() {
        assert_eq!(type_frames("ok€"), Err('€'));
    }
}
