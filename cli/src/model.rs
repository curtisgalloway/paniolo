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
/// The `video.ocr_mode` values (see [`VideoChannel::ocr_mode`]).
pub const VALID_OCR_MODES: [&str; 2] = ["text", "gui"];

/// The lab file is malformed or a mutation would make it invalid.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct LabError(pub String);

fn lab_err<T>(msg: impl Into<String>) -> Result<T, LabError> {
    Err(LabError(msg.into()))
}

/// What a target, host, or serial interface name may look like. Names become
/// path components (`<runtime-base>/serialcap/<target>/`,
/// `~/.local/share/paniolo/<target>/`), daemon query values
/// (`?interface=<name>`), and positional arguments to a re-exec'd `paniolo`
/// on a control host, so they are confined to what survives all three
/// unquoted — and a leading `-` is refused because that positional would be
/// read as an option.
pub const NAME_RULE: &str =
    "letters, digits, `.`, `_` and `-` only; not `.` or `..`; not starting with `-`";

/// Whether `name` satisfies [`NAME_RULE`].
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Reject a `what` (target/host/serial interface) name that fails
/// [`NAME_RULE`], saying which rule and what the name was for. Applied by the
/// editor when a name is chosen (`add`, `rename`, `set`), not on load, so a
/// hand-written lab with an odd name still reads.
pub fn validate_name(what: &str, name: &str) -> Result<(), LabError> {
    if is_valid_name(name) {
        Ok(())
    } else {
        lab_err(format!("invalid {what} name '{name}': {NAME_RULE}"))
    }
}

fn default_baud() -> i64 {
    115200
}

// ── typed schema (serde) ────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Host {
    pub ssh: String,
    /// Free-text description of this control host (its role, location, etc.),
    /// shown by `host show` / `config show`. Purely informational.
    pub description: Option<String>,
    /// The host's fully-qualified hostname, used to recognize *this* machine
    /// (see [`Host::is_local`]). Distinct from `ssh`, which is only how *other*
    /// machines reach it and may be an `~/.ssh/config` alias.
    pub hostname: Option<String>,
    pub identity: Option<String>,
    pub control_path: Option<String>,
    pub paniolo_cmd: Option<String>,
}

impl Host {
    pub fn is_local(&self, name: &str) -> bool {
        if self.ssh == LOCAL || name == LOCAL {
            return true;
        }
        // A host whose declared `hostname` (FQDN) matches this machine's is
        // local — so one shared lab file can be run from any box and each
        // recognizes its own host. `ssh` is intentionally NOT used here: it may
        // be an ~/.ssh/config alias, not the machine's real name.
        match (&self.hostname, local_fqdn()) {
            (Some(h), Some(local)) => h.eq_ignore_ascii_case(local),
            _ => false,
        }
    }

    /// How to invoke paniolo on this host (bare `paniolo` unless pinned).
    pub fn paniolo(&self) -> String {
        self.paniolo_cmd
            .clone()
            .unwrap_or_else(|| "paniolo".to_string())
    }
}

