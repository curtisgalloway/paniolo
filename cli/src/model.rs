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

//! The lab model: one git-tracked file describing all hosts and targets.
//!
//! A *lab* is a single TOML file (default `~/.config/paniolo/lab.toml`, override
//! with `--lab` / `$PANIOLO_LAB`) that declares every control host paniolo
//! reaches over SSH and every target, with each channel of a target's hardware
//! bound to a host (its own `host`, else the target's `host`, else `local`).
//!
//! This module is the typed/read side: `serde` structs, [`validate`], the
//! resolved per-channel view ([`Lab::resolved_target`]), the inverse index
//! ([`Lab::channels_on_host`]), and [`Lab::host_slice`] (the single-host
//! flattening that runs locally or is shipped to a control host). Editing lives
//! in [`crate::labfile`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// ssh destination meaning "the dev machine itself — no SSH".
pub const LOCAL: &str = "local";
pub const DEFAULT_HOST_IP: &str = "192.168.99.1";
pub const VALID_SENSE_SIGNALS: [&str; 4] = ["cts", "dsr", "dcd", "ri"];

/// The lab file is malformed or a mutation would make it invalid.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct LabError(pub String);

fn lab_err<T>(msg: impl Into<String>) -> Result<T, LabError> {
    Err(LabError(msg.into()))
}

fn default_baud() -> i64 {
    115200
}

// ── typed schema (serde) ────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Host {
    pub ssh: String,
    pub identity: Option<String>,
    pub control_path: Option<String>,
    pub paniolo_cmd: Option<String>,
}

impl Host {
    pub fn is_local(&self, name: &str) -> bool {
        self.ssh == LOCAL || name == LOCAL
    }

