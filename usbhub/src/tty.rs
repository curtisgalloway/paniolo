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

//! `usbhub learn run` — an interactive harness over the same discrete learn
//! steps the agent-facing subcommands use. The state machine lives in
//! learn.rs; this file only prompts, relays, and persists, so a session
//! started here can be finished from the step commands and vice versa.
//!
//! The prompt accepts the **same vocabulary as `usbhub learn <cmd>`** — parsed
//! through the shared [`LearnLine`] clap parser — so the `Next:` hints the
//! state machine prints (e.g. `usbhub learn verify 7`) are typeable verbatim
//! here, with the `usbhub learn` prefix optional and subcommands abbreviatable
//! (`ver 7`). Plus two session-only controls, `help` and `quit`.

use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::Result;
use clap::Parser;

use crate::act::DeviceTable;
use crate::learn::{self, Session, Stage, VerifyResult};
use crate::{
    finish_session, profile, topo, walk_port, LearnCmd, LearnLine, ResultArg, VERIFY_SETTLE,
};

/// Read one line; None on EOF.
fn prompt(text: &str) -> Result<Option<String>> {
    print!("{text}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_string()))
}

/// Enter to continue, 'q' (or EOF) to leave. Returns false to quit.
fn prompt_enter(text: &str) -> Result<bool> {
    match prompt(&format!("{text} "))? {
        None => Ok(false),
        Some(s) => Ok(s != "q"),
    }
}

/// Split a line into tokens, honoring single/double quotes so a quoted
/// `--reason "ganged rail"` arrives as one argument.
fn tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                cur.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                in_token = true;
            }
            c if c.is_whitespace() => {
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            _ => {
                cur.push(c);
                in_token = true;
            }
        }
    }
    if in_token {
        out.push(cur);
    }
    out
}

fn print_help() {
    println!(
        "\nThis prompt takes the same commands as `usbhub learn <cmd>` — the \
         `usbhub learn` prefix is optional and commands may be abbreviated \
         (e.g. `ver 7`).\n\n\
         Mapping & verifying:\n  \
         port <n>                       map physical port n, then plug the probe into it\n  \
         verify <n>                     cut power on port n; you confirm if it died\n  \
         verify <n> --result dead|alive [--reason \"...\"]   record without the prompt\n  \
         status                         progress and the suggested next step\n  \
         finish --model <name>          write the profile and print the wiring\n  \
         abort                          discard the session (restores power if mid-verify)\n\n\
         Session controls:\n  \
         help, ?                        show this help\n  \
         quit, q                        leave — the session is saved; resume any time\n"
    );
}

pub fn run(profile_dir: &Path) -> Result<()> {
    let sd = profile::state_dir();
    let mut session = match learn::load_session(&sd) {
        Ok(s) if s.stage != Stage::Finished => {
            println!("Resuming the existing learn session.\n{}", s.status_text());
            s
        }
        _ => {
            let s = Session::start(topo::snapshot()?);
            learn::save_session(&sd, &s)?;
            println!(
                "Session started: {} device(s) on the bus.",
                s.snap_start.len()
            );
            s
        }
    };

    loop {
        match session.stage {
            Stage::Started => {
                if !prompt_enter("Unplug the hub from the host, then press Enter ('q' quits).")? {
                    break;
                }
                match session.unplugged(topo::snapshot()?) {
                    Ok(m) => println!("{m}"),
                    Err(e) => println!("error: {e:#}"),
                }
                learn::save_session(&sd, &session)?;
            }
            Stage::Unplugged => {
                if !prompt_enter(
                    "Plug the hub back in, wait ~5 s for it to settle, then press Enter \
                     ('q' quits).",
                )? {
                    break;
                }
                match session.plugged(topo::snapshot()?) {
                    Ok(m) => println!("{m}"),
                    Err(e) => println!("error: {e:#}"),
                }
                learn::save_session(&sd, &session)?;
            }
            Stage::Captured => {
                if !menu(&mut session, &sd, profile_dir)? {
                    break;
                }
            }
            Stage::Finished => {
                println!("Session finished. `usbhub learn start` begins a new one.");
                break;
            }
        }
    }
    Ok(())
}

/// The captured-stage command loop. Returns false to leave the harness.
fn menu(session: &mut Session, sd: &Path, profile_dir: &Path) -> Result<bool> {
    println!("\nCaptured the hub. Type `help` for commands, `quit` to leave.");
    loop {
        let Some(line) = prompt("learn> ")? else {
            return quit(session, sd);
        };
        let mut args = tokenize(&line);
        // Accept a pasted `usbhub learn ...` Next: hint verbatim.
        if args
            .first()
            .is_some_and(|s| s.eq_ignore_ascii_case("usbhub"))
        {
            args.remove(0);
        }
        if args
            .first()
            .is_some_and(|s| s.eq_ignore_ascii_case("learn"))
        {
            args.remove(0);
        }
        let Some(first) = args.first() else { continue };
        match first.to_ascii_lowercase().as_str() {
            "help" | "h" | "?" => {
                print_help();
                continue;
            }
            "quit" | "q" | "exit" => return quit(session, sd),
            _ => {}
        }
        match LearnLine::try_parse_from(&args) {
            Ok(LearnLine { cmd }) => {
                if !dispatch(session, sd, profile_dir, cmd)? {
                    return Ok(false);
                }
            }
            // Covers parse errors and `<cmd> --help`; clap renders both.
            Err(e) => print!("{e}"),
        }
    }
}

