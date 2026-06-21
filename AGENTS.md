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

4. **Validate before pushing.** Build and run the tests for the crates you
   touched. To catch the Linux-only CI failures without a round-trip, run
   `scripts/ci-local.sh` — it mirrors the GitHub Linux CI jobs (`cli`,
   `serialcap`, `netbootd`, `hdmicap`) in a Linux environment, e.g. a
   Lima VM: `limactl shell <instance> -- bash -l scripts/ci-local.sh`. (The
   macOS-only job — hdmicap AVFoundation + visionocr — runs on the host.) Note
   `cli` is the primary control-plane crate; don't let its tests rot.

5. **Push, open the PR, and merge only when asked.** Get the branch ready by
   committing locally, but treat `git push`, `gh pr create`, and merging as
   gated on the maintainer's explicit instruction — don't push or open a PR on
   your own initiative, and never merge. When told to, open with `gh pr create`;
   the merge decision always belongs to the maintainer.

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
- Power control via DTR (J2 wiring; **opt-in per serial interface** via `power_button = true` — `serial dtr`/`reset` refuse interfaces that haven't declared it) or generic shell-command hooks (`on_cmd`, `off_cmd`, `cycle_cmd`, `state_cmd`): `paniolo serial dtr`, `paniolo power on/off`, `paniolo power-cycle`, `paniolo power-state`. Note: "reboot over the serial console" means `serial send <t> "reboot"` (software), *not* the DTR `serial reset` (hardware). Helpers that wire into the hooks: `cambrionix` (Cambrionix hub port power via control UART), `zigplug` (Zigbee smart plugs via a CC2652 coordinator dongle), `usbhub` (per-port VBUS switching on off-the-shelf USB hubs via hub-class requests, with human-verified port profiles built by `usbhub learn`), and `shellyplug` (Shelly Gen2+ smart plugs/relays over the device's local HTTP RPC API — no cloud/HA/Matter). The dual-board `hidrig` control board can also drive a DUT power relay (`hidrig power off|on|cycle`) as a power-helper backend, consolidating HID + console + power on one USB device

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
  (`paniolo`, `kvm-puppeting`, `usbhub`) install to
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

Deferred (tracked in docs/config-redesign.md): the
Openterface CH9329 HID backend (clean-room spec at docs/ch9329-spec.md — a shim
speaking the HID serial protocol would plug into the existing `hid` channel).

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
                 (aHash + no-signal from ~1k luma samples, resolution-independent)
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

usbhub/          Rust crate: standalone helper for per-port USB hub power control
                 via hub-class requests (pure Rust, nusb; uhubctl mechanism, works
                 on macOS + Linux). Hubs addressed by model profile (signature-first
                 resolution of the internal chip cascade, both USB3 + USB2 sides);
                 ports by physical silkscreen number. Switching refused unless a
                 human verified the port cuts power — profiles are built with the
                 resumable `usbhub learn` step commands (agent-drivable) or the
                 `learn run` guided wizard (rustyline prompts, history). Probe
                 detection is by bus topology, not speed. `state <port>` prints
                 exactly `on`/`off` (state_cmd contract). See docs/power.md.

shellyplug/      Rust crate: standalone helper for Shelly Gen2+ smart plugs/
                 relays (Plus/Pro/Gen3/Gen4) over the device's local HTTP RPC
                 API (Switch.Set/GetStatus; ureq). One-shot, stateless — no
                 daemon. Addressed by `-d <ip|host>` and `[id]` switch (default
                 0). Commands: `status [id]`, `state [id]`, `on/off [id]`,
                 `cycle [id]`; `on/off/cycle` confirm by read-back. Gen2+ only
                 (no Gen1 REST); auth-disabled devices only for now. NB: first
                 helper to reach a LAN device, so first to hit the macOS
                 Local Network privacy gate — see docs/power.md gotchas.

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
  setup commands. Error messages must be generic.
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
checklist); `cambrionix/`, `zigplug/`, `usbhub/`, and `shellyplug/` (the
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

## Linux support

Paniolo runs on Linux as well as macOS. Platform differences:

- **OCR backend is platform-specific.** macOS uses Apple Vision (`visionocr.swift`,
  compiled by `paniolo setup`). Linux uses Tesseract (`ocr/linuxocr`, copied by
  `paniolo setup`; requires `tesseract-ocr` system package). Both expose the same
  stdin-PNG → stdout-text interface via `PANIOLO_VISIONOCR`.
- **Netboot uses `sudo` internally on Linux.** DHCP (port 67) and TFTP (port 69)
  require root on Linux; macOS 14+ allows them rootless. `paniolo netboot start`
  auto-prepends `sudo` when spawning `netbootd` on Linux. With passwordless
  sudoers this is transparent; otherwise sudo prompts for a password. Interface
  config (`ip addr add`) also uses sudo, same as macOS uses it for `ifconfig`.
- **Interface management uses `ip` on Linux.** `netif::configure_interface()`
  runs `ip addr add`/`ip link set up` (iproute2) instead of
  `networksetup`+`ifconfig`; `restore_interface()` flushes with
  `ip addr flush dev <iface>`. (See `cli/src/netif.rs`.)
- **ARP pinning uses `ip neigh replace` on Linux.** netbootd's ARP pin calls
  `arp -s` on macOS and `ip neigh replace ... nud permanent` on Linux.
- **BPF raw-frame sender is macOS-only.** netbootd's BPF sender uses `/dev/bpf*`
  ioctls that don't exist on Linux; on Linux it is unavailable and the server
  falls back to normal `sendto()` with retry. On macOS netbootd receives the
  `/dev/bpf` descriptor from the setuid `netbootd-bpf-helper` and stays
  unprivileged (see the **netbootd** section).
- **hdmicap build deps on Linux.** Building hdmicap requires system packages:
  `build-essential pkg-config libclang-dev clang` (for V4L2 bindgen via
  `v4l2-sys-mit`) plus `cmake nasm` (the `turbojpeg` crate builds a vendored
  libjpeg-turbo — Debian's system libturbojpeg is too old for its pkg-config
  path, and the crate's `require-simd` default makes nasm mandatory on
  x86-64). `make install` fails early with a hint if any are missing
  (`check-deps` in the Makefile); `paniolo setup` prints a reminder.
- **Interface listing uses sysfs on Linux.** `list_usb_ethernet_interfaces()`
  reads `/sys/class/net/` (type, carrier) instead of `networksetup`.
- **Serial device paths use by-path symlinks on Linux.** `list_serial_devices()`
  returns `/dev/serial/by-path/` entries when available; `canonical_device_path()`
  upgrades a raw `ttyUSBX` path to its stable symlink. Store by-path paths in
  target configs so serial interfaces survive USB adapter re-enumeration. The
  serialcap `--interface` parser accepts by-path paths (colons in the path are
  not confused with the optional `:SENSE` suffix because only known signal names
  `cts`, `dsr`, `dcd`, `ri` are treated as the sense suffix).

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
