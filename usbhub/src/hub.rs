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

//! Hub-class control requests: per-port power switching and status.
//!
//! These are the standard requests from the USB hub class specification —
//! the same ones uhubctl issues. On Unix no interface is claimed and the OS hub
//! driver keeps running; Windows has no such route (see [`ControlHandle`]).

use std::time::Duration;

use anyhow::{Context, Result};
use nusb::transfer::{ControlIn, ControlOut, ControlType, Recipient};
use nusb::MaybeFuture;

use crate::topo::Side;

const REQ_GET_DESCRIPTOR: u8 = 0x06;
const REQ_GET_STATUS: u8 = 0x00;
const REQ_CLEAR_FEATURE: u8 = 0x01;
const REQ_SET_FEATURE: u8 = 0x03;
/// PORT_POWER feature selector (hub class).
const PORT_POWER: u16 = 8;
/// Hub descriptor types: USB 2 hub vs SuperSpeed hub.
const DESC_HUB_USB2: u16 = 0x29;
const DESC_HUB_USB3: u16 = 0x2a;
const TIMEOUT: Duration = Duration::from_millis(1000);

/// A device handle that can issue control transfers.
///
/// nusb exposes `Device::control_in`/`control_out` only on Linux, macOS and
/// Android. On Windows the default control endpoint is reachable only through a
/// *claimed interface*, because WinUSB binds per interface rather than per
/// device — so the two platforms need different handles for the same request.
/// This type is that difference, and nothing above it needs to care.
///
/// Note what claiming implies on Windows: an interface can only be claimed from
/// a driver that permits it, and USB hubs are owned by Microsoft's `usbhub.sys`,
/// which does not. So the Windows path compiles and is structurally correct, but
/// `open` is expected to fail against a real hub until a hub-driver route
/// (`IOCTL_USB_HUB_CYCLE_PORT` or similar) is written. It is not a tested
/// hardware path — see AGENTS.md under **Platform support**.
pub struct ControlHandle {
    #[cfg(not(windows))]
    inner: nusb::Device,
    #[cfg(windows)]
    inner: nusb::Interface,
}

impl ControlHandle {
    /// Wrap an opened device in a handle able to issue control transfers.
    #[cfg(not(windows))]
    pub fn open(device: nusb::Device) -> Result<Self> {
        Ok(ControlHandle { inner: device })
    }

    #[cfg(windows)]
    pub fn open(device: nusb::Device) -> Result<Self> {
        let inner = device.claim_interface(0).wait().context(
            "claiming interface 0 for hub-class control transfers (Windows routes \
             control transfers through a claimed interface; a hub bound to usbhub.sys \
             will refuse this)",
        )?;
        Ok(ControlHandle { inner })
    }

    fn control_in(
        &self,
        data: ControlIn,
        timeout: Duration,
    ) -> Result<Vec<u8>, nusb::transfer::TransferError> {
        self.inner.control_in(data, timeout).wait()
    }

    fn control_out(
        &self,
        data: ControlOut,
        timeout: Duration,
    ) -> Result<(), nusb::transfer::TransferError> {
        self.inner.control_out(data, timeout).wait().map(|_| ())
    }
}

/// The PORT_POWER status bit position differs between the two topologies:
/// bit 8 on USB 2 hubs, bit 9 on SuperSpeed hubs.
pub fn power_bit(side: Side) -> u16 {
    match side {
        Side::Usb2 => 0x0100,
        Side::Usb3 => 0x0200,
    }
}

/// Hub-class GetPortStatus: the wPortStatus word for `port` (1-based).
pub fn port_status(device: &ControlHandle, port: u8) -> Result<u16> {
    let data = device
        .control_in(
            ControlIn {
                control_type: ControlType::Class,
                recipient: Recipient::Other,
                request: REQ_GET_STATUS,
                value: 0,
                index: port as u16,
                length: 4,
            },
            TIMEOUT,
        )
        .with_context(|| format!("GetPortStatus for port {port}"))?;
    if data.len() < 2 {
        anyhow::bail!(
            "GetPortStatus for port {port}: short response ({} bytes)",
            data.len()
        );
    }
    Ok(u16::from_le_bytes([data[0], data[1]]))
}

/// Set or clear PORT_POWER on `port` (1-based).
pub fn set_port_power(device: &ControlHandle, port: u8, on: bool) -> Result<()> {
    let request = if on {
        REQ_SET_FEATURE
    } else {
        REQ_CLEAR_FEATURE
    };
    device
        .control_out(
            ControlOut {
                control_type: ControlType::Class,
                recipient: Recipient::Other,
                request,
                value: PORT_POWER,
                index: port as u16,
                data: &[],
            },
            TIMEOUT,
        )
        .with_context(|| {
            format!(
                "{} PORT_POWER for port {port}",
                if on { "SetFeature" } else { "ClearFeature" }
            )
        })?;
    Ok(())
}

/// Whether the PORT_POWER status bit reads as on for `port`.
pub fn port_power_is_on(device: &ControlHandle, side: Side, port: u8) -> Result<bool> {
    Ok(port_status(device, port)? & power_bit(side) != 0)
}

/// What the hub descriptor *claims* about power switching. A hint for
/// humans, never a substitute for physical verification — chips routinely
/// claim per-port switching with no VBUS MOSFETs behind it.
#[derive(Debug, Clone)]
pub struct HubDescInfo {
    pub nbr_ports: u8,
    pub power_switching: &'static str,
}

/// Read the hub descriptor (type 0x29 or 0x2a by side) and summarize it.
pub fn hub_descriptor(device: &ControlHandle, side: Side) -> Result<HubDescInfo> {
    let desc_type = match side {
        Side::Usb2 => DESC_HUB_USB2,
        Side::Usb3 => DESC_HUB_USB3,
    };
    let data = device
        .control_in(
            ControlIn {
                control_type: ControlType::Class,
                recipient: Recipient::Device,
                request: REQ_GET_DESCRIPTOR,
                value: desc_type << 8,
                index: 0,
                length: 12,
            },
            TIMEOUT,
        )
        .context("GetDescriptor (hub)")?;
    if data.len() < 5 {
        anyhow::bail!("hub descriptor: short response ({} bytes)", data.len());
    }
    // Both layouts: bNbrPorts at offset 2, wHubCharacteristics at 3..5.
    let characteristics = u16::from_le_bytes([data[3], data[4]]);
    let power_switching = match characteristics & 0x0003 {
        0b00 => "ganged",
        0b01 => "per-port (claimed)",
        _ => "none",
    };
    Ok(HubDescInfo {
        nbr_ports: data[2],
        power_switching,
    })
}