/// This machine's fully-qualified hostname, detected once and cached
/// (lowercased). [`Host::is_local`] compares it against each host's declared
/// `hostname` to decide which host is *this* one. `hostname -f` is the most
/// portable FQDN source across macOS/Linux; fall back to the bare node name,
/// then give up (None → nothing matches by hostname, so only the `local`
/// sentinel marks a host local — i.e. today's driver-machine behavior).
pub fn local_fqdn() -> Option<&'static str> {
    static FQDN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FQDN.get_or_init(|| {
        let run = |args: &[&str]| {
            std::process::Command::new("hostname")
                .args(args)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .trim()
                        .to_ascii_lowercase()
                })
                .filter(|s| !s.is_empty())
        };
        run(&["-f"]).or_else(|| run(&[]))
    })
    .as_deref()
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct NetbootChannel {
    pub interface: Option<String>,
    pub host_ip: Option<String>,
    pub tftp_root: Option<String>,
    /// Boot program served to UEFI clients: a bare filename relative to
    /// `tftp_root` (e.g. `grubaa64.efi`). Advertised as a TFTP filename to PXE
    /// clients and wrapped in an `http://` URL for HTTP Boot clients.
    pub boot_file: Option<String>,
    /// HTTP server port, also embedded in the HTTP Boot URL (default 80).
    pub http_port: Option<String>,
    /// `Content-Type` for HTTP responses (default `application/octet-stream`).
    pub content_type: Option<String>,
    pub host: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct SerialChannel {
    pub name: String,
    pub device: String,
    #[serde(default = "default_baud")]
    pub baud: i64,
    pub power_sense_signal: Option<String>,
    /// Opt-in: the FTDI DTR line on this interface is wired to the board's J2
    /// power-button header, so `serial dtr` / `serial reset` may drive it.
    /// Off by default — DTR-to-J2 wiring is the rare exception, and toggling an
    /// unwired line silently no-ops. The DTR commands refuse interfaces that
    /// haven't set this. See docs/power.md.
    #[serde(default)]
    pub power_button: bool,
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
    /// Which OCR engine suits this target's screens: `"text"` for firmware,
    /// bootloaders and consoles, `"gui"` for desktops and graphical BIOS pages.
    ///
    /// This exists because on Linux the two need different engines and the
    /// choice cannot be inferred. Measured on a Pi 5 (evals/ocr): PP-OCRv6 via
    /// RapidOCR scores 0.083 token-recall error on GUI screens against
    /// Tesseract's 0.312, while Tesseract edges it on console text (0.019 vs
    /// 0.025 CER) at a third of the latency.
    ///
    /// Confidence cannot decide it at runtime: Apple Vision reports a constant
    /// value, `Windows.Media.Ocr` reports none, and Tesseract's GUI failure is
    /// *silent omission* — it is confident about the rows it did read, so a
    /// low-confidence fallback would never trigger. Hence a config field.
    ///
    /// Unset means the platform default (`visionocr` on macOS, `winocr` on
    /// Windows, `linuxocr` on Linux), which is right for everything except a
    /// Linux host looking at GUI screens.
    pub ocr_mode: Option<String>,
}

/// USB HID input injection: an opaque helper command (e.g. `hidrig -d <uart>`)
/// that `paniolo hid send` appends protocol arguments to — the device-specific
/// tool lives outside paniolo, like the power hooks.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct HidChannel {
    pub cmd: Option<String>,
    pub host: Option<String>,
}

