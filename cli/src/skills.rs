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

//! `paniolo skill` — discover and read the agent-facing skills paniolo ships.
//!
//! A *skill* is a markdown guide (`<dir>/<name>/SKILL.md`, with a `name:` +
//! `description:` YAML frontmatter) that teaches an agent how to drive paniolo:
//! the `paniolo` usage skill, the `kvm-puppeting` GUI doctrine, the `usbhub`
//! power skill. They live in the source tree under `skills/` and install
//! alongside the CLI; this command is how an agent finds and reads them without
//! the harness having them pre-loaded.
//!
//! Skills resolve from a search path that mirrors
//! [`crate::daemons::helper_dirs`] but under `share/` instead of `libexec/`:
//! the in-repo `skills/` when run from a checkout (so an author's edits show up
//! immediately), then the per-user data dir
//! (`~/.local/share/paniolo/skills`), then dirs relative to the running CLI
//! (Homebrew keg / prefix install), then the system package dir
//! (`/usr/share/paniolo/skills`). The command mirrors `paniolo helper`: no NAME
//! lists every skill, a NAME prints that skill's `SKILL.md`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

/// Per-user skills dir: `~/.local/share/paniolo/skills`. The install target
/// for `paniolo setup`; the first installed location [`skills_dirs`] searches.
pub fn user_skills_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".local/share/paniolo/skills"))
}

/// Skills dir of a system package (.deb/tarball): `/usr/share/paniolo/skills`.
/// Always present in the search path, so [`skills_dirs`] is never empty.
fn system_skills_dir() -> PathBuf {
    PathBuf::from("/usr/share/paniolo/skills")
}

/// Skills dir relative to the running CLI, after resolving symlinks — the
/// `share/` analogue of [`crate::daemons`]'s libexec lookup. Homebrew links
/// `<prefix>/bin/paniolo` into the versioned keg, so `<keg>/share/paniolo/skills`
/// is the keg's bundled skills; an FHS-style prefix install resolves the same
/// way. A relocated install thus finds its skills without enumerating package
/// managers.
fn exe_relative_skills_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let prefix = exe.parent()?.parent()?;
    Some(prefix.join("share/paniolo/skills"))
}

/// The skills directories, in resolution order: the in-repo `skills/` when run
/// from a source checkout, then the per-user data dir, the CLI-relative dir
/// (Homebrew keg / prefix), and the system package dir. The first directory
/// that holds a given skill name wins, so a checkout or per-user install
/// shadows the packaged copy.
pub fn skills_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(repo) = crate::setup::find_repo_root() {
        dirs.push(repo.join("skills"));
    }
    dirs.extend(user_skills_dir());
    dirs.extend(exe_relative_skills_dir());
    dirs.push(system_skills_dir());
    dirs
}

/// One discovered skill: its name (the directory name), the `SKILL.md` path,
/// and the one-line description pulled from the frontmatter.
struct Skill {
    name: String,
    path: PathBuf,
    description: String,
}

/// Every skill found across [`skills_dirs`], deduped by name (first dir wins),
/// sorted by name. A "skill" is any `<dir>/<name>/SKILL.md`.
fn discover() -> Vec<Skill> {
    let mut found: Vec<Skill> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for dir in skills_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let manifest = entry.path().join("SKILL.md");
            if !manifest.is_file() {
                continue;
            }
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if !seen.insert(name.clone()) {
                continue; // an earlier (higher-priority) dir already has it.
            }
            let description = read_description(&manifest);
            found.push(Skill {
                name,
                path: manifest,
                description,
            });
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// Pull the `description:` field out of a SKILL.md's YAML frontmatter as a
/// single collapsed line. Handles both an inline value (`description: text`)
/// and a folded/literal block scalar (`description: >` followed by indented
/// lines). Returns an empty string when there is no frontmatter or no field —
/// the skill still lists, just without a summary.
fn read_description(path: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return String::new();
    }
    let mut collecting = false;
    let mut parts: Vec<String> = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break; // end of frontmatter
        }
        if collecting {
            // A block scalar continues while lines stay indented; a new
            // unindented `key:` ends it.
            let indented = line.starts_with(char::is_whitespace);
            if indented && !trimmed.is_empty() {
                parts.push(trimmed.to_string());
                continue;
            }
            if trimmed.is_empty() {
                continue;
            }
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("description:") {
            let rest = rest.trim();
            // `>`, `|`, `>-`, `|+`, … introduce a multi-line block scalar.
            if rest.is_empty() || rest.starts_with('>') || rest.starts_with('|') {
                collecting = true;
            } else {
                return rest.trim_matches(|c| c == '"' || c == '\'').to_string();
            }
        }
    }
    parts.join(" ")
}

/// `paniolo skill [NAME] [--path]`: list every bundled skill, or print one.
pub fn run(name: Option<&str>, path: bool) -> Result<()> {
    match name {
        None => list(),
        Some(name) => show(name, path),
    }
}

/// List each skill — name, then its frontmatter description — with a hint on
/// how to read one in full.
fn list() -> Result<()> {
    let skills = discover();
    if skills.is_empty() {
        let searched = skills_dirs()
            .iter()
            .map(|d| d.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "No skills found (searched {searched}) — install the paniolo package \
             or run `paniolo setup` from a source checkout."
        );
        return Ok(());
    }
    for s in &skills {
        println!("{}", s.name);
        if !s.description.is_empty() {
            println!("    {}", s.description);
        }
        println!();
    }
    println!("Read one with `paniolo skill <name>` (or --path for its file path).");
    Ok(())
}

/// Print a single skill: its `SKILL.md` contents, or (with `path`) just the
/// resolved file path so an agent can `Read` it or a user can open it.
fn show(name: &str, path: bool) -> Result<()> {
    let skills = discover();
    let skill = skills.iter().find(|s| s.name == name).ok_or_else(|| {
        let have: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        anyhow!(
            "skill '{name}' not found (skills: {}). List them with `paniolo skill`.",
            if have.is_empty() {
                "(none)".to_string()
            } else {
                have.join(", ")
            }
        )
    })?;
    if path {
        println!("{}", skill.path.display());
        return Ok(());
    }
    let body = std::fs::read_to_string(&skill.path)
        .map_err(|e| anyhow!("reading {}: {e}", skill.path.display()))?;
    print!("{body}");
    Ok(())
}

/// Install the skills bundled in a source checkout into the per-user data dir,
/// so `paniolo skill` finds them when the installed CLI runs outside the tree.
/// Copies each `skills/<name>/SKILL.md`; returns how many were installed.
pub fn install_bundled(repo: &Path) -> Result<usize> {
    let src = repo.join("skills");
    let dst_root =
        user_skills_dir().ok_or_else(|| anyhow!("could not determine the home directory"))?;
    let entries = std::fs::read_dir(&src).map_err(|e| anyhow!("reading {}: {e}", src.display()))?;
    let mut count = 0;
    for entry in entries.filter_map(|e| e.ok()) {
        let manifest = entry.path().join("SKILL.md");
        if !manifest.is_file() {
            continue;
        }
        let name = entry.file_name();
        let dst_dir = dst_root.join(&name);
        std::fs::create_dir_all(&dst_dir)?;
        std::fs::copy(&manifest, dst_dir.join("SKILL.md"))?;
        count += 1;
    }
    Ok(count)
}
