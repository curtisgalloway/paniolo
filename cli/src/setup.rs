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

//! `paniolo setup` — build and install paniolo's binaries from a source clone.
//!
//! Installs the daemons (hdmicap, serialcap, netbootd) **and the paniolo CLI
//! itself** via `cargo install` into `~/.cargo/bin`, so every control host runs
//! from one stable installed path. On macOS it also setuid-installs the
//! netbootd bpf-helper (the only root component) and compiles the visionocr OCR
//! helper; on Linux it checks dialout/video group membership and installs
//! linuxocr. The legacy `tftp-now` brew step is gone — netbootd serves TFTP.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Result};

/// The crates `setup` builds and installs, in order. `cli` is the paniolo
/// binary itself — the single-binary deployment this rewrite exists for.
const CRATES: [&str; 6] = [
    "hdmicap",
    "serialcap",
    "netbootd",
    "cambrionix",
    "hidrig",
    "cli",
];

fn is_repo_root(d: &Path) -> bool {
    d.join("pyproject.toml").is_file()
        && d.join("ocr").is_dir()
        && d.join("hdmicap/Cargo.toml").is_file()
}

/// Locate the paniolo source checkout: the current directory and its parents.
/// (The installed binary has no `__file__` to climb from; `make install` and
/// hand-run setups both execute inside the clone.)
pub fn find_repo_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let mut d: Option<&Path> = Some(cwd.as_path());
    while let Some(p) = d {
        if is_repo_root(p) {
            return Some(p.to_path_buf());
        }
        d = p.parent();
    }
    None
}

fn cargo_bin() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".cargo/bin")
}

fn user_in_group(group: &str) -> bool {
    Command::new("id")
        .arg("-nG")
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .any(|g| g == group)
        })
        .unwrap_or(false)
}

fn group_exists(group: &str) -> bool {
    Command::new("getent")
        .args(["group", group])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Add the user to dialout/video if needed (Linux). Returns true if anything
/// changed (a re-login is needed for it to take effect).
fn ensure_linux_groups() -> bool {
    let user = std::env::var("USER").unwrap_or_default();
    let mut changed = false;
    for (group, reason) in [
        ("dialout", "serial port access (/dev/ttyUSB*, /dev/ttyACM*)"),
        ("video", "V4L2 capture device access (/dev/video*)"),
    ] {
        if !group_exists(group) {
            continue;
        }
        if user_in_group(group) {
            println!("  ✓ {group:12} already a member");
        } else {
            let ok = Command::new("sudo")
                .args(["usermod", "-aG", group, &user])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                println!("  ✓ {group:12} added ({reason})");
                changed = true;
            } else {
                eprintln!("  ✗ {group:12} could not add ({reason})");
            }
        }
    }
    changed
}

