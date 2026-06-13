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

//! Traversal-safe path resolution shared by the file servers (TFTP + HTTP).
//!
//! Both servers expose a single rooted directory and must never let a request
//! escape it. [`resolve`] joins the requested name under `root`, canonicalizes
//! it, and confirms the result is still inside the canonicalized root —
//! rejecting `..` traversal, symlink escapes, and (because `canonicalize` fails
//! on a missing path) probes for files that do not exist.

use std::path::{Path, PathBuf};

/// Resolve a requested filename inside `root`, rejecting traversal outside it.
///
/// A leading `/` is treated as relative to `root`, never as a host filesystem
/// path. Returns `None` for anything that canonicalizes outside `root` or does
/// not exist.
pub fn resolve(root: &Path, filename: &str) -> Option<PathBuf> {
    let rel = filename.trim_start_matches('/');
    let candidate = root.join(rel);
    let canon = candidate.canonicalize().ok()?;
    let root_canon = root.canonicalize().ok()?;
    canon.starts_with(&root_canon).then_some(canon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A unique, freshly-created temp dir (no `tempfile` dependency, matching
    /// the sibling modules' pattern).
    fn tmp() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "netbootd-served-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn resolve_accepts_file_inside_root() {
        let root = tmp();
        fs::write(root.join("kernel.img"), b"x").unwrap();
        let got = resolve(&root, "kernel.img").expect("file in root resolves");
        assert!(got.ends_with("kernel.img"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_strips_leading_slash() {
        let root = tmp();
        fs::write(root.join("boot.img"), b"x").unwrap();
        // An absolute-looking request is treated as relative to root, never as
        // a host filesystem path.
        assert!(resolve(&root, "/boot.img").is_some());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_accepts_file_in_subdir() {
        let root = tmp();
        let sub = root.join("grub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("grub.cfg"), b"set timeout=0").unwrap();
        // Follow-on fetches (e.g. GRUB reading grub.cfg) live in subdirectories.
        assert!(resolve(&root, "grub/grub.cfg").is_some());
        assert!(resolve(&root, "/grub/grub.cfg").is_some());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_rejects_traversal_outside_root() {
        // Lay out  base/secret  and serve from  base/served . A "../secret"
        // request must be rejected even though the target genuinely exists.
        let base = tmp();
        let served = base.join("served");
        fs::create_dir_all(&served).unwrap();
        fs::write(base.join("secret"), b"top secret").unwrap();

        assert!(
            resolve(&served, "../secret").is_none(),
            "must not escape root"
        );
        assert!(resolve(&served, "../../etc/passwd").is_none());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn resolve_rejects_missing_file() {
        let root = tmp();
        // canonicalize() fails for a nonexistent path -> None (no info leak about
        // whether a sibling outside root exists).
        assert!(resolve(&root, "nope.img").is_none());
        fs::remove_dir_all(&root).ok();
    }
}
