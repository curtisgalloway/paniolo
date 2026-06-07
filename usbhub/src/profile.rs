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

//! Hub model profiles: what a hub product *is* (its internal chip cascade)
//! and what a human has *verified* about it (physical-port mappings and
//! controllability assertions).
//!
//! The contract: every `controllable` flag is a human-reported assertion,
//! produced by physically watching a probe device lose power (`usbhub
//! learn`) or hand-written by someone who knows. The tool never infers
//! controllability from descriptors. A physical port with no entry — or an
//! entry without `controllable = true` — is refused for switching.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::topo::{chain_str, parse_chain, DevRecord, Side};

/// "0bda:0411"-style VID:PID pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbId {
    pub vid: u16,
    pub pid: u16,
}

impl UsbId {
    pub fn parse(s: &str) -> Result<Self> {
        let (v, p) = s
            .split_once(':')
            .with_context(|| format!("bad VID:PID {s:?} (want hhhh:hhhh)"))?;
        Ok(UsbId {
            vid: u16::from_str_radix(v, 16).with_context(|| format!("bad VID in {s:?}"))?,
            pid: u16::from_str_radix(p, 16).with_context(|| format!("bad PID in {s:?}"))?,
        })
    }

    pub fn matches(&self, d: &DevRecord) -> bool {
        d.vid == self.vid && d.pid == self.pid
    }
}

impl fmt::Display for UsbId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04x}:{:04x}", self.vid, self.pid)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMeta {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One hub chip of the product, with its position in each topology. A chip
/// missing from one side (e.g. a USB 2-only product) leaves those fields out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChipEntry {
    /// Human label ("root", "chip4") — cosmetic only.
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb3_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb3_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb2_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb2_id: Option<String>,
}

impl ChipEntry {
    fn side_sig(&self, side: Side) -> Option<(String, UsbId)> {
        let (path, id) = match side {
            Side::Usb3 => (self.usb3_path.as_ref()?, self.usb3_id.as_ref()?),
            Side::Usb2 => (self.usb2_path.as_ref()?, self.usb2_id.as_ref()?),
        };
        UsbId::parse(id).ok().map(|id| (path.clone(), id))
    }
}

/// A physical port's location on one side: which chip (by dotted path from
/// the cascade root) and which port number on that chip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SidePort {
    pub path: String,
    pub port: u8,
}

