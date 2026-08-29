<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-->

# Paniolo — Agent Instructions

## Never commit private infrastructure

**This repository is public. The lab that exercises it is not** — it is
described in a separate, private infrastructure repo, and nothing identifying
it belongs here. This applies to *everything* committed: code, docs, tests,
fixtures, eval scenarios, commit messages, and captured terminal output.

Do not commit:

- **Real hostnames or domains** of machines on a private network, including
  bare short names used as lab-machine labels.
- **Real addresses** on a private network — any RFC 1918 range (`10/8`,
  `172.16/12`, `192.168/16`) — with the one product exception below.
- **Home-directory paths carrying a real account name** (`/home/<user>/…`,
  `/Users/<user>/…`). Use a neutral path or `~`.
- **Identifiers read off real hardware**: MAC addresses, USB or device serial
  numbers, service-processor account names.

Use this vocabulary in examples instead:

| For | Use |
|---|---|
| A control host | `bench1`, `lab-host-1` |
| A target | `target-machine`, `pi5`, `nuc` |
| An address | `192.0.2.10` (RFC 5737 TEST-NET-1, reserved for documentation) |
| A path | `/srv/tftp/<target>`, `~/tftp/<target>` |
| A serial or MAC | `AA00BB11CC22DD33`, `00:11:22:33:44:55` |

**One exception:** `192.168.99.0/24` is paniolo's *own* default for the
point-to-point netboot link (`DEFAULT_HOST_IP` in `cli/src/model.rs`). It is a
product constant, not anyone's network, and belongs in docs and tests.

**Captured output is the easy way to slip.** Terminal casts, screenshots, and
screen recordings reproduce whatever the shell prompt, serial log, and command
line happened to contain at the time. Read a cast before committing it, and
look at a screenshot — a rendered image cannot be scrubbed by a later text
edit, only re-made.

Some material committed before this rule still violates it. That is known, is
not urgent (it is private-range addresses and internal names, no credentials),
and is being cleaned up opportunistically. Do not treat it as precedent, and do
not launch an unrequested scrub.

## Before opening a PR

**Precondition (hard gate): the PR may not be opened until all documentation
*and* usage help reflect the change.** A behavioral or surface change with stale
docs/help is an incomplete PR — bring them current in the *same* PR, never a
follow-up. Run through this checklist before calling `gh pr create`:

