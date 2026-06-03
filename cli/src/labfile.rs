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

//! The editable lab document: surgical, comment-preserving writes via `toml_edit`.
//!
//! The lab file is human-authored, so the CLI edits it *politely* — preserving
//! hand-written comments, key ordering, and formatting, touching only the tables
//! it changes. Reads/resolution live in [`crate::model`]; this is the write side
//! plus a re-run of the shared [`model::validate`] before every save.

use std::path::{Path, PathBuf};

use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

use crate::model::{self, Lab, LabError};

const HEADER: &str = "paniolo lab — managed by the `paniolo` CLI; hand-edits are preserved";

fn lab_err<T>(msg: impl Into<String>) -> Result<T, LabError> {
    Err(LabError(msg.into()))
}

/// A lab file open for editing, backed by a live `toml_edit` document.
pub struct LabFile {
    pub path: PathBuf,
    pub doc: DocumentMut,
    /// A top-of-file header to write for a freshly created lab (None when the
    /// file already existed — its own leading comments are preserved as-is).
    header: Option<&'static str>,
}

impl LabFile {
    pub fn load(path: &Path) -> Result<Self, LabError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| LabError(format!("{}: {e}", path.display())))?;
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|e| LabError(e.to_string()))?;
        validate_doc(&doc)?;
        Ok(Self {
            path: path.to_path_buf(),
            doc,
            header: None,
        })
    }

    pub fn create(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            doc: DocumentMut::new(),
            header: Some(HEADER),
        }
    }

    pub fn save(&self) -> Result<(), LabError> {
        validate_doc(&self.doc)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| LabError(format!("{}: {e}", parent.display())))?;
        }
        // toml_edit stores a leading comment as the first item's prefix, which
        // doesn't exist for a fresh document — so prepend the header to the
        // rendered text rather than into the (item-less) document.
        let mut out = self.doc.to_string();
        if let Some(h) = self.header {
            if !out.trim_start().starts_with('#') {
                out = format!("# {h}\n\n{out}");
            }
        }
        std::fs::write(&self.path, out)
            .map_err(|e| LabError(format!("{}: {e}", self.path.display())))
    }

    // ── hosts ───────────────────────────────────────────────────────────────

    pub fn add_host(
        &mut self,
        name: &str,
        ssh: &str,
        identity: Option<&str>,
        control_path: Option<&str>,
        paniolo_cmd: Option<&str>,
    ) -> Result<(), LabError> {
        let hosts = super_table(&mut self.doc, "hosts");
        if hosts.contains_key(name) {
            return lab_err(format!("host '{name}' already exists"));
        }
        let mut t = Table::new();
        t.insert("ssh", value(ssh));
        set_opt(&mut t, "identity", identity);
        set_opt(&mut t, "control_path", control_path);
        set_opt(&mut t, "paniolo_cmd", paniolo_cmd);
        hosts.insert(name, Item::Table(t));
        Ok(())
    }

    pub fn update_host(
        &mut self,
        name: &str,
        ssh: Option<&str>,
        identity: Option<&str>,
        control_path: Option<&str>,
        paniolo_cmd: Option<&str>,
    ) -> Result<(), LabError> {
        let t = self
            .doc
            .get_mut("hosts")
            .and_then(|i| i.as_table_mut())
            .and_then(|h| h.get_mut(name))
            .and_then(|i| i.as_table_mut())
            .ok_or_else(|| LabError(format!("no host '{name}'")))?;
        set_opt(t, "ssh", ssh);
        set_opt(t, "identity", identity);
        set_opt(t, "control_path", control_path);
        set_opt(t, "paniolo_cmd", paniolo_cmd);
        Ok(())
    }

    pub fn remove_host(&mut self, name: &str) -> Result<(), LabError> {
        let hosts = self
            .doc
            .get_mut("hosts")
            .and_then(|i| i.as_table_mut())
            .ok_or_else(|| LabError(format!("no host '{name}'")))?;
        if !hosts.contains_key(name) {
            return lab_err(format!("no host '{name}'"));
        }
        let refs = self.host_references(name);
        if !refs.is_empty() {
            return lab_err(format!(
                "host '{name}' is still used by: {}",
                refs.join(", ")
            ));
        }
        self.doc["hosts"].as_table_mut().unwrap().remove(name);
        Ok(())
    }

    fn host_references(&self, host: &str) -> Vec<String> {
        let lab: Lab = toml::from_str(&self.doc.to_string()).unwrap_or_default();
        let mut refs = Vec::new();
        for name in lab.targets.keys() {
            if let Some(rt) = lab.resolved_target(name) {
                if rt.default_host == host || rt.channels.iter().any(|c| c.host == host) {
                    refs.push(name.clone());
                }
            }
        }
        refs
    }

    // ── targets ───────────────────────────────────────────────────────────────

    pub fn add_target(
        &mut self,
        name: &str,
        host: Option<&str>,
        note: Option<&str>,
    ) -> Result<(), LabError> {
        let targets = super_table(&mut self.doc, "targets");
        if targets.contains_key(name) {
            return lab_err(format!("target '{name}' already exists"));
        }
        let mut t = Table::new();
        set_opt(&mut t, "host", host);
        set_opt(&mut t, "note", note);
        targets.insert(name, Item::Table(t));
        Ok(())
    }

    pub fn update_target(
        &mut self,
        name: &str,
        host: Option<&str>,
        note: Option<&str>,
    ) -> Result<(), LabError> {
        let t = self.target_mut(name)?;
        set_opt(t, "host", host);
        set_opt(t, "note", note);
        Ok(())
    }

    pub fn remove_target(&mut self, name: &str) -> Result<(), LabError> {
        let targets = self
            .doc
            .get_mut("targets")
            .and_then(|i| i.as_table_mut())
            .ok_or_else(|| LabError(format!("no target '{name}'")))?;
        if targets.remove(name).is_none() {
            return lab_err(format!("no target '{name}'"));
        }
        Ok(())
    }

    // ── serial channels (collection) ─────────────────────────────────────────

    pub fn add_serial(
        &mut self,
        target: &str,
        name: &str,
        device: &str,
        baud: i64,
        sense: Option<&str>,
        host: Option<&str>,
    ) -> Result<(), LabError> {
        let t = self.target_mut(target)?;
        if !t.contains_key("serial") {
            t.insert("serial", Item::ArrayOfTables(ArrayOfTables::new()));
        }
        let aot = t
            .get_mut("serial")
            .and_then(|i| i.as_array_of_tables_mut())
            .ok_or_else(|| LabError(format!("target '{target}': serial is not [[serial]]")))?;
        if aot
            .iter()
            .any(|s| s.get("name").and_then(|v| v.as_str()) == Some(name))
        {
            return lab_err(format!("target '{target}': serial '{name}' already exists"));
        }
        let mut s = Table::new();
        s.insert("name", value(name));
        s.insert("device", value(device));
        s.insert("baud", value(baud));
        set_opt(&mut s, "power_sense_signal", sense);
        set_opt(&mut s, "host", host);
        aot.push(s);
        Ok(())
    }

    pub fn update_serial(
        &mut self,
        target: &str,
        name: &str,
        device: Option<&str>,
        baud: Option<i64>,
        sense: Option<&str>,
        host: Option<&str>,
    ) -> Result<(), LabError> {
        let t = self.target_mut(target)?;
        let aot = t
            .get_mut("serial")
            .and_then(|i| i.as_array_of_tables_mut())
            .ok_or_else(|| LabError(format!("target '{target}': no serial '{name}'")))?;
        let s = aot
            .iter_mut()
            .find(|s| s.get("name").and_then(|v| v.as_str()) == Some(name))
            .ok_or_else(|| LabError(format!("target '{target}': no serial '{name}'")))?;
        set_opt(s, "device", device);
        if let Some(b) = baud {
            s.insert("baud", value(b));
        }
        set_opt(s, "power_sense_signal", sense);
        set_opt(s, "host", host);
        Ok(())
    }

    pub fn remove_serial(&mut self, target: &str, name: &str) -> Result<(), LabError> {
        let t = self.target_mut(target)?;
        let aot = t
            .get_mut("serial")
            .and_then(|i| i.as_array_of_tables_mut())
            .ok_or_else(|| LabError(format!("target '{target}': no serial '{name}'")))?;
        let idx = aot
            .iter()
            .position(|s| s.get("name").and_then(|v| v.as_str()) == Some(name))
            .ok_or_else(|| LabError(format!("target '{target}': no serial '{name}'")))?;
        aot.remove(idx);
        if aot.is_empty() {
            t.remove("serial");
        }
        Ok(())
    }

    // ── singleton channels (netboot / power / video) ─────────────────────────

    fn set_singleton(
        &mut self,
        target: &str,
        kind: &str,
        fields: &[(&str, Option<&str>)],
    ) -> Result<(), LabError> {
        let t = self.target_mut(target)?;
        if !t.contains_key(kind) {
            t.insert(kind, Item::Table(Table::new()));
        }
        let c = t
            .get_mut(kind)
            .and_then(|i| i.as_table_mut())
            .ok_or_else(|| LabError(format!("target '{target}': {kind} is not a table")))?;
        for (k, v) in fields {
            set_opt(c, k, *v);
        }
        Ok(())
    }

    fn remove_singleton(&mut self, target: &str, kind: &str) -> Result<(), LabError> {
        let t = self.target_mut(target)?;
        if t.remove(kind).is_none() {
            return lab_err(format!("target '{target}': no {kind} channel"));
        }
        Ok(())
    }

    pub fn set_netboot(
        &mut self,
        target: &str,
        interface: Option<&str>,
        host_ip: Option<&str>,
        tftp_root: Option<&str>,
        host: Option<&str>,
    ) -> Result<(), LabError> {
        self.set_singleton(
            target,
            "netboot",
            &[
                ("interface", interface),
                ("host_ip", host_ip),
                ("tftp_root", tftp_root),
                ("host", host),
            ],
        )
    }

    pub fn remove_netboot(&mut self, target: &str) -> Result<(), LabError> {
        self.remove_singleton(target, "netboot")
    }

    pub fn set_power(
        &mut self,
        target: &str,
        cycle_cmd: Option<&str>,
        serial_interface: Option<&str>,
        host: Option<&str>,
    ) -> Result<(), LabError> {
        self.set_singleton(
            target,
            "power",
            &[
                ("cycle_cmd", cycle_cmd),
                ("serial_interface", serial_interface),
                ("host", host),
            ],
        )
    }

    pub fn remove_power(&mut self, target: &str) -> Result<(), LabError> {
        self.remove_singleton(target, "power")
    }

    pub fn set_video(
        &mut self,
        target: &str,
        device: Option<&str>,
        host: Option<&str>,
    ) -> Result<(), LabError> {
        self.set_singleton(target, "video", &[("device", device), ("host", host)])
    }

    pub fn remove_video(&mut self, target: &str) -> Result<(), LabError> {
        self.remove_singleton(target, "video")
    }

    fn target_mut(&mut self, name: &str) -> Result<&mut Table, LabError> {
        self.doc
            .get_mut("targets")
            .and_then(|i| i.as_table_mut())
            .and_then(|t| t.get_mut(name))
            .and_then(|i| i.as_table_mut())
            .ok_or_else(|| LabError(format!("no target '{name}'")))
    }
}

