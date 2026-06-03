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

//! Hardware discovery for lab authoring.
//!
//! `paniolo discover` lists this host's lab-relevant hardware (USB-Ethernet,
//! serial, capture devices); `paniolo configure` runs it over SSH on a lab host
//! and renders a proposed `[targets.<name>]` block for review. The proposal is
//! never written — the human approves it by adding it to the lab and committing.

use serde_json::{json, Value};

use crate::{daemons, netif, serial, video};

/// Capture-device names that are built-in cameras, not HDMI capture.
const BUILTIN_CAPTURE: [&str; 5] = ["FaceTime", "Capture screen", "iSight", "iPhone", "iPad"];

/// This host's lab-relevant hardware, in the same JSON shape the Python CLI
/// emits (so mixed-version labs interoperate during the migration).
pub fn local_inventory() -> Value {
    let ethernet: Vec<Value> = netif::list_usb_ethernet_interfaces()
        .iter()
        .map(|e| json!({"port": e.port, "device": e.device, "active": e.active}))
        .collect();
    let serial: Vec<Value> = serial::list_devices()
        .into_iter()
        .map(Value::String)
        .collect();
    let video: Vec<Value> = list_capture_devices()
        .into_iter()
        .map(|d| json!({"index": d.0, "name": d.1, "misc": d.2}))
        .collect();
    json!({"ethernet": ethernet, "serial": serial, "video": video})
}

/// Parse `hdmicap devices` output: `  0  Name  [misc]` per line.
fn list_capture_devices() -> Vec<(u64, String, String)> {
    let Some(binary) = daemons::find_binary(video::DAEMON) else {
        return Vec::new();
    };
    let Ok(out) = std::process::Command::new(binary).arg("devices").output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut devices = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let s = line.trim_start();
        let Some((idx_str, rest)) = s.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(index) = idx_str.parse::<u64>() else {
            continue;
        };
        let rest = rest.trim();
        let (name, misc) = match rest.rfind('[') {
            Some(i) => (
                rest[..i].trim().to_string(),
                rest[i + 1..].trim_end_matches(']').to_string(),
            ),
            None => (rest.to_string(), String::new()),
        };
        devices.push((index, name, misc));
    }
    devices
}

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Render a proposed `[targets.<name>]` lab block from a host's inventory.
/// Best-guesses one value per channel; alternatives become comments. Meant to
/// be reviewed and pasted into the lab — paniolo never writes it.
pub fn propose_target_block(name: &str, host: &str, inv: &Value) -> String {
    let mut out: Vec<String> = Vec::new();
    out.push(format!("[targets.{name}]"));
    out.push(format!("host = \"{host}\""));
    out.push(String::new());

    // netboot: prefer the carrier-up interface (the list is sorted actives-first).
    let eths: Vec<&Value> = inv
        .get("ethernet")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    out.push(format!("[targets.{name}.netboot]"));
    if let Some(first) = eths.first() {
        let dev = str_at(first, "device").unwrap_or("");
        let note = if first
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            "  # carrier up"
        } else {
            ""
        };
        out.push(format!("interface = \"{dev}\"{note}"));
        for e in &eths[1..] {
            let dev = str_at(e, "device").unwrap_or("");
            out.push(format!("# interface = \"{dev}\"  # alternative"));
        }
    } else {
        out.push("# interface = \"\"  # no USB-Ethernet interface discovered".to_string());
    }
    out.push("# tftp_root = \"/path/to/tftp\"  # set to enable netboot".to_string());
    out.push(String::new());

    // serial: first device as the console; the rest as comments.
    let serials: Vec<&str> = inv
        .get("serial")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if let Some(first) = serials.first() {
        out.push(format!("[[targets.{name}.serial]]"));
        out.push("name = \"console\"".to_string());
        out.push(format!("device = \"{first}\""));
        out.push("baud = 115200".to_string());
        for extra in &serials[1..] {
            out.push(format!("# another serial device: {extra}"));
        }
    } else {
        out.push(format!(
            "# [[targets.{name}.serial]]  # no serial devices discovered"
        ));
    }
    out.push(String::new());

    // video: propose the one non-built-in capture device, if unambiguous.
    let captures: Vec<&str> = inv
        .get("video")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|d| str_at(d, "name"))
                .filter(|n| !BUILTIN_CAPTURE.iter().any(|b| n.contains(b)))
                .collect()
        })
        .unwrap_or_default();
    if captures.len() == 1 {
        out.push(format!("[targets.{name}.video]"));
        out.push(format!("device = \"{}\"", captures[0]));
    } else if captures.is_empty() {
        out.push(format!(
            "# [targets.{name}.video]  # no capture device discovered"
        ));
    } else {
        out.push(format!(
            "# [targets.{name}.video]  # multiple capture devices — pick one:"
        ));
        for c in &captures {
            out.push(format!("# device = \"{c}\""));
        }
    }
    out.push(String::new());
    out.push(format!("# [targets.{name}.power]"));
    out.push("# cycle_cmd = \"/path/to/power-cycle.sh\"  # not discoverable".to_string());

    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propose_prefers_active_eth_and_first_serial() {
        let inv = json!({
            "ethernet": [
                {"port": "AX88179A", "device": "en16", "active": true},
                {"port": "Ethernet Adapter", "device": "en8", "active": false},
            ],
            "serial": ["/dev/tty.usbserial-A", "/dev/tty.usbserial-B"],
            "video": [
                {"index": 0, "name": "USB Video", "misc": ""},
                {"index": 1, "name": "FaceTime HD Camera", "misc": ""},
            ],
        });
        let block = propose_target_block("pi5", "bench1", &inv);
        assert!(block.contains("[targets.pi5]"), "{block}");
        assert!(block.contains("host = \"bench1\""), "{block}");
        assert!(
            block.contains("interface = \"en16\"  # carrier up"),
            "{block}"
        );
        assert!(
            block.contains("# interface = \"en8\"  # alternative"),
            "{block}"
        );
        assert!(
            block.contains("device = \"/dev/tty.usbserial-A\""),
            "{block}"
        );
        assert!(
            block.contains("# another serial device: /dev/tty.usbserial-B"),
            "{block}"
        );
        // FaceTime filtered as built-in → USB Video is the unambiguous capture.
        assert!(block.contains("[targets.pi5.video]"), "{block}");
        assert!(block.contains("device = \"USB Video\""), "{block}");
    }

    #[test]
    fn propose_with_empty_inventory_is_all_stubs() {
        let inv = json!({"ethernet": [], "serial": [], "video": []});
        let block = propose_target_block("t", "local", &inv);
        assert!(block.contains("# interface = \"\""), "{block}");
        assert!(block.contains("# [[targets.t.serial]]"), "{block}");
        assert!(block.contains("# [targets.t.video]"), "{block}");
    }
}