/// One physical (silkscreen) port of the product.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortEntry {
    pub physical: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb3: Option<SidePort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb2: Option<SidePort>,
    /// Human-verified assertion: `true` = watching a probe device confirmed
    /// the port actually cuts power; `false` = confirmed it does NOT (see
    /// `reason`); absent = never verified — refused for switching.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controllable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl PortEntry {
    pub fn side(&self, side: Side) -> Option<&SidePort> {
        match side {
            Side::Usb3 => self.usb3.as_ref(),
            Side::Usb2 => self.usb2.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub model: ModelMeta,
    #[serde(default, rename = "chip")]
    pub chips: Vec<ChipEntry>,
    #[serde(default, rename = "port")]
    pub ports: Vec<PortEntry>,
}

impl Profile {
    pub fn port_entry(&self, physical: u16) -> Option<&PortEntry> {
        self.ports.iter().find(|p| p.physical == physical)
    }

    /// Per-side signature: (path from root, expected VID:PID) for every chip
    /// present on that side. Empty if the product has no presence there.
    pub fn side_signature(&self, side: Side) -> Vec<(String, UsbId)> {
        self.chips.iter().filter_map(|c| c.side_sig(side)).collect()
    }

    pub fn sides(&self) -> Vec<Side> {
        [Side::Usb3, Side::Usb2]
            .into_iter()
            .filter(|s| !self.side_signature(*s).is_empty())
            .collect()
    }

    /// Refuse-unless-asserted gate for switching operations.
    pub fn check_switchable(&self, physical: u16) -> Result<&PortEntry> {
        let entry = self.port_entry(physical).with_context(|| {
            format!(
                "physical port {physical} has no entry in model {:?} — \
                 nobody has verified it; map and verify it with `usbhub learn`",
                self.model.name
            )
        })?;
        match entry.controllable {
            Some(true) => Ok(entry),
            Some(false) => bail!(
                "physical port {physical} is marked not controllable: {}",
                entry.reason.as_deref().unwrap_or("no reason recorded")
            ),
            None => bail!(
                "physical port {physical} is mapped but its power control was \
                 never verified — verify it with `usbhub learn`, or set \
                 `controllable = true` in the profile if you have verified it \
                 by hand"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Profile storage
// ---------------------------------------------------------------------------

/// Durable state dir: $PANIOLO_STATE_DIR (set by paniolo for hook and
/// `paniolo helper` invocations), with the literal fallback for standalone
/// runs, per the helper contract in docs/adding-power-helpers.md.
pub fn state_dir() -> PathBuf {
    if let Ok(d) = std::env::var("PANIOLO_STATE_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".config/paniolo/helpers/usbhub")
}

pub fn profiles_dir(override_dir: Option<&Path>) -> PathBuf {
    match override_dir {
        Some(d) => d.to_path_buf(),
        None => state_dir().join("profiles"),
    }
}

pub fn load_profile(dir: &Path, model: &str) -> Result<Profile> {
    let path = dir.join(format!("{model}.toml"));
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("no profile for model {model:?} at {}", path.display()))?;
    let profile: Profile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(profile)
}

pub fn save_profile(dir: &Path, profile: &Profile) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating profile dir {}", dir.display()))?;
    let path = dir.join(format!("{}.toml", profile.model.name));
    let text = to_toml(profile)?;
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

pub fn to_toml(profile: &Profile) -> Result<String> {
    Ok(toml::to_string_pretty(profile)?)
}

pub fn list_models(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "toml") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(stem.to_string());
                }
            }
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Instance resolution
// ---------------------------------------------------------------------------

/// `--at` pin: per-side anchor (bus id, port chain) for disambiguating
/// multiple attached instances of the same model.
/// Syntax: `usb3=BUS:CHAIN[,usb2=BUS:CHAIN]`, e.g. `usb3=2:1.4,usb2=1:3.4`.
pub type AtSpec = HashMap<Side, (String, Vec<u8>)>;

pub fn parse_at(s: &str) -> Result<AtSpec> {
    let mut out = HashMap::new();
    for part in s.split(',') {
        let (side, rest) = part
            .split_once('=')
            .with_context(|| format!("bad --at element {part:?} (want side=bus:chain)"))?;
        let side = match side.trim() {
            "usb3" => Side::Usb3,
            "usb2" => Side::Usb2,
            other => bail!("bad --at side {other:?} (want usb3 or usb2)"),
        };
        let (bus, chain) = rest
            .split_once(':')
            .with_context(|| format!("bad --at anchor {rest:?} (want bus:chain)"))?;
        out.insert(side, (bus.trim().to_string(), parse_chain(chain.trim())?));
    }
    Ok(out)
}

pub fn format_anchor(side: Side, dev: &DevRecord) -> String {
    format!("{side}={}:{}", dev.bus_id, chain_str(&dev.port_chain))
}

/// A resolved side: the live device record for every chip path of the model.
#[derive(Debug, Clone)]
pub struct SideInstance {
    pub chips: HashMap<String, DevRecord>,
}

impl SideInstance {
    pub fn root(&self) -> &DevRecord {
        &self.chips[""]
    }
}

/// Resolution outcome across sides; sides that failed carry their error so
/// read-only commands can degrade and switching commands can refuse loudly.
#[derive(Debug, Default)]
pub struct Instance {
    pub sides: HashMap<Side, SideInstance>,
    pub errors: HashMap<Side, String>,
}

impl Instance {
    pub fn side(&self, side: Side) -> Result<&SideInstance> {
        self.sides.get(&side).ok_or_else(|| {
            anyhow::anyhow!(
                "{side} side not resolved: {}",
                self.errors
                    .get(&side)
                    .map(String::as_str)
                    .unwrap_or("model has no chips on this side")
            )
        })
    }
}

/// Find the model's chip cascade in a topology snapshot, signature-first:
/// a candidate root must have every chip of the side's signature at its
/// expected relative path. Exactly one candidate resolves; several demand an
/// `--at` pin; the ambiguity error lists ready-to-paste pins.
pub fn resolve(profile: &Profile, snapshot: &[DevRecord], at: &AtSpec) -> Instance {
    let mut inst = Instance::default();
    for side in profile.sides() {
        match resolve_side(profile, snapshot, side, at.get(&side)) {
            Ok(si) => {
                inst.sides.insert(side, si);
            }
            Err(e) => {
                inst.errors.insert(side, format!("{e:#}"));
            }
        }
    }
    inst
}

fn resolve_side(
    profile: &Profile,
    snapshot: &[DevRecord],
    side: Side,
    pin: Option<&(String, Vec<u8>)>,
) -> Result<SideInstance> {
    let sig = profile.side_signature(side);
    let root_id = sig
        .iter()
        .find(|(path, _)| path.is_empty())
        .map(|(_, id)| *id)
        .with_context(|| format!("model {:?} has no root chip on {side}", profile.model.name))?;

    let by_pos: HashMap<(String, Vec<u8>), &DevRecord> = snapshot
        .iter()
        .map(|d| ((d.bus_id.clone(), d.port_chain.clone()), d))
        .collect();

    let candidates: Vec<&DevRecord> = snapshot
        .iter()
        .filter(|d| d.is_hub() && d.side() == Some(side) && root_id.matches(d))
        .filter(|d| match pin {
            Some((bus, chain)) => d.bus_id == *bus && d.port_chain == *chain,
            None => true,
        })
        .collect();

    if candidates.is_empty() {
        match pin {
            Some((bus, chain)) => bail!(
                "no {side} hub {root_id} at pinned anchor {bus}:{} — \
                 was the hub moved or unplugged? re-run `usbhub probe`",
                chain_str(chain)
            ),
            None => bail!("no {side} hub matching {root_id} found"),
        }
    }

    let mut resolved: Vec<SideInstance> = Vec::new();
    let mut rejects: Vec<String> = Vec::new();
    for root in candidates {
        match collect_chips(&sig, root, &by_pos, side) {
            Ok(chips) => resolved.push(SideInstance { chips }),
            Err(e) => rejects.push(format!("  {} — {e:#}", root.describe())),
        }
    }

    match resolved.len() {
        1 => Ok(resolved.pop().unwrap()),
        0 => bail!(
            "{side}: hubs matching {root_id} found, but none carries the full \
             {:?} chip cascade:\n{}",
            profile.model.name,
            rejects.join("\n")
        ),
        _ => bail!(
            "{side}: {} instances of model {:?} attached — pin one with --at:\n{}",
            resolved.len(),
            profile.model.name,
            resolved
                .iter()
                .map(|si| format!("  --at {}", format_anchor(side, si.root())))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

fn collect_chips(
    sig: &[(String, UsbId)],
    root: &DevRecord,
    by_pos: &HashMap<(String, Vec<u8>), &DevRecord>,
    side: Side,
) -> Result<HashMap<String, DevRecord>> {
    let mut chips = HashMap::new();
    for (path, id) in sig {
        let mut chain = root.port_chain.clone();
        chain.extend(parse_chain(path)?);
        let dev = by_pos
            .get(&(root.bus_id.clone(), chain.clone()))
            .filter(|d| d.is_hub() && d.side() == Some(side) && id.matches(d))
            .with_context(|| {
                format!(
                    "expected chip {id} at internal path {:?} ({}:{}), not present",
                    path,
                    root.bus_id,
                    chain_str(&chain)
                )
            })?;
        chips.insert(path.clone(), (*dev).clone());
    }
    Ok(chips)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn rsh_profile() -> Profile {
        Profile {
            model: ModelMeta {
                name: "rsh-st10c-6".to_string(),
                description: None,
            },
            chips: vec![
                ChipEntry {
                    label: "root".to_string(),
                    usb3_path: Some(String::new()),
                    usb3_id: Some("0bda:0411".to_string()),
                    usb2_path: Some(String::new()),
                    usb2_id: Some("0bda:5411".to_string()),
                },
                ChipEntry {
                    label: "chip4".to_string(),
                    usb3_path: Some("4".to_string()),
                    usb3_id: Some("0bda:0411".to_string()),
                    usb2_path: Some("4".to_string()),
                    usb2_id: Some("0bda:5411".to_string()),
                },
            ],
            ports: vec![
                PortEntry {
                    physical: 7,
                    usb3: Some(SidePort {
                        path: "4".to_string(),
                        port: 1,
                    }),
                    usb2: Some(SidePort {
                        path: "4".to_string(),
                        port: 1,
                    }),
                    controllable: Some(true),
                    reason: None,
                    note: None,
                },
                PortEntry {
                    physical: 1,
                    usb3: Some(SidePort {
                        path: String::new(),
                        port: 1,
                    }),
                    usb2: Some(SidePort {
                        path: String::new(),
                        port: 1,
                    }),
                    controllable: Some(false),
                    reason: Some("shared VBUS rail".to_string()),
                    note: None,
                },
                PortEntry {
                    physical: 8,
                    usb3: Some(SidePort {
                        path: "4".to_string(),
                        port: 2,
                    }),
                    usb2: Some(SidePort {
                        path: "4".to_string(),
                        port: 2,
                    }),
                    controllable: None,
                    reason: None,
                    note: None,
                },
            ],
        }
    }

    fn one_instance() -> Vec<DevRecord> {
        vec![
            rec("2", &[1], 0x0bda, 0x0411, 9, "super"),
            rec("2", &[1, 4], 0x0bda, 0x0411, 9, "super"),
            rec("1", &[3], 0x0bda, 0x5411, 9, "high"),
            rec("1", &[3, 4], 0x0bda, 0x5411, 9, "high"),
            // Noise: unrelated devices and hubs.
            rec("1", &[5], 0x05e3, 0x0610, 9, "high"),
            rec("2", &[1, 4, 2], 0x0781, 0x5581, 0, "super"),
        ]
    }

    #[test]
    fn resolves_single_instance() {
        let inst = resolve(&rsh_profile(), &one_instance(), &AtSpec::new());
        assert!(inst.errors.is_empty(), "{:?}", inst.errors);
        let usb3 = inst.side(Side::Usb3).unwrap();
        assert_eq!(usb3.root().port_chain, vec![1]);
        assert_eq!(usb3.chips["4"].port_chain, vec![1, 4]);
        let usb2 = inst.side(Side::Usb2).unwrap();
        assert_eq!(usb2.root().port_chain, vec![3]);
    }

    #[test]
    fn leaf_does_not_resolve_as_root() {
        // The leaf chip matches the root VID:PID but has no chip below it at
        // path "4", so the signature check must reject it.
        let inst = resolve(&rsh_profile(), &one_instance(), &AtSpec::new());
        let usb3 = inst.side(Side::Usb3).unwrap();
        assert_eq!(
            usb3.root().port_chain,
            vec![1],
            "root must be the outer chip"
        );
    }

    #[test]
    fn two_instances_demand_pin() {
        let mut snap = one_instance();
        snap.push(rec("2", &[7], 0x0bda, 0x0411, 9, "super"));
        snap.push(rec("2", &[7, 4], 0x0bda, 0x0411, 9, "super"));
        let inst = resolve(&rsh_profile(), &snap, &AtSpec::new());
        let err = inst.side(Side::Usb3).unwrap_err().to_string();
        assert!(err.contains("--at usb3=2:1"), "{err}");
        assert!(err.contains("--at usb3=2:7"), "{err}");
    }

    #[test]
    fn pin_selects_instance() {
        let mut snap = one_instance();
        snap.push(rec("2", &[7], 0x0bda, 0x0411, 9, "super"));
        snap.push(rec("2", &[7, 4], 0x0bda, 0x0411, 9, "super"));
        let at = parse_at("usb3=2:7").unwrap();
        let inst = resolve(&rsh_profile(), &snap, &at);
        let usb3 = inst.side(Side::Usb3).unwrap();
        assert_eq!(usb3.root().port_chain, vec![7]);
    }

    #[test]
    fn missing_chip_is_reported() {
        let snap = vec![rec("2", &[1], 0x0bda, 0x0411, 9, "super")];
        let inst = resolve(&rsh_profile(), &snap, &AtSpec::new());
        let err = inst.side(Side::Usb3).unwrap_err().to_string();
        assert!(err.contains("internal path \"4\""), "{err}");
    }

    #[test]
    fn switchable_gate() {
        let p = rsh_profile();
        assert!(p.check_switchable(7).is_ok());
        let e = p.check_switchable(1).unwrap_err().to_string();
        assert!(e.contains("shared VBUS rail"), "{e}");
        let e = p.check_switchable(8).unwrap_err().to_string();
        assert!(e.contains("never verified"), "{e}");
        let e = p.check_switchable(9).unwrap_err().to_string();
        assert!(e.contains("no entry"), "{e}");
    }

    #[test]
    fn toml_roundtrip() {
        let text = to_toml(&rsh_profile()).unwrap();
        let back: Profile = toml::from_str(&text).unwrap();
        assert_eq!(back.model.name, "rsh-st10c-6");
        assert_eq!(back.chips.len(), 2);
        assert_eq!(back.ports.len(), 3);
        assert_eq!(back.port_entry(7).unwrap().controllable, Some(true));
        assert_eq!(
            back.port_entry(1).unwrap().reason.as_deref(),
            Some("shared VBUS rail")
        );
    }

    #[test]
    fn at_parse() {
        let at = parse_at("usb3=2:1.4,usb2=1:3.4").unwrap();
        assert_eq!(at[&Side::Usb3], ("2".to_string(), vec![1, 4]));
        assert_eq!(at[&Side::Usb2], ("1".to_string(), vec![3, 4]));
        assert!(parse_at("nonsense").is_err());
        assert!(parse_at("usb4=2:1").is_err());
    }
}
