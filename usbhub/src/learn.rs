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

//! The learn session: a resumable, step-at-a-time state machine that builds
//! a model profile from physical actions a human performs at the bench.
//!
//! Built agent-first: every step is a discrete command that does one
//! observation or records one human report, persists the session, prints
//! what happened, and ends with a `Next:` line. An agent (or the `learn run`
//! TTY harness, which drives these same functions) relays the physical
//! instructions to the human and reports their observations back.
//!
//! The division of labor is strict: the tool observes *enumeration*, the
//! human observes *physics*. Controllability is only ever recorded from a
//! human's `--result dead|alive` report — never inferred from descriptors.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::act::PortSwitch;
use crate::profile::{format_anchor, ChipEntry, ModelMeta, PortEntry, Profile, SidePort};
use crate::topo::{
    attribute_arrival, derive_cascades, diff_added, diff_removed, Cascade, DevKey, DevRecord, Side,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    /// `start` ran: baseline snapshot taken, waiting for the hub to be
    /// unplugged.
    Started,
    /// `unplugged` ran: waiting for the hub to be plugged back in.
    Unplugged,
    /// `plugged` ran: cascade captured; port walk and verification under way.
    Captured,
    /// `finish` ran: profile emitted.
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerifyResult {
    /// Human confirmed the probe device lost power.
    Dead,
    /// Human confirmed the probe device stayed powered.
    Alive,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PortLearn {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb3: Option<SidePort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb2: Option<SidePort>,
    /// Key of the probe device seen on the USB 3 side during the walk — the
    /// verify step checks whether it drops off the bus when power is cut.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb3_probe: Option<DevKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb2_probe: Option<DevKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<VerifyResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PortLearn {
    fn probe(&self, side: Side) -> Option<&DevKey> {
        match side {
            Side::Usb3 => self.usb3_probe.as_ref(),
            Side::Usb2 => self.usb2_probe.as_ref(),
        }
    }
}

/// What re-enumerating the bus after cutting a port's power revealed about the
/// probe device. A hint that sharpens the human's call — never a substitute
/// for it (see [`Session::begin_verify`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Liveness {
    /// The probe dropped off the bus on every checked side — consistent with
    /// real power loss. (A data-only disconnect, or a self-powered device
    /// like a phone or powered hub, looks identical, so the human still
    /// confirms with eyes/meter.)
    Disappeared,
    /// The probe is still enumerated on these sides after PORT_POWER was
    /// cleared — strong evidence the port did NOT actually cut VBUS there.
    StillPresent(Vec<Side>),
    /// Couldn't tell: the probe wasn't on the bus before the cut, or
    /// enumeration failed.
    Unknown,
}

/// One side's result from a port walk: where the probe landed, and the key of
/// the probe device itself (for the later liveness check).
#[derive(Debug, Clone, PartialEq)]
pub struct PortFinding {
    pub side: Side,
    pub at: SidePort,
    pub probe: DevKey,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub stage: Stage,
    pub snap_start: Vec<DevRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap_unplugged: Option<Vec<DevRecord>>,
    #[serde(default)]
    pub cascades: Vec<Cascade>,
    #[serde(default)]
    pub ports: BTreeMap<u16, PortLearn>,
    /// Physical port currently powered off awaiting the human's
    /// `--result` report. At most one at a time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_verify: Option<u16>,
    /// What the enumeration check saw when the pending port's power was cut —
    /// used to recommend a verdict and cross-check the human's answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_liveness: Option<Liveness>,
}

/// Outcome of one port-walk poll.
#[derive(Debug, PartialEq)]
pub enum WalkOutcome {
    /// Nothing new yet — keep polling.
    Nothing,
    /// The probe arrived on one or more sides under the cascade's chips.
    /// (A USB 3 probe hub maps both sides in one plug.)
    Found(Vec<PortFinding>),
    /// Something arrived, but not under any chip of the cascade.
    Elsewhere(Vec<DevRecord>),
}

impl Session {
    pub fn start(snap: Vec<DevRecord>) -> Session {
        Session {
            version: 1,
            stage: Stage::Started,
            snap_start: snap,
            snap_unplugged: None,
            cascades: Vec::new(),
            ports: BTreeMap::new(),
            pending_verify: None,
            pending_liveness: None,
        }
    }

    fn expect_stage(&self, want: Stage, hint: &str) -> Result<()> {
        if self.stage != want {
            bail!(
                "session is at stage {:?}, expected {:?} — {hint}",
                self.stage,
                want
            );
        }
        Ok(())
    }

    /// Record the snapshot taken after the hub was unplugged.
    pub fn unplugged(&mut self, snap: Vec<DevRecord>) -> Result<String> {
        self.expect_stage(Stage::Started, "run `usbhub learn start` first")?;
        let removed = diff_removed(&self.snap_start, &snap);
        let mut msg = String::new();
        if removed.is_empty() {
            msg.push_str(
                "Nothing disappeared from the bus — the hub may not have been \
                 plugged in before `start`. Proceeding; the capture only needs \
                 the plug-in that comes next.\n",
            );
        } else {
            msg.push_str(&format!("{} device(s) disappeared:\n", removed.len()));
            for d in &removed {
                msg.push_str(&format!("  {}\n", d.describe()));
            }
        }
        self.snap_unplugged = Some(snap);
        self.stage = Stage::Unplugged;
        msg.push_str("Next: plug the hub back in, wait ~5 s, then run `usbhub learn plugged`");
        Ok(msg)
    }

    /// Diff the post-replug snapshot and derive the product's chip cascades.
    pub fn plugged(&mut self, snap: Vec<DevRecord>) -> Result<String> {
        self.expect_stage(Stage::Unplugged, "run `usbhub learn unplugged` first")?;
        let base = self.snap_unplugged.as_ref().expect("unplugged stage set");
        let added = diff_added(base, &snap);
        let cascades = derive_cascades(&added)?;

        let mut msg = String::from("Captured the hub's chip cascade:\n");
        for c in &cascades {
            msg.push_str(&format!("  [{}] root: {}\n", c.side, c.root().describe()));
            for chip in c.chips.iter().filter(|ch| !ch.path.is_empty()) {
                msg.push_str(&format!(
                    "  [{}] chip at internal path {:?}: {}\n",
                    c.side,
                    chip.path,
                    chip.dev.describe()
                ));
            }
            for occ in &c.occupants {
                msg.push_str(&format!(
                    "  [{}] occupied port: {}\n",
                    c.side,
                    occ.describe()
                ));
            }
        }
        if cascades.len() == 1 {
            msg.push_str(&format!(
                "Note: only the {} side enumerated — a USB 3 hub on a USB 2 \
                 uplink looks like this. Both-side control will not be \
                 possible for this capture.\n",
                cascades[0].side
            ));
        }
        self.cascades = cascades;
        self.stage = Stage::Captured;
        msg.push_str(
            "Next: map physical ports with `usbhub learn port <n>` — pick the \
             silkscreen number, run the command, then plug the probe device \
             into that port. Tip: a small USB 3 hub as the probe maps both \
             sides in one plug; a flash drive maps only its own side.",
        );
        Ok(msg)
    }

    /// One poll of the port walk: anything new under the cascade's chips?
    pub fn walk_check(&self, baseline: &[DevRecord], now: &[DevRecord]) -> WalkOutcome {
        let added = diff_added(baseline, now);
        if added.is_empty() {
            return WalkOutcome::Nothing;
        }
        let mut found = Vec::new();
        for cascade in &self.cascades {
            if let Some((path, port, probe)) = attribute_arrival(cascade, &added) {
                found.push(PortFinding {
                    side: cascade.side,
                    at: SidePort { path, port },
                    probe,
                });
            }
        }
        if found.is_empty() {
            WalkOutcome::Elsewhere(added)
        } else {
            WalkOutcome::Found(found)
        }
    }

    /// Record where the probe arrived for a physical port. Re-running the
    /// walk for the same port (e.g. with the other-speed probe) merges.
    pub fn record_port(&mut self, physical: u16, findings: &[PortFinding]) -> Result<String> {
        self.expect_stage(Stage::Captured, "run the capture steps first")?;
        let entry = self.ports.entry(physical).or_default();
        let mut msg = format!("Physical port {physical}:\n");
        for f in findings {
            msg.push_str(&format!(
                "  [{}] chip path {:?}, chip port {}\n",
                f.side, f.at.path, f.at.port
            ));
            match f.side {
                Side::Usb3 => {
                    entry.usb3 = Some(f.at.clone());
                    entry.usb3_probe = Some(f.probe.clone());
                }
                Side::Usb2 => {
                    entry.usb2 = Some(f.at.clone());
                    entry.usb2_probe = Some(f.probe.clone());
                }
            }
        }
        if entry.usb3.is_none() || entry.usb2.is_none() {
            let missing = if entry.usb3.is_none() { "usb3" } else { "usb2" };
            msg.push_str(&format!(
                "  ({missing} side still unmapped — re-run `usbhub learn port \
                 {physical}` with a {missing}-speed probe to map it)\n"
            ));
        }
        msg.push_str(&format!(
            "Next: `usbhub learn verify {physical}` (leave the probe plugged \
             in), or map another port"
        ));
        Ok(msg)
    }

    fn cascade_chip(&self, side: Side, path: &str) -> Result<&DevRecord> {
        self.cascades
            .iter()
            .find(|c| c.side == side)
            .and_then(|c| c.chip(path))
            .with_context(|| format!("no captured {side} chip at path {path:?}"))
    }

    /// Mapped sides of a physical port, with their chip records.
    fn mapped_sides(&self, physical: u16) -> Result<Vec<(Side, DevRecord, u8)>> {
        let entry = self.ports.get(&physical).with_context(|| {
            format!(
                "physical port {physical} not mapped — run `usbhub learn port {physical}` first"
            )
        })?;
        let mut out = Vec::new();
        if let Some(sp) = &entry.usb3 {
            out.push((
                Side::Usb3,
                self.cascade_chip(Side::Usb3, &sp.path)?.clone(),
                sp.port,
            ));
        }
        if let Some(sp) = &entry.usb2 {
            out.push((
                Side::Usb2,
                self.cascade_chip(Side::Usb2, &sp.path)?.clone(),
                sp.port,
            ));
        }
        Ok(out)
    }

    /// Cut power on a mapped port so the human can observe whether the probe
    /// actually dies. Refuses to run with another verify pending, and — by
    /// default — refuses single-side mappings, where "off" can mean "fell
    /// back to the other topology, still powered".
    pub fn begin_verify(
        &mut self,
        physical: u16,
        hw: &mut dyn PortSwitch,
        allow_single_side: bool,
        settle: Duration,
    ) -> Result<String> {
        self.expect_stage(Stage::Captured, "run the capture steps first")?;
        if let Some(p) = self.pending_verify {
            bail!(
                "port {p} is still powered off awaiting its verdict — report it \
                 with `usbhub learn verify {p} --result dead|alive` first"
            );
        }
        let sides = self.mapped_sides(physical)?;
        let both = self.cascades.len() < 2
            || (self.ports[&physical].usb3.is_some() && self.ports[&physical].usb2.is_some());
        if !both && !allow_single_side {
            bail!(
                "physical port {physical} is mapped on only one side; cutting \
                 one side can leave the device powered on the other. Map the \
                 other side first, or pass --allow-single-side if this port \
                 (or the uplink) genuinely has only this side."
            );
        }

        // Snapshot the bus before cutting power, so the after-shot can tell us
        // whether the probe device actually dropped off.
        let before = hw.live_keys().unwrap_or_default();

        let mut msg = format!("Powering OFF physical port {physical}:\n");
        for (side, chip, port) in &sides {
            hw.set_power(*side, chip, *port, false)
                .with_context(|| format!("powering off {side} chip port {port}"))?;
            let bit = hw.power_is_on(*side, chip, *port);
            msg.push_str(&format!(
                "  [{side}] chip port {port}: PORT_POWER cleared (status bit now reads {})\n",
                match bit {
                    Ok(true) => "ON — chip likely lies about per-port switching",
                    Ok(false) => "off",
                    Err(_) => "unreadable",
                }
            ));
        }
        self.pending_verify = Some(physical);

        // Give the device a moment to fall off the bus, then re-enumerate and
        // see whether the probe is still there.
        thread::sleep(settle);
        let after = hw.live_keys().unwrap_or_default();
        let entry = &self.ports[&physical];
        let mut still = Vec::new();
        let mut checked = false;
        for (side, _chip, _port) in &sides {
            if let Some(key) = entry.probe(*side) {
                if before.contains(key) {
                    checked = true;
                    if after.contains(key) {
                        still.push(*side);
                    }
                }
            }
        }
        let liveness = if !checked {
            Liveness::Unknown
        } else if still.is_empty() {
            Liveness::Disappeared
        } else {
            Liveness::StillPresent(still.clone())
        };

        match &liveness {
            Liveness::StillPresent(sides) => {
                let which = sides
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join("+");
                msg.push_str(&format!(
                    "Enumeration check: the probe is STILL on the bus ({which}) after \
                     clearing PORT_POWER — this port did NOT cut VBUS. Recommended \
                     verdict: alive (not controllable).\n"
                ));
            }
            Liveness::Disappeared => msg.push_str(
                "Enumeration check: the probe dropped off the bus — consistent with \
                 power loss. But a data-only disconnect looks the same, and a \
                 self-powered device (phone, powered hub) also drops off without \
                 losing its own power. Confirm on the device itself.\n",
            ),
            Liveness::Unknown => msg.push_str(
                "Enumeration check: couldn't track the probe on the bus — rely on the \
                 device's own indicator.\n",
            ),
        }
        self.pending_liveness = Some(liveness);

        msg.push_str(&format!(
            "Look at the probe device in port {physical}: did it actually lose power \
             — charging stopped / LED off / no current on a meter? The status and \
             enumeration above are hints; your eyes decide.\n\
             Next: `usbhub learn verify {physical} --result dead` (it lost power, \
             controllable) or `--result alive [--reason \"...\"]` (still powered)"
        ));
        Ok(msg)
    }

    /// A verdict to default to, when the enumeration check is conclusive
    /// enough to suggest one. Only ever recommends `alive` (the safe,
    /// under-claiming direction) — disappearance alone never proves control.
    pub fn pending_recommendation(&self) -> Option<VerifyResult> {
        match &self.pending_liveness {
            Some(Liveness::StillPresent(_)) => Some(VerifyResult::Alive),
            _ => None,
        }
    }

    /// Record the human's verdict and restore power.
    pub fn record_verify(
        &mut self,
        physical: u16,
        result: VerifyResult,
        reason: Option<String>,
        hw: &mut dyn PortSwitch,
    ) -> Result<String> {
        self.expect_stage(Stage::Captured, "run the capture steps first")?;
        if self.pending_verify != Some(physical) {
            bail!(
                "port {physical} is not awaiting a verdict{}",
                match self.pending_verify {
                    Some(p) => format!(" (port {p} is)"),
                    None => " — run `usbhub learn verify <n>` first".to_string(),
                }
            );
        }
        let sides = self.mapped_sides(physical)?;
        let mut msg = String::new();
        let mut restore_failed = false;
        for (side, chip, port) in &sides {
            match hw.set_power(*side, chip, *port, true) {
                Ok(()) => {}
                Err(e) => {
                    restore_failed = true;
                    msg.push_str(&format!(
                        "  WARNING: restoring power on [{side}] chip port {port} failed: {e:#}\n"
                    ));
                }
            }
        }
        if restore_failed {
            msg.push_str("  Power may still be off — unplug and replug the hub to recover.\n");
        } else {
            msg.push_str(&format!("Power restored on port {physical}.\n"));
        }

        // Warn if the human's verdict contradicts conclusive enumeration
        // evidence (claiming controllable while the probe never left the bus).
        if result == VerifyResult::Dead {
            if let Some(Liveness::StillPresent(sides)) = &self.pending_liveness {
                let which = sides
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join("+");
                msg.push_str(&format!(
                    "  NOTE: you marked this controllable, but the probe stayed on \
                     the bus ({which}) after power-off — double-check it really lost \
                     power before trusting this.\n"
                ));
            }
        }

        let entry = self.ports.get_mut(&physical).expect("mapped_sides checked");
        entry.result = Some(result);
        entry.reason = match (&result, reason) {
            (VerifyResult::Alive, None) => {
                Some("probe stayed powered when PORT_POWER was cleared".to_string())
            }
            (_, r) => r,
        };
        self.pending_verify = None;
        self.pending_liveness = None;

        msg.push_str(&format!(
            "Recorded: port {physical} {}\n",
            match result {
                VerifyResult::Dead => "is controllable (verified by human observation)",
                VerifyResult::Alive => "is NOT controllable",
            }
        ));
        msg.push_str("Next: map/verify another port, or `usbhub learn finish --model <name>`");
        Ok(msg)
    }

    /// Compile the session into a model profile.
    pub fn finish(&mut self, model: &str, description: Option<String>) -> Result<Profile> {
        self.expect_stage(Stage::Captured, "nothing captured yet")?;
        if let Some(p) = self.pending_verify {
            bail!("port {p} is still awaiting its verdict — report it before finishing");
        }
        if self.ports.is_empty() {
            bail!("no ports mapped — walk at least one port before finishing");
        }

        // One chip entry per internal path; sides sharing a path share an
        // entry (cosmetic grouping — resolution treats sides independently).
        let mut paths: Vec<String> = self
            .cascades
            .iter()
            .flat_map(|c| c.chips.iter().map(|ch| ch.path.clone()))
            .collect();
        paths.sort();
        paths.dedup();
        let chips = paths
            .iter()
            .map(|path| {
                let label = if path.is_empty() {
                    "root".to_string()
                } else {
                    format!("chip{}", path.replace('.', "-"))
                };
                let side_of = |side: Side| {
                    self.cascades
                        .iter()
                        .find(|c| c.side == side)
                        .and_then(|c| c.chip(path))
                };
                let fmt_id = |d: &DevRecord| format!("{:04x}:{:04x}", d.vid, d.pid);
                ChipEntry {
                    label,
                    usb3_path: side_of(Side::Usb3).map(|_| path.clone()),
                    usb3_id: side_of(Side::Usb3).map(fmt_id),
                    usb2_path: side_of(Side::Usb2).map(|_| path.clone()),
                    usb2_id: side_of(Side::Usb2).map(fmt_id),
                }
            })
            .collect();

        let ports = self
            .ports
            .iter()
            .map(|(physical, p)| PortEntry {
                physical: *physical,
                usb3: p.usb3.clone(),
                usb2: p.usb2.clone(),
                controllable: p.result.map(|r| r == VerifyResult::Dead),
                reason: p.reason.clone(),
                note: None,
            })
            .collect();

        self.stage = Stage::Finished;
        Ok(Profile {
            model: ModelMeta {
                name: model.to_string(),
                description,
            },
            chips,
            ports,
        })
    }

    /// Best-effort power restore when aborting a session that has a verify
    /// pending — never leave a port dark behind a deleted session.
    pub fn abort_restore(&mut self, hw: &mut dyn PortSwitch) -> Option<String> {
        let p = self.pending_verify.take()?;
        self.pending_liveness = None;
        let mut notes = vec![format!(
            "Port {p} was powered off pending a verdict; restoring:"
        )];
        match self.mapped_sides(p) {
            Ok(sides) => {
                for (side, chip, port) in sides {
                    match hw.set_power(side, &chip, port, true) {
                        Ok(()) => notes.push(format!("  [{side}] chip port {port}: on")),
                        Err(e) => notes.push(format!(
                            "  [{side}] chip port {port}: restore FAILED: {e:#} — \
                             unplug and replug the hub to recover"
                        )),
                    }
                }
            }
            Err(e) => notes.push(format!("  cannot locate chips: {e:#}")),
        }
        Some(notes.join("\n"))
    }

    /// The current `--at` anchors of the captured instance, for the finish
    /// message (informational — resolution is signature-first).
    pub fn anchors(&self) -> String {
        self.cascades
            .iter()
            .map(|c| format_anchor(c.side, c.root()))
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn status_text(&self) -> String {
        let mut msg = format!("Learn session stage: {:?}\n", self.stage);
        for c in &self.cascades {
            msg.push_str(&format!(
                "  [{}] {} chip(s), root {}\n",
                c.side,
                c.chips.len(),
                c.root().describe()
            ));
        }
        if !self.ports.is_empty() {
            msg.push_str("Ports:\n");
            for (n, p) in &self.ports {
                msg.push_str(&format!(
                    "  {n}: usb3={} usb2={} verdict={}\n",
                    p.usb3
                        .as_ref()
                        .map(|s| format!("{}@{}", s.path, s.port))
                        .unwrap_or_else(|| "-".to_string()),
                    p.usb2
                        .as_ref()
                        .map(|s| format!("{}@{}", s.path, s.port))
                        .unwrap_or_else(|| "-".to_string()),
                    match (&p.result, &p.reason) {
                        (Some(VerifyResult::Dead), _) => "controllable".to_string(),
                        (Some(VerifyResult::Alive), Some(r)) => format!("NOT controllable ({r})"),
                        (Some(VerifyResult::Alive), None) => "NOT controllable".to_string(),
                        (None, _) => "unverified".to_string(),
                    }
                ));
            }
        }
        if let Some(p) = self.pending_verify {
            msg.push_str(&format!(
                "PENDING: port {p} is powered off awaiting --result\n"
            ));
        }
        msg.push_str(&format!("Next: {}", self.suggest_next()));
        msg
    }

    fn suggest_next(&self) -> &'static str {
        match self.stage {
            Stage::Started => "unplug the hub, then `usbhub learn unplugged`",
            Stage::Unplugged => "plug the hub back in, then `usbhub learn plugged`",
            Stage::Captured => {
                if self.pending_verify.is_some() {
                    "`usbhub learn verify <n> --result dead|alive`"
                } else if self.ports.is_empty() {
                    "`usbhub learn port <n>` to map your first physical port"
                } else {
                    "`usbhub learn port <n>`, `usbhub learn verify <n>`, or \
                     `usbhub learn finish --model <name>`"
                }
            }
            Stage::Finished => "`usbhub learn start --force` to begin a new session",
        }
    }
}

// ---------------------------------------------------------------------------
// Session persistence
// ---------------------------------------------------------------------------

pub fn session_path(state_dir: &Path) -> PathBuf {
    state_dir.join("learn.json")
}

pub fn load_session(state_dir: &Path) -> Result<Session> {
    let path = session_path(state_dir);
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "no learn session at {} — start one with `usbhub learn start`",
            path.display()
        )
    })?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