/// Run the local setup from a source checkout at `repo`.
pub fn run(repo: &Path) -> Result<()> {
    let bin_dir = cargo_bin();

    if cfg!(target_os = "macos") {
        println!("  ℹ macOS: netbootd serves DHCP+TFTP; no system TFTP tool needed.");
    } else {
        println!(
            "  ℹ Linux: before building, ensure system packages are installed:\n\
             \x20   sudo apt-get install build-essential pkg-config libudev-dev libclang-dev"
        );
        println!("\nChecking group membership…");
        if ensure_linux_groups() {
            println!(
                "\nNote: group changes take effect after you log out and back in \
                 (or run `newgrp dialout` in the current shell)."
            );
        }
    }

    let cargo = crate::daemons::find_binary("cargo")
        .ok_or_else(|| anyhow!("cargo not found — install Rust (https://rustup.rs)"))?;

    for crate_name in CRATES {
        let crate_dir = repo.join(crate_name);
        if !crate_dir.join("Cargo.toml").is_file() {
            println!(
                "  … {crate_name}: source not found at {}, skipping",
                crate_dir.display()
            );
            continue;
        }
        println!("  building {crate_name} (cargo install — may take a few minutes)…");
        let status = Command::new(&cargo)
            .args(["install", "--path"])
            .arg(&crate_dir)
            .arg("--force")
            .status()?;
        if !status.success() {
            bail!("{crate_name}: cargo install failed");
        }
        // The `cli` package installs a binary named `paniolo`.
        let bin_name = if crate_name == "cli" {
            "paniolo"
        } else {
            crate_name
        };
        println!("  ✓ {bin_name:12} {}", bin_dir.join(bin_name).display());
    }

    // netbootd's macOS raw-frame send path needs a /dev/bpf descriptor, which
    // only root can open. The setuid bpf-helper is the ONLY root component; its
    // sole job is opening /dev/bpf and handing the fd to the unprivileged
    // netbootd. cargo install resets the mode, so the setuid bit is re-applied
    // after every (re)install.
    if cfg!(target_os = "macos") {
        let helper = bin_dir.join("netbootd-bpf-helper");
        if helper.is_file() {
            println!("  … installing netbootd-bpf-helper setuid-root (one-time sudo)");
            let chown = Command::new("sudo")
                .args(["chown", "root:wheel"])
                .arg(&helper)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            let chmod = Command::new("sudo")
                .args(["chmod", "4755"])
                .arg(&helper)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if chown && chmod {
                println!("  ✓ {:12} setuid-root  {}", "bpf-helper", helper.display());
            } else {
                eprintln!(
                    "  ! could not setuid netbootd-bpf-helper; the netboot send path \
                     falls back to the kernel (broken on macOS 15+). Re-run \
                     `paniolo setup` with sudo access to fix."
                );
            }
        } else {
            println!("  … netbootd-bpf-helper not found; skipping setuid install");
        }
    }

    // OCR helper: visionocr (swiftc) on macOS, linuxocr copy on Linux.
    if cfg!(target_os = "macos") {
        let source = repo.join("ocr/visionocr.swift");
        let dest = bin_dir.join("visionocr");
        if !source.is_file() {
            println!("  … visionocr: source not found, skipped");
        } else if crate::daemons::find_binary("swiftc").is_none() {
            println!("  … visionocr: swiftc not found (install Xcode CLT), skipped");
        } else {
            let ok = Command::new("swiftc")
                .args(["-O", "-o"])
                .arg(&dest)
                .arg(&source)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                println!("  ✓ {:12} {}", "visionocr", dest.display());
            } else {
                println!("  … visionocr: build failed, skipped");
            }
        }
    } else {
        let source = repo.join("ocr/linuxocr");
        let dest = bin_dir.join("linuxocr");
        if source.is_file() {
            std::fs::copy(&source, &dest)?;
            let mut perms = std::fs::metadata(&dest)?.permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
            std::fs::set_permissions(&dest, perms)?;
            println!("  ✓ {:12} {}", "linuxocr", dest.display());
        } else {
            println!("  … linuxocr: source not found, skipped");
        }
        if crate::daemons::find_binary("tesseract").is_none() {
            println!(
                "  ! tesseract not found — install it for OCR:\n\
                 \x20   sudo apt-get install tesseract-ocr"
            );
        }
    }

    // zigplug: Python (zigpy-znp) Zigbee smart plug helper, installed as a uv
    // tool so the `zigplug` command resolves from power hooks without a venv.
    let zigplug_dir = repo.join("zigplug");
    if !zigplug_dir.join("pyproject.toml").is_file() {
        println!("  … zigplug: source not found, skipped");
    } else if let Some(uv) = crate::daemons::find_binary("uv") {
        let ok = Command::new(&uv)
            .args(["tool", "install", "--force", "--quiet"])
            .arg(&zigplug_dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            println!("  ✓ {:12} (uv tool install)", "zigplug");
        } else {
            eprintln!("  ! zigplug: uv tool install failed, skipped");
        }
    } else {
        println!("  … zigplug: uv not found (https://docs.astral.sh/uv), skipped");
    }

    println!("\nSetup complete.");
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d == bin_dir))
        .unwrap_or(false);
    if !on_path {
        println!(
            "Note: add {} to your PATH so the binaries resolve.",
            bin_dir.display()
        );
    }
    Ok(())
}
