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

use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::Result;

use crate::act::DeviceTable;
use crate::learn::{self, Session, Stage, VerifyResult};
use crate::{finish_session, profile, topo, walk_port};

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

/// The captured-stage menu. Returns false to leave the harness.
fn menu(session: &mut Session, sd: &Path, profile_dir: &Path) -> Result<bool> {
    println!(
        "\nCommands:\n  <port#>      map a physical port (then plug the probe into it)\n  \
         v <port#>    verify a mapped port (cuts power; you report dead/alive)\n  \
         vf <port#>   verify a single-side-mapped port (see learn docs)\n  \
         s            session status\n  \
         f <model>    finish: write the profile\n  \
         q            quit (session persists; resume any time)"
    );
    loop {
        let Some(line) = prompt("learn> ")? else {
            return Ok(false);
        };
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            [] => continue,
            ["q"] => return Ok(false),
            ["s"] => println!("{}", session.status_text()),
            ["f", model] => match finish_session(session, model, None, profile_dir) {
                Ok(m) => {
                    learn::save_session(sd, session)?;
                    println!("{m}");
                    return Ok(false);
                }
                Err(e) => println!("error: {e:#}"),
            },
            ["v", n] | ["vf", n] => {
                let force = words[0] == "vf";
                match n.parse::<u16>() {
                    Ok(physical) => verify_flow(session, sd, physical, force)?,
                    Err(_) => println!("bad port number {n:?}"),
                }
            }
            [n] => match n.parse::<u16>() {
                Ok(physical) => {
                    match walk_port(session, physical, 120) {
                        Ok(m) => println!("{m}"),
                        Err(e) => println!("error: {e:#}"),
                    }
                    learn::save_session(sd, session)?;
                }
                Err(_) => println!("unknown command {n:?}"),
            },
            _ => println!("unknown command {line:?}"),
        }
    }
}

fn verify_flow(session: &mut Session, sd: &Path, physical: u16, force: bool) -> Result<()> {
    let (_, mut table) = DeviceTable::snapshot()?;
    match session.begin_verify(physical, &mut table, force) {
        Ok(m) => println!("{m}"),
        Err(e) => {
            println!("error: {e:#}");
            return Ok(());
        }
    }
    learn::save_session(sd, session)?;

    loop {
        let ans = prompt("Did the probe lose power? [d]ead / [a]live / [x] cancel: ")?;
        let verdict = match ans.as_deref() {
            Some("d") => Some((VerifyResult::Dead, None)),
            Some("a") => {
                let reason =
                    prompt("Reason, if known (Enter to skip): ")?.filter(|s| !s.is_empty());
                Some((VerifyResult::Alive, reason))
            }
            Some("x") | None => None,
            _ => continue,
        };
        match verdict {
            Some((result, reason)) => {
                match session.record_verify(physical, result, reason, &mut table) {
                    Ok(m) => println!("{m}"),
                    Err(e) => println!("error: {e:#}"),
                }
            }
            None => {
                if let Some(m) = session.abort_restore(&mut table) {
                    println!("{m}");
                }
            }
        }
        break;
    }
    learn::save_session(sd, session)?;
    Ok(())
}
