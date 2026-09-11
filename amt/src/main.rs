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

//! amt — power control for Intel AMT (vPro) machines over WS-Management.
//!
//! A one-shot paniolo power helper: each invocation speaks SOAP-over-HTTP to
//! the machine's Management Engine and exits. The ME answers whether the host
//! is on, off, or has no OS installed at all, which is what makes AMT both a
//! power switch and — unlike a smart plug driven blind — a power *sensor*.
//! Hook-facing subcommands follow the paniolo helper conventions
//! (docs/adding-power-helpers.md):
//!   state         prints exactly `on` or `off` (errors on an unmapped state)
//!   on/off        request the state + confirm by read-back
//!   cycle         off → confirm → delay → on → confirm

mod rpc;

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context as _, Result};
use clap::{Parser, Subcommand};

use rpc::{
    is_transient, power_state_name, Client, CALL_TIMEOUT, KVM_DISABLED, KVM_ENABLED,
    KVM_ENABLED_OFFLINE, MIN_CALL_TIMEOUT, PS_OFF_SOFT, PS_ON,
};

/// How long a commanded transition may take before read-back confirmation
/// fails. Power rail changes are visible to the ME quickly; this bounds a
/// machine that ignored the request.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(20);

/// Read-back polling interval while waiting for a transition.
const POLL: Duration = Duration::from_millis(1000);

#[derive(Parser)]
#[command(
    name = "amt",
    version,
    about = "Power control for Intel AMT (vPro) machines over WS-Management",
    long_about = "Power control for Intel AMT (vPro) machines over WS-Management (SOAP over \
HTTP on port 16992, HTTP Digest auth).

MENTAL MODEL
  - Commands talk to the machine's Management Engine (ME), which runs on
    standby power: it answers with the host on, off, or bare-metal (no OS).
    So `state` is a true power *sensor*, not a guess.
  - A machine is addressed by -d/--device (a hostname, IPv4 address, or
    bracketed IPv6 literal, optionally with :port; port 16992 by default)
    and -u/--user (default admin).
  - The Digest password comes ONLY from the AMT_PASSWORD environment
    variable — never from a flag or config file, so it cannot leak into a
    lab file, shell history, or `ps` output. Inject it at call time, e.g.:
      op run --env-file .env -- bash -c 'amt state -d 10.0.0.5'
    (single quotes: the parent shell must not expand $AMT_PASSWORD itself).
  - on/off/cycle confirm by reading the power state back, so a request the
    firmware ignored surfaces as a non-zero exit.
  - `off` is an unconditional hardware power-off (CIM \"Off - Soft\", like
    holding the power button) — the OS does not shut down gracefully.
  - `cycle` holds the machine off unless it is already off: a sleeping or
    hibernating host is powered off and cold-booted, not merely resumed.

TYPICAL USE
  amt -d 10.0.0.5 status          firmware identity + power state detail
  amt -d 10.0.0.5 state           prints exactly `on` or `off`
  amt -d 10.0.0.5 on|off
  amt -d 10.0.0.5 cycle [--delay-ms 3000]

TLS-provisioned AMT (port 16993) is not supported; this helper speaks the
plain WS-Man port only."
)]
struct Cli {
    /// AMT address: a hostname, IPv4 address, or bracketed IPv6 literal
    /// (`[fe80::1]`), optionally with a port (default 16992); an `http://`
    /// prefix is accepted. Anything URL-shaped beyond host[:port] is rejected.
    #[arg(short = 'd', long = "device", value_name = "HOST", global = true)]
    device: Option<String>,

    /// AMT Digest username.
    #[arg(
        short = 'u',
        long = "user",
        value_name = "USER",
        global = true,
        default_value = "admin"
    )]
    user: String,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print exactly `on` or `off` (hook: state_cmd). `on` means the host is
    /// running (PowerState 2); sleep, hibernate, and soft-off all print `off`.
    /// Any other reported PowerState is an error (non-zero exit), not a guess.
    State,
    /// Power the host on and confirm by read-back (hook: on_cmd).
    On,
    /// Power the host off (unconditional, not a graceful OS shutdown) and
    /// confirm by read-back (hook: off_cmd).
    Off,
    /// Power-cycle: off → confirm → delay → on → confirm (hook: cycle_cmd). A
    /// genuine cold boot — a sleeping or hibernating host is held off too, so
    /// it does not merely resume — and the off-hold lets the PSU drain before
    /// power returns.
    Cycle {
        /// Milliseconds to hold the machine off before restoring power.
        #[arg(long, default_value_t = 3000)]
        delay_ms: u64,
    },
    /// Human-readable AMT firmware identity and power state detail.
    Status,
    /// Inspect or change KVM redirection (the ME's built-in VNC server).
    Kvm {
        #[command(subcommand)]
        cmd: KvmCmd,
    },
}