1. **Update docs that the PR affects.** For each changed subsystem, check:
   - `docs/<subsystem>.md` — commands, config fields, workflows
   - `docs/architecture.md` — whole-system design, data flows, runtime paths (if structure changed)
   - `docs/README.md` — the docs index (if a doc was added/removed)
   - `docs/requirements.md` — the requirements tracker status (if scope/progress changed)
   - `README.md` — capabilities table, installation steps
   - `AGENTS.md` — module layout, command descriptions, architecture notes
   Include doc updates in the same PR, not a follow-up.

   Also check the diff for private infrastructure — real hostnames, private
   addresses, `/home/<user>` paths, hardware serials — per
   [Never commit private infrastructure](#never-commit-private-infrastructure).
   Captured casts and screenshots need reading, not just grepping.

2. **Update the CLI usage help.** Every new/changed command, subcommand, flag,
   or argument must have an accurate clap doc comment (the `///` lines that
   become `--help` text). Verify the rendered output for each touched command
   (`paniolo <cmd> --help`, `paniolo <cmd> <sub> --help`) — including parent
   summaries (e.g. a group's one-line description) so they still match the
   subcommands beneath them.

3. **Update the usage skill (`skills/paniolo/SKILL.md`).** This is the
   agent-facing skill for *driving* a target. If the PR adds, removes,
   or changes a user-facing command, flag, or workflow, update the relevant
   section (and the "gotchas" list) so an agent using paniolo sees the new
   surface. The repo copy at `skills/paniolo/SKILL.md` is the canonical source;
   edit it here (however you install or link it into your agent's skills
   directory). Purely internal changes that don't alter the CLI surface can skip
   this. A companion skill, `skills/kvm-puppeting/SKILL.md`, teaches the
   GUI-puppeting *doctrine* (the look-act-settle-verify loop, keyboard-first
   navigation, pixel→logical mouse scaling) on top of the `video`+`hid`
   commands; update it too if you change the surface it relies on. These
   skills ship with paniolo and are reachable via `paniolo skill` (see the
   Rust control-plane notes). **Adding or removing a skill** also means a new
   `contents` entry in `packaging/nfpm.yaml` (one explicit file→dst line per
   skill) and a copy line is unnecessary for `setup.rs`/the tarball (both
   enumerate `skills/` automatically).

4. **Validate before pushing.** For every crate you touched, run all three
   gates from that crate's directory. CI runs them per crate and fails the
   job on any one of them:

   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

   `cargo fmt --check` is the one that is easy to skip and cheap to fail on.
   Passing tests and clean clippy say nothing about it, and a formatting diff
   fails the crate's job exactly as hard as a broken test. Run `cargo fmt`
   (no `--check`) to fix it, and note the check covers the whole crate, not
   just your diff.

   To catch the Linux-only failures without a round-trip, run
   `scripts/ci-local.sh` — it mirrors every GitHub Linux CI job (`cli`,
   `serialcap`, `netbootd`, `hdmicap`, `cambrionix`, `ch9329`, `hidrig`,
   `shellyplug`) in a Linux environment, e.g. a Lima VM:
   `limactl shell <instance> -- bash -l scripts/ci-local.sh`. It needs a Linux
   box or VM — it apt-installs and copies the tree — so treat it as the fuller
   check rather than the quick one; the three commands above are the minimum
   for a pure-Rust change and run fine on the host. (The macOS-only job —
   hdmicap AVFoundation + visionocr — runs on the host.) Note `cli` is the
   primary control-plane crate; don't let its tests rot.

5. **Every crate has a CI job — no exceptions.** A crate is not finished until
   `.github/workflows/ci.yml` has a job for it (`working-directory: <crate>`,
   then fmt + clippy `-D warnings` + test, copied from an existing crate job)
   *and* `scripts/ci-local.sh` has a matching `crate_job` line. This is
   enforced mechanically, not by memory: the `coverage` job runs
   `scripts/ci-coverage-check.sh`, which fails the build when a crate has
   neither. If a crate genuinely cannot run in CI, add it to `EXEMPT` in that
   script **with the reason written down** — never weaken or delete the check.

   The rule exists because five crates (`cambrionix`, `ch9329`, `hidrig`,
   `shellyplug`) went months with no CI at all: jobs were added
   per-crate as each one happened to matter, and nothing noticed the ones that
   were skipped.

6. **Code scanning is advanced setup, not default setup.** CodeQL runs from
   `.github/workflows/codeql.yml`, which this repo owns; GitHub's "default
   setup" must stay disabled, because the two configurations cannot both be
   active. The workflow exists so the language list is explicit — it
   deliberately omits Swift, since `ocr/visionocr.swift` is a macOS-only
   Vision wrapper with no Swift package for the extractor to build — and so
   there is somewhere to put extractor options, which default setup does not
   expose. Don't move it back to default setup to silence a prompt.

   The workflow pins a **Rust 1.94 sysroot** for the extractor via
   `CODEQL_EXTRACTOR_RUST_OPTION_SYSROOT` and `..._SYSROOT_SRC`. Don't remove
   that step: CodeQL 2.26.x cannot parse std newer than 1.94 and the runners
   ship 1.97+, so without it macro expansion fails for `format!`, `vec!`,
   `assert_eq!` and friends, and ~68 of 69 files come back "extracted with
   errors" (upstream github/codeql#19982). Two dead ends already ruled out:
   installing `rust-src` for the ambient toolchain does nothing (1577 macro
   failures with and without it, identical), and setting only `_SYSROOT_SRC`
   leaves the ambient binary sysroot in place. Revisit the pin when #19982
   closes.

7. **Push, open the PR, and merge only when asked.** Get the branch ready by
   committing locally, but treat `git push`, `gh pr create`, and merging as
   gated on the maintainer's explicit instruction — don't push or open a PR on
   your own initiative, and never merge. When told to, open with `gh pr create`;
   the merge decision always belongs to the maintainer.

## Cutting a release

Releases are **tag-driven**: pushing an annotated `v*` tag to `origin` triggers
`.github/workflows/release.yml`, which builds the Linux packages, publishes a
GitHub Release, and bumps the Homebrew tap. There is no version to edit in
source — the **tag is the single source of truth**. Every crate's
`Cargo.toml` stays at `version = "0.1.0"`; the workflow derives the package
version from the tag name (`${GITHUB_REF_NAME#v}`), so don't bump the manifests.

Steps:

1. **Pre-flight.** Be on `main`, clean, and in sync (`git fetch origin && git
   status` → `main...origin/main`, nothing ahead/behind). Confirm CI is green on
   the HEAD commit you're about to tag (`gh run list --branch main --limit 5`) —
   the release builds that exact commit, so a red main means a broken release.

2. **Pick the version.** Patch-bump from the latest tag
   (`git tag --sort=-creatordate | head`); the project has stayed on `0.1.z`.

3. **Create an annotated tag** whose subject mirrors the prior ones —
   `vX.Y.Z: <lowercase one-line summary of the headline change>` — with an
   optional body paragraph for detail. Tag as the releaser, matching the
   email convention already in the history so GitHub doesn't reject the push
   (`git log --format='%ae' | sort -u`; the project uses each author's
   `<ID>+<user>@users.noreply.github.com` form):

   ```bash
   git -c user.email='<your-github-noreply-email>' -c user.name='<Your Name>' \
       tag -a vX.Y.Z -m 'vX.Y.Z: <summary>' -m '<body paragraph>'
   ```

4. **Push the tag** (this is the outward-facing, hard-to-reverse step — gated on
   the maintainer's explicit go-ahead, like push/merge):

   ```bash
   git push origin vX.Y.Z
   ```

   The push fans out to three jobs: `package` builds amd64 + arm64 `.deb`s and
   tarballs inside a `debian:bookworm` container (glibc 2.36, so the packages run
   on Debian 12+ and current Raspberry Pi OS) and smoke-tests the `.deb`;
   `release` attaches them to a new GitHub Release with auto-generated notes; and
   `bump-tap` fires a `repository_dispatch` at `curtisgalloway/homebrew-tap` so it
   re-pins its formula to the new tag (needs the `HOMEBREW_TAP_DISPATCH_TOKEN`
   secret — absent, it warns and skips rather than failing).

5. **Watch it land.** `gh run list --workflow=release.yml --limit 1`, then
   `gh release view vX.Y.Z` once green to confirm the artifacts and notes.

A botched tag that hasn't shipped can be moved (`git tag -f` + `git push -f
origin vX.Y.Z`), but once the Release and tap bump are public, roll forward with
a new patch tag instead.

## Purpose

Paniolo is a CLI tool that lets an AI agent fully control a target machine
during low-level software development (bootloader, firmware, OS bring-up).
"Paniolo" is the Hawaiian word for cowboy — the agent wrangles the target.

Current capabilities:
- DHCP + TFTP + HTTP netboot over a direct USB-Ethernet link (`paniolo netboot`) —
  Raspberry Pi (TFTP) plus UEFI **PXE** and **HTTP Boot** (IPv4) for EDK2 boards,
  selected per-request by DHCP vendor class (option 60)
- HDMI/USB capture via hdmicap warm-stream daemon (`paniolo video`)
- Serial console — interactive (tio) or daemon-backed for the web dashboard (`paniolo serial`);
  one daemon **per target** owns that target's several named interfaces, each with a
  timestamped rolling capture log queryable by line range (`paniolo serial log -i <name>`)
- Combined video+serial web dashboard (hdmicap's `GET /`: video on top, xterm.js terminal below)
- On-device OCR of the captured screen (`paniolo video read [target] [--stable]`, which wraps hdmicap's `GET /ocr`; also the dashboard OCR button): Apple Vision on macOS, Tesseract on Linux
- USB HID input (keyboard/mouse injection) via a generic helper hook (`paniolo hid send`); the `hidrig` helper drives the dual-board KB2040 injector — it composes HID reports in Rust and writes binary frames to the control board's USB-CDC endpoint, which relays them over I2C1 to the target board (the "dumb pipe", docs/hid-dual-board-design.md; command vocabulary in docs/hid-serial-protocol.md). `hidrig serve` runs a daemon that owns the control link and re-exposes the command vocabulary over a WebSocket, so `paniolo console` works as a **KVM** — stream the browser's keyboard + absolute mouse (`moveabs`) to the target, intermixed with CLI injection on the one wire. The same control board can also **bridge the DUT serial console** (its hardware UART, re-exported by the daemon as a PTY into the `serial` channel) and **switch DUT power** via a relay (`hidrig power off|on|cycle`), so one USB device backs the target's HID, console, and power (design §6–§7; the relay/power path is hardware-verified, incl. NVM state persistence across a control-board reset — the console bridge is not yet)
- Power control via DTR (J2 wiring; **opt-in per serial interface** via `power_button = true` — `serial dtr`/`reset` refuse interfaces that haven't declared it) or generic shell-command hooks (`on_cmd`, `off_cmd`, `cycle_cmd`, `state_cmd`): `paniolo serial dtr`, `paniolo power on/off`, `paniolo power-cycle`, `paniolo power-state`. Note: "reboot over the serial console" means `serial send <t> "reboot"` (software), *not* the DTR `serial reset` (hardware). Helpers that wire into the hooks: `cambrionix` (Cambrionix hub port power via control UART), `zigplug` (Zigbee smart plugs via a CC2652 coordinator dongle), `shellyplug` (Shelly Gen2+ smart plugs/relays over the device's local HTTP RPC API — no cloud/HA/Matter), and `amt` (Intel AMT/vPro machines over WS-Management on port 16992 with HTTP Digest auth — per-target power with no plug hardware, plus true power-state readback from the ME; password only via `AMT_PASSWORD` env). The dual-board `hidrig` control board can also drive a DUT power relay (`hidrig power off|on|cycle`) as a power-helper backend, consolidating HID + console + power on one USB device

## Architecture

**Option A (current):** one daemon per subsystem, controlled via SSH. No
long-running parent process; state lives in JSON + PID files under
`~/.local/share/paniolo/<target>/`. The `paniolo` binary is the only process
that needs to persist in PATH; each subsystem daemon is a backgrounded
subprocess.

**Option B (future):** single long-running server with socket-based RPC,
enabling inter-subsystem coordination (e.g., "stream serial output whenever
a netboot attempt fires"). Will be implemented in Rust when the complexity
of option A is no longer sufficient.

## Rust control plane (`cli/` — the current implementation)

The CLI + orchestration + device glue is rewritten in Rust (the `cli/` crate),
finishing the Python→Rust migration the daemons started. Design + status:
[`docs/config-redesign.md`](docs/config-redesign.md). Key differences from the
Python tree below:

- **Config is one CLI-managed lab file** (`~/.config/paniolo/lab.toml`, or
  `--lab`/`PANIOLO_LAB`): hosts + targets, each target's hardware as *channels*
  (`netboot`, `serial[]`, `power`, `video`, `hid`, `adb`) with per-channel host
  binding.
  Edited surgically via `toml_edit` (hand-comments survive); validated on load
  and before every save. The legacy `~/.config/paniolo/targets/*.toml` files are
  not used by the Rust CLI.
- **Dispatch is per-channel**: a command resolves the host of the channel it
  touches and re-execs there over SSH against a shipped one-target slice.
  Composites (`console`) require co-located channels.
- **Daemons bind OS-assigned ports** (port 0) and are found via their
  `daemon.json` discovery files — fixed defaults collided with stale tunnels.
- **Netboot is rust-engine only** (netbootd); the pure-Python DHCP/TFTP engine
  exists only in the legacy tree.
- **Helpers live off PATH** in the private libexec dir
  (`~/.local/libexec/paniolo/bin`): only `paniolo` itself installs to
  `~/.cargo/bin`. `daemons::find_binary` resolves libexec → PATH → legacy
  `~/.cargo/bin`; hook commands (`*_cmd`, hid `cmd`) run via `sh -c` with
  libexec prepended to PATH, so lab files keep referencing helpers by bare
  name. `paniolo helper [NAME] [ARGS…]` lists or runs them directly.
- **Bundled skills are self-describing**: the agent skills under `skills/`
  (`paniolo`, `kvm-puppeting`) install to
  `~/.local/share/paniolo/skills` (and `/usr/share/paniolo/skills` for the
  Linux packages). `paniolo skill [NAME]` lists them with their frontmatter
  descriptions, or prints one `SKILL.md` (`--path` for the file path) — the
  share/ analogue of `paniolo helper`, so an agent can discover and read them
  without the harness pre-loading them (skills.rs).
- **CLI argument convention**: every runtime command takes the target as an
  optional positional (`netboot start pi5`, `serial log pi5`, `video stop
  pi5`); channel-config commands (`set`/`add`/`rm`) take `-t/--target`.
  `serial send` and `serial log` accept `-t` as well (`serial send` reads two
  positionals as `<target> <text>`, one as just the text); `hid send`, `adb
  run`, and `adb input` take `-t` only, because their positional tail is the
  helper's / `adb`'s args.
- **`paniolo daemons`** is the unified daemon inventory: every discovery-file
  daemon under `/tmp/paniolo-<uid>/` (the per-target capture daemons listed as
  `serialcap[<target>]`, `hdmicap[<target>]`, `hid[<target>]`; plus host-singleton
  zigplug), netbootd via its state files, plus *stray* helper processes running out of
  the libexec dir (wedged one-shots). `paniolo daemons stop [NAME…|--all]
  [--force]` TERMs them (netbootd via its proper interface-restoring stop),
  escalating to KILL with `--force` after a 3 s grace period.
  A daemon keeps running its binary from when it started; an upgrade or rebuild
  replaces that binary on disk but not the running process. The CLI stamps each
  capture daemon's binary identity at spawn (`binmeta.json`) and flags a daemon
  whose binary has since changed as **stale** in `paniolo daemons` (and on
  `serial show` / `video show`). `paniolo daemons restart [NAME…|--all|--stale]`
  cleanly cycles serialcap/hdmicap from the current binary (reusing the lab's
  channel config; it waits for the old process to exit so the new one doesn't
  race it for an exclusive device). netbootd is not auto-restarted — cycle it
  via `paniolo netboot start/stop`, since that touches an in-flight boot.

```
cli/src/
  main.rs       clap CLI — all command groups + runtime handler bodies
  model.rs      typed lab (serde), validate(), resolved per-channel view, channel_host
  labfile.rs    toml_edit comment-preserving lab editor (the write side)
  dispatch.rs   per-channel re-exec: slice building/shipping, maybe_dispatch,
                run_subcommand, remote_daemon_port
  ssh.rs        SSH transport: ControlMaster run/passthrough/interactive, forward (tunnels)
  daemons.rs    shared daemon contract: find_binary (libexec → PATH →
                legacy ~/.cargo/bin), hook_path, daemon.json discovery, wait
  serial.rs     serialcap orchestration + tio exec + /input + device listing
  video.rs      hdmicap orchestration (daemon start/stop, client passthrough)
  adb.rs        adb transport (argv build, shell exec, run/input passthrough,
                exec-out screencap → PNG) — a generic transport in core, no helper
  netboot.rs    netbootd lifecycle (spawn with log, stop, status)
  netif.rs      interface discovery/config (sudo), netboot/link/ffx/off modes
  power.rs      generic power hooks (on/off/cycle/state_cmd via sh -c), DTR via
                serialcap /button (+ direct-serial fallback), power_on sense
  state.rs      netboot state files (JSON-compatible with the Python's)
  doctor.rs     config-vs-reality probing (local + over SSH)
  discover.rs   hardware inventory + the configure proposal block
  setup.rs      installer: paniolo CLI onto PATH (~/.cargo/bin); helpers into
                the private libexec dir (~/.local/libexec/paniolo/bin) via
                cargo install --root; bpf-helper setuid, OCR helpers, zigplug
                (uv tool, shim in libexec), Linux groups; --rust-only fast path;
                installs the bundled skills into ~/.local/share/paniolo/skills
  skills.rs     `paniolo skill`: discover + read the bundled agent skills
                (skills_dirs: repo checkout → ~/.local/share → CLI-relative
                share → /usr/share/paniolo/skills, like daemons.rs helper_dirs
                but under share/), list with frontmatter descriptions, print
                one SKILL.md (or --path), install_bundled() for setup.rs
```

The Openterface CH9329 HID backend (once deferred in
docs/config-redesign.md) is **implemented and hardware-verified**: the `ch9329/`
crate is a helper that speaks the HID serial protocol surface into the existing
`hid` channel, with no device-specific code in `cli/`. Clean-room protocol
reference: docs/ch9329-spec.md.

**Helper state/runtime-dir API** (daemons.rs `helper_env`): paniolo exports
`PANIOLO_STATE_DIR` (`~/.config/paniolo/helpers/<name>/`, durable) and
`PANIOLO_RUNTIME_DIR` (`/tmp/paniolo-<uid>/<name>/`, discovery/locks/logs) —
directories pre-created — on every helper invocation: hook commands (named by
the hook's program basename, see `hook_helper_name`), `paniolo helper`
passthrough, and daemon spawns. The per-target capture daemons
(serialcap/hdmicap/hid) append a `/<target>` segment to their runtime dir
(`/tmp/paniolo-<uid>/<name>/<target>/`) so multiple targets capture concurrently
on one host; host-singleton helpers (zigplug/cambrionix/netbootd) use the base
`<name>/` form above. The base honors `$PANIOLO_RUNTIME_BASE` (default `/tmp`).
Channel daemons use the channel name (hidrig
publishes under `hid`). Helpers prefer the env vars, falling back to the same
literal paths standalone; hdmicap/serialcap/hidrig/zigplug all do, and
zigplug lazily migrates its `zigbee.db` from the legacy top-level
`~/.config/paniolo/` location into its namespaced dir. Contract for new
helpers: docs/adding-power-helpers.md.

## Module layout

```
hdmicap/         Rust crate: warm-stream HDMI capture daemon
  build.rs       compiles src/capture_avf.m via cc on macOS, links AVFoundation
  src/
    main.rs      CLI subcommands: daemon, devices, shot, watch, preview, stop
    capture.rs   capture backends: v4l (Linux, raw MJPEG tee + turbojpeg);
                 macOS module wraps the C ABI of the ObjC layer below
    capture_avf.m  our ObjC AVFoundation layer (macOS): enumeration, open at
                 native resolution with NV12 delivery, blocking frame wait.
                 Never sets frame durations — MS2109-class HDMI sticks throw
                 NSException on those setters
    capture_thread.rs  std::thread owning device, publishes into watch channel
    frame.rs     FrameState, Signal enum, one-pass strided classification
                 (aHash + no-signal from 4k luma samples, resolution-independent)
    pixel.rs     PixelData (Rgb/Nv12/Empty) + NV12/YUYV -> RGB converters
    server.rs    axum HTTP API: GET / (dashboard), /status, /snapshot, /preview,
                 /ocr, /devices, POST /power-cycle, and /xterm.* static assets
    daemon.rs    advisory lock, discovery file, tokio runtime, graceful shutdown
  assets/        index.html (combined dashboard) + vendored xterm.js/css/fit addon

cambrionix/      Rust crate: standalone helper binary for Cambrionix USB hub control
                 (control UART, 115200 8N1); wired into paniolo via generic power hooks.
                 Commands: `state [port]`, `on <port>`, `off <port>`, `cycle <port>`
                 `state <port>` prints exactly `on` or `off` (matches paniolo state_cmd
                 contract). Built/installed by `make install` / `paniolo setup`.

shellyplug/      Rust crate: standalone helper for Shelly Gen2+ smart plugs/
                 relays (Plus/Pro/Gen3/Gen4) over the device's local HTTP RPC
                 API (Switch.Set/GetStatus; ureq). One-shot, stateless — no
                 daemon. Addressed by `-d <ip|host>` and `[id]` switch (default
                 0). Commands: `status [id]`, `state [id]`, `on/off [id]`,
                 `cycle [id]`; `on/off/cycle` confirm by read-back. Gen2+ only
                 (no Gen1 REST); auth-disabled devices only for now. NB: first
                 helper to reach a LAN device, so first to hit the macOS
                 Local Network privacy gate — see docs/power.md gotchas.

amt/             Rust crate: standalone helper for Intel AMT (vPro) machine
                 power over WS-Management (SOAP over HTTP, port 16992; ureq).
                 One-shot, stateless — the switch is the machine's own
                 Management Engine, so `state` is a true sensor (ME answers
                 with the host on, off, or bare-metal). HTTP Digest (MD5,
                 RFC 2617) implemented in-crate — AMT 11+ is Digest-only.
                 Addressed by `-d <ip|host>` and `-u <user>` (default admin);
                 password ONLY via the AMT_PASSWORD env var (never in the lab
                 file). Commands: `status`, `state` (prints exactly `on`/
                 `off`; PowerState 2 = on, sleep/hibernate/off = off), `on`,
                 `off` (hard power-off), `cycle [--delay-ms 3000]` (off →
                 delay → on, a genuine cold boot). Requests and read-back
                 retry transient transport errors ~20 s — the AMT NIC drops
                 link around host power transitions. TLS AMT (16993) is
                 unsupported (clear error). Hardware-verified against a Dell
                 OptiPlex 7060 (AMT 12). See docs/power.md.

zigplug/         Python (uv) helper: Zigbee smart plug control via a CC2652 (ZNP)
                 coordinator dongle, using zigpy-znp. CLI wired into paniolo
                 via generic power hooks, like cambrionix — but operations
                 proxy through a persistent daemon (`_daemon.py`, aiohttp on
                 localhost, standard daemon.json discovery) that owns the
                 coordinator session: one-shots reset the CC2652 on every
                 serial open (auto-BSL lines) and collide on the stateful ZNP
                 session, so the daemon serializes ops with hard timeouts.
                 Auto-spawned on first use; hook strings stay one-shot-shaped.
                 Commands: `form` (one-time network setup), `permit` (pairing
                 window), `list`, `on/off/state/cycle <ieee>`, `remove <ieee>`,
                 `serve/stop/status` (daemon), `backup`/`restore` (coordinator
                 NVRAM recovery from zigpy's auto-backups — no re-pairing);
                 `state <ieee>` prints exactly `on` or `off` (state_cmd
                 contract). Device DB at
                 ~/.config/paniolo/helpers/zigplug/zigbee.db (auto-migrated
                 from the legacy top-level location).
                 Installed by `paniolo setup` via `uv tool install` when uv is
                 present (shim in the libexec dir via UV_TOOL_BIN_DIR, off
                 PATH). See docs/power.md for pairing, hook wiring, recovery.

serialcap/       Rust crate: serial console daemon (parallels hdmicap)
  src/
    main.rs      CLI subcommands: daemon (--interface NAME=DEV[@BAUD], repeatable),
                 log (-i NAME), devices, stop
    serial_io.rs one supervisor per interface: tokio-serial port owner; reconnect
                 loop; broadcast fan-out to WS clients; mpsc client->port; 64KB
                 scrollback ring; tees every chunk to that interface's capture
                 thread (off the live fan-out path). `Serials` holds the named set
    capture.rs   line assembler: splits bytes into timestamped, sequence-numbered
                 lines; appends them to a rotating on-disk JSONL log under
                 capture/<name>/ (survives restarts; resumes the seq counter);
                 mirrors the current unterminated line to a pending sidecar. Also
                 the `log` reader (interface select; tail / range / since,
                 ANSI-stripped by default) + UTC formatting
    server.rs    axum: GET /stream (bidirectional WebSocket), /status, /interfaces,
                 /devices; POST /button (DTR pulse), /input (write bytes to port,
                 ?pace_ms=N drips one byte per N ms for a slow polled console).
                 Per-interface endpoints take ?interface=NAME, defaulting to the
                 first configured interface
    daemon.rs    advisory lock, discovery file, tokio runtime, graceful shutdown;
                 spawns one supervisor per interface

ocr/             OCR helpers (compiled/installed binaries are gitignored):
                   visionocr.swift  Apple Vision OCR (macOS); built by paniolo setup via swiftc
                   linuxocr         Tesseract OCR wrapper (Linux); copied by paniolo setup

hidrig/          USB HID injector: host CLI + daemon (Rust) + dual-board KB2040 firmware
  src/main.rs      `hidrig` CLI — one-shot subcommands of the HID command
                   vocabulary (type/key/.../moveabs/ping/version) + `run` command
                   files; `serve`/`stop` for the daemon. A `Sender` routes each
                   one-shot through a running daemon (POST /send) when one owns
                   the same device, else opens the control CDC link and composes
                   frames in-process
  src/compose.rs   HID composition: turns each command into HID report bytes and
                   wraps them in the binary frames the boards relay (F_HID 0x01 /
                   F_CTRL 0x02). Holds the held-key + virtual-cursor state so
                   relative `move` and `moveabs` share one absolute-pointer device
  src/proto.rs     control-link transport for the *direct one-shot* path: writes
                   binary frames to the control board's data CDC endpoint (no baud
                   negotiation — CDC; nominal 115200), reads `0x02` control-frame
                   replies (ping/version/power); command-file sequence parser +
                   clamp_abs
  src/uart.rs      the control-link owner (daemon path): a dedicated *blocking-
                   serialport* thread (NOT tokio-serial — its async reads don't
                   get read-readiness on a macOS tty) running a full-duplex poll
                   loop. It drains an mpsc command queue (CLI + web, serialized
                   onto one wire), pumps PTY console input down as 0x03, then
                   reads + demuxes inbound frames: 0x02 replies fulfil the in-
                   flight control request (deadline-tracked), 0x03 payloads go to
                   the console PTY master. HID is fire-and-forget; broadcast
                   transcript; lazy open + reopen-on-transport-error
  src/pty.rs       allocates a PTY (libc posix_openpt) for the DUT serial-console
                   bridge: the owner holds the master; paniolo's serial channel
                   opens the slave via the stable symlink the daemon publishes
  src/server.rs    axum: GET /hid (WebSocket carrier), POST /send, /status,
                   /version. WS clients send command lines; all results are
                   broadcast as `evt ok|err …` frames so observers see the
                   intermixed stream
  src/daemon.rs    advisory lock, discovery file at /tmp/paniolo-<uid>/hid/<target>/
                   (the channel name, not "hidrig", so paniolo finds it without
                   knowing the helper); brings up the console PTY + publishes its
                   stable symlink (recorded as discovery `console`); tokio
                   runtime, graceful shutdown (also removes the symlink)
  firmware/dual/control/  control board (CircuitPython 9.x): USB-CDC <-> I2C1
                   controller; reads framed input from usb_cdc.data, relays 0x01
                   HID frames verbatim over I2C1 to the target, answers 0x02
                   control frames (ping/version/power -> dual-control/1; power
                   drives a DUT relay on D5) locally, and bridges 0x03 console
                   frames to/from the DUT UART (TX=GP0/RX=GP1)
  firmware/dual/target/   target board (CircuitPython 9.x): I2C1 peripheral that
                   relays report bytes to usb_hid send_report — no adafruit_hid,
                   no parsing. boot.py holds the HID descriptor (keyboard + custom
                   absolute-pointer, 0..32767 axes) and the dev/HID-only NVM flag
                   (BOOT button GP11 toggles; D2->GND at reset forces dev)
  firmware/{boot,code,config}.py  retired single-board "smart" firmware (line
                   protocol + adafruit_hid); kept for the future dumb single-board
  host/hid_seize_reports.c  macOS IOKit tool: seizes the HID device exclusively
                   and prints raw input reports — for pipeline testing without
                   keystrokes reaching the focused app. Build with host/Makefile.
  README.md        topology, wiring, frame protocol, CLI usage. The command
                   vocabulary spec is docs/hid-serial-protocol.md; the dual-board
                   design + frame format is docs/hid-dual-board-design.md

ch9329/          Rust crate: the *other* hid helper — a WCH CH9329 UART->USB-HID
                 bridge client, hardware-verified against an Openterface
                 Mini-KVM (the Sipeed NanoKVM-USB speaks the same frame
                 protocol at 57600 but is not bench-verified here). Same
                 CLI surface as hidrig, so it drops into a `hid` channel
                 identically (`paniolo hid set --cmd "ch9329 -d <uart>"`); the
                 chip *is* the HID device, so it speaks the binary frame
                 protocol (HEAD 57 AB / ADDR / CMD / LEN / DATA / SUM) rather
                 than relaying the line protocol. Clean-room spec:
                 docs/ch9329-spec.md
  src/proto.rs     the HID serial protocol grammar executed against a Session
                   instead of forwarded to a microcontroller; `execute_line` is
                   the one backend for CLI subcommands and `run` files, so the
                   accepted command set matches hidrig exactly (sequence parser
                   + moveabs clamp ported from hidrig/src/proto.rs)
  src/session.rs   the link itself: framing/checksum, GET_INFO, held-key/pointer
                   state, and `open()`'s baud probe (BAUD_CANDIDATES = 115200
                   then 9600; force with -b — e.g. -b 57600, the NanoKVM-USB
                   default). Holds two verified CH9329-on-Linux workarounds —
                   clicks go through the *relative* report (libinput coalesces a
                   button transition in an absolute report at an unchanged
                   coordinate), and `moveabs` nudges one unit before the exact
                   target (an absolute report equal to the previous one is
                   coalesced away)
  src/uart.rs      the UART owner (daemon path): one dedicated thread holding a
                   long-lived Session, serializing CLI- and WebSocket-injected
                   commands onto the one wire, one in flight — which is also
                   what makes held state survive across separate invocations
  src/keys.rs      key-name -> USB HID usage mapping (adafruit_hid Keycode names,
                   US layout), shared with the hidrig vocabulary
  src/server.rs    axum: GET /hid (WebSocket), POST /send, /status, /version
  src/daemon.rs    `serve`/`stop`: owns the UART, publishes the same
                   /tmp/paniolo-<uid>/hid/ discovery file paniolo's console reads
  README.md        wiring, extras beyond hidrig's surface (`info` reports target
                   USB enumeration + lock LEDs; `baud` persists a rate to flash),
                   and the hardware-verified status notes
```

### hid daemon + KVM (`hidrig serve`)

The control link can have only one owner, so KVM streaming and CLI injection
can't both open it. `hidrig serve` resolves this: it owns the link and
re-exposes the command vocabulary over a WebSocket (`GET /hid`) and `POST
/send`. Every command — from a
browser, from `paniolo hid send`, from another script — flows through one
`mpsc` queue in `uart.rs`, one in flight, request/reply; that single queue is
what makes events intermix correctly. `paniolo console` starts the daemon when
the target has a `hid` channel (local: `?hid=PORT`; remote: an SSH-tunnelled
`?hidws=` URL). The **`⌨ Capture input`** overlay button toggles capture (no click-to-grab, no
host-key release): engaged, the page streams `down`/`up`/`moveabs`/`scroll` to
the daemon; click the button again to release. The mouse is absolute (the
firmware's custom HID descriptor), so the cursor follows where you point in the
video, and the local cursor stays **visible as a crosshair** (no Pointer Lock —
deliberately, so you never lose your pointer). Mouse listeners live on the
`<img>`, so the overlay buttons never inject; window blur releases. paniolo
discovers the daemon by the
channel name `hid` (`daemons::daemon_port("hid")`), staying agnostic to the
helper. Hardware-verified end-to-end on the pi5 Linux desktop (2026-06-04).

**Latency.** HID frames are **fire-and-forget** over the USB-CDC link (no
per-frame round-trip), so streaming stays responsive without a baud
negotiation — the control board is USB-CDC and USB sets the rate. The dashboard
also **coalesces mouse moves** to one `moveabs` per `requestAnimationFrame`
(newest position only). The remaining floor is the target board's USB interrupt
`bInterval` (~8 ms per report on the CircuitPython firmware). Only `0x02`
control frames (ping/version) draw a reply; macOS drops the `IOSSDATALAT` read
timer on open to keep those round trips prompt.

## Combined dashboard (video + serial)

hdmicap's `GET /` serves a two-pane page: the MJPEG video on top, an xterm.js
terminal below. The terminal opens a WebSocket to **serialcap** (a separate
daemon/port), so the two subsystems stay decoupled — hdmicap only references
serialcap by URL. Defaults to `ws://<host>:8724/stream`; override with
`?serial=<port>` or `?serialws=<url>`. Local `paniolo console` passes the
serialcap daemon's OS-assigned port as `?serial=PORT`; the remote/tunnel path
passes `?serialws=` (unchanged). serialcap sends serial bytes as binary
frames and accepts keystrokes back over the same socket. xterm.js is vendored
(not CDN) so the dashboard works on an isolated lab network. This is the first
concrete instance of the "Option B" inter-subsystem coordination described above.

**Multi-pane serial:** the page fetches `GET /interfaces` from serialcap on
load and calls `buildPanes(names)`. With one interface a single terminal fills
the panel and connection status appears in the top bar. With multiple interfaces
each gets its own `.serial-pane` div (label + status bar + xterm.js terminal),
laid out side by side in bottom mode or stacked in right-panel mode. All fits
are tracked in `allFits[]` so resize and layout-toggle events re-fit every
terminal. `?interface=<name>` bypasses the fetch and opens single-pane mode
pinned to that interface.

**Layout toggle:** a button in the status bar switches the serial panel between
bottom (default, 40 vh) and right-panel (380 px fixed, video fills remaining
width) layouts. The choice is persisted in `localStorage` under the key
`paniolo-serial-layout`.

**Power controls:** an on/off **toggle switch** (`Power [switch] ON/OFF`,
reflecting live state) plus a separate **⟳ Cycle** button appear in the video
overlay, each gated by a confirmation modal. Availability + state come from
**`GET /power`** — non-acting: it runs `paniolo power-state <target>` and returns
`on`/`off`/`unknown`, and the dashboard polls it every 5 s to keep the toggle
synced. The actions are **`POST /power-on` | `/power-off` | `/power-cycle`** →
`paniolo power on|off` / `power-cycle <target>`. All use the `PANIOLO_TARGET` env
var set when the daemon starts via `paniolo video watch <target>`; the controls
are hidden (501) if no target was passed, so shared dashboards are safe.
(Previously the availability probe was `POST /power-cycle`, which *triggered* a
cycle on every page load — the probe is now the read-only `GET /power`.)

## OCR

Two entry points, both feeding the same warm frame:
- **`paniolo video read [target] [--stable]`** — OCRs the current frame (wraps
  the daemon's `GET /ocr`).
- **Dashboard button + hdmicap `GET /ocr`** — the daemon PNG-encodes the current
  frame and pipes it to the OCR tool (`tokio::process`), returning the text. The
  daemon finds the tool via `PANIOLO_VISIONOCR` (the installed path, set by
  `paniolo video watch`), then a `visionocr`/`linuxocr` sibling of its own
  executable (both live in the libexec dir), then bare PATH; if absent, `/ocr`
  returns 501 and the button shows an error.

`paniolo setup` installs the platform-appropriate tool. `PANIOLO_VISIONOCR` is
set to the resolved path when the daemon starts, so the daemon always uses the
installed binary (never a stale PATH hit).

**macOS — `ocr/visionocr.swift`** (`VNRecognizeTextRequest`, Apple Vision):
on-device, no network, no model download. `paniolo setup` compiles it (`swiftc`)
into the libexec dir (`~/.local/libexec/paniolo/bin`).

Tuning that matters for small console text:
- `recognitionLevel = .fast` is the default, not `.accurate`. `.accurate` is
  tuned for natural document text and returns *nothing* on thin console fonts.
- The tool 2×-upscales and black-pads the frame before recognition (fixes colon
  misreads and first-character clipping at the frame edge).
- `minimumTextHeight` is lowered (it's a fraction of image height; the default
  1/32 skips ~16px console text).

**Linux — `ocr/linuxocr`** (Tesseract via `tesseract-ocr` system package):
`paniolo setup` copies the script into the libexec dir. Requires
`sudo apt-get install tesseract-ocr`; Pillow (`pip install Pillow`) is optional
but enables the same 2×-upscale + black-pad preprocessing as visionocr.

**Do not change the target's console font** to try to improve OCR accuracy —
the font is relied upon by other agents (e.g. the Fuchsia bring-up agent that
reads kernel/bootloader output). Character confusions on thin console fonts
(`1`↔`l`↔`I`, IPv6 colons, etc.) are better addressed by increasing capture
resolution or adjusting Tesseract's `--psm` mode.

## netbootd (Rust netboot engine)

`netbootd/` is a single-binary DHCP + read-only TFTP + HTTP server — all as
tokio tasks in one process. It is the **only** netboot engine for `paniolo
netboot start` (originally ported from a pure-Python `_dhcp`/`_tftp` pair, since
removed).

The pure protocol logic is unit-tested (`dhcp.rs` / `tftp.rs` `#[cfg(test)]`
modules): packet parse/build, RRQ option negotiation, path-traversal rejection,
and full loopback DATA/ACK transfers (multi-block, OACK, retransmit-on-loss,
error packets). A 65 K-round-trip block-wraparound test is marked `#[ignore]` —
run it with `cargo test -- --ignored`.

Key differences from the Python servers:

- **In-process MAC handoff.** The DHCP task publishes the client's hardware
  address to the TFTP task via `tokio::sync::watch` — no on-disk `client-mac`
  file.
- **Privilege-separated `/dev/bpf` on macOS.** The macOS raw-frame send path
  (the Sequoia workaround) needs a BPF descriptor, which only root can open.
  Rather than run the daemon as root, a tiny **setuid-root** helper —
  `netbootd-bpf-helper` — opens `/dev/bpfN`, binds it (`BIOCSETIF`), sets
  `BIOCSHDRCMPLT`, and passes the fd back over a `socketpair` via `SCM_RIGHTS`
  (`src/handoff.rs`), then exits. netbootd itself runs **unprivileged** and only
  `write(2)`s frames to the fd (`src/bpf.rs::BpfSender::from_handoff`). The
  helper is the *only* component that runs as root; `paniolo setup` installs it
  setuid (the one-time sudo). If the helper is missing/not-setuid, netbootd logs
  it and falls back to the kernel `send_to` path (broken on macOS 15+).
- **Primary-NIC guard.** `netcfg::is_primary_interface` mirrors the Python
  guard; `main()` refuses to start, and `monitor_interface` refuses to enforce,
  on the default-route interface.
- **Layout.** `src/lib.rs` exposes `frame` (frame builder, unit-tested) and
  `handoff` (BPF open + fd passing) so both the `netbootd` and
  `netbootd-bpf-helper` binaries share them. On Linux netbootd uses the kernel
  send path (no BPF), matching the Python behavior.

## hidrig (USB HID injector)

The `hidrig/` directory is the USB HID injector: a Rust host CLI/daemon plus
CircuitPython 9.x firmware for the **dual-board "dumb pipe"** KB2040 rig.

### Architecture

```
[control host]
  |-- USB-CDC (hidrig writes binary HID frames) --> [Control KB2040]
                                                      |-- I2C1 (GP10 SDA / GP19 SCL,
                                                      |   addr 0x41, 4.7k pull-ups) -->
                                                    [Target KB2040]
                                                      |-- USB HID --> [target / DUT]
```

The host composes HID reports (`src/compose.rs`) and writes binary frames to the
**control** board's data CDC endpoint; the control board relays `0x01` HID
frames verbatim over I2C1 to the **target** board, which calls `send_report` —
neither board parses HID semantics (the "dumb pipe", `docs/hid-dual-board-design.md`).
The target board's USB faces the DUT as a device-mode HID keyboard + absolute
mouse (and is DUT-powered, so it reboots with the DUT); the control board is
independently host-powered. The command vocabulary (`type`/`key`/`moveabs`/…)
is the device-independent **HID serial protocol v1** (`docs/hid-serial-protocol.md`),
but it is the *external* interface only — `hidrig` consumes it and composes; the
line protocol never reaches a wire. `hidrig` (`src/main.rs`, `src/compose.rs`,
`src/proto.rs`) is the host client; `firmware/dual/{control,target}/` are the
reference firmware. The retired single-board "smart" firmware
(`firmware/{boot,code,config}.py`, line protocol + `adafruit_hid`) is kept for a
future dumb single-board on the same composition.


### USB identity (`firmware/boot.py`)

In normal operation the target must see a plain keyboard + mouse, so boot.py
disables the CIRCUITPY drive, the CDC REPL, and MIDI. Jumpering **D2 to GND**
at reset re-enables them for firmware updates (plug into a dev machine, not
the target). boot.py only re-runs on hard reset. The status NeoPixel is driven
via the core `neopixel_write` module (no /lib dependency): blinking red =
waiting for target enumeration, green blip = serving, solid red = last
command failed.

### paniolo integration

`paniolo hid set -t <target> --cmd "hidrig -d <uart>"` stores an opaque
command prefix in the lab file's `[targets.<name>.hid]` channel (mirroring the
generic power hooks; no device-specific code in `cli/`). `paniolo hid send -t
<target> <args...>` shell-quotes and appends the args and runs the result via
`sh -c` on the channel's host (transparent SSH dispatch via
`ChannelKind::Hid`). `paniolo doctor` probes absolute-path helpers with
`test -e`.

### Host testing tool (`hidrig/host/`)

`hid_seize_reports.c` is a macOS IOKit utility that opens the injector's HID
interface with `kIOHIDOptionsTypeSeizeDevice`, preventing any keystroke from
reaching the focused application. It registers an input report callback and
prints hex dumps of every keyboard and mouse report. Use it to verify the full
pipeline end-to-end without a target:

```bash
cd hidrig/host && make
sudo ./hid_seize_reports   # grant Input Monitoring in System Settings first
```

Run `hidrig -d <adapter> type/key/move/click/scroll ...` in a second terminal
and read the reports. The tool prints the 156-byte report descriptor on first
device match, so you can verify the HID descriptor matches expectations.

VID/PID are 0x239A/0x8106 (KB2040 running CircuitPython). The built binary is
gitignored; re-run `make` after cloning.

### Negative number arguments (`move`, `scroll`)

clap treats a token starting with `-` as a potential option flag; the `dx`/
`dy`/`amount` args use `allow_hyphen_values` so `hidrig move 50 -30` and
`hidrig scroll -3` work without a `--` separator (same for `paniolo hid send`,
whose trailing args allow hyphen values — keep `-t` before them).

## Runtime paths

| Purpose | Path |
|---|---|
| Target configs | `~/.config/paniolo/targets/<name>.toml` |
| Video config | `~/.config/paniolo/video.toml` |
| Netboot daemon state | `~/.local/share/paniolo/<name>/netboot.json` |
| Combined netboot log | `~/.local/share/paniolo/<name>/netboot.log` |
| hdmicap discovery file | `/tmp/paniolo-<uid>/hdmicap/<target>/daemon.json` (`{pid, port}`) |
| hdmicap advisory lock | `/tmp/paniolo-<uid>/hdmicap/<target>/daemon.lock` |
| hdmicap stderr log | `/tmp/paniolo-<uid>/hdmicap/<target>/daemon.log` (truncated on each CLI-spawned start) |
| serialcap discovery file | `/tmp/paniolo-<uid>/serialcap/<target>/daemon.json` (`{pid, port, interfaces:[{name, device, baud}]}`) |
| serialcap advisory lock | `/tmp/paniolo-<uid>/serialcap/<target>/daemon.lock` |
| serialcap stderr log | `/tmp/paniolo-<uid>/serialcap/<target>/daemon.log` (truncated on each CLI-spawned start) |
| serialcap capture log | `/tmp/paniolo-<uid>/serialcap/<target>/capture/<name>/serial.jsonl(.1..)` (rotated JSONL, per interface) |
| serialcap pending line | `/tmp/paniolo-<uid>/serialcap/<target>/capture/<name>/pending.json` (current unterminated line) |
| hid daemon discovery file | `/tmp/paniolo-<uid>/hid/<target>/daemon.json` (channel name, any injector) |

The per-target capture daemons (hdmicap/serialcap/hid) add the `<target>`
segment so multiple targets capture concurrently on one host; host-singleton
daemons (zigplug/cambrionix/netbootd) have no `<target>` segment. The runtime
base honors `$PANIOLO_RUNTIME_BASE` (default `/tmp`).

## Source code constraints

- **No hardcoded network addresses, URLs, or hostnames.** All site-specific
  values go in config files under `~/.config/paniolo/` and are populated via
  setup commands. Error messages must be generic. The same rule extends past
  code to docs, fixtures, and captured output — see
  [Never commit private infrastructure](#never-commit-private-infrastructure).
- **No new dependencies without discussion.** Keep each crate's dependency set
  lean and justify any new crate in review. (The `zigplug` Zigbee helper is the
  one remaining Python component — its deps live in `zigplug/pyproject.toml`.)
- **Rust is formatted with `rustfmt` and linted with `clippy`.** CI runs
  `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` per crate
  (`cli`, `serialcap`, `netbootd`, `hdmicap`) — keep both clean before pushing.
  Run `make fmt` to format every crate. The `zigplug` Python helper is formatted
  with `pyink` and linted with `pylint` at line-length 88.
- **`paniolo setup` builds the native components from the source tree** when
  run from a clone — `make install` (which invokes the *installed* CLI)
  resolves the checkout by walking up from the cwd (`setup::find_repo_root`).
  Outside a checkout (a packaged install: Homebrew, .deb, tarball), it runs
  the platform-finish steps only (`setup::run_packaged`): setuid the
  installed `netbootd-bpf-helper` on macOS, group membership on Linux — no
  builds. `--rust-only` still requires a clone and errors clearly without
  one.

## Remote control pattern

```bash
ssh control-mac "paniolo target set target-machine --interface en3 --tftp-root ~/pxe"
ssh control-mac "paniolo power set -t target-machine \
  --cycle-cmd /Users/you/.config/paniolo/scripts/power-cycle-target-machine.sh"
ssh control-mac "paniolo netboot start target-machine"
TFTP_ROOT=$(ssh control-mac "paniolo netboot tftp-root target-machine")
scp kernel.img control-mac:"${TFTP_ROOT}/kernel_2712.img"
ssh control-mac "paniolo netboot logs -f target-machine"
op run --env-file .env -- ssh control-mac "paniolo power-cycle target-machine"
ssh control-mac "paniolo netboot stop target-machine"
```

## Adding a new subsystem

**Adding support for new power-switching hardware is not a subsystem** — it's
a standalone helper binary wired in via the generic power hooks. Follow
[docs/adding-power-helpers.md](docs/adding-power-helpers.md) (hook contract,
helper CLI conventions, Rust/Python skeletons, verification ladder, PR
checklist); `cambrionix/`, `zigplug/`, and `shellyplug/` (the
simplest one — a stateless HTTP one-shot) are the exemplars.

For a genuine new subsystem (a channel with its own commands/daemon), in the
Rust `cli/` crate:

1. Add a module `cli/src/<subsystem>.rs` for its logic.
2. Add a clap subcommand group + its handlers in `cli/src/main.rs`.
3. Add the channel's config fields to the data model (`cli/src/model.rs`) and
   surgical lab-file editing (`cli/src/labfile.rs`) so they round-trip.
4. If it's a daemon with a PID, add its state/discovery handling alongside the
   others (`cli/src/state.rs`, `cli/src/daemons.rs`).
5. Regenerate the skill (`paniolo skill`) and update this file and `docs/`.

A new **crate** (helper or otherwise) additionally needs a CI job and a
`scripts/ci-local.sh` line — see the CI-coverage rule in "Before opening a PR".
`scripts/ci-coverage-check.sh` fails the build until both exist.

A new **helper binary** must also be registered everywhere the helper set is
named, or it builds green in CI and then silently never reaches users:

- `Makefile` — `CRATES`
- `cli/src/setup.rs` — `HELPER_CRATES` (what `paniolo setup` installs from a
  source clone)
- `.github/workflows/release.yml` — the `HELPERS` env list **and** the
  rust-cache `workspaces` block (what the released `.deb`/tarball ship;
  v0.1.13 shipped without the `amt` helper because this list was missed)
- `packaging/nfpm.yaml` — the helper list in the package description

`scripts/ci-coverage-check.sh` (run by the `coverage` CI job) fails the build
when the `Makefile`, release `HELPERS`, or `HELPER_CRATES` lists omit a crate,
so the mechanical ones can't drift again; the release smoke test also verifies
every `HELPERS` binary landed in the installed `.deb`.

## Platform support

Paniolo runs on three host platforms:

| Platform | Status | CI | Release artifacts |
| --- | --- | --- | --- |
| macOS (Apple Silicon) | Supported | `macos` job in `ci.yml` | Homebrew tap (arm64 bottle) |
| Linux (Debian/Ubuntu) | Supported | the `ubuntu-latest` jobs in `ci.yml` | `.deb` + tarball from `release.yml` |
| Windows (x86_64-msvc) | Supported; power and video hardware-verified | `windows` job in `ci.yml` | portable zip + winget |

All ten crates build, lint clean under `clippy -D warnings`, and pass their
unit tests on all three. The `windows` CI job runs the same fmt/clippy/test
triple as the Linux jobs, because a Unix-only CI cannot tell you whether the
Windows `#[cfg]` arm still compiles.

**Hardware-verified on Windows** (bench host `brik`, 2026-08-28, against a
Shelly Plug and an Openterface KVM-GO):

- **power** — `shellyplug` on/off/cycle, each confirmed by read-back.
- **video** — `hdmicap` enumerates the capture device by its symbolic-link id
  and captures native 3840x2160 frames through the Media Foundation backend;
  daemon start, `shot` and `stop` all work.
- **hid** — `ch9329` over a COM port reports `target_connected=true` and its
  mouse moves wake the attached machine, which is how the first (black) capture
  turned into a real frame. Keystroke delivery is confirmed by that wake rather
  than by the CH9329's LED read-back, which stayed `false`.

**What is still NOT verified on Windows:**

- **netbootd falls back to `send_to`.** The `/dev/bpf` raw-frame sender and the
  `SCM_RIGHTS` fd handoff are Unix-only, exactly as on Linux. Whether DHCP/TFTP
  netboot works against a real target on Windows is untested, and `netif` has no
  Windows implementation to configure the interface with.
- **hidrig has no console bridge.** The PTY the `serial` channel points its
  `device =` at has no Windows analogue — ConPTY is not a substitute, since it
  exposes no filesystem node another process can open. HID and control are
  unaffected.
- **OCR is absent.** Neither Apple Vision nor Tesseract has a Windows
  counterpart wired up. `Windows.Media.Ocr` is the in-box candidate, and the
  `windows` crate is already a hdmicap dependency for capture.
- **`paniolo doctor` cannot probe a Windows control host.** Probes run natively
  when the host is local, but the SSH path still renders them as POSIX shell
  (`sh -c 'test -e …'`), which a PowerShell host cannot run. Everything else
  dispatches fine (see **Dispatching to a Windows control host** below); doctor
  is the one remaining verb that speaks shell over the wire rather than pure
  paniolo.

### Dispatching to a Windows control host

`paniolo <cmd> -t <target>` works when the target's host is Windows. The
mechanism is worth knowing, because the thing that used to break it is a trap
anywhere the remote shell is not ours to choose.

Dispatch does three things over SSH: ship a lab slice, re-exec `paniolo` against
it, and clean the slice up. Only the middle one was ever shell-safe.

- **Shipping and cleanup now go over SFTP** (`ssh::sftp_put` / `ssh::sftp_rm`).
  They used to be shell: `f=$(mktemp …) && cat > "$f" && printf %s "$f"` and
  `rm -f`. On a PowerShell host the first is not a command at all, and the
  second fails with *"parameter 'f' is ambiguous. Possible matches include:
  -Filter -Force."* SFTP is a protocol, so it behaves identically whatever the
  far side runs, and it reuses the session's ControlMaster so it costs no extra
  handshake.
- **The re-exec was already fine.** `remote_command` quotes each argument
  POSIX-style, and PowerShell reads `'C:\Users\curti\lab.toml'` as the same
  literal a POSIX shell would. Verified directly. The `VAR='v' cmd` env-prefix
  form is *not* portable — but the dispatch path passes no env, so it never
  appears there.
- **The slice path is deliberately relative** (`.paniolo-lab-<pid>-<ns>.toml`).
  SFTP reports a Windows home as `/C:/Users/name`, an SFTP-protocol path no
  native Windows program can open, so an absolute path from `pwd` is unusable as
  a `--lab` argument. Both platforms start an SSH session in the user's home,
  which is also SFTP's default directory, so a relative name resolves on both.

Verified end to end against `brik`: `paniolo hid send -t <target> info` returned
real CH9329 state, `hid send … move` drove the KVM, `video show` read the
channel, and the slice was cleaned up. Unix dispatch to `waldo` is unchanged.

**What still speaks shell over the wire:** `doctor`. Its probes execute natively
against a local host but render to `sh -c` for a remote one, so probing a
Windows control host fails. Narrowing that to pure paniolo verbs — the remote
side running `paniolo doctor` rather than the near side shipping shell — is the
obvious fix and is not done.

### Writing portable code here

The POSIX primitives paniolo depends on live behind `cli/src/platform.rs`, with
one implementation per platform and a single documented contract each:
`current_uid`, `runtime_root`, `ensure_private_dir`, `make_executable`,
`pid_alive`, `signal_pid` / `try_signal_pid` / `terminate_pid`, `is_superuser`,
`detach`, `exec_replace`. **Reach for those rather than `libc` or
`std::os::unix` at a call site** — that is the rule the port established, and
the reason it is one small module instead of `#[cfg]` scattered through twenty
files.

A trimmed copy of the module is duplicated into hdmicap, serialcap, ch9329 and
hidrig. That is deliberate and matches how `daemon.rs` is already duplicated:
these are standalone cargo projects with no crate between them. **Keep the five
copies in sync.**

Two semantic differences are worth knowing before you rely on them:

- **Windows has no signals.** Both `Signal::Term` and `Signal::Kill` land on
  `TerminateProcess`, so a Windows daemon never runs its graceful-shutdown path
  (discovery-file removal, the 300 ms grace) and may leave a stale discovery
  file for the next `pid_alive` probe to reap.
- **Windows has no uid.** The runtime namespace uses an FNV hash of `%USERNAME%`
  — stable per user, meaningless on its own, and never used for an
  authorization decision. The 0700-plus-ownership check on the runtime dir
  becomes "exists and is a directory", because the path sits inside the user's
  own profile whose inherited ACL already excludes other non-admin users.

### Testing the platform split

A `#[cfg]` that compiles is not a `#[cfg]` that works, and the tests have to
know the difference. `paniolo doctor` shipped in the first cut of the Windows
port depending on `sh -c` — a binary Windows does not have — and CI was green,
because the doctor tests asserted on the **text** of the generated shell script
rather than running it. A string-shape test passes identically on every
platform; it can only tell you what a command *would* say, never whether it can
launch.

So: **anything that spawns a process, touches the filesystem, or resolves a
name gets a test that executes it.** The ones that matter live in
`cli/src/platform.rs` (`pid_alive` against a real child, `shell_command`
running a command and reading its exit code back, `ensure_private_dir`
creating then revalidating), `cli/src/doctor.rs` (every `Probe` variant run
natively via `run_local`), and `cli/src/daemons.rs` (`find_binary` resolving a
bare name to a real file through `$PATH`).

Two bugs were found by writing exactly those tests, one of them serious:

- **Non-positive pids were unguarded on Unix.** `kill()` reads 0 as "every
  process in my group" and -1 as "every process I may signal", so a zero or
  corrupt pid in a discovery file made `pid_alive` answer *running* and would
  have made `signal_pid(.., Kill)` take down paniolo and the shell that
  launched it. `is_real_pid` now guards every entry point, in all five copies
  of the module, with a test in each so the guard cannot quietly go missing.
- **Opaque lab-file commands ran through `sh -c`.** Power hooks and the hid
  `cmd` are user-written strings that need a shell; `platform::shell_command`
  now supplies `sh` on Unix and `cmd.exe` on Windows.

`doctor` no longer needs a shell locally at all. Probes are a `Probe` enum with
two views — `run_local()` executes natively, `to_posix()` renders the POSIX
script still used for the SSH hop to a Unix control host — so the local path
and the remote path cannot drift apart silently.

### Windows packaging

The Windows artifact is a **portable zip**, not an MSI — paniolo is a CLI plus
nine helpers, so the Windows analogue of the Homebrew tap and the `.deb` is a
tarball of binaries, not an installer. (Contrast `~/src/oh-brother`, which needs
WiX + a Burn bundle because it is a GUI app bootstrapping a WebView2 runtime.)

The layout is flat, because Windows has no bin/libexec split to hang helpers
off — the exe's own directory *is* the install prefix:

```
paniolo\
  paniolo.exe          <- the only thing that goes on PATH
  libexec\
    hdmicap.exe  serialcap.exe  netbootd.exe  hidrig.exe  …
```

`daemons::exe_relative_dirs` adds `<exe dir>\libexec` on Windows, and
`daemons::find_binary` tries `<name>.exe` before the bare name. `paniolo setup`
on Windows does nothing but verify that layout: there is no setuid bit, no
`dialout` group, and no OCR helper to build.

Signing reuses oh-brother's Azure Artifact Signing setup — same account, same
OIDC identity, same action. Three differences from that pipeline: the **exes are
signed before zipping** (a zip carries no Authenticode signature, and SmartScreen
judges the exe), it is **ten files rather than three** (one glob, no Burn engine
detach/reattach — that dance is MSI-bundle-specific), and the RFC3161 timestamp
is asserted in a verify step for the same reason as there: Artifact Signing
certificates live 72 hours, so an untimestamped signature passes on release day
and dies in the field three days later.

`release.yml` gains `package-windows` (build → sign → verify → zip) and a
`winget` job. Both no-op until their credentials exist: signing skips while
`vars.AZURE_SIGNING_ACCOUNT` is unset, winget while `secrets.WINGET_TOKEN` is.
**The first winget version must be submitted by hand** (`wingetcreate` or
`komac`) — the action only updates a package that already exists.

### Developing against Windows

`brik.h.curtisg.xyz` (10.66.30.58) is the Windows bench host: SSH lands in
PowerShell 7, `$HOME` is `C:\Users\curti` (not `curtisg`), and the repo is
cloned at `C:\Users\curti\src\paniolo`. `scripts/sync-brik.sh <crate>…`
pushes sources there (it excludes `target/`, which is gigabytes and stalls the
transfer). The MSVC linker resolves without a Developer Prompt.

A local `cargo check --target x86_64-pc-windows-msvc` catches most `#[cfg]`
mistakes without leaving macOS, but **only for crates with no C dependency** —
anything pulling `ring` (i.e. anything with TLS) fails at its build script for
want of Windows headers. Those need the real host.

Per-subsystem behavior:

- **OCR backend.**
  - *macOS:* Apple Vision (`visionocr.swift`, compiled by `paniolo setup`).
  - *Linux:* Tesseract (`ocr/linuxocr`, copied by `paniolo setup`; requires the
    `tesseract-ocr` system package).
  - *Windows:* none. A port would supply a third binary behind the same
    stdin-PNG → stdout-text interface that both current backends expose via
    `PANIOLO_VISIONOCR`.
- **Netboot privileges.**
  - *macOS:* 14+ allows DHCP (port 67) and TFTP (port 69) rootless; `sudo` is
    used only for interface config (`ifconfig`).
  - *Linux:* both ports require root, so `paniolo netboot start` auto-prepends
    `sudo` when spawning `netbootd`, and `ip addr add` uses sudo too. With
    passwordless sudoers this is transparent; otherwise sudo prompts.
  - *Windows:* neither port is privileged — Windows has no low-port
    restriction — so `netbootd` is spawned directly with no `sudo` prefix.
- **Interface management** (`cli/src/netif.rs`, `netif::configure_interface()` /
  `restore_interface()`).
  - *macOS:* `networksetup` + `ifconfig`.
  - *Linux:* iproute2 — `ip addr add` / `ip link set up`, flushed with
    `ip addr flush dev <iface>`.
  - *Windows:* not implemented.
- **ARP pinning** (netbootd).
  - *macOS:* `arp -s`.
  - *Linux:* `ip neigh replace ... nud permanent`.
  - *Windows:* not implemented.
- **Raw-frame sender** (netbootd).
  - *macOS:* BPF sender over `/dev/bpf*` ioctls; netbootd receives the `/dev/bpf`
    descriptor from the setuid `netbootd-bpf-helper` and stays unprivileged (see
    the **netbootd** section).
  - *Linux:* no BPF path — the ioctls don't exist; the server falls back to a
    normal `sendto()` with retry.
  - *Windows:* not implemented.
- **Interface listing** (`list_usb_ethernet_interfaces()`).
  - *macOS:* `networksetup`.
  - *Linux:* sysfs — `/sys/class/net/` (type, carrier).
  - *Windows:* not implemented.
- **Serial device paths** (`serial::list_devices()`, `cli/src/serial.rs`).
  - *macOS:* scans `/dev` for `tty.usbserial-*` and `tty.usbmodem*` nodes.
  - *Linux:* one entry per physical port, named by its stable `/dev/serial`
    symlink — `by-id` preferred (names the adapter; what lab files typically
    use), `by-path` as the fallback (port-derived; the only stable name for
    adapters without a serial number), picked by `preferred_alias()`; raw
    `/dev/ttyUSB*`/`ttyACM*` only if no symlinks exist. Store a stable symlink
    path in target configs so serial interfaces survive USB adapter
    re-enumeration. The serialcap `--interface` parser accepts these paths
    (colons in a by-path name are not confused with the optional `:SENSE`
    suffix because only the known signal names `cts`, `dsr`, `dcd`, `ri` are
    treated as the sense suffix).
  - *Windows:* `serialport::available_ports()` returns the OS's `COM<n>`
    names. There is no by-path analogue, so a lab file pinned to `COM7` is
    less stable than a Linux by-id path — the number can move when an adapter
    is re-enumerated.
- **hdmicap capture + build deps.**
  - *macOS:* our own AVFoundation layer (`hdmicap/src/capture_avf.m`); no extra
    system packages beyond the Xcode toolchain.
  - *Linux:* V4L2 with a raw-MJPEG tee. Building requires `build-essential
    pkg-config libclang-dev clang` (V4L2 bindgen via `v4l2-sys-mit`) plus `cmake
    nasm` (the `turbojpeg` crate builds a vendored libjpeg-turbo — Debian's
    system libturbojpeg is too old for its pkg-config path, and the crate's
    `require-simd` default makes nasm mandatory on x86-64). `make install` fails
    early with a hint if any are missing (`check-deps` in the Makefile);
    `paniolo setup` prints a reminder.
  - *Windows:* our own Media Foundation layer (`hdmicap/src/capture_mf.rs`),
    delivering NV12 so it reuses the macOS pixel path. It enumerates the
    device's *native* media types and selects one explicitly — the Windows form
    of the AVFoundation lesson below, and the reason an Openterface captures at
    its real 3840x2160 instead of a rescaled 1080p. MJPEG-only devices are not
    yet supported.

## Removed: `usbhub` (per-port USB hub power)

paniolo shipped a `usbhub` helper that switched VBUS on individual ports of
off-the-shelf hubs via USB hub-class requests — the uhubctl mechanism, in pure
Rust. It was removed in full. Do not rebuild it without reading this.

Why it went:

- **The `learn` procedure was unusable.** Hubs do not report which port maps to
  which silkscreen number, and chips routinely claim per-port switching with no
  VBUS MOSFETs behind it. So a profile could only be built by a human physically
  watching a device lose power, port by port, through a resumable wizard. That is
  a lot of ceremony to configure one power hook, and it had to be redone per hub
  model.
- **The hardware is hard to get and unreliable.** Hubs with genuinely switchable
  per-port VBUS are a shrinking niche, and the ones that work are not
  consistently available.
- **Port state did not survive the hub losing power.** Power-cycling the hub
  turned every port back on, so the helper's state could silently disagree with
  reality.

It was also the only power helper with device-specific support inside `cli/`
(`usbhub_profiles.rs`, a `USBHUB_LIBRARY_PATH` special case in
`daemons::helper_env`, and a bundled profile library in the `.deb`). That
contradicted the rule in **Source code constraints** that device knowledge lives
in helper binaries behind the generic hooks — so removing it took a carve-out
out of the core as well as a crate out of the tree.

**What to use instead:** `shellyplug` (Shelly Gen2+ over local HTTP RPC),
`zigplug` (Zigbee), `amt` (Intel AMT/vPro, with true power-state readback),
`cambrionix` (Cambrionix hub ports), or a relay driven through the generic
hooks — which is what the CI rack actually uses. If per-port USB power is ever
needed again, the git history has the implementation; the mechanism was never
the problem.

## Known limitations / gotchas

- **Drive through paniolo, not around its devices.** The agent-facing rule
  (`skills/paniolo/SKILL.md`): never reconfigure or open a paniolo-managed device
  by hand — `ifconfig`/`ip`/`ethtool` on the netboot interface, `screen`/`tio` on
  a serial port, `kill` on a daemon. A background daemon (netbootd/serialcap/
  hdmicap/hid) holds the device and tracks state; touching it directly desyncs
  that state (a stray interface address silently changes what `netif status`
  reports; opening the serial port collides with the exclusive serialcap daemon).
  Use `netif mode …`, `serial log`/`send`, `daemons stop`.
- **`netif mode off`/`link` toggle the host IP, not the carrier; `netif
  down-hard` drops the carrier.** `mode link` assigns just the host IP (no
  daemon); `mode off` releases it. Neither admin-downs the interface, and `netif
  status` reports `carrier` independently of `mode` — a NIC with **Wake-on-LAN**
  enabled keeps the PHY energized when the interface is down, so `carrier` can
  read `up` in `off` mode. `netif down-hard` is the hard down (for testing
  link-drop *detection*): `mode off` + disable WoL (`ethtool -s <iface> wol d`,
  Linux) + admin-down (`ip link set down` / `ifconfig down`). `mode link`/`netboot`
  bring it back up; WoL stays off until re-enabled or the adapter is replugged.
  (macOS WoL is a system-wide `pmset womp` pref, so `down-hard` relies on
  admin-down there.) Heads-up: `del_host_ip` releases the macOS static IP via
  `networksetup -setdhcp` (an `ifconfig` delete won't unset a `-setmanual` IP),
  so `mode off` from `link` mode actually clears the address on macOS.
- **Interface configuration requires root.** `netif::configure_interface()`
  needs NOPASSWD sudo (`ifconfig`/`networksetup` on macOS, `ip` on Linux).
- **SSH PATH.** Non-interactive SSH shells often lack `/opt/homebrew/bin`;
  paniolo probes the Homebrew paths on macOS and `/usr/sbin`+`/sbin` on Linux
  when resolving helper binaries.
- **hdmicap device identity.** Capture devices have a stable, port-derived id
  (AVFoundation `uniqueID` on macOS, `/dev/v4l/by-path` symlink on Linux) shown
  by `hdmicap devices` / `paniolo video devices`. Prefer the id in lab files —
  identical dongles (MS2109s ship without USB serials) are indistinguishable by
  name. A name substring matching more than one device is a hard error listing
  the candidates' ids; with several non-built-in captures (e.g. MS2109 + Razer
  Kiyo), `paniolo configure` lists the id alternatives as comments.
- **macOS capture is our own AVFoundation layer** (`hdmicap/src/capture_avf.m`,
  C ABI consumed by `capture.rs`), replacing nokhwa + a vendored bindings fork.
  Two hard-won rules live in it: (1) never set
  `activeVideoMin/MaxFrameDuration` — MS2109-class HDMI sticks throw
  NSException from those KVC paths (the bug the old vendor patch existed for);
  (2) `activeFormat` alone is ignored — the session's default preset scales
  output to 1080p-class, so native resolution requires explicit
  `kCVPixelBufferWidth/HeightKey` in the output's `videoSettings`
  (`AVCaptureSessionPresetInputPriority` is iOS-only). Note the macOS UVC
  stack decodes MJPEG before AVFoundation — raw-MJPEG passthrough (the Linux
  tee) is impossible on macOS; frames arrive as NV12 ('420v', video-range)
  and RGB materializes lazily per request.
- **Capture daemons are per-target; singleton daemons are per-host.** Discovery,
  lock, and stderr log live under `${PANIOLO_RUNTIME_BASE:-/tmp}/paniolo-<uid>/`
  — the base is deliberately env-independent (NOT `$TMPDIR`, which macOS varies
  per environment so a running daemon was invisible from other shells; NOT
  `$XDG_RUNTIME_DIR`, which systemd deletes when the user's last session ends,
  breaking daemons that outlive their SSH session). The capture daemons
  (serialcap/hdmicap/hid) are **per target** — each gets its own daemon dir
  `<base>/paniolo-<uid>/<daemon>/<target>/`, so multiple targets capture
  concurrently on one host (multiple hdmicap = multiple capture devices). The
  host-singleton daemons (zigplug/cambrionix/netbootd) stay **one per host** at
  `<base>/paniolo-<uid>/<daemon>/` with no `<target>` segment.
- **Daemon shutdown hard-exits.** Both hdmicap (`/preview` MJPEG) and serialcap
  (`/stream` WebSocket) serve infinite responses, so a plain axum graceful
  shutdown would block on them forever. On SIGTERM each daemon removes its
  discovery file, gives a 300 ms grace, then `std::process::exit(0)`. The OS
  releases the capture device / serial port on exit.
- **Serial ports are exclusive.** Only one of `tio`/`screen`/serialcap can hold
  a port at a time. `paniolo serial watch` and `paniolo serial connect` conflict
  on the same device — use one or the other.
- **macOS serialport can't open PTYs.** The `serialport` crate sets baud via the
  `IOSSIOSPEED` ioctl, which returns ENOTTY ("Not a typewriter") on pseudo-
  terminals. serialcap byte-flow can only be tested against a real serial device,
  not a `pty.openpty()` pair.
- **OCR character confusions on small console fonts.** Both visionocr and linuxocr
  2×-upscale and black-pad before recognition, but thin terminal fonts still
  produce confusions (`1`↔`l`↔`I`, `2`↔`Z`, colon spacing in IPv6). Accuracy
  improves markedly on larger boot-screen text. Do not change the target's console
  font to work around this — the font is relied upon by other agents (see OCR section).
  On macOS, `VNRecognizeTextRequest` `.accurate` returns nothing on thin console
  fonts; visionocr uses `.fast`.
- **macOS Local Network privacy gate.** On macOS (Sequoia+), a freshly built,
  non-Apple-signed helper that reaches a LAN host (e.g. `shellyplug`) fails with
  `No route to host` (EHOSTUNREACH) until the launching app is granted Local
  Network access (System Settings → Privacy & Security → Local Network). iTerm's
  detached server can need an iTerm restart for the grant to take; loopback
  daemons and Apple-signed `curl` are exempt. The tell: it reaches the internet
  but not a same-subnet host. See docs/power.md (shellyplug gotchas).
- **Homebrew tap upgrades.** The tap formula re-pins on each release. A
  same-version change to install logic won't `brew upgrade` — use `brew
  reinstall`, or bump the formula `revision`. (After a local `make rust`, the
  `~/.cargo/bin` paniolo can also be shadowed on PATH by the Homebrew keg.)
- **Reproducing CI locally.** GitHub's Linux jobs are easiest to mirror with
  `scripts/ci-local.sh` in a Lima VM; it copies the working tree to the VM's own
  disk first, because building on the shared virtiofs/9p mount fails (setuptools'
  editable `egg-info` can't update timestamps there, and cargo would clobber the
  host `target/`).
