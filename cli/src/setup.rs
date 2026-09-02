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
//!
//! Without a checkout (a packaged install — Homebrew, .deb, tarball),
//! `setup` skips the builds and runs just the platform steps against the
//! installed binaries: see [`run_packaged`].

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Result};

/// The helper crates `setup` builds and installs into libexec, in order. The
/// `cli` crate (the `paniolo` binary itself) installs separately onto PATH.
const HELPER_CRATES: [&str; 8] = [
    "hdmicap",
    "serialcap",
    "netbootd",
    "cambrionix",
    "hidrig",
    "ch9329",
    "shellyplug",
    "amt",
];

fn is_repo_root(d: &Path) -> bool {
    d.join("Makefile").is_file()
        && d.join("ocr").is_dir()
        && d.join("cli/Cargo.toml").is_file()
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

/// netbootd's macOS raw-frame send path needs a /dev/bpf descriptor, which
/// only root can open. The setuid bpf-helper is the ONLY root component; its
/// sole job is opening /dev/bpf and handing the fd to the unprivileged
/// netbootd. Installs and upgrades (cargo and packages alike) reset the mode,
/// so the setuid bit must be re-applied after each one.
///
/// Mode 4755 (world-executable) is acceptable because the helper gates
/// itself rather than relying on the file mode: it refuses any caller whose
/// real uid is not the owner of the directory it lives in (the installing
/// user), refuses to bind the default-route interface, and hands out only a
/// write-only descriptor with a reject-all filter. Another local user can
/// run it and gets nothing from it.
///
/// Before touching the file, [`helper_safe_to_setuid`] confirms it is what
/// the invoking user installed — a regular file (not a symlink) they own, or
/// the root-owned setuid helper a previous run already produced. Anything
/// else is refused rather than promoted to setuid-root.
fn setuid_bpf_helper(helper: &Path) {
    if let Err(why) = helper_safe_to_setuid(helper) {
        eprintln!(
            "  ! refusing to setuid {}: {why}. Reinstall the helper \
             (`make install` or `brew reinstall paniolo`) and re-run `paniolo setup`.",
            helper.display()
        );
        return;
    }
    println!("  … installing netbootd-bpf-helper setuid-root (one-time sudo)");
    let chown = Command::new("sudo")
        .args(["chown", "root:wheel"])
        .arg(helper)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let chmod = Command::new("sudo")
        .args(["chmod", "4755"])
        .arg(helper)
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
}

/// Whether `helper` is a file `paniolo setup` may promote to setuid-root:
/// a regular file — `symlink_metadata`, so a symlink is seen as a symlink and
/// refused rather than followed — that is either owned by the invoking user
/// (freshly installed by `cargo install` / `make install` / the keg) or
/// already root-owned with the setuid bit (a previous run). A file some other
/// uid placed there is refused: `sudo chown root` + `chmod 4755` on it would
/// hand that uid a root-run binary of their choosing.
#[cfg(unix)]
fn helper_safe_to_setuid(helper: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let meta = std::fs::symlink_metadata(helper).map_err(|e| anyhow!("cannot stat: {e}"))?;
    if !meta.file_type().is_file() {
        bail!("not a regular file (a symlink?)");
    }
    // Direct getuid rather than platform::current_uid(): that wrapper's
    // contract excludes authorization decisions (it is a hash on Windows), and
    // this compares against a real file owner. Unix-only, so it is exact here.
    let me = unsafe { libc::getuid() };
    // POSIX setuid bit; spelled out because libc::S_ISUID is u16 on macOS and
    // u32 on Linux, so a cast is needed on one and flagged on the other.
    const S_ISUID: u32 = 0o4000;
    let already_setuid_root = meta.uid() == 0 && meta.mode() & S_ISUID != 0;
    if meta.uid() != me && !already_setuid_root {
        bail!(
            "owned by uid {}, not the invoking user (uid {me})",
            meta.uid()
        );
    }
    Ok(())
}

/// setuid is a Unix concept; the macOS-only caller never runs here.
#[cfg(not(unix))]
fn helper_safe_to_setuid(_helper: &Path) -> Result<()> {
    bail!("setuid is not supported on this platform")
}

/// Finish platform setup for a packaged install (Homebrew, .deb, tarball) —
/// Build the venv `ocr/rapidocr` re-execs into, when a lab file asks for it.
///
/// Only built when some target sets `ocr_mode = "gui"`. It is ~317 MB
/// (onnxruntime 58 MB, PP-OCRv6 models 31 MB, numpy/opencv the rest) and most
/// control hosts never look at a GUI screen, so it is opt-in rather than part
/// of every setup.
///
/// A venv rather than a system install because Pi OS is PEP 668-managed and
/// refuses one — better a self-contained directory than asking anyone to reach
/// for `--break-system-packages`.
///
/// `opencv-python-headless` is forced over the `opencv-python` rapidocr pulls
/// in: the full build needs `libGL.so.1`, absent on a headless Pi OS, and the
/// failure is an ImportError at first OCR rather than at install time.
#[cfg(target_os = "linux")]
fn install_rapidocr_venv(libexec: &Path) {
    if !lab_wants_gui_ocr() {
        println!(
            "  … rapidocr venv: skipped (no target sets video ocr_mode = \"gui\"; \
             it is ~317 MB — set the field and re-run to install)"
        );
        return;
    }
    let venv = libexec.join("ocr-venv");
    if venv.join("bin/python3").is_file() {
        println!("  ✓ {:12} {}", "ocr-venv", venv.display());
        return;
    }
    println!("  … building the rapidocr venv (~317 MB, a few minutes)…");
    let made = Command::new("python3")
        .args(["-m", "venv"])
        .arg(&venv)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !made {
        println!("  ! rapidocr venv: `python3 -m venv` failed (is python3-venv installed?)");
        return;
    }
    let pip = venv.join("bin/pip");
    let installed = Command::new(&pip)
        .args(["install", "--quiet", "rapidocr>=3,<4", "onnxruntime"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !installed {
        println!("  ! rapidocr venv: pip install failed; GUI OCR falls back to tesseract");
        return;
    }
    let _ = Command::new(&pip)
        .args(["uninstall", "-y", "-q", "opencv-python"])
        .status();
    let headless = Command::new(&pip)
        .args([
            "install",
            "--quiet",
            "--force-reinstall",
            "--no-deps",
            "opencv-python-headless",
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if headless {
        println!("  ✓ {:12} {}", "ocr-venv", venv.display());
    } else {
        println!("  ! rapidocr venv: headless opencv install failed; it will fail on libGL");
    }
}

#[cfg(not(target_os = "linux"))]
fn install_rapidocr_venv(_libexec: &Path) {}

/// Does the user's lab file ask for GUI-mode OCR anywhere?
///
/// Read as text rather than parsed: this runs before any lab is loaded, and the
/// only question is whether to spend 317 MB.
#[cfg(target_os = "linux")]
fn lab_wants_gui_ocr() -> bool {
    std::fs::read_to_string(crate::model::default_lab_path())
        .map(|s| s.contains("ocr_mode") && s.contains("\"gui\""))
        .unwrap_or(false)
}

/// Verify the portable Windows layout: helpers alongside the CLI in `libexec`.
///
/// This is `paniolo setup`'s whole job on Windows — see [`run_packaged`]. It
/// reports rather than repairs, because the fix (re-extract the zip, or
/// reinstall via winget) is the user's to make.
#[cfg(windows)]
fn check_windows_layout() {
    let helpers = [
        "hdmicap",
        "serialcap",
        "netbootd",
        "cambrionix",
        "hidrig",
        "ch9329",
        "shellyplug",
        "amt",
    ];
    println!("\nChecking the installed helper layout…");
    let mut missing = Vec::new();
    for h in helpers {
        match crate::daemons::find_binary(h) {
            Some(p) => println!("  ✓ {h:12} {}", p.display()),
            None => missing.push(h),
        }
    }
    if missing.is_empty() {
        return;
    }
    println!(
        "  ! missing helpers: {}\n\
         \x20   Expected them in a `libexec` directory beside paniolo.exe. \
         Re-extract the release zip keeping its directory structure, or \
         reinstall with `winget install CurtisGalloway.Paniolo`.",
        missing.join(", ")
    );
}

#[cfg(not(windows))]
fn check_windows_layout() {}

/// One platform-specific step of a packaged (no-checkout) setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackagedStep {
    /// macOS: make the installed netbootd-bpf-helper setuid-root.
    SetuidBpfHelper,
    /// Windows: check the portable zip layout is intact.
    WindowsLayout,
    /// Linux: add the user to the device-access groups.
    LinuxGroups,
    /// Linux: hint at installing tesseract for OCR.
    TesseractHint,
}

/// The packaged-setup steps for `os` (a `std::env::consts::OS` value). Kept
/// as a pure function so each CI platform can assert its own steps: the
/// Linux group step once sat under a Windows branch and nothing noticed.
fn packaged_steps(os: &str) -> Vec<PackagedStep> {
    match os {
        "macos" => vec![PackagedStep::SetuidBpfHelper],
        "windows" => vec![PackagedStep::WindowsLayout],
        _ => vec![PackagedStep::LinuxGroups, PackagedStep::TesseractHint],
    }
}

/// no source checkout, so no builds: setuid the installed bpf-helper on
/// macOS (located via `find_binary`, which knows the per-user libexec, the
/// Homebrew keg, and `/usr/libexec/paniolo/bin`), and check group
/// membership on Linux. Building or refreshing the daemons, OCR helper, and
/// zigplug still needs a clone (`make install`).
pub fn run_packaged() -> Result<()> {
    println!("No source checkout found — finishing setup for the installed paniolo.");
    for step in packaged_steps(std::env::consts::OS) {
        match step {
            PackagedStep::SetuidBpfHelper => {
                match crate::daemons::find_binary("netbootd-bpf-helper") {
                    Some(helper) => setuid_bpf_helper(&helper),
                    None => {
                        println!("  … netbootd-bpf-helper not found; skipping setuid install")
                    }
                }
            }
            // Nothing to grant on Windows: there is no setuid bit, no dialout
            // group, and the OCR helper has no Windows build. The one thing
            // worth checking is that the portable layout is intact, since a zip
            // extracted without its `libexec` directory yields a CLI that runs
            // and then fails on the first channel it needs a helper for.
            PackagedStep::WindowsLayout => check_windows_layout(),
            PackagedStep::LinuxGroups => {
                if ensure_linux_groups() {
                    println!(
                        "\nNote: group changes take effect after you log out and back in \
                         (or run `newgrp dialout` in the current shell)."
                    );
                }
            }
            PackagedStep::TesseractHint => {
                if crate::daemons::find_binary("tesseract").is_none() {
                    println!(
                        "  ! tesseract not found — install it for OCR:\n\
                         \x20   sudo apt-get install tesseract-ocr"
                    );
                }
            }
        }
    }
    println!(
        "\nSetup complete. (Rebuilding the daemons, OCR helper, or zigplug \
         needs a source checkout — see `make install` in the repo.)"
    );
    println!("Agent skills shipped with the package — list them with `paniolo skill`.");
    Ok(())
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
                 \x20   sudo apt-get install build-essential pkg-config libudev-dev libclang-dev cmake nasm"
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

    if cfg!(target_os = "macos") {
        let helper = libexec.join("netbootd-bpf-helper");
        if helper.is_file() {
            setuid_bpf_helper(&helper);
        } else {
            println!("  … netbootd-bpf-helper not found; skipping setuid install");
        }
    }

    // OCR helper, one per platform: visionocr (swiftc) on macOS, winocr (cargo)
    // on Windows, a linuxocr copy on Linux. See docs/ocr.md.
    if cfg!(windows) {
        let source = repo.join("ocr/winocr");
        if !source.join("Cargo.toml").is_file() {
            println!("  … winocr: source not found, skipped");
        } else {
            let ok = Command::new(&cargo)
                .args(["build", "--release", "--manifest-path"])
                .arg(source.join("Cargo.toml"))
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            let built = source.join("target/release/winocr.exe");
            if ok && built.is_file() {
                let dest = libexec.join("winocr.exe");
                std::fs::copy(&built, &dest)?;
                println!("  ✓ {:12} {}", "winocr", dest.display());
            } else {
                println!("  … winocr: build failed, skipped");
            }
        }
    } else if cfg!(target_os = "macos") {
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
            crate::platform::make_executable(&dest)?;
            println!("  ✓ {:12} {}", "linuxocr", dest.display());
        } else {
            println!("  … linuxocr: source not found, skipped");
        }
        // rapidocr: the GUI-mode engine on Linux. The script is small and always
        // copied; the heavy part is its venv, installed only when a lab file
        // actually asks for GUI OCR.
        let rsrc = repo.join("ocr/rapidocr");
        if rsrc.is_file() {
            let rdest = libexec.join("rapidocr");
            std::fs::copy(&rsrc, &rdest)?;
            crate::platform::make_executable(&rdest)?;
            println!("  ✓ {:12} {}", "rapidocr", rdest.display());
            install_rapidocr_venv(&libexec);
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

    // Agent skills: copy the bundled SKILL.md guides into the per-user data
    // dir so `paniolo skill` finds them when the installed CLI runs outside
    // this tree. From a checkout the repo copy is used directly, so this keeps
    // an installed paniolo in sync. (Linux packages ship them to /usr/share.)
    match crate::skills::install_bundled(repo) {
        Ok(0) => println!(
            "  … skills: none found under {}",
            repo.join("skills").display()
        ),
        Ok(n) => {
            let dst = crate::skills::user_skills_dir().unwrap_or_default();
            println!("  ✓ {:12} {n} installed → {}", "skills", dst.display());
        }
        Err(e) => eprintln!("  ! skills: {e}"),
    }

    println!("\nSetup complete.");
    println!(
        "Helpers live in {} — list or run them via `paniolo helper`.",
        libexec.display()
    );
    println!("Agent skills are bundled too — list them with `paniolo skill`.");
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `setup` promotes the bpf-helper to setuid-root, so it must only do so
    /// to a regular file the invoking user owns — never to a symlink, which
    /// `chown`/`chmod` would follow to wherever it points.
    #[cfg(unix)]
    #[test]
    fn helper_safe_to_setuid_accepts_own_file_and_refuses_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("netbootd-bpf-helper");
        std::fs::write(&real, b"").unwrap();
        helper_safe_to_setuid(&real).expect("own regular file is accepted");

        let link = dir.path().join("netbootd-bpf-helper-link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let err = helper_safe_to_setuid(&link).unwrap_err().to_string();
        assert!(err.contains("not a regular file"), "{err}");

        let missing = dir.path().join("absent");
        let err = helper_safe_to_setuid(&missing).unwrap_err().to_string();
        assert!(err.contains("cannot stat"), "{err}");
    }

    #[test]
    fn packaged_steps_grant_groups_on_linux_not_windows() {
        // Regression: the Linux group/tesseract steps were nested under the
        // Windows branch, so a .deb install never joined dialout/video.
        let linux = packaged_steps("linux");
        assert!(linux.contains(&PackagedStep::LinuxGroups));
        assert!(linux.contains(&PackagedStep::TesseractHint));
        assert!(!linux.contains(&PackagedStep::WindowsLayout));
        assert!(!linux.contains(&PackagedStep::SetuidBpfHelper));
        assert_eq!(packaged_steps("windows"), vec![PackagedStep::WindowsLayout]);
        assert_eq!(packaged_steps("macos"), vec![PackagedStep::SetuidBpfHelper]);
    }

    /// The markers `is_repo_root` keys off must match the *real* tree.
    ///
    /// A tmpdir-fixture test would be useless here: it would create whatever
    /// files the predicate currently names and keep passing forever after one
    /// of them was deleted from the repo. That is exactly how the
    /// `pyproject.toml` marker outlived the legacy Python CLI removal and
    /// silently broke `paniolo setup --rust-only` (and, quietly, the
    /// repo-checkout branch of the skill search path).
    /// Assert against the actual checkout so marker drift fails the build.
    #[test]
    fn repo_root_detects_the_real_checkout() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the cli crate always has a parent directory");
        assert!(
            is_repo_root(repo),
            "is_repo_root() no longer recognizes the checkout at {}: a marker \
             it checks for was renamed or removed",
            repo.display()
        );
    }

    #[test]
    fn repo_root_rejects_a_non_repo_directory() {
        assert!(!is_repo_root(Path::new("/")));
        let cli = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(!is_repo_root(cli), "the cli crate dir is not the repo root");
    }
}