pub fn save_session(state_dir: &Path, session: &Session) -> Result<()> {
    std::fs::create_dir_all(state_dir)
        .with_context(|| format!("creating {}", state_dir.display()))?;
    let path = session_path(state_dir);
    let text = serde_json::to_string_pretty(session)?;
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

pub fn delete_session(state_dir: &Path) -> Result<()> {
    let path = session_path(state_dir);
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::time::Duration;

    fn rec(bus: &str, chain: &[u8], vid: u16, pid: u16, class: u8, speed: &str) -> DevRecord {
        DevRecord {
            bus_id: bus.to_string(),
            port_chain: chain.to_vec(),
            vid,
            pid,
            class,
            speed: Some(speed.to_string()),
            product: None,
            serial: None,
        }
    }

    /// Host baseline: a keyboard, nothing else interesting.
    fn base_snap() -> Vec<DevRecord> {
        vec![rec("1", &[9], 0x05ac, 0x024f, 0, "full")]
    }

    /// Baseline plus the RSH-style cascade (usb3 root+leaf, usb2 companions).
    fn hub_snap() -> Vec<DevRecord> {
        let mut v = base_snap();
        v.extend([
            rec("2", &[1], 0x0bda, 0x0411, 9, "super"),
            rec("2", &[1, 4], 0x0bda, 0x0411, 9, "super"),
            rec("1", &[3], 0x0bda, 0x5411, 9, "high"),
            rec("1", &[3, 4], 0x0bda, 0x5411, 9, "high"),
        ]);
        v
    }

    /// A USB 3 hub probe maps physical port 7 on both sides (usb3 leaf port 1,
    /// usb2 leaf port 1). These are the probe devices for that mapping.
    fn probe_u3() -> DevRecord {
        rec("2", &[1, 4, 1], 0x05e3, 0x0626, 9, "super")
    }
    fn probe_u2() -> DevRecord {
        rec("1", &[3, 4, 1], 0x05e3, 0x0610, 9, "high")
    }

    fn finding(side: Side, path: &str, port: u8, probe: DevRecord) -> PortFinding {
        PortFinding {
            side,
            at: SidePort {
                path: path.into(),
                port,
            },
            probe: probe.key(),
        }
    }

    /// Mock hardware: records set_power calls and reports the last commanded
    /// state as the status bit. `probe_at` ties a chip-port to the probe key
    /// that lives there so `live_keys` can model the device dropping off the
    /// bus when power is cut; `lying` ports keep the device enumerated even
    /// when "off" (a hub that claims switching it doesn't do).
    #[derive(Default)]
    struct MockHw {
        state: HashMap<(Side, Vec<u8>, u8), bool>,
        calls: Vec<String>,
        fail_restore: bool,
        probe_at: HashMap<(Side, Vec<u8>, u8), DevKey>,
        lying: HashSet<(Side, Vec<u8>, u8)>,
    }

    impl MockHw {
        fn with_probe(
            mut self,
            side: Side,
            chain: &[u8],
            port: u8,
            key: DevKey,
            lying: bool,
        ) -> Self {
            let pos = (side, chain.to_vec(), port);
            self.probe_at.insert(pos.clone(), key);
            if lying {
                self.lying.insert(pos);
            }
            self
        }
    }

    /// A mock wired for the port-7 both-sides mapping; `lying` simulates a hub
    /// whose PORT_POWER clears but VBUS stays on.
    fn hw_port7(lying: bool) -> MockHw {
        MockHw::default()
            .with_probe(Side::Usb3, &[1, 4], 1, probe_u3().key(), lying)
            .with_probe(Side::Usb2, &[3, 4], 1, probe_u2().key(), lying)
    }

    impl PortSwitch for MockHw {
        fn set_power(&mut self, side: Side, chip: &DevRecord, port: u8, on: bool) -> Result<()> {
            if on && self.fail_restore {
                bail!("injected restore failure");
            }
            self.calls.push(format!(
                "{side} {} p{port} {}",
                crate::topo::chain_str(&chip.port_chain),
                if on { "on" } else { "off" }
            ));
            self.state.insert((side, chip.port_chain.clone(), port), on);
            Ok(())
        }

        fn power_is_on(&mut self, side: Side, chip: &DevRecord, port: u8) -> Result<bool> {
            Ok(*self
                .state
                .get(&(side, chip.port_chain.clone(), port))
                .unwrap_or(&true))
        }

        fn live_keys(&mut self) -> Result<HashSet<DevKey>> {
            let mut out = HashSet::new();
            for (pos, key) in &self.probe_at {
                let powered = self.state.get(pos).copied().unwrap_or(true);
                if powered || self.lying.contains(pos) {
                    out.insert(key.clone());
                }
            }
            Ok(out)
        }
    }

    fn captured_session() -> Session {
        let mut s = Session::start(hub_snap());
        s.unplugged(base_snap()).unwrap();
        s.plugged(hub_snap()).unwrap();
        s
    }

    #[test]
    fn full_capture_flow() {
        let s = captured_session();
        assert_eq!(s.stage, Stage::Captured);
        assert_eq!(s.cascades.len(), 2);
    }

    #[test]
    fn stage_gating() {
        let mut s = Session::start(hub_snap());
        assert!(s.plugged(hub_snap()).is_err());
        let mut s2 = captured_session();
        assert!(s2.unplugged(base_snap()).is_err());
    }

    #[test]
    fn unplug_of_missing_hub_proceeds() {
        let mut s = Session::start(base_snap());
        let msg = s.unplugged(base_snap()).unwrap();
        assert!(msg.contains("Nothing disappeared"), "{msg}");
        assert!(s.plugged(hub_snap()).is_ok());
    }

    #[test]
    fn walk_and_record_both_sides() {
        let mut s = captured_session();
        // A USB3-hub probe: appears on both sides at physical port 7
        // (usb3 leaf port 1, usb2 leaf port 1).
        let mut now = hub_snap();
        now.push(rec("2", &[1, 4, 1], 0x05e3, 0x0626, 9, "super"));
        now.push(rec("1", &[3, 4, 1], 0x05e3, 0x0610, 9, "high"));
        let outcome = s.walk_check(&hub_snap(), &now);
        let WalkOutcome::Found(findings) = outcome else {
            panic!("expected Found, got {outcome:?}");
        };
        assert_eq!(findings.len(), 2);
        let msg = s.record_port(7, &findings).unwrap();
        assert!(msg.contains("Next:"), "{msg}");
        let p = &s.ports[&7];
        assert_eq!(p.usb3.as_ref().unwrap().path, "4");
        assert_eq!(p.usb2.as_ref().unwrap().port, 1);
    }

    #[test]
    fn walk_single_side_merges_on_rerun() {
        let mut s = captured_session();
        let mut now = hub_snap();
        now.push(rec("2", &[1, 4, 1], 0x0781, 0x5581, 0, "super"));
        let WalkOutcome::Found(f1) = s.walk_check(&hub_snap(), &now) else {
            panic!()
        };
        let msg = s.record_port(7, &f1).unwrap();
        assert!(msg.contains("usb2 side still unmapped"), "{msg}");

        let mut now2 = hub_snap();
        now2.push(rec("1", &[3, 4, 1], 0x0951, 0x1666, 0, "high"));
        let WalkOutcome::Found(f2) = s.walk_check(&hub_snap(), &now2) else {
            panic!()
        };
        s.record_port(7, &f2).unwrap();
        let p = &s.ports[&7];
        assert!(p.usb3.is_some() && p.usb2.is_some());
    }

    #[test]
    fn walk_elsewhere() {
        let s = captured_session();
        let mut now = hub_snap();
        now.push(rec("1", &[8], 0x0781, 0x5581, 0, "high"));
        let outcome = s.walk_check(&hub_snap(), &now);
        assert!(matches!(outcome, WalkOutcome::Elsewhere(_)), "{outcome:?}");
    }

    fn mapped_session() -> Session {
        let mut s = captured_session();
        s.record_port(
            7,
            &[
                finding(Side::Usb3, "4", 1, probe_u3()),
                finding(Side::Usb2, "4", 1, probe_u2()),
            ],
        )
        .unwrap();
        s
    }

    #[test]
    fn verify_dead_records_controllable_and_restores() {
        let mut s = mapped_session();
        let mut hw = hw_port7(false);
        let msg = s.begin_verify(7, &mut hw, false, Duration::ZERO).unwrap();
        assert!(msg.contains("status bit now reads off"), "{msg}");
        // The probe really dropped off the bus when power was cut.
        assert!(msg.contains("dropped off the bus"), "{msg}");
        assert_eq!(s.pending_recommendation(), None);
        assert_eq!(s.pending_verify, Some(7));
        assert_eq!(hw.calls, vec!["usb3 1.4 p1 off", "usb2 3.4 p1 off"]);

        let msg = s
            .record_verify(7, VerifyResult::Dead, None, &mut hw)
            .unwrap();
        assert!(msg.contains("Power restored"), "{msg}");
        assert_eq!(s.pending_verify, None);
        assert_eq!(s.pending_liveness, None);
        assert_eq!(s.ports[&7].result, Some(VerifyResult::Dead));
        assert_eq!(hw.calls.len(), 4, "two offs then two ons");
    }

    #[test]
    fn verify_still_present_recommends_alive_and_warns() {
        // A lying hub: PORT_POWER clears but the probe never leaves the bus.
        let mut s = mapped_session();
        let mut hw = hw_port7(true);
        let msg = s.begin_verify(7, &mut hw, false, Duration::ZERO).unwrap();
        assert!(msg.contains("STILL on the bus"), "{msg}");
        assert_eq!(s.pending_recommendation(), Some(VerifyResult::Alive));

        // If the human nonetheless claims it's controllable, warn them.
        let msg = s
            .record_verify(7, VerifyResult::Dead, None, &mut hw)
            .unwrap();
        assert!(msg.contains("NOTE: you marked this controllable"), "{msg}");
        assert_eq!(s.pending_liveness, None);
    }

    #[test]
    fn verify_alive_records_reason() {
        let mut s = mapped_session();
        let mut hw = MockHw::default();
        s.begin_verify(7, &mut hw, false, Duration::ZERO).unwrap();
        s.record_verify(7, VerifyResult::Alive, None, &mut hw)
            .unwrap();
        assert_eq!(s.ports[&7].result, Some(VerifyResult::Alive));
        assert!(s.ports[&7]
            .reason
            .as_ref()
            .unwrap()
            .contains("stayed powered"));
    }

    #[test]
    fn verify_single_side_refused_without_flag() {
        let mut s = captured_session();
        s.record_port(7, &[finding(Side::Usb3, "4", 1, probe_u3())])
            .unwrap();
        let mut hw = MockHw::default();
        let err = s
            .begin_verify(7, &mut hw, false, Duration::ZERO)
            .unwrap_err()
            .to_string();
        assert!(err.contains("only one side"), "{err}");
        assert!(s.begin_verify(7, &mut hw, true, Duration::ZERO).is_ok());
    }

    #[test]
    fn second_verify_blocked_while_pending() {
        let mut s = mapped_session();
        s.record_port(
            8,
            &[
                finding(
                    Side::Usb3,
                    "4",
                    2,
                    rec("2", &[1, 4, 2], 0x0781, 0x5581, 0, "super"),
                ),
                finding(
                    Side::Usb2,
                    "4",
                    2,
                    rec("1", &[3, 4, 2], 0x0781, 0x5581, 0, "high"),
                ),
            ],
        )
        .unwrap();
        let mut hw = MockHw::default();
        s.begin_verify(7, &mut hw, false, Duration::ZERO).unwrap();
        let err = s
            .begin_verify(8, &mut hw, false, Duration::ZERO)
            .unwrap_err()
            .to_string();
        assert!(err.contains("still powered off"), "{err}");
    }

    #[test]
    fn restore_failure_is_loud() {
        let mut s = mapped_session();
        let mut hw = MockHw::default();
        s.begin_verify(7, &mut hw, false, Duration::ZERO).unwrap();
        hw.fail_restore = true;
        let msg = s
            .record_verify(7, VerifyResult::Dead, None, &mut hw)
            .unwrap();
        assert!(msg.contains("WARNING"), "{msg}");
        assert!(msg.contains("unplug and replug"), "{msg}");
    }

    #[test]
    fn finish_compiles_profile() {
        let mut s = mapped_session();
        let mut hw = MockHw::default();
        s.begin_verify(7, &mut hw, false, Duration::ZERO).unwrap();
        s.record_verify(7, VerifyResult::Dead, None, &mut hw)
            .unwrap();
        // Port 8 mapped but never verified; port 9 verified alive.
        s.record_port(
            8,
            &[finding(
                Side::Usb3,
                "4",
                2,
                rec("2", &[1, 4, 2], 0x0781, 0x5581, 0, "super"),
            )],
        )
        .unwrap();
        s.record_port(
            9,
            &[
                finding(
                    Side::Usb3,
                    "",
                    2,
                    rec("2", &[1, 2], 0x0781, 0x5581, 0, "super"),
                ),
                finding(
                    Side::Usb2,
                    "",
                    2,
                    rec("1", &[3, 2], 0x0781, 0x5581, 0, "high"),
                ),
            ],
        )
        .unwrap();
        s.begin_verify(9, &mut hw, false, Duration::ZERO).unwrap();
        s.record_verify(9, VerifyResult::Alive, Some("ganged rail".into()), &mut hw)
            .unwrap();

        let profile = s.finish("rsh-st10c-6", Some("test".into())).unwrap();
        assert_eq!(profile.chips.len(), 2);
        let root = profile.chips.iter().find(|c| c.label == "root").unwrap();
        assert_eq!(root.usb3_id.as_deref(), Some("0bda:0411"));
        assert_eq!(root.usb2_id.as_deref(), Some("0bda:5411"));
        let leaf = profile.chips.iter().find(|c| c.label == "chip4").unwrap();
        assert_eq!(leaf.usb3_path.as_deref(), Some("4"));

        assert_eq!(profile.port_entry(7).unwrap().controllable, Some(true));
        assert_eq!(profile.port_entry(8).unwrap().controllable, None);
        let p9 = profile.port_entry(9).unwrap();
        assert_eq!(p9.controllable, Some(false));
        assert_eq!(p9.reason.as_deref(), Some("ganged rail"));
        assert_eq!(s.stage, Stage::Finished);
    }

    #[test]
    fn finish_blocked_with_pending_verify() {
        let mut s = mapped_session();
        let mut hw = MockHw::default();
        s.begin_verify(7, &mut hw, false, Duration::ZERO).unwrap();
        let err = s.finish("x", None).unwrap_err().to_string();
        assert!(err.contains("awaiting its verdict"), "{err}");
    }

    #[test]
    fn session_json_roundtrip() {
        let s = mapped_session();
        let text = serde_json::to_string(&s).unwrap();
        let back: Session = serde_json::from_str(&text).unwrap();
        assert_eq!(back.stage, Stage::Captured);
        assert_eq!(back.cascades.len(), 2);
        assert_eq!(back.ports.len(), 1);
    }
}