/// Leave the loop, restoring power if a verify was left pending (the session
/// itself is kept — resume later).
fn quit(session: &mut Session, sd: &Path) -> Result<bool> {
    if session.pending_verify.is_some() {
        let (_, mut table) = DeviceTable::snapshot()?;
        if let Some(m) = session.abort_restore(&mut table) {
            println!("{m}");
        }
        learn::save_session(sd, session)?;
    }
    Ok(false)
}

/// Run one parsed command. Returns false to leave the loop (finish/abort).
fn dispatch(session: &mut Session, sd: &Path, profile_dir: &Path, cmd: LearnCmd) -> Result<bool> {
    match cmd {
        LearnCmd::Port {
            physical,
            timeout_secs,
        } => {
            match walk_port(session, physical, timeout_secs) {
                Ok(m) => println!("{m}"),
                Err(e) => println!("error: {e:#}"),
            }
            learn::save_session(sd, session)?;
            Ok(true)
        }
        LearnCmd::Verify {
            physical,
            result,
            reason,
        } => {
            verify(session, sd, physical, result, reason)?;
            Ok(true)
        }
        LearnCmd::Status => {
            println!("{}", session.status_text());
            Ok(true)
        }
        LearnCmd::Finish { model, description } => {
            match finish_session(session, &model, description, profile_dir) {
                Ok(m) => {
                    learn::save_session(sd, session)?;
                    println!("{m}");
                    Ok(false)
                }
                Err(e) => {
                    println!("error: {e:#}");
                    Ok(true)
                }
            }
        }
        LearnCmd::Abort => {
            let (_, mut table) = DeviceTable::snapshot()?;
            if let Some(m) = session.abort_restore(&mut table) {
                println!("{m}");
            }
            learn::delete_session(sd)?;
            println!("Session discarded.");
            Ok(false)
        }
        LearnCmd::Start { .. } => {
            println!("Already in a session. `quit`, then `usbhub learn start --force` to restart.");
            Ok(true)
        }
        LearnCmd::Unplugged | LearnCmd::Plugged => {
            println!("That capture step is already done (stage: Captured). Try `status`.");
            Ok(true)
        }
        LearnCmd::Run => {
            println!("Already in the interactive session.");
            Ok(true)
        }
    }
}

/// `verify` from the prompt: with `--result`, record directly (beginning the
/// power-off first if needed); without it, cut power and ask interactively.
fn verify(
    session: &mut Session,
    sd: &Path,
    physical: u16,
    result: Option<ResultArg>,
    reason: Option<String>,
) -> Result<()> {
    let (_, mut table) = DeviceTable::snapshot()?;
    match result {
        Some(r) => {
            let vr = match r {
                ResultArg::Dead => VerifyResult::Dead,
                ResultArg::Alive => VerifyResult::Alive,
            };
            if session.pending_verify != Some(physical) {
                match session.begin_verify(physical, &mut table, VERIFY_SETTLE) {
                    Ok(m) => println!("{m}"),
                    Err(e) => {
                        println!("error: {e:#}");
                        return Ok(());
                    }
                }
            }
            match session.record_verify(physical, vr, reason, &mut table) {
                Ok(m) => println!("{m}"),
                Err(e) => println!("error: {e:#}"),
            }
            learn::save_session(sd, session)?;
        }
        None => {
            match session.begin_verify(physical, &mut table, VERIFY_SETTLE) {
                Ok(m) => println!("{m}"),
                Err(e) => {
                    println!("error: {e:#}");
                    return Ok(());
                }
            }
            learn::save_session(sd, session)?;
            interactive_verdict(session, sd, physical, &mut table)?;
        }
    }
    Ok(())
}

/// Ask the human the yes/no question "did it lose power?" and record the
/// verdict. Defaults to "no" (the device stayed powered → not controllable)
/// when the enumeration check already showed the probe never left the bus.
fn interactive_verdict(
    session: &mut Session,
    sd: &Path,
    physical: u16,
    table: &mut DeviceTable,
) -> Result<()> {
    // The probe never dropped off the bus → it almost certainly stayed
    // powered, so default the yes/no answer to "no".
    let default_no = session.pending_recommendation() == Some(VerifyResult::Alive);
    let prompt_str = if default_no {
        "Did it lose power (charging stopped / LED off)? [y]es / [n]o / [c]ancel [default: no]: "
    } else {
        "Did it lose power (charging stopped / LED off)? [y]es / [n]o / [c]ancel: "
    };
    loop {
        let ans = prompt(prompt_str)?;
        let Some(s) = ans else {
            // EOF: cancel and restore.
            if let Some(m) = session.abort_restore(table) {
                println!("{m}");
            }
            break;
        };
        // "yes, it lost power" => controllable (Dead); "no" => not (Alive).
        // d/dead and a/alive are accepted as silent aliases.
        let (result, reason) = match s.to_ascii_lowercase().as_str() {
            "y" | "yes" | "d" | "dead" => (VerifyResult::Dead, None),
            "n" | "no" | "a" | "alive" => {
                let reason =
                    prompt("Reason, if known (Enter to skip): ")?.filter(|s| !s.is_empty());
                (VerifyResult::Alive, reason)
            }
            "" if default_no => (VerifyResult::Alive, None),
            "c" | "cancel" | "x" => {
                if let Some(m) = session.abort_restore(table) {
                    println!("{m}");
                }
                break;
            }
            _ => {
                println!("Please answer 'y' (lost power), 'n' (stayed powered), or 'c' (cancel).");
                continue;
            }
        };
        match session.record_verify(physical, result, reason, table) {
            Ok(m) => println!("{m}"),
            Err(e) => println!("error: {e:#}"),
        }
        break;
    }
    learn::save_session(sd, session)?;
    Ok(())
}
