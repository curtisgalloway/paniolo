// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! The error contract, end to end: run the built binary and check the exit
//! status and the JSON error object for each kind of failure
//! (`docs/dev/error-contract/design.md`). The inputs are the failures a
//! consumer has to tell apart.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const LAB: &str = r#"
[targets.dut]

[[targets.dut.serial]]
name = "console"
device = "/dev/ttyUSB-test"
baud = 115200
"#;

/// A fresh directory holding `lab.toml` (and an empty runtime dir, so no
/// daemon running on the machine under test can answer).
fn scratch(name: &str, lab: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("error_contract")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("run")).unwrap();
    std::fs::write(dir.join("lab.toml"), lab).unwrap();
    dir
}

fn paniolo(dir: &Path, json: bool, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_paniolo"));
    cmd.arg("--lab")
        .arg(dir.join("lab.toml"))
        .args(args)
        .env("PANIOLO_RUNTIME_BASE", dir.join("run"))
        .env_remove("PANIOLO_JSON_ERRORS")
        .env_remove("PANIOLO_LAB");
    if json {
        cmd.env("PANIOLO_JSON_ERRORS", "1");
    }
    cmd.output().unwrap()
}

/// The JSON object: the last line of stderr, parsed.
fn error_object(out: &Output) -> Value {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let last = stderr.lines().last().expect("stderr is empty");
    let v: Value = serde_json::from_str(last)
        .unwrap_or_else(|e| panic!("last stderr line is not JSON ({e}): {last}"));
    v["error"].clone()
}

