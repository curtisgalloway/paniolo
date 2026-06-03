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

//! Power control: the FTDI DTR line (wired to the target's J2 power-button
//! header) via the serialcap daemon, with a direct-serial fallback; plus
//! power-state sensing from the daemon's `/status`.
//!
//! DTR pulse guidance (Raspberry Pi 5 / DA9091 PMIC): ≤500 ms is a power-button
//! event the OS handles (graceful reboot/halt); ≥3000 ms is a hard PMIC
//! power-off (pulse again to power back on).

use std::time::Duration;

use anyhow::{anyhow, Context, Result};

/// Assert DTR for `ms` via the running serialcap daemon (it owns the port).
pub fn dtr_press_daemon(base_url: &str, interface: &str, ms: u64) -> Result<()> {
    let url = format!("{base_url}/button?interface={interface}&ms={ms}");
    let timeout = std::cmp::max(15_000, ms + 5_000);
    ureq::post(&url)
        .timeout(Duration::from_millis(timeout))
        .send_bytes(&[])
        .map(|_| ())
        .map_err(|e| anyhow!("serialcap /button failed: {e}"))
}

/// Assert DTR for `ms` directly (fallback when the daemon isn't running).
pub fn dtr_press_direct(device: &str, ms: u64) -> Result<()> {
    let mut port = serialport::new(device, 115200)
        .timeout(Duration::from_millis(250))
        .open()
        .with_context(|| format!("opening {device} for DTR control"))?;
    port.write_data_terminal_ready(false)?;
    std::thread::sleep(Duration::from_millis(50)); // settle after open
    port.write_data_terminal_ready(true)?;
    std::thread::sleep(Duration::from_millis(ms));
    port.write_data_terminal_ready(false)?;
    Ok(())
}

/// Current power state from the daemon's sense line: Some(on) or None when the
/// sense signal isn't configured (power_on is null) or the daemon is unreachable.
pub fn read_power_state(base_url: &str, interface: &str) -> Option<bool> {
    let resp = ureq::get(&format!("{base_url}/status?interface={interface}"))
        .timeout(Duration::from_secs(2))
        .call()
        .ok()?;
    let v: serde_json::Value = resp.into_json().ok()?;
    v.get("power_on")?.as_bool()
}