/// Get or create an implicit super-table so children render as `[key.child]`.
fn super_table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Table {
    if doc.get(key).is_none() {
        let mut t = Table::new();
        t.set_implicit(true);
        doc.insert(key, Item::Table(t));
    }
    doc[key].as_table_mut().expect("super table")
}

/// Set a key when the value is present; leave it untouched otherwise.
fn set_opt(t: &mut Table, key: &str, v: Option<&str>) {
    if let Some(val) = v {
        t.insert(key, value(val));
    }
}

/// Validate by deserializing the live document and running the shared rulebook.
fn validate_doc(doc: &DocumentMut) -> Result<(), LabError> {
    let lab: Lab = toml::from_str(&doc.to_string()).map_err(|e| LabError(e.to_string()))?;
    model::validate(&lab)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lab.toml");
        (dir, path)
    }

    #[test]
    fn build_round_trips() {
        let (_d, path) = tmp();
        let mut lf = LabFile::create(&path);
        lf.add_host("bench1", "u@bench1", Some("~/.ssh/id"), None, None)
            .unwrap();
        lf.add_target("fortune", Some("bench1"), Some("note"))
            .unwrap();
        lf.set_netboot("fortune", Some("en0"), None, Some("/srv/tftp"), None)
            .unwrap();
        lf.add_serial("fortune", "console", "/dev/ttyUSB0", 115200, None, None)
            .unwrap();
        lf.add_serial("fortune", "bmc", "/dev/ttyUSB1", 9600, Some("cts"), None)
            .unwrap();
        lf.save().unwrap();

        let lab = model::load(&path).unwrap();
        let t = &lab.targets["fortune"];
        assert_eq!(t.host.as_deref(), Some("bench1"));
        assert_eq!(
            t.netboot.as_ref().unwrap().interface.as_deref(),
            Some("en0")
        );
        assert_eq!(t.serial.len(), 2);
        assert_eq!(t.serial[1].baud, 9600);
    }

    #[test]
    fn comments_preserved_across_edit() {
        let (_d, path) = tmp();
        std::fs::write(
            &path,
            "# hand-written\n[hosts.bench1]\nssh = \"u@b1\"  # noisy\n",
        )
        .unwrap();
        let mut lf = LabFile::load(&path).unwrap();
        lf.update_host("bench1", None, Some("~/.ssh/id"), None, None)
            .unwrap();
        lf.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# hand-written"), "{text}");
        assert!(text.contains("# noisy"), "{text}");
        assert!(text.contains("identity"), "{text}");
    }

    #[test]
    fn remove_host_blocked_while_referenced() {
        let (_d, path) = tmp();
        let mut lf = LabFile::create(&path);
        lf.add_host("bench1", "u@b1", None, None, None).unwrap();
        lf.add_target("fortune", Some("bench1"), None).unwrap();
        let e = lf.remove_host("bench1").unwrap_err();
        assert!(e.0.contains("still used by: fortune"), "{}", e.0);
    }

    #[test]
    fn duplicate_serial_rejected() {
        let (_d, path) = tmp();
        let mut lf = LabFile::create(&path);
        lf.add_target("t", None, None).unwrap();
        lf.add_serial("t", "console", "/dev/a", 115200, None, None)
            .unwrap();
        let e = lf
            .add_serial("t", "console", "/dev/b", 115200, None, None)
            .unwrap_err();
        assert!(e.0.contains("already exists"), "{}", e.0);
    }

    #[test]
    fn unknown_host_ref_fails_on_save() {
        let (_d, path) = tmp();
        let mut lf = LabFile::create(&path);
        lf.add_target("t", Some("ghost"), None).unwrap();
        assert!(lf.save().is_err());
    }

    #[test]
    fn remove_last_serial_drops_array() {
        let (_d, path) = tmp();
        let mut lf = LabFile::create(&path);
        lf.add_target("t", None, None).unwrap();
        lf.add_serial("t", "console", "/dev/a", 115200, None, None)
            .unwrap();
        lf.remove_serial("t", "console").unwrap();
        lf.save().unwrap();
        let lab = model::load(&path).unwrap();
        assert!(lab.targets["t"].serial.is_empty());
    }
}