/// adb transport to an Android target (DUT). The device is named by its
/// `adb -s <serial>` id (omit for the sole attached device); `adb` overrides
/// the binary. Like SSH this is a generic transport, not a device-specific
/// helper, so `paniolo adb …` shells out to it directly on the bound host.
/// A shared USB device that a KVM can route to either side — the Openterface
/// KVM-Go's onboard microSD reader, or the Mini-KVM's switchable USB-A port.
/// Like `hid`, the device-specific tool lives outside paniolo in an opaque
/// helper command; unlike `hid send`, paniolo appends a *fixed* verb rather
/// than passing arguments through, so a remote host only ever sees
/// `usb host`, `usb target`, or `usb state`.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct UsbChannel {
    pub cmd: Option<String>,
    pub host: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct AdbChannel {
    pub serial: Option<String>,
    pub adb: Option<String>,
    pub host: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Target {
    pub host: Option<String>,
    /// Free-text description of this target, shown by `target show` /
    /// `config show`. The legacy key `note` is accepted as an alias so older
    /// lab files keep parsing.
    #[serde(alias = "note")]
    pub description: Option<String>,
    pub netboot: Option<NetbootChannel>,
    #[serde(default)]
    pub serial: Vec<SerialChannel>,
    pub power: Option<PowerChannel>,
    pub video: Option<VideoChannel>,
    pub hid: Option<HidChannel>,
    pub usb: Option<UsbChannel>,
    pub adb: Option<AdbChannel>,
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
    Hid,
    Usb,
    Adb,
}

impl ChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChannelKind::Netboot => "netboot",
            ChannelKind::Serial => "serial",
            ChannelKind::Power => "power",
            ChannelKind::Video => "video",
            ChannelKind::Hid => "hid",
            ChannelKind::Usb => "usb",
            ChannelKind::Adb => "adb",
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
    pub description: Option<String>,
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
            push_opt(&mut f, "boot_file", &nb.boot_file);
            push_opt(&mut f, "http_port", &nb.http_port);
            push_opt(&mut f, "content_type", &nb.content_type);
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
            if s.power_button {
                f.push(("power_button", "true".to_string()));
            }
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
            push_opt(&mut f, "ocr_mode", &v.ocr_mode);
            channels.push(ResolvedChannel {
                kind: ChannelKind::Video,
                name: "video".into(),
                host: host_of(&v.host),
                fields: f,
            });
        }
        if let Some(h) = &t.hid {
            let mut f = Vec::new();
            push_opt(&mut f, "cmd", &h.cmd);
            channels.push(ResolvedChannel {
                kind: ChannelKind::Hid,
                name: "hid".into(),
                host: host_of(&h.host),
                fields: f,
            });
        }
        if let Some(u) = &t.usb {
            let mut f = Vec::new();
            push_opt(&mut f, "cmd", &u.cmd);
            channels.push(ResolvedChannel {
                kind: ChannelKind::Usb,
                name: "usb".into(),
                host: host_of(&u.host),
                fields: f,
            });
        }
        if let Some(a) = &t.adb {
            let mut f = Vec::new();
            push_opt(&mut f, "serial", &a.serial);
            push_opt(&mut f, "adb", &a.adb);
            channels.push(ResolvedChannel {
                kind: ChannelKind::Adb,
                name: "adb".into(),
                host: host_of(&a.host),
                fields: f,
            });
        }
        Some(ResolvedTarget {
            name: name.to_string(),
            default_host,
            description: t.description.clone(),
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
/// with a name uses that interface's host — a name the target does not have
/// is an error here, before anything is dispatched, rather than a silent
/// fall-through to the default host (where `console -i typo` used to open a
/// dashboard on the wrong interface); serial without a name uses the common
/// host of all interfaces, erroring if they span hosts (the `serial watch`
/// case, where the daemon owns every interface). A missing channel *kind*
/// falls back to the target's default host so the body can report it.
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
            return serials
                .iter()
                .find(|c| c.name == n)
                .map(|c| c.host.clone())
                .ok_or_else(|| {
                    let have: Vec<&str> = serials.iter().map(|c| c.name.as_str()).collect();
                    LabError(format!(
                        "target '{}' has no serial interface '{n}' (have: {})",
                        rt.name,
                        if have.is_empty() {
                            "none".to_string()
                        } else {
                            have.join(", ")
                        }
                    ))
                });
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
        if name.is_empty() {
            return lab_err("a host has an empty name");
        }
        if h.ssh.trim().is_empty() {
            return lab_err(format!("host '{name}': missing required 'ssh' field"));
        }
        // `local` is the sentinel for this machine and resolves as local
        // whatever its `ssh` says; a declared [hosts.local] with a real
        // destination would silently run its channels here instead.
        if name == LOCAL && h.ssh != LOCAL {
            return lab_err(format!(
                "host '{LOCAL}' means this machine and cannot have ssh = \"{}\"; \
                 give the remote host another name",
                h.ssh
            ));
        }
        // These become ssh arguments; one starting with `-` would be read as
        // an option (`-oProxyCommand=…`) rather than a destination or path.
        for (field, value) in [
            ("ssh", Some(h.ssh.as_str())),
            ("identity", h.identity.as_deref()),
            ("control_path", h.control_path.as_deref()),
        ] {
            if value.is_some_and(|v| v.starts_with('-')) {
                return lab_err(format!(
                    "host '{name}': {field} '{}' must not begin with '-'",
                    value.unwrap_or_default()
                ));
            }
        }
    }
    for (name, t) in &lab.targets {
        if name.is_empty() {
            return lab_err("a target has an empty name");
        }
        let default_host = t.default_host();
        check_host_ref(default_host, &declared, &format!("target '{name}'"))?;
        if let Some(nb) = &t.netboot {
            let h = nb.host.as_deref().unwrap_or(default_host);
            check_host_ref(h, &declared, &format!("target '{name}' netboot"))?;
            if let Some(ip) = &nb.host_ip {
                if ip.parse::<std::net::Ipv4Addr>().is_err() {
                    return lab_err(format!(
                        "target '{name}' netboot: host_ip '{ip}' is not an IPv4 address"
                    ));
                }
            }
            if let Some(port) = &nb.http_port {
                if !matches!(port.parse::<u16>(), Ok(p) if p != 0) {
                    return lab_err(format!(
                        "target '{name}' netboot: http_port '{port}' is not a port number (1-65535)"
                    ));
                }
            }
        }
        if let Some(p) = &t.power {
            let h = p.host.as_deref().unwrap_or(default_host);
            check_host_ref(h, &declared, &format!("target '{name}' power"))?;
        }
        if let Some(v) = &t.video {
            let h = v.host.as_deref().unwrap_or(default_host);
            check_host_ref(h, &declared, &format!("target '{name}' video"))?;
            if let Some(mode) = &v.ocr_mode {
                if !VALID_OCR_MODES.contains(&mode.as_str()) {
                    return lab_err(format!(
                        "target '{name}' video: invalid ocr_mode '{mode}' (valid: {})",
                        VALID_OCR_MODES.join(", ")
                    ));
                }
            }
        }
        if let Some(hid) = &t.hid {
            let h = hid.host.as_deref().unwrap_or(default_host);
            check_host_ref(h, &declared, &format!("target '{name}' hid"))?;
        }
        if let Some(adb) = &t.adb {
            let h = adb.host.as_deref().unwrap_or(default_host);
            check_host_ref(h, &declared, &format!("target '{name}' adb"))?;
        }
        if let Some(usb) = &t.usb {
            let h = usb.host.as_deref().unwrap_or(default_host);
            check_host_ref(h, &declared, &format!("target '{name}' usb"))?;
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

/// [`validate`], plus the cross-references a *write* must not leave dangling:
/// a `power.serial_interface` has to name one of the target's own serial
/// interfaces. Applied by the editor before every save and not on load, so a
/// lab that already carries a stale reference still loads (`doctor` reports
/// it) — but no CLI edit may create one, or remove the interface it names.
pub fn validate_for_save(lab: &Lab) -> Result<(), LabError> {
    validate(lab)?;
    for (name, t) in &lab.targets {
        let Some(si) = t.power.as_ref().and_then(|p| p.serial_interface.as_deref()) else {
            continue;
        };
        if !t.serial.iter().any(|s| s.name == si) {
            let have: Vec<&str> = t.serial.iter().map(|s| s.name.as_str()).collect();
            return lab_err(format!(
                "target '{name}': power.serial_interface '{si}' names no serial interface \
                 of this target (have: {}); point it at one with \
                 `paniolo power set -t {name} --serial-interface <name>`, or remove the \
                 power channel first",
                if have.is_empty() {
                    "none".to_string()
                } else {
                    have.join(", ")
                }
            ));
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

    #[test]
    fn is_local_via_sentinels() {
        let by_ssh = Host {
            ssh: "local".into(),
            ..Default::default()
        };
        assert!(by_ssh.is_local("anything"));
        let by_name = Host {
            ssh: "u@remote".into(),
            ..Default::default()
        };
        assert!(by_name.is_local("local"));
    }

    #[test]
    fn is_local_via_hostname_fqdn() {
        // A non-matching FQDN is remote.
        let remote = Host {
            ssh: "u@x".into(),
            hostname: Some("nope.invalid.example".into()),
            ..Default::default()
        };
        assert!(!remote.is_local("dev"));
        // A host whose declared FQDN equals this machine's resolves as local,
        // case-insensitively. (Skipped when the FQDN can't be detected.)
        if let Some(fqdn) = local_fqdn() {
            let me = Host {
                ssh: "u@x".into(),
                hostname: Some(fqdn.to_ascii_uppercase()),
                ..Default::default()
            };
            assert!(me.is_local("dev"));
        }
    }

    #[test]
    fn description_accepts_legacy_note_alias() {
        // Canonical `description` and the legacy `note` key both land in
        // Target::description, and a host carries its own description.
        let lab = parse(
            r#"
            [hosts.bench1]
            ssh = "u@bench1"
            description = "the bench Mac"
            [targets.canon]
            description = "Pi 5 DUT"
            [targets.legacy]
            note = "old-style note"
            "#,
        )
        .unwrap();
        assert_eq!(
            lab.hosts["bench1"].description.as_deref(),
            Some("the bench Mac")
        );
        assert_eq!(
            lab.targets["canon"].description.as_deref(),
            Some("Pi 5 DUT")
        );
        assert_eq!(
            lab.targets["legacy"].description.as_deref(),
            Some("old-style note"),
            "legacy `note` key parses into `description` via the serde alias"
        );
    }

    fn multihost() -> Lab {
        parse(
            r#"
            [hosts.bench1]
            ssh = "u@bench1"
            [hosts.bench2]
            ssh = "u@bench2"
            [targets.fortune]
            host = "bench1"
            description = "n"
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
    fn resolves_adb_channel() {
        let lab = parse(
            r#"
            [hosts.bench1]
            ssh = "u@bench1"
            [targets.pixel]
            host = "bench1"
            [targets.pixel.adb]
            serial = "39021FDH200xyz"
            "#,
        )
        .unwrap();
        let rt = lab.resolved_target("pixel").unwrap();
        let adb = rt
            .channels
            .iter()
            .find(|c| c.kind == ChannelKind::Adb)
            .expect("adb channel");
        assert_eq!(adb.host, "bench1");
        assert_eq!(
            adb.fields.iter().find(|(k, _)| *k == "serial").unwrap().1,
            "39021FDH200xyz"
        );
        // The device id is plumbed via channel_host like any singleton kind.
        assert_eq!(channel_host(&rt, ChannelKind::Adb, None).unwrap(), "bench1");
    }

    #[test]
    fn validate_rejects_unknown_host() {
        let e = parse("[targets.t]\nhost = \"ghost\"\n").unwrap_err();
        assert!(e.0.contains("unknown host 'ghost'"), "{}", e.0);
    }

    #[test]
    fn validate_rejects_unknown_usb_host() {
        // Every channel's host reference is checked, the usb channel included:
        // an unknown name here used to pass validation and then silently
        // resolve to the local host at runtime.
        let e =
            parse("[targets.t.usb]\ncmd = \"ch9329 -d /dev/x\"\nhost = \"ghost\"\n").unwrap_err();
        assert!(e.0.contains("target 't' usb"), "{}", e.0);
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

    /// `[hosts.local]` with a real destination would resolve as *this*
    /// machine (the name is the sentinel) while claiming to be remote.
    #[test]
    fn validate_rejects_a_local_host_with_a_remote_ssh() {
        let e = parse("[hosts.local]\nssh = \"u@bench1\"\n").unwrap_err();
        assert!(e.0.contains("host 'local' means this machine"), "{}", e.0);
        // The redundant-but-harmless spelling still parses.
        assert!(parse("[hosts.local]\nssh = \"local\"\n").is_ok());
    }

    #[test]
    fn validate_rejects_empty_host_and_target_names() {
        let e = parse("[hosts.\"\"]\nssh = \"u@b\"\n").unwrap_err();
        assert!(e.0.contains("host has an empty name"), "{}", e.0);
        let e = parse("[targets.\"\"]\n").unwrap_err();
        assert!(e.0.contains("target has an empty name"), "{}", e.0);
    }

    /// `ssh`, `identity` and `control_path` all end up as ssh arguments; a
    /// value starting with `-` would be parsed as an option there.
    #[test]
    fn validate_rejects_host_fields_that_look_like_options() {
        for (field, toml) in [
            ("ssh", "[hosts.b]\nssh = \"-oProxyCommand=x\"\n"),
            ("identity", "[hosts.b]\nssh = \"u@b\"\nidentity = \"-x\"\n"),
            (
                "control_path",
                "[hosts.b]\nssh = \"u@b\"\ncontrol_path = \"-x\"\n",
            ),
        ] {
            let e = parse(toml).unwrap_err();
            assert!(
                e.0.contains(&format!("{field} '-")) && e.0.contains("must not begin with '-'"),
                "{field}: {}",
                e.0
            );
        }
    }

    #[test]
    fn validate_rejects_bad_host_ip_and_http_port() {
        let e = parse("[targets.t]\n[targets.t.netboot]\nhost_ip = \"192.168.99\"\n").unwrap_err();
        assert!(
            e.0.contains("host_ip '192.168.99' is not an IPv4"),
            "{}",
            e.0
        );
        let e = parse("[targets.t]\n[targets.t.netboot]\nhost_ip = \"fe80::1\"\n").unwrap_err();
        assert!(e.0.contains("not an IPv4"), "{}", e.0);
        for bad in ["http", "0", "65536", "-1"] {
            let e = parse(&format!(
                "[targets.t]\n[targets.t.netboot]\nhttp_port = \"{bad}\"\n"
            ))
            .unwrap_err();
            assert!(e.0.contains("is not a port number"), "{bad}: {}", e.0);
        }
        assert!(parse(
            "[targets.t]\n[targets.t.netboot]\nhost_ip = \"192.168.99.1\"\nhttp_port = \"8080\"\n"
        )
        .is_ok());
    }

    #[test]
    fn validate_rejects_unknown_ocr_mode() {
        let e = parse("[targets.t]\n[targets.t.video]\nocr_mode = \"fast\"\n").unwrap_err();
        assert!(e.0.contains("invalid ocr_mode 'fast'"), "{}", e.0);
        for ok in VALID_OCR_MODES {
            assert!(parse(&format!(
                "[targets.t]\n[targets.t.video]\nocr_mode = \"{ok}\"\n"
            ))
            .is_ok());
        }
    }

    /// A dangling `power.serial_interface` still *loads* (so a lab edited by
    /// hand can be repaired with the CLI) but must not be *saved*.
    #[test]
    fn validate_for_save_rejects_a_dangling_serial_interface() {
        let dangling = parse(
            "[targets.t]\n[targets.t.power]\nserial_interface = \"console\"\n\
             [[targets.t.serial]]\nname = \"other\"\ndevice = \"/dev/a\"\n",
        )
        .unwrap();
        let e = validate_for_save(&dangling).unwrap_err();
        assert!(
            e.0.contains("power.serial_interface 'console' names no serial interface"),
            "{}",
            e.0
        );
        assert!(e.0.contains("have: other"), "{}", e.0);
        let fine = parse(
            "[targets.t]\n[targets.t.power]\nserial_interface = \"console\"\n\
             [[targets.t.serial]]\nname = \"console\"\ndevice = \"/dev/a\"\n",
        )
        .unwrap();
        validate_for_save(&fine).unwrap();
    }

    /// Naming an interface the target does not have is an error at
    /// resolution, not a fall-through to the default host.
    #[test]
    fn channel_host_rejects_an_unknown_serial_interface() {
        let rt = multihost().resolved_target("fortune").unwrap();
        assert_eq!(
            channel_host(&rt, ChannelKind::Serial, Some("console")).unwrap(),
            "bench1"
        );
        let e = channel_host(&rt, ChannelKind::Serial, Some("typo")).unwrap_err();
        assert!(
            e.0.contains("no serial interface 'typo' (have: console)"),
            "{}",
            e.0
        );
        // No serial at all, none asked for by name: still the default host,
        // so the command body can say what is missing.
        let lab = parse("[targets.bare]\n").unwrap();
        let rt = lab.resolved_target("bare").unwrap();
        assert_eq!(channel_host(&rt, ChannelKind::Serial, None).unwrap(), LOCAL);
    }

    #[test]
    fn name_rule_accepts_plain_names_and_rejects_the_rest() {
        for ok in ["pi5", "lab-nuc-1", "bench_2", "v1.2", "a"] {
            assert!(is_valid_name(ok), "{ok}");
        }
        for bad in ["", ".", "..", "-x", "a b", "a/b", "a\tb", "é", "a:b", "x\0"] {
            assert!(!is_valid_name(bad), "{bad:?}");
        }
        let e = validate_name("target", "a b").unwrap_err();
        assert!(e.0.starts_with("invalid target name 'a b':"), "{}", e.0);
    }
}