/// Run `args` with JSON errors on; assert the code, kind, target and channel,
/// and that the message is the prose line printed above the object.
fn expect(
    name: &str,
    args: &[&str],
    code: i32,
    kind: &str,
    target: Option<&str>,
    channel: Option<&str>,
) {
    let dir = scratch(name, LAB);
    let out = paniolo(&dir, true, args);
    assert_eq!(out.status.code(), Some(code), "{args:?}: {out:?}");
    let e = error_object(&out);
    assert_eq!(e["kind"], kind, "{e}");
    assert_eq!(e["code"], code, "{e}");
    assert_eq!(e["target"].as_str(), target, "{e}");
    assert_eq!(e["channel"].as_str(), channel, "{e}");
    assert!(e["host"].is_null() && e["child_exit"].is_null(), "{e}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let prose = stderr.lines().rev().nth(1).unwrap();
    assert_eq!(e["message"], prose, "{e}");
}

#[test]
fn unknown_target_is_not_configured() {
    expect(
        "unknown_pc",
        &["power-cycle", "nosuch"],
        3,
        "not_configured",
        Some("nosuch"),
        None,
    );
    expect(
        "unknown_log",
        &["serial", "log", "-t", "nosuch"],
        3,
        "not_configured",
        Some("nosuch"),
        None,
    );
}

#[test]
fn missing_channel_is_not_configured() {
    expect(
        "no_hid",
        &["hid", "send", "-t", "dut", "releaseall"],
        3,
        "not_configured",
        Some("dut"),
        Some("hid"),
    );
    expect(
        "no_power",
        &["power-cycle", "dut"],
        3,
        "not_configured",
        Some("dut"),
        Some("power"),
    );
}

#[test]
fn missing_serial_interface_is_not_configured() {
    expect(
        "no_iface",
        &["serial", "log", "-t", "dut", "-i", "nosuch"],
        3,
        "not_configured",
        Some("dut"),
        Some("serial"),
    );
}

#[test]
fn missing_lab_file_is_not_configured() {
    let dir = scratch("no_lab", LAB);
    std::fs::remove_file(dir.join("lab.toml")).unwrap();
    let out = paniolo(&dir, true, &["power-cycle", "dut"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert_eq!(error_object(&out)["kind"], "not_configured");
}

#[test]
fn ambiguous_target_is_usage() {
    let dir = scratch("two", "[targets.a]\n[targets.b]\n");
    let out = paniolo(&dir, true, &["power-cycle"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(error_object(&out)["kind"], "usage");
}

#[test]
fn clap_usage_errors_exit_2_with_a_usage_object() {
    let dir = scratch("clap", LAB);
    let out = paniolo(&dir, true, &["power-cycle", "--bogus"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let e = error_object(&out);
    assert_eq!(e["kind"], "usage");
    assert_eq!(e["message"], "unexpected argument '--bogus' found");
    // The flag form works too, though parsing (which reads it) failed.
    let out = paniolo(&dir, false, &["--json-errors", "power-cycle", "--bogus"]);
    assert_eq!(error_object(&out)["code"], 2);
    // --help is not a failure: no object.
    let out = paniolo(&dir, true, &["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("\"error\""));
}

/// Not yet classified: the stopped-daemon check. M3 of the plan moves it to
/// `daemon_down` (100); until then it is the standing example of `internal`.
#[test]
fn unclassified_error_is_internal() {
    expect(
        "internal",
        &["serial", "send", "-t", "dut", "hello"],
        109,
        "internal",
        None,
        None,
    );
}

#[test]
fn no_json_unless_requested_and_the_flag_matches_the_variable() {
    let dir = scratch("opt_in", LAB);
    let plain = paniolo(&dir, false, &["power-cycle", "nosuch"]);
    assert_eq!(plain.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&plain.stderr),
        "target 'nosuch' not found in lab\n",
        "without the opt-in, stderr is exactly the 0.4 text"
    );
    let flag = paniolo(&dir, false, &["--json-errors", "power-cycle", "nosuch"]);
    let var = paniolo(&dir, true, &["power-cycle", "nosuch"]);
    assert_eq!(flag.stderr, var.stderr);
    assert!(flag.stdout.is_empty(), "stdout stays clean");
}

/// Serial interfaces on two hosts and no `-i`: the caller must name one, so
/// this is a usage error (like an ambiguous target), decided before any
/// dispatch is attempted.
#[test]
fn serial_on_two_hosts_without_an_interface_is_usage() {
    let lab = r#"
[hosts.bench1]
ssh = "u@bench1"
[hosts.bench2]
ssh = "u@bench2"
[targets.nuc]
host = "bench1"
[[targets.nuc.serial]]
name = "console"
device = "/dev/ttyUSB0"
[[targets.nuc.serial]]
name = "debug"
device = "/dev/ttyUSB1"
host = "bench2"
"#;
    let dir = scratch("two_hosts", lab);
    let out = paniolo(&dir, true, &["serial", "log", "-t", "nuc"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let e = error_object(&out);
    assert_eq!(e["kind"], "usage");
    assert_eq!(e["target"], "nuc");
    assert_eq!(e["channel"], "serial");
}

#[test]
fn unknown_dtr_interface_is_not_configured() {
    expect(
        "dtr_iface",
        &["serial", "dtr", "-t", "dut", "-i", "nosuch"],
        3,
        "not_configured",
        Some("dut"),
        Some("serial"),
    );
}

#[test]
fn ambiguous_dtr_interface_is_usage() {
    let lab = r#"
[targets.nuc]
[[targets.nuc.serial]]
name = "a"
device = "/dev/ttyUSB0"
power_button = true
[[targets.nuc.serial]]
name = "b"
device = "/dev/ttyUSB1"
power_button = true
"#;
    let dir = scratch("dtr_two", lab);
    let out = paniolo(&dir, true, &["serial", "dtr", "-t", "nuc"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(error_object(&out)["kind"], "usage");
}

#[test]
fn serial_watch_without_interfaces_is_not_configured() {
    let dir = scratch("watch_bare", "[targets.bare]\n");
    let out = paniolo(&dir, true, &["serial", "watch", "-t", "bare"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let e = error_object(&out);
    assert_eq!(e["target"], "bare");
    assert_eq!(e["channel"], "serial");
}

#[test]
fn missing_subcommand_is_usage_with_a_plain_message() {
    let dir = scratch("no_sub", LAB);
    let out = paniolo(&dir, true, &["power"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert_eq!(error_object(&out)["message"], "a subcommand is required");
}

#[test]
fn a_repeated_json_errors_flag_is_accepted() {
    let dir = scratch("repeat", LAB);
    let out = paniolo(
        &dir,
        false,
        &["--json-errors", "--json-errors", "power-cycle", "nosuch"],
    );
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert_eq!(error_object(&out)["kind"], "not_configured");
}
