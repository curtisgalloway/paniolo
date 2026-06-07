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

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::act::PortSwitch;
use crate::profile::{format_anchor, ChipEntry, ModelMeta, PortEntry, Profile, SidePort};
use crate::topo::{
    attribute_arrival, derive_cascades, diff_added, diff_removed, Cascade, DevRecord, Side,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<VerifyResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
}

/// Outcome of one port-walk poll.
#[derive(Debug, PartialEq)]
pub enum WalkOutcome {
    /// Nothing new yet — keep polling.
    Nothing,
    /// The probe arrived on one or more sides under the cascade's chips.
    /// (A USB 3 probe hub maps both sides in one plug.)
    Found(Vec<(Side, SidePort)>),
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
            if let Some((path, port)) = attribute_arrival(cascade, &added) {
                found.push((cascade.side, SidePort { path, port }));
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
    pub fn record_port(&mut self, physical: u16, findings: &[(Side, SidePort)]) -> Result<String> {
        self.expect_stage(Stage::Captured, "run the capture steps first")?;
        let entry = self.ports.entry(physical).or_default();
        let mut msg = format!("Physical port {physical}:\n");
        for (side, sp) in findings {
            msg.push_str(&format!(
                "  [{side}] chip path {:?}, chip port {}\n",
                sp.path, sp.port
            ));
            match side {
                Side::Usb3 => entry.usb3 = Some(sp.clone()),
                Side::Usb2 => entry.usb2 = Some(sp.clone()),
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
        msg.push_str(&format!(
            "Look at the probe device in port {physical}: did it actually lose \
             power (LED off / dead)? The status bit above is the chip's claim, \
             not the truth.\n\
             Next: `usbhub learn verify {physical} --result dead` or \
             `--result alive [--reason \"...\"]`"
        ));
        Ok(msg)
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

        let entry = self.ports.get_mut(&physical).expect("mapped_sides checked");
        entry.result = Some(result);
        entry.reason = match (&result, reason) {
            (VerifyResult::Alive, None) => {
                Some("probe stayed powered when PORT_POWER was cleared".to_string())
            }
            (_, r) => r,
        };
        self.pending_verify = None;

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
    use std::collections::HashMap;

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

    /// Mock hardware: records (side, chip vid:pid+chain, port, on) calls and
    /// reports the last commanded state as the status bit.
    #[derive(Default)]
    struct MockHw {
        state: HashMap<(Side, Vec<u8>, u8), bool>,
        calls: Vec<String>,
        fail_restore: bool,
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
                (
                    Side::Usb3,
                    SidePort {
                        path: "4".into(),
                        port: 1,
                    },
                ),
                (
                    Side::Usb2,
                    SidePort {
                        path: "4".into(),
                        port: 1,
                    },
                ),
            ],
        )
        .unwrap();
        s
    }

    #[test]
    fn verify_dead_records_controllable_and_restores() {
        let mut s = mapped_session();
        let mut hw = MockHw::default();
        let msg = s.begin_verify(7, &mut hw, false).unwrap();
        assert!(msg.contains("status bit now reads off"), "{msg}");
        assert_eq!(s.pending_verify, Some(7));
        assert_eq!(hw.calls, vec!["usb3 1.4 p1 off", "usb2 3.4 p1 off"]);

        let msg = s
            .record_verify(7, VerifyResult::Dead, None, &mut hw)
            .unwrap();
        assert!(msg.contains("Power restored"), "{msg}");
        assert_eq!(s.pending_verify, None);
        assert_eq!(s.ports[&7].result, Some(VerifyResult::Dead));
        assert_eq!(hw.calls.len(), 4, "two offs then two ons");
    }

    #[test]
    fn verify_alive_records_reason() {
        let mut s = mapped_session();
        let mut hw = MockHw::default();
        s.begin_verify(7, &mut hw, false).unwrap();
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
        s.record_port(
            7,
            &[(
                Side::Usb3,
                SidePort {
                    path: "4".into(),
                    port: 1,
                },
            )],
        )
        .unwrap();
        let mut hw = MockHw::default();
        let err = s.begin_verify(7, &mut hw, false).unwrap_err().to_string();
        assert!(err.contains("only one side"), "{err}");
        assert!(s.begin_verify(7, &mut hw, true).is_ok());
    }

    #[test]
    fn second_verify_blocked_while_pending() {
        let mut s = mapped_session();
        s.record_port(
            8,
            &[
                (
                    Side::Usb3,
                    SidePort {
                        path: "4".into(),
                        port: 2,
                    },
                ),
                (
                    Side::Usb2,
                    SidePort {
                        path: "4".into(),
                        port: 2,
                    },
                ),
            ],
        )
        .unwrap();
        let mut hw = MockHw::default();
        s.begin_verify(7, &mut hw, false).unwrap();
        let err = s.begin_verify(8, &mut hw, false).unwrap_err().to_string();
        assert!(err.contains("still powered off"), "{err}");
    }

    #[test]
    fn restore_failure_is_loud() {
        let mut s = mapped_session();
        let mut hw = MockHw::default();
        s.begin_verify(7, &mut hw, false).unwrap();
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
        s.begin_verify(7, &mut hw, false).unwrap();
        s.record_verify(7, VerifyResult::Dead, None, &mut hw)
            .unwrap();
        // Port 8 mapped but never verified; port 9 verified alive.
        s.record_port(
            8,
            &[(
                Side::Usb3,
                SidePort {
                    path: "4".into(),
                    port: 2,
                },
            )],
        )
        .unwrap();
        s.record_port(
            9,
            &[
                (
                    Side::Usb3,
                    SidePort {
                        path: String::new(),
                        port: 2,
                    },
                ),
                (
                    Side::Usb2,
                    SidePort {
                        path: String::new(),
                        port: 2,
                    },
                ),
            ],
        )
        .unwrap();
        s.begin_verify(9, &mut hw, false).unwrap();
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
        s.begin_verify(7, &mut hw, false).unwrap();
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
