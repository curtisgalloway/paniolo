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

//! USB topology snapshots and cascade derivation.
//!
//! Everything here operates on [`DevRecord`] — a plain, serializable
//! description of one enumerated device — so the topology logic is unit
//! testable and learn sessions persist as JSON. nusb is touched only in
//! [`snapshot`] / [`snapshot_with_handles`].

use std::collections::HashMap;
use std::fmt;

use anyhow::{bail, Result};
use nusb::MaybeFuture;
use serde::{Deserialize, Serialize};

/// USB device class code for hubs (`bDeviceClass`).
pub const USB_CLASS_HUB: u8 = 0x09;

/// Which of the two parallel topologies a device enumerated on. A USB 3 hub
/// chip is two logical devices: a SuperSpeed hub on the USB 3 topology and a
/// companion hub on the USB 2 topology, with independent port power control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Usb2,
    Usb3,
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Side::Usb2 => write!(f, "usb2"),
            Side::Usb3 => write!(f, "usb3"),
        }
    }
}

/// Identity of an enumerated device that is stable across re-enumeration at
/// the same physical location (device addresses are not).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DevKey {
    pub bus_id: String,
    pub port_chain: Vec<u8>,
    pub vid: u16,
    pub pid: u16,
}

impl fmt::Display for DevKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04x}:{:04x} at {}:{}",
            self.vid,
            self.pid,
            self.bus_id,
            chain_str(&self.port_chain)
        )
    }
}

/// Plain record of one enumerated USB device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevRecord {
    pub bus_id: String,
    pub port_chain: Vec<u8>,
    pub vid: u16,
    pub pid: u16,
    pub class: u8,
    /// Connection speed as reported by the OS ("low", "full", "high",
    /// "super", "super-plus"), if known.
    pub speed: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
}

impl DevRecord {
    pub fn key(&self) -> DevKey {
        DevKey {
            bus_id: self.bus_id.clone(),
            port_chain: self.port_chain.clone(),
            vid: self.vid,
            pid: self.pid,
        }
    }

    pub fn is_hub(&self) -> bool {
        self.class == USB_CLASS_HUB
    }

    /// Which topology this device lives on, judged by connection speed.
    pub fn side(&self) -> Option<Side> {
        match self.speed.as_deref() {
            Some("low") | Some("full") | Some("high") => Some(Side::Usb2),
            Some("super") | Some("super-plus") => Some(Side::Usb3),
            _ => None,
        }
    }

    /// Key of this device's parent position (same bus, chain minus the last
    /// element), or None for devices attached directly to a root port.
    pub fn parent_pos(&self) -> Option<(String, Vec<u8>)> {
        self.port_chain
            .split_last()
            .map(|(_, rest)| (self.bus_id.clone(), rest.to_vec()))
    }

    pub fn describe(&self) -> String {
        let mut s = format!(
            "{:04x}:{:04x} {} at {}:{}",
            self.vid,
            self.pid,
            if self.is_hub() { "hub" } else { "dev" },
            self.bus_id,
            chain_str(&self.port_chain)
        );
        if let Some(sp) = &self.speed {
            s.push_str(&format!(" [{sp}]"));
        }
        if let Some(p) = &self.product {
            s.push_str(&format!(" \"{p}\""));
        }
        if let Some(sn) = &self.serial {
            s.push_str(&format!(" serial {sn}"));
        }
        s
    }
}

/// Render a port chain as dotted numbers ("1.4.2"); root-attached devices
/// have a single element, an empty chain renders as "-".
pub fn chain_str(chain: &[u8]) -> String {
    if chain.is_empty() {
        return "-".to_string();
    }
    chain
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

/// Parse a dotted port chain ("1.4.2"); "" and "-" mean the empty chain.
pub fn parse_chain(s: &str) -> Result<Vec<u8>> {
    if s.is_empty() || s == "-" {
        return Ok(vec![]);
    }
    s.split('.')
        .map(|p| {
            p.parse::<u8>()
                .map_err(|_| anyhow::anyhow!("bad port chain element {p:?} in {s:?}"))
        })
        .collect()
}

/// One hub chip of a product cascade, located by its dotted path of port
/// numbers from the cascade root ("" = the root chip itself).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadeChip {
    pub path: String,
    pub dev: DevRecord,
}

