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

//! `usbhub learn run` — a guided, end-to-end profile-building wizard over the
//! same learn state machine the discrete `usbhub learn <step>` subcommands
//! drive. It asks for the model name and port count, walks you through the
//! unplug/replug capture, then maps-and-verifies each physical port in turn,
//! and finally writes the profile.
//!
//! Prompts go through [`rustyline`], so they get line editing and ↑/↓ history
//! (persisted under the state dir). Everything it does is also reachable via
//! the discrete subcommands for anyone who'd rather drive it by hand; a
//! session started here can be finished there and vice versa.

use std::path::{Path, PathBuf};

use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::act::DeviceTable;
use crate::learn::{self, Session, Stage, VerifyResult};
use crate::{finish_session, profile, topo, walk_port, VERIFY_SETTLE};

struct Wizard {
    rl: DefaultEditor,
    sd: PathBuf,
    profile_dir: PathBuf,
}

pub fn run(profile_dir: &Path) -> Result<()> {
    let sd = profile::state_dir();
    let _ = std::fs::create_dir_all(&sd);
    let history_path = sd.join("history.txt");
    let mut rl = DefaultEditor::new()?;
    let _ = rl.load_history(&history_path);
    let mut wiz = Wizard {
        rl,
        sd,
        profile_dir: profile_dir.to_path_buf(),
    };
    let result = wiz.drive();
    let _ = wiz.rl.save_history(&history_path);
    result
}

