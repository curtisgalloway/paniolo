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

//! `usbhub learn run` — a guided profile-building wizard over the same learn
//! state machine the discrete `usbhub learn <step>` subcommands drive.
//!
//! You name a model: if a profile for it already exists, it's loaded for
//! editing (resolved against the live hub, verdicts pre-filled) and you go
//! straight to a review loop where you can re-do any port; if it's new, the
//! wizard asks the port count, walks the unplug/replug capture, then
//! maps-and-verifies each port in a first pass before the same review loop.
//! Either way, you write the profile from the review step.
//!
//! Prompts go through [`rustyline`], so they get line editing and ↑/↓ history
//! (persisted under the state dir). Everything it does is also reachable via
//! the discrete subcommands for anyone who'd rather drive it by hand; a
//! session started here can be finished there and vice versa.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::act::DeviceTable;
use crate::learn::{self, PortLearn, Session, Stage, VerifyResult};
use crate::profile::AtSpec;
use crate::topo::{Cascade, CascadeChip, Side};
use crate::{finish_session, profile, topo, walk_port, VERIFY_SETTLE};

/// How the wizard's session was obtained, which decides whether to do the
/// guided linear first pass (new captures) or jump straight to review.
enum Source {
    New,
    Existing,
}

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
        let Some((mut session, source)) = self.establish()? else {
            return Ok(());
        };

        // A guided linear first pass over unverified ports, but only for a
        // fresh capture — when editing an existing profile we go straight to
        // the review loop so you can pick the port you came to fix.
        if matches!(source, Source::New) {
            let n = session.port_count.unwrap_or(0);
            println!(
                "\nMapping {n} port(s). For each: plug a probe in and confirm whether power \
                 was cut. ('s' skips a port; you can revisit any port in the review step.)"
            );
            for k in 1..=n {
                if session.ports.get(&k).and_then(|p| p.result).is_some() {
                    continue;
                }
                if !self.do_port(&mut session, k)? {
                    self.save(&session)?;
                    println!("Stopped. Re-run `usbhub edit` to resume.");
                    return Ok(());
                }
                self.save(&session)?;
            }
        }

        self.review(&mut session)
    }

    /// Obtain the session to work on: resume an in-progress one for this model,
    /// load a saved profile to edit, or start a new capture. `None` = quit.
    fn establish(&mut self) -> Result<Option<(Session, Source)>> {
        let model =
            match self.require("Model name (an existing profile to edit, or a new name): ")? {
                Some(m) => m,
                None => return Ok(None),
            };
        self.remember(&model);

        // Resume an unsaved in-progress session for the same model.
        if let Ok(s) = learn::load_session(&self.sd) {
            if s.stage != Stage::Finished {
                if s.model.as_deref() == Some(model.as_str()) {
                    println!("Resuming your in-progress session for {model:?}.");
                    let src = if s.cascades.is_empty() {
                        Source::New
                    } else {
                        Source::Existing
                    };
                    return Ok(Some((s, src)));
                }
                println!(
                    "(Discarding an unsaved in-progress session for {:?}.)",
                    s.model.as_deref().unwrap_or("?")
                );
            }
        }

        // Edit a saved profile if one exists for this model.
        let profile_path = self.profile_dir.join(format!("{model}.toml"));
        if profile_path.exists() {
            let session = reconstruct_session(&self.profile_dir, &model)?;
            println!(
                "Editing profile {model:?} ({} port(s) already recorded).",
                session.ports.len()
            );
            self.save(&session)?;
            return Ok(Some((session, Source::Existing)));
        }

        // Otherwise, a new capture.
        println!("Creating a new profile {model:?}.");
        let mut session = Session::start(topo::snapshot()?);
        session.model = Some(model);
        match self.require_count("How many physical (silkscreen) ports does it have? ")? {
            Some(n) => session.port_count = Some(n),
            None => return Ok(None),
        }
        self.save(&session)?;
        if !self.capture(&mut session)? {
            self.save(&session)?;
            return Ok(None);
        }
        self.save(&session)?;
        Ok(Some((session, Source::New)))
    }

    /// Show the current port table and let the user (re)do any port or write
    /// the profile.
    fn review(&mut self, session: &mut Session) -> Result<()> {
        loop {
            self.print_ports(session);
            let resp = self.line(
                "Port number to map/verify, 'save' to write the profile, or 'quit' to exit \
                 without saving: ",
            )?;
            match resp.as_deref() {
                None | Some("quit") | Some("q") => {
                    self.save(session)?;
                    println!("Exited without saving. Re-run `usbhub edit` to resume.");
                    return Ok(());
                }
                Some("save") | Some("write") | Some("w") => {
                    let model = session.model.clone().unwrap_or_default();
                    match finish_session(session, &model, None, &self.profile_dir) {
                        Ok(m) => {
                            self.save(session)?;
                            println!("\n{m}");
                            return Ok(());
                        }
                        Err(e) => println!("error: {e:#}"),
                    }
                }
                Some(tok) => match tok.parse::<u16>() {
                    Ok(k) => {
                        if !self.do_port(session, k)? {
                            self.save(session)?;
                            println!("Exited without saving. Re-run `usbhub edit` to resume.");
                            return Ok(());
                        }
                        self.save(session)?;
                    }
                    Err(_) => println!("Type a port number, 'save', or 'quit'."),
                },
            }
        }
    }

    fn print_ports(&self, session: &Session) {
        let count = session.port_count.unwrap_or(0);
        let mut nums: Vec<u16> = (1..=count).collect();
        for &k in session.ports.keys() {
            if !nums.contains(&k) {
                nums.push(k);
            }
        }
        nums.sort_unstable();
        println!("\n  {:<5} {:<11} {:<11} verdict", "port", "usb3", "usb2");
        for k in nums {
            let p = session.ports.get(&k);
            let loc = |sp: Option<&crate::profile::SidePort>| {
                sp.map(|s| {
                    format!(
                        "{}@{}",
                        if s.path.is_empty() { "root" } else { &s.path },
                        s.port
                    )
                })
                .unwrap_or_else(|| "-".to_string())
            };
            let verdict = match p {
                None => "(unmapped)".to_string(),
                Some(p) => match (&p.result, &p.reason) {
                    (Some(VerifyResult::Dead), _) => "controllable".to_string(),
                    (Some(VerifyResult::Alive), Some(r)) => format!("NOT controllable ({r})"),
                    (Some(VerifyResult::Alive), None) => "NOT controllable".to_string(),
                    (None, _) => "mapped, unverified".to_string(),
                },
            };
            println!(
                "  {:<5} {:<11} {:<11} {}",
                k,
                loc(p.and_then(|p| p.usb3.as_ref())),
                loc(p.and_then(|p| p.usb2.as_ref())),
                verdict
            );
        }
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

/// Rebuild an editable, already-captured session from a saved profile by
/// resolving its chip cascade against the live hub. Shared by the wizard and
/// the discrete `usbhub learn edit <model>` command.
pub(crate) fn reconstruct_session(profile_dir: &Path, model: &str) -> Result<Session> {
    let profile = profile::load_profile(profile_dir, model)?;
    let snapshot = topo::snapshot()?;
    let instance = profile::resolve(&profile, &snapshot, &AtSpec::new());

    let mut cascades = Vec::new();
    for side in [Side::Usb3, Side::Usb2] {
        if let Ok(si) = instance.side(side) {
            let mut chips: Vec<CascadeChip> = si
                .chips
                .iter()
                .map(|(path, dev)| CascadeChip {
                    path: path.clone(),
                    dev: dev.clone(),
                })
                .collect();
            chips.sort_by(|a, b| a.path.cmp(&b.path));
            cascades.push(Cascade {
                side,
                chips,
                occupants: Vec::new(),
            });
        }
    }
    if cascades.is_empty() {
        bail!(
            "couldn't find the {model} hub on the bus to edit it — plug it in \
             (check `usbhub probe`), then try again"
        );
    }

    let ports: BTreeMap<u16, PortLearn> = profile
        .ports
        .iter()
        .map(|e| {
            (
                e.physical,
                PortLearn {
                    usb3: e.usb3.clone(),
                    usb2: e.usb2.clone(),
                    usb3_probe: None,
                    usb2_probe: None,
                    result: e.controllable.map(|c| {
                        if c {
                            VerifyResult::Dead
                        } else {
                            VerifyResult::Alive
                        }
                    }),
                    reason: e.reason.clone(),
                },
            )
        })
        .collect();
    let port_count = profile.ports.iter().map(|e| e.physical).max().unwrap_or(0);
    Ok(Session::for_edit(model, port_count, cascades, ports))
}
