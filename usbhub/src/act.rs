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

//! The acting edge: opening live hub devices that correspond to snapshot
//! records, and the [`PortSwitch`] abstraction that lets the learn state
//! machine run against mock hardware in tests.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use nusb::MaybeFuture;

use crate::hub;
use crate::topo::{snapshot, snapshot_with_handles, DevKey, DevRecord, Side};

/// Power switching against a chip located by its snapshot record. The learn
/// state machine and the CLI commands act through this; tests mock it.
pub trait PortSwitch {
    fn set_power(&mut self, side: Side, chip: &DevRecord, port: u8, on: bool) -> Result<()>;
    fn power_is_on(&mut self, side: Side, chip: &DevRecord, port: u8) -> Result<bool>;

    /// Keys of every device currently enumerated on the bus. The verify step
    /// snapshots this before and after cutting a port's power to see whether
    /// the probe device actually dropped off the bus.
    fn live_keys(&mut self) -> Result<HashSet<DevKey>>;
}

/// Live topology snapshot plus the handles to open devices from it.
pub struct DeviceTable {
    handles: HashMap<DevKey, nusb::DeviceInfo>,
    open: HashMap<DevKey, nusb::Device>,
}

impl DeviceTable {
    /// Snapshot the live topology, returning the records for resolution and
    /// the table for acting on them.
    pub fn snapshot() -> Result<(Vec<DevRecord>, Self)> {
        let (recs, handles) = snapshot_with_handles()?;
        Ok((
            recs,
            DeviceTable {
                handles,
                open: HashMap::new(),
            },
        ))
    }

    /// Open (or reuse) the live device for a snapshot record.
    pub fn device(&mut self, rec: &DevRecord) -> Result<&nusb::Device> {
        let key = rec.key();
        if !self.open.contains_key(&key) {
            let info = self.handles.get(&key).with_context(|| {
                format!(
                    "device {} is no longer present — was the hub moved or \
                     unplugged? re-run `usbhub probe`",
                    key
                )
            })?;
            let dev = info.open().wait().with_context(|| {
                format!(
                    "opening {} (on Linux this needs a udev rule or root; see docs/power.md)",
                    key
                )
            })?;
            self.open.insert(key.clone(), dev);
        }
        Ok(&self.open[&key])
    }
}

impl PortSwitch for DeviceTable {
    fn set_power(&mut self, _side: Side, chip: &DevRecord, port: u8, on: bool) -> Result<()> {
        let dev = self.device(chip)?;
        hub::set_port_power(dev, port, on)
    }

    fn power_is_on(&mut self, side: Side, chip: &DevRecord, port: u8) -> Result<bool> {
        let dev = self.device(chip)?;
        hub::port_power_is_on(dev, side, port)
    }

    fn live_keys(&mut self) -> Result<HashSet<DevKey>> {
        Ok(snapshot()?.iter().map(DevRecord::key).collect())
    }
}