impl Wizard {
    /// Read one line. `None` means the user bailed (Ctrl-D / Ctrl-C).
    fn line(&mut self, prompt: &str) -> Result<Option<String>> {
        match self.rl.readline(prompt) {
            Ok(s) => Ok(Some(s.trim().to_string())),
            Err(ReadlineError::Eof) | Err(ReadlineError::Interrupted) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn remember(&mut self, s: &str) {
        if !s.is_empty() {
            let _ = self.rl.add_history_entry(s);
        }
    }

    /// Prompt until a non-empty answer; `None` if the user bails.
    fn require(&mut self, prompt: &str) -> Result<Option<String>> {
        loop {
            match self.line(prompt)? {
                None => return Ok(None),
                Some(s) if s.is_empty() => {
                    println!("(required — type a value, or Ctrl-D to quit)")
                }
                Some(s) => return Ok(Some(s)),
            }
        }
    }

    fn require_count(&mut self, prompt: &str) -> Result<Option<u16>> {
        loop {
            match self.line(prompt)? {
                None => return Ok(None),
                Some(s) => match s.parse::<u16>() {
                    Ok(n) if n >= 1 => return Ok(Some(n)),
                    _ => println!("Enter a whole number of ports (1 or more)."),
                },
            }
        }
    }

    fn save(&self, session: &Session) -> Result<()> {
        learn::save_session(&self.sd, session)
    }

    fn drive(&mut self) -> Result<()> {
        let mut session = match learn::load_session(&self.sd) {
            Ok(s) if s.stage != Stage::Finished => {
                println!("Resuming the learn session already in progress.");
                s
            }
            _ => Session::start(topo::snapshot()?),
        };

        // 1. Model name.
        if session.model.is_none() {
            match self.require("Model name for this hub (e.g. rsh-st10c-6): ")? {
                Some(m) => {
                    self.remember(&m);
                    session.model = Some(m);
                }
                None => return Ok(()),
            }
        }

        // 2. Physical port count.
        if session.port_count.is_none() {
            match self.require_count("How many physical (silkscreen) ports does it have? ")? {
                Some(n) => session.port_count = Some(n),
                None => {
                    self.save(&session)?;
                    return Ok(());
                }
            }
        }
        self.save(&session)?;

        // 3. Capture the chip cascade (unplug / replug) once.
        if session.cascades.is_empty() && !self.capture(&mut session)? {
            self.save(&session)?;
            return Ok(());
        }
        self.save(&session)?;

        // 4. Map and verify each port.
        let n = session.port_count.unwrap_or(0);
        let model = session.model.clone().unwrap_or_default();
        println!(
            "\nMapping {n} port(s) for model {model:?}. For each port you'll plug a \
             probe in and confirm whether power was cut."
        );
        for k in 1..=n {
            if session.ports.get(&k).and_then(|p| p.result).is_some() {
                println!("Port {k}: already verified — skipping.");
                continue;
            }
            if !self.do_port(&mut session, k)? {
                self.save(&session)?;
                println!("Stopped. Re-run `usbhub learn run` to pick up where you left off.");
                return Ok(());
            }
            self.save(&session)?;
        }

        // 5. Write the profile.
        match finish_session(&mut session, &model, None, &self.profile_dir) {
            Ok(m) => {
                self.save(&session)?;
                println!("\n{m}");
            }
            Err(e) => println!("error: {e:#}"),
        }
        Ok(())
    }

    /// Guide the unplug/replug capture. Returns false if the user bailed.
    fn capture(&mut self, session: &mut Session) -> Result<bool> {
        loop {
            match session.stage {
                Stage::Started => {
                    println!();
                    if self
                        .line("Step 1/2 — unplug the hub from the host, then press Enter (Ctrl-D quits): ")?
                        .is_none()
                    {
                        return Ok(false);
                    }
                    match session.unplugged(topo::snapshot()?) {
                        Ok(m) => println!("{m}"),
                        Err(e) => println!("error: {e:#}"),
                    }
                    self.save(session)?;
                }
                Stage::Unplugged => {
                    println!();
                    if self
                        .line("Step 2/2 — plug the hub back in, wait ~5 s, then press Enter (Ctrl-D quits): ")?
                        .is_none()
                    {
                        return Ok(false);
                    }
                    match session.plugged(topo::snapshot()?) {
                        Ok(m) => println!("{m}"),
                        Err(e) => println!("error: {e:#}\nMake sure it's plugged back into the same host port, then try again."),
                    }
                    self.save(session)?;
                }
                Stage::Captured | Stage::Finished => return Ok(true),
            }
        }
    }

    /// Map one physical port, then verify it. Returns false if the user bailed.
    fn do_port(&mut self, session: &mut Session, k: u16) -> Result<bool> {
        loop {
            println!();
            let resp = self.line(&format!(
                "Port {k}: make sure the probe is UNPLUGGED, then press Enter to watch for it \
                 ('s' skips this port, Ctrl-D quits): "
            ))?;
            match resp.as_deref() {
                None => return Ok(false),
                Some("s") | Some("skip") => {
                    println!("Skipped port {k} (left unmapped/unverified).");
                    return Ok(true);
                }
                Some(_) => match walk_port(session, k, 120) {
                    Ok(m) => {
                        println!("{m}");
                        return self.verify_port(session, k);
                    }
                    Err(e) => {
                        println!("error: {e:#}");
                        // Loop back to retry or skip.
                    }
                },
            }
        }
    }

    /// Cut the port's power and record the human's verdict. Returns false if
    /// the user bailed (power is restored first).
    fn verify_port(&mut self, session: &mut Session, k: u16) -> Result<bool> {
        let (_, mut table) = DeviceTable::snapshot()?;
        match session.begin_verify(k, &mut table, VERIFY_SETTLE) {
            Ok(m) => println!("{m}"),
            Err(e) => {
                println!("error: {e:#}");
                return Ok(true);
            }
        }
        self.save(session)?;

        let default_no = session.pending_recommendation() == Some(VerifyResult::Alive);
        let prompt = if default_no {
            "Did it lose power (charging stopped / LED off)? [y]es / [n]o / [c]ancel [default: no]: "
        } else {
            "Did it lose power (charging stopped / LED off)? [y]es / [n]o / [c]ancel: "
        };
        loop {
            let Some(ans) = self.line(prompt)? else {
                // Ctrl-D: restore power, then quit the wizard.
                if let Some(m) = session.abort_restore(&mut table) {
                    println!("{m}");
                }
                self.save(session)?;
                return Ok(false);
            };
            let (result, reason) = match ans.to_ascii_lowercase().as_str() {
                "y" | "yes" | "d" | "dead" => (VerifyResult::Dead, None),
                "n" | "no" | "a" | "alive" => {
                    let reason = self
                        .line("Reason, if known (Enter to skip): ")?
                        .filter(|s| !s.is_empty());
                    if let Some(r) = &reason {
                        self.remember(r);
                    }
                    (VerifyResult::Alive, reason)
                }
                "" if default_no => (VerifyResult::Alive, None),
                "c" | "cancel" | "x" => {
                    if let Some(m) = session.abort_restore(&mut table) {
                        println!("{m}");
                    }
                    self.save(session)?;
                    return Ok(true);
                }
                _ => {
                    println!(
                        "Please answer 'y' (lost power), 'n' (stayed powered), or 'c' (cancel)."
                    );
                    continue;
                }
            };
            match session.record_verify(k, result, reason, &mut table) {
                Ok(m) => println!("{m}"),
                Err(e) => println!("error: {e:#}"),
            }
            self.save(session)?;
            return Ok(true);
        }
    }
}