#[derive(Subcommand)]
enum KvmCmd {
    /// Report whether port 5900 is open, whether a local user must consent,
    /// and the redirection service's EnabledState.
    Status,
    /// Open port 5900 to standard VNC clients and enable redirection.
    ///
    /// The RFB password comes from AMT_RFB_PASSWORD in the environment, never
    /// a flag: it must contain a special character, and a shell eats those.
    Enable {
        /// Require a person at the machine to consent to each session. Off by
        /// default — a headless bench target has nobody to click the prompt.
        #[arg(long)]
        opt_in: bool,
        /// Idle minutes before the ME drops a session; 0 disables the timeout.
        #[arg(long, default_value_t = 0)]
        session_timeout: u16,
    },
    /// Close port 5900 and disable redirection. Leaves the stored RFB password
    /// alone, so re-enabling does not require setting it again.
    Disable,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let device = cli
        .device
        .as_deref()
        .ok_or_else(|| anyhow!("required option '--device <HOST>' (-d) was not provided"))?;
    let password = std::env::var("AMT_PASSWORD").map_err(|_| {
        anyhow!(
            "AMT_PASSWORD is not set — the AMT Digest password comes from the \
             environment, never from a flag or config file. Inject it at call \
             time, e.g.: op run --env-file .env -- bash -c 'amt state -d <host>'"
        )
    })?;
    let client = Client::new(device, &cli.user, &password)?;
    match cli.cmd {
        Cmd::State => cmd_state(&client),
        Cmd::On => cmd_on(&client),
        Cmd::Off => cmd_off(&client),
        Cmd::Cycle { delay_ms } => cmd_cycle(&client, delay_ms),
        Cmd::Status => cmd_status(&client),
        Cmd::Kvm { cmd } => match cmd {
            KvmCmd::Status => cmd_kvm_status(&client),
            KvmCmd::Enable {
                opt_in,
                session_timeout,
            } => cmd_kvm_enable(&client, opt_in, session_timeout),
            KvmCmd::Disable => cmd_kvm_disable(&client),
        },
    }
}

/// Name a `CIM_KVMRedirectionSAP.EnabledState` value.
fn kvm_state_name(state: u16) -> &'static str {
    match state {
        KVM_ENABLED => "enabled",
        KVM_DISABLED => "disabled",
        KVM_ENABLED_OFFLINE => "enabled (no session attached)",
        _ => "unknown",
    }
}

fn cmd_kvm_status(client: &Client) -> Result<()> {
    let s = client.kvm_settings()?;
    let state = client.kvm_enabled_state()?;
    println!("redirection    {} ({state})", kvm_state_name(state));
    println!(
        "port 5900      {}",
        if s.port_5900_enabled {
            "open to VNC clients"
        } else {
            "closed"
        }
    );
    println!(
        "user consent   {}",
        if s.opt_in_policy {
            "required (a person must approve each session)"
        } else {
            "not required"
        }
    );
    println!(
        "session timeout {}",
        if s.session_timeout == 0 {
            "none".to_string()
        } else {
            format!("{} min", s.session_timeout)
        }
    );
    println!(
        "enabled in MEBx {}",
        if s.enabled_by_mebx { "yes" } else { "NO" }
    );
    Ok(())
}

