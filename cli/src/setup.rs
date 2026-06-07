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
//! The paniolo CLI installs via `cargo install` into `~/.cargo/bin` — the one
//! user-facing command. The helpers (hdmicap, serialcap, netbootd, cambrionix,
//! hidrig, the OCR helper, zigplug) install into the private libexec dir
//! (`daemons::libexec_dir()`, `~/.local/libexec/paniolo/bin`) so they stay off
//! PATH; paniolo resolves them itself and `paniolo helper <name> …` runs one
//! directly. On macOS setup also setuid-installs the netbootd bpf-helper (the
//! only root component) and compiles the visionocr OCR helper; on Linux it
//! checks dialout/video group membership and installs linuxocr. The legacy
//! `tftp-now` brew step is gone — netbootd serves TFTP.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Result};

/// The helper crates `setup` builds and installs into libexec, in order. The
/// `cli` crate (the `paniolo` binary itself) installs separately onto PATH.
const HELPER_CRATES: [&str; 6] = [
    "hdmicap",
    "serialcap",
    "netbootd",
    "cambrionix",
    "hidrig",
    "usbhub",
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

/// Run the local setup from a source checkout at `repo`. With `rust_only`,
/// stop after the cargo installs (skip the OCR, setuid, zigplug, and
/// stale-copy-cleanup steps) — the fast path for iterating on the Rust code.
pub fn run(repo: &Path, rust_only: bool) -> Result<()> {
    let bin_dir = cargo_bin();
    let libexec_root = crate::daemons::libexec_root()
        .ok_or_else(|| anyhow!("could not determine the home directory"))?;
    // cargo install --root appends bin/ itself; keep in sync with
    // daemons::libexec_dir().
    let libexec = libexec_root.join("bin");
    std::fs::create_dir_all(&libexec)?;

    if !rust_only {
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
    }

    let cargo = crate::daemons::find_binary("cargo")
        .ok_or_else(|| anyhow!("cargo not found — install Rust (https://rustup.rs)"))?;

    // Helpers go to the private libexec dir (--root), keeping them off PATH.
    for crate_name in HELPER_CRATES {
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
            .arg("--root")
            .arg(&libexec_root)
            .arg("--force")
            .status()?;
        if !status.success() {
            bail!("{crate_name}: cargo install failed");
        }
        println!("  ✓ {crate_name:12} {}", libexec.join(crate_name).display());
    }

    // The paniolo CLI itself: the one user-facing binary, installed on PATH.
    println!("  building cli (cargo install — may take a few minutes)…");
    let status = Command::new(&cargo)
        .args(["install", "--path"])
        .arg(repo.join("cli"))
        .arg("--force")
        .status()?;
    if !status.success() {
        bail!("cli: cargo install failed");
    }
    println!("  ✓ {:12} {}", "paniolo", bin_dir.join("paniolo").display());

    if rust_only {
        println!("\nRust crates installed (skipped OCR/setuid/zigplug — run `paniolo setup`).");
        return Ok(());
    }

    // One-time migration: drop pre-libexec helper copies from ~/.cargo/bin so
    // a stale binary can't shadow or version-skew against the libexec install.
    // cargo uninstall keeps the install receipts tidy; the direct remove
    // covers receiptless leftovers (and visionocr/linuxocr, never cargo's).
    for crate_name in HELPER_CRATES {
        let installed = bin_dir.join(crate_name);
        if !installed.is_file() {
            continue;
        }
        let _ = Command::new(&cargo)
            .args(["uninstall", crate_name])
            .output();
        if installed.is_file() {
            let _ = std::fs::remove_file(&installed);
        }
        if !installed.is_file() {
            println!("  ✓ removed stale {}", installed.display());
        }
    }
    for loose in ["netbootd-bpf-helper", "visionocr", "linuxocr"] {
        let stale = bin_dir.join(loose);
        if stale.is_file() && std::fs::remove_file(&stale).is_ok() {
            println!("  ✓ removed stale {}", stale.display());
        }
    }

    // netbootd's macOS raw-frame send path needs a /dev/bpf descriptor, which
    // only root can open. The setuid bpf-helper is the ONLY root component; its
    // sole job is opening /dev/bpf and handing the fd to the unprivileged
    // netbootd. cargo install resets the mode, so the setuid bit is re-applied
    // after every (re)install.
    if cfg!(target_os = "macos") {
        let helper = libexec.join("netbootd-bpf-helper");
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
        let dest = libexec.join("visionocr");
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
        let dest = libexec.join("linuxocr");
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
    // tool. UV_TOOL_BIN_DIR points the shim at libexec (the venv stays in
    // uv's tool dir) so the command resolves from power hooks without living
    // on PATH. The uninstall first clears any pre-libexec shim from uv's
    // default bin dir (~/.local/bin).
    let zigplug_dir = repo.join("zigplug");
    if !zigplug_dir.join("pyproject.toml").is_file() {
        println!("  … zigplug: source not found, skipped");
    } else if let Some(uv) = crate::daemons::find_binary("uv") {
        let _ = Command::new(&uv)
            .args(["tool", "uninstall", "zigplug"])
            .output();
        let ok = Command::new(&uv)
            .env("UV_TOOL_BIN_DIR", &libexec)
            .args(["tool", "install", "--force", "--quiet"])
            .arg(&zigplug_dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            println!("  ✓ {:12} {}", "zigplug", libexec.join("zigplug").display());
        } else {
            eprintln!("  ! zigplug: uv tool install failed, skipped");
        }
        // Belt and braces: an orphaned pre-libexec shim survives a lost uv
        // receipt; remove it so PATH can't resolve a stale zigplug.
        if let Some(stale) = dirs::home_dir().map(|h| h.join(".local/bin/zigplug")) {
            if stale.is_file() && std::fs::remove_file(&stale).is_ok() {
                println!("  ✓ removed stale {}", stale.display());
            }
        }
    } else {
        println!("  … zigplug: uv not found (https://docs.astral.sh/uv), skipped");
    }

    println!("\nSetup complete.");
    println!(
        "Helpers live in {} — list or run them via `paniolo helper`.",
        libexec.display()
    );
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d == bin_dir))
        .unwrap_or(false);
    if !on_path {
        println!(
            "Note: add {} to your PATH so `paniolo` resolves.",
            bin_dir.display()
        );
    }
    Ok(())
}
