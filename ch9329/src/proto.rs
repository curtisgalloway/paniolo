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

//! The HID serial protocol grammar (`docs/hid-serial-protocol.md` §3), executed
//! against a CH9329 [`Session`] instead of forwarded to a microcontroller.
//!
//! [`execute_line`] is the single backend for both the CLI subcommands and
//! `run`-file lines, so a `ch9329` helper accepts exactly the command set the
//! KB2040 `hidrig` does. Command-file parsing (`parse_sequence`/[`Step`]) and
//! the `moveabs` clamp are ported from `hidrig/src/proto.rs`.

use anyhow::{anyhow, bail, Result};

use crate::keys::name_to_key;
use crate::session::Session;

/// `version` reply data (the part after `OK`): protocol version, impl id, and
/// the optional capabilities this injector advertises. The CH9329 has a true
/// absolute pointer, so it offers `moveabs`; `baud` renegotiation is not yet
/// implemented (would need the SET_PARA_CFG flash dance, see ch9329-spec §5).
pub const VERSION_REPLY: &str = "1 ch9329/0.1.0 moveabs";

/// The `moveabs` logical maximum (matches hidrig's ABS_MAX).
pub const ABS_MAX: i32 = 32_767;

/// Clamp a value into the `moveabs` logical range.
pub fn clamp_abs(v: i32) -> i32 {
    v.clamp(0, ABS_MAX)
}

/// Execute one protocol command line. Returns the `OK` reply data (empty for a
/// bare `OK`, the capability string for `version`). Errors map to the `ERR`
/// the protocol requires.
pub fn execute_line(s: &mut Session, line: &str) -> Result<String> {
    let line = line.trim();
    let (head, rest) = line.split_once(' ').unwrap_or((line, ""));
    let rest = rest.trim();
    match head.to_ascii_lowercase().as_str() {
        "type" => {
            // Everything after `type ` is literal text (spaces and # included).
            let text = line
                .strip_prefix(head)
                .unwrap_or("")
                .strip_prefix(' ')
                .unwrap_or("");
            s.type_text(text)?;
        }
        "key" => {
            let name = one_arg(rest, "key")?;
            s.tap(name_to_key(name)?)?;
        }
        "combo" => {
            let mut chord = Vec::new();
            for name in rest.split_whitespace() {
                chord.push(name_to_key(name)?);
            }
            if chord.is_empty() {
                bail!("combo needs at least one key name");
            }
            s.combo(&chord)?;
        }
        "down" => s.key_down(name_to_key(one_arg(rest, "down")?)?)?,
        "up" => s.key_up(name_to_key(one_arg(rest, "up")?)?)?,
        "releaseall" => s.release_all()?,
        "move" => {
            let (dx, dy) = two_ints(rest, "move")?;
            s.move_rel(dx, dy)?;
        }
        "moveabs" => {
            let (x, y) = two_ints(rest, "moveabs")?;
            s.move_abs(clamp_abs(x), clamp_abs(y))?;
        }
        "click" => s.click(button_or_default(rest))?,
        "mdown" => s.mouse_down(button_or_default(rest))?,
        "mup" => s.mouse_up(button_or_default(rest))?,
        "scroll" => {
            let amount: i32 = one_arg(rest, "scroll")?
                .parse()
                .map_err(|_| anyhow!("scroll amount must be an integer: {rest:?}"))?;
            s.scroll(amount)?;
        }
        "ping" => {
            s.get_info()?;
        }
        "version" => return Ok(VERSION_REPLY.to_string()),
        "baud" => bail!("baud renegotiation not supported by this CH9329 helper"),
        other => bail!("unknown command: {other}"),
    }
    Ok(String::new())
}

fn one_arg<'a>(rest: &'a str, verb: &str) -> Result<&'a str> {
    let arg = rest.split_whitespace().next();
    arg.ok_or_else(|| anyhow!("{verb} needs an argument"))
}

fn two_ints(rest: &str, verb: &str) -> Result<(i32, i32)> {
    let mut it = rest.split_whitespace();
    let a = it.next().and_then(|v| v.parse().ok());
    let b = it.next().and_then(|v| v.parse().ok());
    match (a, b) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(anyhow!("{verb} needs two integer arguments: {rest:?}")),
    }
}

fn button_or_default(rest: &str) -> &str {
    let b = rest.split_whitespace().next().unwrap_or("left");
    if b.is_empty() {
        "left"
    } else {
        b
    }
}

/// One step of a command file (ported from hidrig).
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A protocol command line, executed verbatim.
    Cmd(String),
    /// A pause, in seconds.
    Delay(f64),
}

/// Parse a command file into steps: non-blank, non-`#` lines are commands;
/// `delay <ms>` / `sleep <seconds>` are timing directives.
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
    fn clamp_abs_bounds() {
        assert_eq!(clamp_abs(-1), 0);
        assert_eq!(clamp_abs(0), 0);
        assert_eq!(clamp_abs(ABS_MAX), ABS_MAX);
        assert_eq!(clamp_abs(ABS_MAX + 100), ABS_MAX);
    }

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
    fn command_lines_pass_through_verbatim() {
        // `type` text may contain '#'; no inline-comment stripping on commands.
        let steps = parse_sequence("type issue #42\n").unwrap();
        assert_eq!(steps, vec![Step::Cmd("type issue #42".into())]);
    }

    #[test]
    fn two_ints_parses_negatives() {
        assert_eq!(two_ints("300 -50", "move").unwrap(), (300, -50));
        assert!(two_ints("300", "move").is_err());
    }

    #[test]
    fn button_defaulting() {
        assert_eq!(button_or_default(""), "left");
        assert_eq!(button_or_default("right"), "right");
    }
}
