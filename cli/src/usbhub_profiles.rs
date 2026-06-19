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

//! Bundled `usbhub` hub profiles — the `share/paniolo/usbhub/profiles` analogue
//! of [`crate::skills`]. paniolo ships human-verified per-port power profiles
//! for off-the-shelf USB hubs; this module resolves and installs them so the
//! `usbhub` helper can switch a known hub's ports without the user re-running
//! `usbhub learn`.
//!
//! `usbhub` is a separate binary, so it can't climb to the repo or know the
//! package layout itself. paniolo passes the resolved search path to it as
//! `USBHUB_LIBRARY_PATH` (see [`crate::daemons::helper_env`]); the order here
//! mirrors [`crate::skills::skills_dirs`]: the in-repo `usbhub/profiles/` when
//! run from a checkout, then the per-user data dir, the CLI-relative dir
//! (Homebrew keg / prefix install), then the system package dir. A profile the
//! user verified locally still wins — usbhub puts its own config dir ahead of
//! this library path.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

/// Per-user shipped-profiles dir: `~/.local/share/paniolo/usbhub/profiles`. The
/// install target for `paniolo setup`; the first library dir [`library_dirs`]
/// lists after the in-repo copy.
pub fn user_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".local/share/paniolo/usbhub/profiles"))
}

/// Shipped-profiles dir of a system package (.deb/tarball):
/// `/usr/share/paniolo/usbhub/profiles`. Always present in the search path.
fn system_dir() -> PathBuf {
    PathBuf::from("/usr/share/paniolo/usbhub/profiles")
}

/// Shipped-profiles dir relative to the running CLI, after resolving symlinks —
/// the `usbhub/profiles` analogue of [`crate::skills`]'s keg/prefix lookup.
fn exe_relative_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let prefix = exe.parent()?.parent()?;
    Some(prefix.join("share/paniolo/usbhub/profiles"))
}

/// The shipped-profile directories, highest priority first: the in-repo
/// `usbhub/profiles/` when run from a source checkout (so an author's edits
/// show up without reinstalling), then the per-user data dir, the CLI-relative
/// dir (Homebrew keg / prefix), and the system package dir. paniolo passes
/// these to the usbhub helper as `USBHUB_LIBRARY_PATH`.
pub fn library_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(repo) = crate::setup::find_repo_root() {
        dirs.push(repo.join("usbhub/profiles"));
    }
    dirs.extend(user_dir());
    dirs.extend(exe_relative_dir());
    dirs.push(system_dir());
    dirs
}

/// Install the profiles bundled in a source checkout into the per-user data
/// dir, so the usbhub helper finds them when the installed CLI runs outside the
/// tree. Copies each `usbhub/profiles/*.toml`; returns how many were installed
/// (0 if the source tree has no profiles dir).
pub fn install_bundled(repo: &Path) -> Result<usize> {
    let src = repo.join("usbhub/profiles");
    if !src.is_dir() {
        return Ok(0);
    }
    let dst = user_dir().ok_or_else(|| anyhow!("could not determine the home directory"))?;
    std::fs::create_dir_all(&dst)?;
    let entries = std::fs::read_dir(&src).map_err(|e| anyhow!("reading {}: {e}", src.display()))?;
    let mut count = 0;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|x| x == "toml") {
            if let Some(name) = path.file_name() {
                std::fs::copy(&path, dst.join(name))?;
                count += 1;
            }
        }
    }
    Ok(count)
}