/// The per-side tree of hub chips that arrived together when the product was
/// plugged in, plus any non-hub devices that came with it (ports that were
/// already occupied).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cascade {
    pub side: Side,
    pub chips: Vec<CascadeChip>,
    pub occupants: Vec<DevRecord>,
}

impl Cascade {
    pub fn root(&self) -> &DevRecord {
        // Constructed with the root at path "" — enforced by derive_cascades.
        &self.chips[0].dev
    }

    /// Find the chip at `path`, if present.
    pub fn chip(&self, path: &str) -> Option<&DevRecord> {
        self.chips.iter().find(|c| c.path == path).map(|c| &c.dev)
    }
}

/// Devices in `after` whose key is absent from `before`.
pub fn diff_added(before: &[DevRecord], after: &[DevRecord]) -> Vec<DevRecord> {
    let known: std::collections::HashSet<DevKey> = before.iter().map(|d| d.key()).collect();
    after
        .iter()
        .filter(|d| !known.contains(&d.key()))
        .cloned()
        .collect()
}

/// Devices in `before` whose key is absent from `after`.
pub fn diff_removed(before: &[DevRecord], after: &[DevRecord]) -> Vec<DevRecord> {
    diff_added(after, before)
}

/// Derive the per-side hub cascades from the set of devices that arrived
/// together when the product was plugged in. For each side present, the root
/// is the arrived hub whose parent position is not another arrived hub of the
/// same side; every other arrived hub gets a dotted path relative to the
/// root. Errors if a side has more than one root (two products plugged in
/// during the capture window?) or a hub that doesn't descend from the root.
pub fn derive_cascades(added: &[DevRecord]) -> Result<Vec<Cascade>> {
    let mut out = Vec::new();
    for side in [Side::Usb3, Side::Usb2] {
        let on_side: Vec<&DevRecord> = added.iter().filter(|d| d.side() == Some(side)).collect();
        if on_side.is_empty() {
            continue;
        }
        let hubs: Vec<&DevRecord> = on_side.iter().filter(|d| d.is_hub()).copied().collect();
        if hubs.is_empty() {
            continue;
        }
        let hub_pos: HashMap<(String, Vec<u8>), &DevRecord> = hubs
            .iter()
            .map(|h| ((h.bus_id.clone(), h.port_chain.clone()), *h))
            .collect();
        let roots: Vec<&DevRecord> = hubs
            .iter()
            .filter(|h| match h.parent_pos() {
                Some(pos) => !hub_pos.contains_key(&pos),
                None => true,
            })
            .copied()
            .collect();
        let root = match roots.as_slice() {
            [r] => *r,
            [] => bail!("{side}: arrived hubs form a cycle? no root found"),
            many => bail!(
                "{side}: {} unrelated hub roots arrived together — replug with only \
                 the one hub under test:\n{}",
                many.len(),
                many.iter()
                    .map(|d| format!("  {}", d.describe()))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        };

        let mut chips = vec![CascadeChip {
            path: String::new(),
            dev: root.clone(),
        }];
        for h in &hubs {
            if h.key() == root.key() {
                continue;
            }
            let rel = relative_path(root, h)?;
            chips.push(CascadeChip {
                path: rel,
                dev: (*h).clone(),
            });
        }
        chips.sort_by(|a, b| a.path.cmp(&b.path));
        // Re-establish the root-first invariant after sorting ("" sorts
        // first already, but be explicit about the contract).
        debug_assert!(chips[0].path.is_empty());

        let occupants: Vec<DevRecord> = on_side
            .iter()
            .filter(|d| !d.is_hub())
            .map(|d| (*d).clone())
            .collect();

        out.push(Cascade {
            side,
            chips,
            occupants,
        });
    }
    if out.is_empty() {
        bail!(
            "no hubs arrived — the replug was not detected (was the hub plugged \
             back into the same host?)"
        );
    }
    Ok(out)
}

/// Dotted path of `dev` relative to `root` (both on the same bus; `dev`
/// strictly below `root`).
pub fn relative_path(root: &DevRecord, dev: &DevRecord) -> Result<String> {
    if dev.bus_id != root.bus_id
        || dev.port_chain.len() <= root.port_chain.len()
        || !dev.port_chain.starts_with(&root.port_chain)
    {
        bail!(
            "device {} does not descend from root {}",
            dev.describe(),
            root.describe()
        );
    }
    Ok(chain_str(&dev.port_chain[root.port_chain.len()..]))
}

/// Given a device that arrived during a port walk, attribute it to a chip
/// port: the device (or its topmost arrived ancestor, for compound probe
/// devices that contain their own hub) must be an immediate child of one of
/// the cascade's chips. Returns (chip path, port number on that chip, and the
/// key of that topmost device — so the verify step can later check whether it
/// disappears when the port's power is cut).
pub fn attribute_arrival(cascade: &Cascade, added: &[DevRecord]) -> Option<(String, u8, DevKey)> {
    // Topmost first: shorter chains are closer to the root.
    let mut on_side: Vec<&DevRecord> = added
        .iter()
        .filter(|d| d.side() == Some(cascade.side))
        .collect();
    on_side.sort_by_key(|d| d.port_chain.len());
    for dev in on_side {
        for chip in &cascade.chips {
            let c = &chip.dev;
            if dev.bus_id == c.bus_id
                && dev.port_chain.len() == c.port_chain.len() + 1
                && dev.port_chain.starts_with(&c.port_chain)
            {
                return Some((
                    chip.path.clone(),
                    *dev.port_chain.last().unwrap(),
                    dev.key(),
                ));
            }
        }
    }
    None
}

fn speed_str(speed: Option<nusb::Speed>) -> Option<String> {
    speed.map(|s| {
        match s {
            nusb::Speed::Low => "low",
            nusb::Speed::Full => "full",
            nusb::Speed::High => "high",
            nusb::Speed::Super => "super",
            nusb::Speed::SuperPlus => "super-plus",
            _ => "unknown",
        }
        .to_string()
    })
}

fn record_of(info: &nusb::DeviceInfo) -> DevRecord {
    DevRecord {
        bus_id: info.bus_id().to_string(),
        port_chain: info.port_chain().to_vec(),
        vid: info.vendor_id(),
        pid: info.product_id(),
        class: info.class(),
        speed: speed_str(info.speed()),
        product: info.product_string().map(str::to_string),
        serial: info.serial_number().map(str::to_string),
    }
}

/// Enumerate the live USB topology as plain records.
pub fn snapshot() -> Result<Vec<DevRecord>> {
    Ok(nusb::list_devices()
        .wait()?
        .map(|i| record_of(&i))
        .collect())
}

/// Enumerate as records plus a handle table for opening devices by key.
pub fn snapshot_with_handles() -> Result<(Vec<DevRecord>, HashMap<DevKey, nusb::DeviceInfo>)> {
    let mut recs = Vec::new();
    let mut handles = HashMap::new();
    for info in nusb::list_devices().wait()? {
        let rec = record_of(&info);
        handles.insert(rec.key(), info);
        recs.push(rec);
    }
    Ok((recs, handles))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn rec(bus: &str, chain: &[u8], vid: u16, pid: u16, class: u8, speed: &str) -> DevRecord {
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

    /// The RSH-ST10C-6 shape: USB3 root+leaf (0bda:0411) on bus "2", USB2
    /// companions (0bda:5411) on bus "1", leaf hanging off root port 4.
    pub fn rsh_arrival() -> Vec<DevRecord> {
        vec![
            rec("2", &[1], 0x0bda, 0x0411, 9, "super"),
            rec("2", &[1, 4], 0x0bda, 0x0411, 9, "super"),
            rec("1", &[3], 0x0bda, 0x5411, 9, "high"),
            rec("1", &[3, 4], 0x0bda, 0x5411, 9, "high"),
        ]
    }

    #[test]
    fn cascade_derivation_two_sides() {
        let cascades = derive_cascades(&rsh_arrival()).unwrap();
        assert_eq!(cascades.len(), 2);
        let usb3 = cascades.iter().find(|c| c.side == Side::Usb3).unwrap();
        assert_eq!(usb3.root().port_chain, vec![1]);
        assert_eq!(usb3.chips.len(), 2);
        assert_eq!(usb3.chips[1].path, "4");
        let usb2 = cascades.iter().find(|c| c.side == Side::Usb2).unwrap();
        assert_eq!(usb2.root().port_chain, vec![3]);
        assert_eq!(usb2.chips[1].path, "4");
    }

    #[test]
    fn cascade_includes_occupants() {
        let mut added = rsh_arrival();
        // A serial adapter that was already plugged into the hub.
        added.push(rec("1", &[3, 2], 0x0403, 0x6001, 0, "full"));
        let cascades = derive_cascades(&added).unwrap();
        let usb2 = cascades.iter().find(|c| c.side == Side::Usb2).unwrap();
        assert_eq!(usb2.occupants.len(), 1);
        assert_eq!(usb2.occupants[0].vid, 0x0403);
    }

    #[test]
    fn cascade_rejects_two_roots() {
        let mut added = rsh_arrival();
        added.push(rec("2", &[7], 0x05e3, 0x0626, 9, "super"));
        let err = derive_cascades(&added).unwrap_err().to_string();
        assert!(err.contains("2 unrelated hub roots"), "{err}");
    }

    #[test]
    fn cascade_empty_diff_errors() {
        assert!(derive_cascades(&[]).is_err());
    }

    #[test]
    fn arrival_attribution_simple() {
        let cascades = derive_cascades(&rsh_arrival()).unwrap();
        let usb3 = cascades.iter().find(|c| c.side == Side::Usb3).unwrap();
        // Probe flash drive appears below the leaf chip, port 2.
        let probe = rec("2", &[1, 4, 2], 0x0781, 0x5581, 0, "super");
        let added = vec![probe.clone()];
        assert_eq!(
            attribute_arrival(usb3, &added),
            Some(("4".to_string(), 2, probe.key()))
        );
    }

    #[test]
    fn arrival_attribution_compound_probe() {
        // A probe with its own internal hub: the inner hub lands on chip
        // port 3, its child below it. Attribution must pick the topmost.
        let cascades = derive_cascades(&rsh_arrival()).unwrap();
        let usb3 = cascades.iter().find(|c| c.side == Side::Usb3).unwrap();
        let inner_hub = rec("2", &[1, 4, 3], 0x05e3, 0x0626, 9, "super");
        let added = vec![
            rec("2", &[1, 4, 3, 1], 0x046d, 0xc52b, 0, "super"),
            inner_hub.clone(),
        ];
        assert_eq!(
            attribute_arrival(usb3, &added),
            Some(("4".to_string(), 3, inner_hub.key()))
        );
    }

    #[test]
    fn arrival_attribution_root_port() {
        let cascades = derive_cascades(&rsh_arrival()).unwrap();
        let usb3 = cascades.iter().find(|c| c.side == Side::Usb3).unwrap();
        // Probe directly on the root chip, port 6.
        let probe = rec("2", &[1, 6], 0x0781, 0x5581, 0, "super");
        let added = vec![probe.clone()];
        assert_eq!(
            attribute_arrival(usb3, &added),
            Some(("".to_string(), 6, probe.key()))
        );
    }

    #[test]
    fn arrival_elsewhere_is_none() {
        let cascades = derive_cascades(&rsh_arrival()).unwrap();
        let usb3 = cascades.iter().find(|c| c.side == Side::Usb3).unwrap();
        // Device appeared on a different bus position entirely.
        let added = vec![rec("2", &[5], 0x0781, 0x5581, 0, "super")];
        assert_eq!(attribute_arrival(usb3, &added), None);
    }

    #[test]
    fn diff_roundtrip() {
        let a = rsh_arrival();
        let mut b = a.clone();
        let extra = rec("2", &[1, 4, 2], 0x0781, 0x5581, 0, "super");
        b.push(extra.clone());
        assert_eq!(diff_added(&a, &b), vec![extra.clone()]);
        assert_eq!(diff_removed(&b, &a), vec![extra]);
    }

    #[test]
    fn chain_parse_roundtrip() {
        assert_eq!(parse_chain("1.4.2").unwrap(), vec![1, 4, 2]);
        assert_eq!(parse_chain("").unwrap(), Vec::<u8>::new());
        assert_eq!(chain_str(&[1, 4, 2]), "1.4.2");
        assert!(parse_chain("1.x").is_err());
    }
}