/// Validate the RFB password here rather than letting AMT reject it: AMT
/// **locks** the password after a few failed authentication attempts, so a
/// malformed one is worth catching before it is ever written or tried.
fn check_rfb_password(pw: &str) -> Result<()> {
    let mut problems = Vec::new();
    if pw.chars().count() != 8 {
        problems.push(format!(
            "must be exactly 8 characters (got {})",
            pw.chars().count()
        ));
    }
    if !pw.chars().any(|c| c.is_ascii_uppercase()) {
        problems.push("needs a capital letter".into());
    }
    if !pw.chars().any(|c| c.is_ascii_lowercase()) {
        problems.push("needs a lowercase letter".into());
    }
    if !pw.chars().any(|c| c.is_ascii_digit()) {
        problems.push("needs a digit".into());
    }
    if !pw.chars().any(|c| !c.is_alphanumeric()) {
        problems.push("needs a special character".into());
    }
    // Intel's IPS_KVMRedirectionSettingData reference: "RFB password can't
    // accept the characters: '"' ',' ':'". They satisfy the special-character
    // rule above, so without this a password like `Ab3:defG` looks valid here
    // and is refused by the firmware instead.
    let forbidden: Vec<char> = pw
        .chars()
        .filter(|c| matches!(c, '"' | ',' | ':'))
        .collect();
    if !forbidden.is_empty() {
        problems.push(format!(
            "AMT forbids these characters in an RFB password: {}",
            forbidden
                .iter()
                .map(|c| format!("'{c}'"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        bail!("AMT_RFB_PASSWORD rejected: {}", problems.join("; "))
    }
}

fn cmd_kvm_enable(client: &Client, opt_in: bool, session_timeout: u16) -> Result<()> {
    let before = client.kvm_settings()?;
    if !before.enabled_by_mebx {
        bail!(
            "KVM is not enabled in MEBx on this machine; no remote setting can \
             turn it on — enable it in the firmware setup screen first"
        );
    }
    let rfb = std::env::var("AMT_RFB_PASSWORD").ok();
    if let Some(pw) = rfb.as_deref() {
        check_rfb_password(pw)?;
    }

    // Port 5900 can be opened when a password "is already set or is set in the
    // same Put request" (Intel's IPS_KVMRedirectionSettingData reference), and
    // a Get never reveals whether one is stored — it always returns the field
    // empty. So do not refuse up front for a missing AMT_RFB_PASSWORD: a
    // previous `kvm enable` may have set one that `kvm disable` preserved.
    // Attempt the Put and explain only if the firmware actually objects. This
    // costs nothing: a Put authenticates with the AMT admin Digest credential,
    // so a rejection cannot contribute to the RFB password lockout.
    client
        .set_kvm(rfb.as_deref(), true, opt_in, session_timeout)
        .map_err(|e| {
            if rfb.is_none() {
                e.context(
                    "AMT_RFB_PASSWORD is not set, and the firmware refused to open port \
                     5900 — most likely no RFB password is stored. It is a separate \
                     secret from AMT_PASSWORD: exactly 8 characters with a capital, a \
                     lowercase, a digit and a special character, and not '\"' ',' or \
                     ':'. Pass it in the environment, never on the command line, where \
                     a shell will eat the special character.",
                )
            } else {
                e
            }
        })?;

    // The settings are now applied. If enabling redirection fails, say so
    // rather than returning a bare error: the firmware has already changed,
    // and an operator who does not know that will not think to back it out.
    client.kvm_request_state(KVM_ENABLED).map_err(|e| {
        e.context(
            "the settings were written (RFB password, consent policy and timeout are \
             applied) but redirection could not be enabled. The machine is in a \
             half-applied state: re-run `kvm enable` to retry just the state change, \
             or `kvm disable` to back it out.",
        )
    })?;
    cmd_kvm_status(client)
}

fn cmd_kvm_disable(client: &Client) -> Result<()> {
    let before = client.kvm_settings()?;
    // Close the port before stopping redirection, so that a failure partway
    // leaves the more conservative half applied: a closed 5900 with redirection
    // still running is strictly less exposed than an open one.
    client
        .set_kvm(None, false, before.opt_in_policy, before.session_timeout)
        .context("port 5900 could not be closed; nothing was changed")?;
    client.kvm_request_state(KVM_DISABLED).map_err(|e| {
        e.context(
            "port 5900 is now closed, but redirection could not be stopped. No VNC \
             client can reach the machine, so this is not an exposure — but the \
             machine is half-applied: re-run `kvm disable` to finish.",
        )
    })?;
    cmd_kvm_status(client)
}

/// The `state_cmd` token for a CIM PowerState. `on` only for On (2); `off`
/// for every documented state in which the OS is not running; anything else
/// is an error carrying the raw value — the contract wants a real read-back,
/// never a guess.
fn onoff(ps: u16) -> Result<&'static str> {
    match ps {
        PS_ON => Ok("on"),
        // DMTF CIM PowerState value map (see `power_state_name`): 3 Sleep -
        // Light, 4 Sleep - Deep, 6 Off - Hard, 7 Hibernate (Off - Soft),
        // 8 Off - Soft, 12 Off - Soft Graceful, 13 Off - Hard Graceful. All
        // are resting states with the host not running — the documented
        // "sleep/hibernate = off". The power-cycle and reset values (5, 9,
        // 10, 11, 14–16) name transitions rather than states, and 1 (Other),
        // 0, or an undocumented value is something this helper cannot vouch
        // for either way.
        3 | 4 | 6 | 7 | PS_OFF_SOFT | 12 | 13 => Ok("off"),
        other => bail!(
            "AMT reports PowerState {other} ({}), which does not map to on or off",
            power_state_name(other)
        ),
    }
}

/// Whether `cycle` must hold the machine off before powering it on. Only a
/// host already at Off - Soft skips the off phase: a sleeping (S3) or
/// hibernating host still holds its state in RAM or on disk, and a bare On
/// request would merely resume it instead of cold-booting.
fn needs_off_phase(ps: u16) -> bool {
    ps != PS_OFF_SOFT
}

fn cmd_state(client: &Client) -> Result<()> {
    let ps = client.power_state()?;
    println!("{}", onoff(ps)?);
    Ok(())
}

fn cmd_on(client: &Client) -> Result<()> {
    if client.power_state()? == PS_ON {
        println!("power: already on");
        return Ok(());
    }
    request_retrying(client, PS_ON)?;
    wait_until(client, |ps| ps == PS_ON, "on")?;
    println!("power: on");
    Ok(())
}

fn cmd_off(client: &Client) -> Result<()> {
    if client.power_state()? == PS_OFF_SOFT {
        println!("power: already off");
        return Ok(());
    }
    request_retrying(client, PS_OFF_SOFT)?;
    wait_until(client, |ps| ps == PS_OFF_SOFT, "off")?;
    println!("power: off");
    Ok(())
}

fn cmd_cycle(client: &Client, delay_ms: u64) -> Result<()> {
    let before = client.power_state()?;
    let held_off = needs_off_phase(before);
    if held_off {
        request_retrying(client, PS_OFF_SOFT)?;
        wait_until(client, |ps| ps == PS_OFF_SOFT, "off")?;
        thread::sleep(Duration::from_millis(delay_ms));
    }
    request_retrying(client, PS_ON)?;
    wait_until(client, |ps| ps == PS_ON, "on")?;
    if before == PS_ON {
        println!("power: cycled ({delay_ms} ms off)");
    } else if held_off {
        println!(
            "power: cycled (was {}; held off {delay_ms} ms)",
            power_state_name(before)
        );
    } else {
        println!("power: cycled (was already off; powered on)");
    }
    Ok(())
}

fn cmd_status(client: &Client) -> Result<()> {
    let ps = client.power_state()?;
    let ident = client
        .server_ident()
        .unwrap_or_else(|| "(no Server header)".to_string());
    println!("firmware {ident}");
    println!(
        "power    {} — {} (PowerState {ps})",
        onoff(ps).unwrap_or("unmapped"),
        power_state_name(ps)
    );
    Ok(())
}

/// The HTTP budget for one attempt against `deadline`: the normal
/// [`CALL_TIMEOUT`], shortened to whatever is left of the retry budget so a
/// hung target cannot overrun the deadline by a whole extra attempt — but
/// never below [`MIN_CALL_TIMEOUT`], so an attempt issued right at the
/// deadline still gets a real chance to answer.
fn attempt_timeout(now: Instant, deadline: Instant) -> Duration {
    deadline
        .saturating_duration_since(now)
        .clamp(MIN_CALL_TIMEOUT, CALL_TIMEOUT)
}

/// Issue a power request, retrying transient transport failures until
/// [`CONFIRM_TIMEOUT`]. The machine's NIC drops link for a few seconds
/// around power transitions (observed on the bench), and power requests are
/// idempotent, so retrying through that window is safe.
fn request_retrying(client: &Client, state: u16) -> Result<()> {
    let deadline = Instant::now() + CONFIRM_TIMEOUT;
    loop {
        let timeout = attempt_timeout(Instant::now(), deadline);
        match client.request_power_state_within(state, timeout) {
            Err(e) if is_transient(&e) && Instant::now() < deadline => thread::sleep(POLL),
            other => return other,
        }
    }
}

/// Poll the power state until `done` accepts it, or fail after
/// [`CONFIRM_TIMEOUT`] naming the state the machine is stuck in. Transient
/// transport errors keep polling (see [`request_retrying`]); at the deadline
/// they propagate.
fn wait_until(client: &Client, done: impl Fn(u16) -> bool, what: &str) -> Result<()> {
    let deadline = Instant::now() + CONFIRM_TIMEOUT;
    loop {
        let timeout = attempt_timeout(Instant::now(), deadline);
        match client.power_state_within(timeout) {
            Ok(ps) if done(ps) => return Ok(()),
            Ok(ps) if Instant::now() >= deadline => bail!(
                "commanded {what} but the machine still reports {} (PowerState {ps}) \
                 after {}s",
                power_state_name(ps),
                CONFIRM_TIMEOUT.as_secs()
            ),
            Err(e) if !is_transient(&e) || Instant::now() >= deadline => return Err(e),
            _ => {}
        }
        thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_holds_off_unless_already_soft_off() {
        // On, sleep (light/deep), hibernate, and Other all get a real
        // off-hold; only Off - Soft (8) skips straight to power-on.
        for ps in [2, 3, 4, 7, 1] {
            assert!(needs_off_phase(ps), "PowerState {ps} must be held off");
        }
        assert!(!needs_off_phase(8));
    }

    #[test]
    fn state_token_maps_only_documented_states() {
        assert_eq!(onoff(2).unwrap(), "on");
        for ps in [3, 4, 6, 7, 8, 12, 13] {
            assert_eq!(onoff(ps).unwrap(), "off", "PowerState {ps}");
        }
    }

    #[test]
    fn state_token_refuses_to_guess() {
        // Other, zero, the transition values, and anything undocumented are
        // errors that name the raw value rather than a guessed `off`.
        for ps in [0, 1, 5, 9, 10, 11, 14, 15, 16, 99] {
            let err = onoff(ps).expect_err(&format!("PowerState {ps} must not map"));
            assert!(
                err.to_string().contains(&format!("PowerState {ps}")),
                "error for {ps} must carry the raw value: {err}"
            );
        }
    }

    #[test]
    fn attempt_timeout_is_bounded_by_the_remaining_budget() {
        let now = Instant::now();
        // Plenty of budget left: the normal per-call timeout.
        assert_eq!(
            attempt_timeout(now, now + Duration::from_secs(60)),
            CALL_TIMEOUT
        );
        // Less budget than a full call: only what is left.
        assert_eq!(
            attempt_timeout(now, now + Duration::from_secs(4)),
            Duration::from_secs(4)
        );
        // At or past the deadline: the floor, never zero.
        assert_eq!(attempt_timeout(now, now), MIN_CALL_TIMEOUT);
        assert_eq!(
            attempt_timeout(now + Duration::from_secs(5), now),
            MIN_CALL_TIMEOUT
        );
    }

    #[test]
    fn rfb_password_accepts_a_conforming_secret() {
        assert!(check_rfb_password("Ab3!defG").is_ok());
        assert!(check_rfb_password("xY9@zwvU").is_ok());
    }

    #[test]
    fn rfb_password_enforces_length_and_all_four_classes() {
        // Each of these is wrong in exactly one way.
        for (pw, want) in [
            ("Ab3!def", "exactly 8"),
            ("Ab3!defGH", "exactly 8"),
            ("ab3!defg", "capital"),
            ("AB3!DEFG", "lowercase"),
            ("Abc!defG", "digit"),
            ("Ab3xdefG", "special"),
        ] {
            let err = check_rfb_password(pw)
                .expect_err(&format!("{pw:?} should have been rejected"))
                .to_string();
            assert!(
                err.contains(want),
                "{pw:?} rejected for the wrong reason: {err}"
            );
        }
    }

    /// Intel forbids `"`, `,` and `:` in an RFB password. All three satisfy
    /// the special-character rule, so without an explicit check they look
    /// valid locally and are refused by the firmware instead — which is the
    /// failure this local check exists to avoid.
    #[test]
    fn rfb_password_rejects_the_characters_amt_forbids() {
        for pw in ["Ab3:defG", "Ab3,defG", "Ab3\"defG"] {
            let err = check_rfb_password(pw)
                .expect_err(&format!("{pw:?} should have been rejected"))
                .to_string();
            assert!(
                err.contains("forbids"),
                "{pw:?} rejected for the wrong reason: {err}"
            );
        }
    }
}