    /// How to invoke paniolo on this host (bare `paniolo` unless pinned).
    pub fn paniolo(&self) -> String {
        self.paniolo_cmd
            .clone()
            .unwrap_or_else(|| "paniolo".to_string())
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct NetbootChannel {
    pub interface: Option<String>,
    pub host_ip: Option<String>,
    pub tftp_root: Option<String>,
    pub host: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct SerialChannel {
    pub name: String,
    pub device: String,
    #[serde(default = "default_baud")]
    pub baud: i64,
    pub power_sense_signal: Option<String>,
    pub host: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct PowerChannel {
    pub cycle_cmd: Option<String>,
    pub on_cmd: Option<String>,
    pub off_cmd: Option<String>,
    pub state_cmd: Option<String>,
    pub serial_interface: Option<String>,
    pub host: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct VideoChannel {
    pub device: Option<String>,
    pub host: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Target {
    pub host: Option<String>,
    pub note: Option<String>,
    pub netboot: Option<NetbootChannel>,
    #[serde(default)]
    pub serial: Vec<SerialChannel>,
    pub power: Option<PowerChannel>,
    pub video: Option<VideoChannel>,
}

impl Target {
    pub fn default_host(&self) -> &str {
        self.host.as_deref().unwrap_or(LOCAL)
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Lab {
    #[serde(default)]
    pub hosts: BTreeMap<String, Host>,
    #[serde(default)]
    pub targets: BTreeMap<String, Target>,
}

// ── resolved (read) view ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Netboot,
    Serial,
    Power,
    Video,
}

impl ChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChannelKind::Netboot => "netboot",
            ChannelKind::Serial => "serial",
            ChannelKind::Power => "power",
            ChannelKind::Video => "video",
        }
    }
}

/// One channel of a target with its physical host resolved.
#[derive(Debug, Clone)]
pub struct ResolvedChannel {
    pub kind: ChannelKind,
    /// Serial interface name, or the kind name for singleton channels.
    pub name: String,
    pub host: String,
    /// Remaining scalar config, in display order (host and name excluded).
    pub fields: Vec<(&'static str, String)>,
}

/// A target's channels with per-channel hosts resolved (no single-host rule).
#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub name: String,
    pub default_host: String,
    pub note: Option<String>,
    pub channels: Vec<ResolvedChannel>,
}

impl ResolvedTarget {
    /// The distinct hosts this target's channels live on.
    pub fn hosts(&self) -> Vec<String> {
        let mut s: BTreeSet<String> = self.channels.iter().map(|c| c.host.clone()).collect();
        if s.is_empty() {
            s.insert(self.default_host.clone());
        }
        s.into_iter().collect()
    }
}

impl Lab {
    pub fn target_names(&self) -> Vec<&str> {
        self.targets.keys().map(String::as_str).collect()
    }

    /// Look up a control host by name. `local` (and any undeclared name, which
    /// validation forbids) resolves to a synthetic local host.
    pub fn host(&self, name: &str) -> Host {
        if let Some(h) = self.hosts.get(name) {
            return h.clone();
        }
        Host {
            ssh: LOCAL.to_string(),
            ..Default::default()
        }
    }

    /// Flatten a target to its channels with per-channel hosts resolved.
    pub fn resolved_target(&self, name: &str) -> Option<ResolvedTarget> {
        let t = self.targets.get(name)?;
        let default_host = t.default_host().to_string();
        let host_of = |h: &Option<String>| h.clone().unwrap_or_else(|| default_host.clone());
        let mut channels = Vec::new();

        if let Some(nb) = &t.netboot {
            let mut f = Vec::new();
            push_opt(&mut f, "interface", &nb.interface);
            push_opt(&mut f, "host_ip", &nb.host_ip);
            push_opt(&mut f, "tftp_root", &nb.tftp_root);
            channels.push(ResolvedChannel {
                kind: ChannelKind::Netboot,
                name: "netboot".into(),
                host: host_of(&nb.host),
                fields: f,
            });
        }
        for s in &t.serial {
            let mut f = vec![("device", s.device.clone()), ("baud", s.baud.to_string())];
            push_opt(&mut f, "power_sense_signal", &s.power_sense_signal);
            channels.push(ResolvedChannel {
                kind: ChannelKind::Serial,
                name: s.name.clone(),
                host: host_of(&s.host),
                fields: f,
            });
        }
        if let Some(p) = &t.power {
            let mut f = Vec::new();
            push_opt(&mut f, "cycle_cmd", &p.cycle_cmd);
            push_opt(&mut f, "on_cmd", &p.on_cmd);
            push_opt(&mut f, "off_cmd", &p.off_cmd);
            push_opt(&mut f, "state_cmd", &p.state_cmd);
            push_opt(&mut f, "serial_interface", &p.serial_interface);
            channels.push(ResolvedChannel {
                kind: ChannelKind::Power,
                name: "power".into(),
                host: host_of(&p.host),
                fields: f,
            });
        }
        if let Some(v) = &t.video {
            let mut f = Vec::new();
            push_opt(&mut f, "device", &v.device);
            channels.push(ResolvedChannel {
                kind: ChannelKind::Video,
                name: "video".into(),
                host: host_of(&v.host),
                fields: f,
            });
        }
        Some(ResolvedTarget {
            name: name.to_string(),
            default_host,
            note: t.note.clone(),
            channels,
        })
    }

    /// Every (target, channel) pair whose channel resolves to `host`.
    pub fn channels_on_host(&self, host: &str) -> Vec<(String, ResolvedChannel)> {
        let mut out = Vec::new();
        for name in self.targets.keys() {
            if let Some(rt) = self.resolved_target(name) {
                for ch in rt.channels {
                    if ch.host == host {
                        out.push((name.clone(), ch));
                    }
                }
            }
        }
        out
    }
}

fn push_opt(fields: &mut Vec<(&'static str, String)>, key: &'static str, v: &Option<String>) {
    if let Some(val) = v {
        fields.push((key, val.clone()));
    }
}

/// Resolve the host a command should run on, given the channel it touches.
///
/// Singleton kinds use that channel's host (else the target default). Serial
/// with a name uses that interface's host; serial without a name uses the common
/// host of all interfaces, erroring if they span hosts (the `serial watch` case,
/// where the daemon owns every interface). A missing channel falls back to the
/// target's default host so the body can report it.
pub fn channel_host(
    rt: &ResolvedTarget,
    kind: ChannelKind,
    serial_name: Option<&str>,
) -> Result<String, LabError> {
    if kind == ChannelKind::Serial {
        let serials: Vec<&ResolvedChannel> = rt
            .channels
            .iter()
            .filter(|c| c.kind == ChannelKind::Serial)
            .collect();
        if let Some(n) = serial_name {
            return Ok(serials
                .iter()
                .find(|c| c.name == n)
                .map(|c| c.host.clone())
                .unwrap_or_else(|| rt.default_host.clone()));
        }
        if serials.is_empty() {
            return Ok(rt.default_host.clone());
        }
        let hosts: BTreeSet<&str> = serials.iter().map(|c| c.host.as_str()).collect();
        if hosts.len() > 1 {
            let list: Vec<&str> = hosts.into_iter().collect();
            return lab_err(format!(
                "target '{}' has serial interfaces on multiple hosts ({}); \
                 specify one with --interface",
                rt.name,
                list.join(", ")
            ));
        }
        return Ok(serials[0].host.clone());
    }
    for c in &rt.channels {
        if c.kind == kind {
            return Ok(c.host.clone());
        }
    }
    Ok(rt.default_host.clone())
}

// ── validation (shared by load and the editor's save) ───────────────────────

fn check_host_ref(host: &str, declared: &BTreeSet<&str>, ctx: &str) -> Result<(), LabError> {
    if !declared.contains(host) {
        let mut known: Vec<&str> = declared.iter().copied().collect();
        known.sort_unstable();
        return lab_err(format!(
            "{ctx} references unknown host '{host}' (declared: {})",
            known.join(", ")
        ));
    }
    Ok(())
}

/// Raise [`LabError`] if `lab` is not a structurally valid lab.
pub fn validate(lab: &Lab) -> Result<(), LabError> {
    let mut declared: BTreeSet<&str> = lab.hosts.keys().map(String::as_str).collect();
    declared.insert(LOCAL);
    for (name, h) in &lab.hosts {
        if h.ssh.trim().is_empty() {
            return lab_err(format!("host '{name}': missing required 'ssh' field"));
        }
    }
    for (name, t) in &lab.targets {
        let default_host = t.default_host();
        check_host_ref(default_host, &declared, &format!("target '{name}'"))?;
        if let Some(nb) = &t.netboot {
            let h = nb.host.as_deref().unwrap_or(default_host);
            check_host_ref(h, &declared, &format!("target '{name}' netboot"))?;
        }
        if let Some(p) = &t.power {
            let h = p.host.as_deref().unwrap_or(default_host);
            check_host_ref(h, &declared, &format!("target '{name}' power"))?;
        }
        if let Some(v) = &t.video {
            let h = v.host.as_deref().unwrap_or(default_host);
            check_host_ref(h, &declared, &format!("target '{name}' video"))?;
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for s in &t.serial {
            if s.name.is_empty() || s.device.is_empty() {
                return lab_err(format!(
                    "target '{name}': each [[serial]] needs name + device"
                ));
            }
            if !seen.insert(s.name.as_str()) {
                return lab_err(format!(
                    "target '{name}': duplicate serial name '{}'",
                    s.name
                ));
            }
            if let Some(sense) = &s.power_sense_signal {
                if !VALID_SENSE_SIGNALS.contains(&sense.as_str()) {
                    return lab_err(format!(
                        "target '{name}' serial '{}': invalid power_sense_signal '{sense}' \
                         (valid: {})",
                        s.name,
                        VALID_SENSE_SIGNALS.join(", ")
                    ));
                }
            }
            let h = s.host.as_deref().unwrap_or(default_host);
            check_host_ref(
                h,
                &declared,
                &format!("target '{name}' serial '{}'", s.name),
            )?;
        }
    }
    Ok(())
}

// ── parsing & path discovery ────────────────────────────────────────────────

/// Parse and validate a lab from TOML text.
pub fn parse(text: &str) -> Result<Lab, LabError> {
    let lab: Lab = toml::from_str(text).map_err(|e| LabError(e.to_string()))?;
    validate(&lab)?;
    Ok(lab)
}

/// Read and validate the lab at `path`.
pub fn load(path: &Path) -> Result<Lab, LabError> {
    let text =
        std::fs::read_to_string(path).map_err(|e| LabError(format!("{}: {e}", path.display())))?;
    parse(&text)
}

/// Expand a leading `~/` to the user's home directory.
pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(p)
}

/// The lab file used when neither `--lab` nor `$PANIOLO_LAB` is given.
pub fn default_lab_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".config/paniolo/lab.toml")
}

/// Resolve the active lab path: `--lab`, then `$PANIOLO_LAB`, then the default
/// path if it exists. Returns None when none resolve.
pub fn resolve_lab_path(flag: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = flag {
        return Some(expand_tilde(p));
    }
    if let Ok(p) = std::env::var("PANIOLO_LAB") {
        if !p.is_empty() {
            return Some(expand_tilde(&p));
        }
    }
    let d = default_lab_path();
    if d.exists() {
        Some(d)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn multihost() -> Lab {
        parse(
            r#"
            [hosts.bench1]
            ssh = "u@bench1"
            [hosts.bench2]
            ssh = "u@bench2"
            [targets.fortune]
            host = "bench1"
            note = "n"
            [targets.fortune.netboot]
            interface = "en0"
            [[targets.fortune.serial]]
            name = "console"
            device = "/dev/ttyUSB0"
            [targets.fortune.video]
            device = "/dev/video0"
            host = "bench2"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn resolves_per_channel_host() {
        let rt = multihost().resolved_target("fortune").unwrap();
        let by: BTreeMap<_, _> = rt
            .channels
            .iter()
            .map(|c| ((c.kind.as_str(), c.name.clone()), c.host.clone()))
            .collect();
        assert_eq!(by[&("netboot", "netboot".into())], "bench1");
        assert_eq!(by[&("serial", "console".into())], "bench1");
        assert_eq!(by[&("video", "video".into())], "bench2");
        assert_eq!(rt.hosts(), vec!["bench1", "bench2"]);
    }

    #[test]
    fn channels_on_host_is_the_inverse_index() {
        let lab = multihost();
        let on2 = lab.channels_on_host("bench2");
        assert_eq!(on2.len(), 1);
        assert_eq!(on2[0].0, "fortune");
        assert_eq!(on2[0].1.kind, ChannelKind::Video);
    }

    #[test]
    fn validate_rejects_unknown_host() {
        let e = parse("[targets.t]\nhost = \"ghost\"\n").unwrap_err();
        assert!(e.0.contains("unknown host 'ghost'"), "{}", e.0);
    }

    #[test]
    fn validate_rejects_bad_sense() {
        let toml = "[targets.t]\n[[targets.t.serial]]\nname=\"c\"\ndevice=\"/d\"\npower_sense_signal=\"bogus\"\n";
        let e = parse(toml).unwrap_err();
        assert!(e.0.contains("invalid power_sense_signal"), "{}", e.0);
    }

    #[test]
    fn validate_rejects_missing_ssh() {
        // ssh is a required field, so this fails at deserialize time.
        assert!(parse("[hosts.bench1]\n").is_err());
    }
}
