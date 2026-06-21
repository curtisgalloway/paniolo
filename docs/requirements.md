# Paniolo — Requirements & progress tracker

> Project-wide requirements for **paniolo**, an agent-controlled target-machine wrangler for
> low-level software development (bootloaders, firmware, OS bring-up). This is the single
> source of truth for *what paniolo must do* and *how far along each capability is* — covering
> both shipped capabilities and planned work.
>
> Scope note: paniolo is a **device-control / "wrangling" layer** (power, serial, deploy/netboot,
> video, HID). It deliberately does **not** own test orchestration or result production — when
> integrated with hardware-CI ecosystems, those stay above paniolo (see §9).
>
> Companion design docs live under [`docs/ci-integration/`](ci-integration/gap-analysis.md) (gap analysis +
> integration design) and per-feature docs under [`docs/`](README.md). **Update the Status column as
> work lands.**
>
> Last updated: 2026-06-05.

## Status legend

| Symbol | Meaning |
|---|---|
| ☑ | Done / shipped |
| ◐ | In progress |
| ☐ | Not started |
| ⤵ | Deferred (planned, later) |
| ⊘ | Out of scope (recorded, not planned) |

**Pri** = M(ust) / S(hould) / C(ould). **Source/Notes** cites the driving need or contract.

---

## 1. Foundations / platform

| ID | Requirement | Pri | Status | Notes |
|---|---|---|---|---|
| CORE-1 | Daemonless target config readable by every command; single target is the default | M | ☑ | *originally per-target `targets/<name>.toml` files (`_config.py`); superseded by the one-file lab model — CORE-7* |
| CORE-2 | CLI (`paniolo`) over subcommands; SSH-drivable from a dev machine into the control host | M | ☑ | Remote-control pattern (README) |
| CORE-3 | Run on macOS 10.14+ and Linux (x86-64/arm64) | M | ☑ | `paniolo setup` installs daemons/tools |
| CORE-4 | Rust daemons (`hdmicap`, `serialcap`) + Swift `visionocr` helper build & install | M | ☑ | `paniolo setup` |
| CORE-5 | Predictable runtime paths (configs, daemon discovery, capture logs) | M | ☑ | [architecture.md §4](architecture.md#4-configuration-and-state-model) |
| CORE-6 | Agent-oriented guidance kept current (`AGENTS.md`) as the surface changes | M | ◐ | Must track power/serial changes in §9 |
| CORE-7 | One-file **lab** model (`--lab`/`PANIOLO_LAB`): hosts + targets, per-channel host binding | S | ☑ | `cli/src/model.rs`, `labfile.rs`; [distributed-control](distributed-control.md) (the Rust CLI reads no legacy targets dir) |
| CORE-8 | Transparent re-exec of host-operating commands on a target's **remote control host** over SSH | S | ☑ | `cli/src/dispatch.rs` (ships a lab slice + `--lab`); `cli/src/ssh.rs` transport |
| CORE-9 | Tunnelled `console` for a remote target (dashboard reachable locally) | S | ☑ | `remote_console` in `cli/src/main.rs`; `?serialws=` stitch |
| CORE-10 | Multi-host targets (one target spanning control hosts) | C | ◐ | per-channel dispatch routes each command to its channel's host (`dispatch.rs`); composite `console` still requires co-located channels |
| CORE-11 | Remote `setup --host` + discovery-assisted `configure` | C | ☑ | `paniolo setup --host`, `discover`, `configure` (`cli/src/main.rs`) |

## 2. Netboot / deploy

| ID | Requirement | Pri | Status | Notes |
|---|---|---|---|---|
| NET-1 | Built-in DHCP + TFTP over a direct USB-Ethernet link | M | ☑ | `netbootd/` (Rust engine, default); `192.168.99.1/24` |
| NET-2 | `netboot start/stop/status`, `tftp-root`, `logs` (filterable, followable) | M | ☑ | `cli/src/netboot.rs` |
| NET-3 | Bare-link up/down/status for interface testing | M | ☑ | `netif mode link`/`off` + `netif status` (carrier); `netif down-hard` forces a real carrier drop (WoL off + admin-down) for link-drop-detection tests. Replaced the `netboot link-up/down/status` trio |
| NET-4 | TFTP root configurable per target (`--tftp-root`) | M | ☑ | required for `netboot start` |
| NET-5 | `netif mode netboot\|link\|ffx\|off` — atomic, idempotent link-mode switch; `link` = bare host IP (no daemon) for link testing; stops netboot before SD boot, sets up host `fe80::1`/64 for ffx; `netif status` probes the active mode + carrier | S | ☑ | `cli/src/netif.rs`; from rpi5-bringup ffx-over-network bring-up |

## 3. Serial console

| ID | Requirement | Pri | Status | Notes |
|---|---|---|---|---|
| SER-A | `serialcap` daemon owns the port exclusively; supervisor fans out reads | M | ☑ | `serialcap/`; lockfile |
| SER-B | Timestamped rolling JSONL capture log, addressable by seq; rotation | M | ☑ | `capture.rs`; `serial log` |
| SER-C | Interactive terminal via `tio` (`serial connect`) | M | ☑ | |
| SER-D | Bidirectional live `/stream` (WebSocket) — read + write-back | M | ☑ | `server.rs` (used by dashboard) |
| SER-E | `serial add/set/rm/devices/show`, multi-interface per target | M | ☑ | |
| SER-F | DTR control: `serial dtr`, `serial reset` (soft-reset semantics) | M | ☑ | `cli/src/power.rs` |
| SER-G | Power-sense read via modem-control input (`--power-sense cts\|dsr\|dcd\|ri`) | S | ☑ | `/status` → `power_on` |

## 4. Power control

| ID | Requirement | Pri | Status | Notes |
|---|---|---|---|---|
| PWR-A | `power-cycle` via configurable script (`--power-cycle-cmd`) | M | ☑ | *superseded by the `[power]` hook block (`--cycle-cmd` et al.) — PWR-5 in §9* |
| PWR-B | `power-state` (read-only on/off via sense signal) | M | ☑ | `power-state` |
| PWR-C | DTR-based hardware power-button toggling (J2 header): ≤500ms soft / ≥3s hard | M | ☑ | `cli/src/power.rs` |

## 5. Video / OCR

| ID | Requirement | Pri | Status | Notes |
|---|---|---|---|---|
| VID-1 | HDMI/USB capture via warm-stream `hdmicap` daemon | M | ☑ | `hdmicap/`; Linux V4L2 + macOS |
| VID-2 | `video watch/preview/shot/read/devices/show/stop`; stable & changed-since capture | M | ☑ | |
| VID-3 | On-device OCR (`video read`): Apple Vision (macOS), Tesseract (Linux) | S | ☑ | `ocr/` helpers via hdmicap `GET /ocr` (the legacy `--json` flag was not carried into the Rust CLI) |

## 6. HID injection

| ID | Requirement | Pri | Status | Notes |
|---|---|---|---|---|
| HID-1 | USB keyboard/mouse injection via KB2040 injector (dual-board "dumb pipe": host composes, control board CDC → I2C1 → target HID) | S | ☑ | `hidrig/` crate + `firmware/dual/` |
| HID-2 | Device-independent HID serial protocol (v1) so other microcontrollers can implement the injector | S | ☑ | `docs/hid-serial-protocol.md` |
| HID-3 | Generic `hid` lab channel: `paniolo hid set/rm/send` appends args to an opaque helper cmd | S | ☑ | mirrors power hooks; SSH dispatch |
| HID-4 | Absolute mouse (`moveabs`, advertised capability) for click-where-you-point | S | ☑ | abs-pointer HID descriptor in firmware |
| HID-5 | `hidrig serve` daemon: owns the control link, re-exposes the command vocabulary over a WebSocket; one-shots route through it | S | ☑ | `paniolo hid serve/stop` |
| HID-6 | KVM in `paniolo console`: stream web keyboard + absolute mouse, intermixed with CLI injection | S | ☑ | hardware-verified on pi5 Linux desktop |
| HID-7 | KVM latency: HID frames fire-and-forget over USB-CDC (no per-frame round-trip), coalesce mouse moves (per-frame); floor is the target's USB `bInterval` (~8 ms) | S | ☑ | macOS `IOSSDATALAT` floored for control-frame replies |

## 7. Dashboard

| ID | Requirement | Pri | Status | Notes |
|---|---|---|---|---|
| DASH-1 | Combined video + serial web UI (`paniolo console`); auto-starts daemons | S | ☑ | preselect serial via `-i` |
| DASH-2 | Dashboard power-cycle control | S | ☑ | |

## 8. Cross-cutting / non-functional

| ID | Requirement | Pri | Status | Notes |
|---|---|---|---|---|
| NF-1 | Interactive/agent bring-up workflow (dashboard, OCR, HID, `tio`, JSONL) never regresses | M | ◐ | Regression guard on every change, esp. §9 serial work |
| NF-2 | Changes land as smallest reversible steps, each with tests | M | ◐ | |
| NF-3 | Core power/serial path stays functional on both macOS and Linux | M | ☑ | CI-only features may be Linux-only (see §9) |
| NF-4 | External contracts re-verified against upstream before relying on them | M | ◐ | re-check Fuchsia `device.go` (FX-4) |

---

## 9. Hardware-CI integration (KernelCI/LAVA + Fuchsia/botanist)

> Goal: make paniolo's primitives **consumable by** LAVA (under KernelCI's Maestro) and
> `botanist`+`testrunner` (under Fuchsia/LUCI) — *without* paniolo owning orchestration or
> results. Full analysis: [`ci-integration/gap-analysis.md`](ci-integration/gap-analysis.md);
> design: [`ci-integration/design.md`](ci-integration/design.md).
>
> **Current focus:** the owner is doing a **Fuchsia port** with an agent — single user, no
> existing users, breaking changes are free. M1 leads with the Fuchsia-critical path (PTY +
> power); the botanist adapter is sequenced before LAVA.

### 9.0 Decisions (locked 2026-05-29)

| ID | Decision | Resolution |
|---|---|---|
| D-1 | KCIDB results path | ⊘ Out of scope — LAVA-lab path only for KernelCI |
| D-2 | Fuchsia serial ownership | PTY proxy; paniolo keeps the physical port (JSONL/dashboard stay live) |
| D-3 | Serial write arbitration | Cooperative last-writer-wins + advisory lock in `/status` + opt-in `--exclusive`, **auto-released on client disconnect** (+ optional `--lock-timeout`) |
| D-4 | JTAG in v1 | Extension point only (schema + verb stubs); OpenOCD backend deferred |
| D-5 | CI control-host OS | Linux-only for CI; macOS stays first-class for interactive bring-up |
| D-6 | Deploy ownership in CI | Orchestrator owns deploy (LAVA TFTP / botanist pave); paniolo netboot stands down |
| D-7 | Serial TCP endpoint | Native Rust TCP listener in serialcap; ser2net-on-PTY as LAVA fallback |
| D-8 | `[power]` config | Breaking change accepted — clean `[power]` block, no `power_cycle_cmd` alias; update `AGENTS.md` |

### 9.1 Agnostic device-control API — Power

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| PWR-1 | `paniolo power on` — applies power; DUT begins booting unattended | LAVA | M | ☑ | `on_cmd` hook (2026-06-04) |
| PWR-2 | `paniolo power off` — cuts power | LAVA | M | ☑ | `off_cmd` hook (2026-06-04) |
| PWR-3 | `paniolo power reset` — off+delay+on (hard reset) | LAVA | M | ☐ | verb is still `power-cycle`; the `cambrionix` helper's `cycle` does off+delay+on |
| PWR-4 | `paniolo power state` — read on/off | BOTH | M | ☑ | `power-state`, now `state_cmd`-backed when configured; rename only |
| PWR-5 | `[power]` config block w/ `backend = script\|dtr\|pdu\|jtag` + on/off/reset cmds | BOTH | M | ☑ | landed 2026-06-04 as generic hooks (`cycle/on/off/state_cmd`) — no backend enum; device-specific logic lives in helper binaries |
| PWR-6 | Power commands usable as plain shell cmds (string or list) from a generator | LAVA | M | ☑ | hooks are plain `sh -c` strings |
| PWR-7 | Update `AGENTS.md` for the new `[power]` config + verbs | OWNER | M | ☑ | done with the hooks change |

### 9.2 Agnostic device-control API — Serial (core gap)

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| SER-1 | serialcap exposes a **raw bidirectional TCP listener** (ser2net-equivalent) | LAVA | M | ☐ | backs `connection_command = telnet host port` |
| SER-2 | serialcap exposes a **PTY** whose slave path is a real device file | FX | M | ☐ | handed to botanist as `DeviceConfig.serial` |
| SER-3 | New endpoints **tee off the existing supervisor** (JSONL/WS/dashboard unaffected) | OWNER | M | ☐ | preserves NF-1 |
| SER-4 | `paniolo serial send <bytes\|->` one-shot write (agent feature) | OWNER | M | ☐ | same `write_tx` channel; `--enter`/`--hex`/stdin |
| SER-5 | Write arbitration per D-3 (lock, `/status` holder, `--exclusive`, auto-release) | OWNER | M | ☐ | |
| SER-6 | Stable socket/PTY paths under `$XDG_RUNTIME_DIR/paniolo/<target>/` | BOTH | S | ☐ | predictable for adapters |
| SER-7 | Existing JSONL log, `/stream`, `tio`, `serial log/dtr/reset` unchanged | OWNER | M | ☐ | regression guard / tests |

### 9.3 Agnostic device-control API — Deploy / boot / debug

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| DEP-1 | netboot **stands down** under CI; no DHCP/TFTP contention | BOTH | M | ☐ | guard `netboot start` when CI attach active |
| DEP-2 | netboot remains available for interactive/non-CI use | OWNER | M | ☑ | exists (NET-1..4); just not the CI path |
| DEP-3 | (Full) paniolo-serves-images as a non-standard LAVA deploy method | LAVA | C | ⤵ | only if a board can't use LAVA TFTP |
| BOOT-1 | `paniolo serial wait --match <regex> [--timeout]` boot-detect helper | OWNER | S | ⤵ | not required by either orchestrator; ergonomics |
| JTAG-1 | `[jtag]`/`[debug]` config schema + `paniolo debug {halt\|resume\|reset\|gdb}` stubs | OWNER | C | ☐ | extension point only per D-4 |
| JTAG-2 | OpenOCD backend: reset, flash-deploy, GDB `:3333` / Tcl `:6666` sockets | OWNER | C | ⤵ | deferred |

### 9.4 Adapter A — LAVA lab

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| LAVA-1 | Device-dictionary + device-type template generator (`paniolo lava device-dict`) | LAVA | M | ☐ | power_* → `paniolo power …`; connection → telnet |
| LAVA-2 | Generator supports list-valued power commands | LAVA | S | ☐ | |
| LAVA-3 | "First device" onboarding doc (Debian worker, ser2net/TCP wiring, tokens) | LAVA | S | ☐ | internet-reachable lab; tokens to KernelCI admins |
| LAVA-4 | Verified on a Debian LAVA worker against a real board | LAVA | S | ☐ | macOS unsupported (D-5) |

### 9.5 Adapter B — Fuchsia / botanist

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| FX-1 | botanist device-config emitter (`paniolo botanist device-config`) → PTY path | FX | M | ☐ | `{network,keys,serial}`; serial = PTY (SER-2) |
| FX-2 | Bot-host/recipe **power wrapper** calling `paniolo power {on\|reset\|off}` | FX | M | ☐ | power is NOT a device-config field |
| FX-3 | `bot_config.py` `get_dimensions()` snippet advertising `device_type:<board>` | FX | S | ☐ | + `bots.cfg`, `platforms.gni` (upstream coord) |
| FX-4 | Verify `DeviceConfig`/power plumbing against a real Fuchsia checkout | FX | M | ☐ | confirm `tools/botanist/target/device.go` |
| FX-5 | Document RFC-0130 Experimental tier (self-hosted CI) | FX | C | ☐ | community board is not "Supported" tier |

---

### 9.6 Adapter C — Redfish provider

> **Decision (D-9, 2026-05-29):** Redfish interop = **provider** direction (paniolo exposes a
> Redfish API in front of BMC-less boards), **not client**. Higher-leverage than per-ecosystem
> adapters because Redfish is the bare-metal lingua franca (Ironic/Metal3 primary control plane;
> LAVA can `curl` it). Sequenced **after** M1 — consumes the power verbs (PWR-1..6) and the raw
> serial socket (SER-1). Design sketch: [`ci-integration/redfish-provider.md`](ci-integration/redfish-provider.md).
> Verified against DMTF canonical CSDL (DSP0266 v1.22.0, DSP8010 2025.2), OpenBMC, Ironic/sushy.

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| RF-1 | Redfish provider service: `ServiceRoot` → `ComputerSystem` → `Manager` (→ `VirtualMedia`) resource tree | OWNER | S | ⤵ | deferred after M1; provider, not client |
| RF-2 | `#ComputerSystem.Reset` → power verbs (On→on, ForceOff→off, PowerCycle/ForceRestart→reset); `PowerState` → power-state | OWNER | S | ⤵ | depends on PWR-1..6 |
| RF-3 | `Boot.BootSourceOverrideTarget=Pxe` + `BootSourceOverrideEnabled=Once` → netboot | OWNER | S | ⤵ | maps to existing netboot |
| RF-4 | `VirtualMedia` `InsertMedia`/`EjectMedia` → image deploy | OWNER | C | ⤵ | open: needed vs. Pxe-once sufficient? |
| RF-5 | `SerialConsole` advertises out-of-band SSH/console endpoint pointing at paniolo raw-serial socket (metadata only) | OWNER | S | ⤵ | depends on SER-1; Redfish carries no serial bytes |
| RF-6 | Honest per-node `ResetType@Redfish.AllowableValues` / `ActionInfo` for the supported subset | OWNER | S | ⤵ | relay/DTR boards can't do every `ResetType` |
| RF-7 | Implement via a sushy-tools-style emulator + paniolo backend driver (not a hand-rolled OData service) | OWNER | S | ⤵ | open: dependency footprint (core = `typer` only) |
| RF-8 | Document/decide whether Redfish provider replaces or complements LAVA/botanist adapters | OWNER | S | ⤵ | botanist PTY serial seam still needs the direct path → not a full replacement |

## 10. Security

> **TODO — owner to populate.** This section needs dedicated attention and is intentionally
> unfinished. Paniolo grants an agent physical-equivalent control of a target (power, raw
> serial read/write, netboot/TFTP, HID injection) and, with the §9 work, opens **network-facing
> serial endpoints** and is **SSH-driven from a dev machine into the control host** — so the
> threat model and controls deserve first-class requirements, not afterthoughts.

| ID | Requirement | Pri | Status | Notes |
|---|---|---|---|---|
| SEC-0 | Define paniolo's threat model and security requirements | M | ☐ | **Placeholder — to be written.** |

Prompts to resolve when populating (not yet requirements — discussion seeds):

- **Serial endpoint exposure (§9):** the raw TCP listener (SER-1) currently mirrors serialcap's
  loopback-only bind (`127.0.0.1`). For a LAVA worker / Swarming bot, who may connect? Auth,
  bind address, TLS, or rely on SSH-tunnel/localhost-only + network isolation?
- **Write arbitration as a safety control (SER-5/D-3):** is `--exclusive` purely cooperative, or
  also a guard against an unexpected writer driving the target?
- **Netboot/DHCP/TFTP:** read-only TFTP, single-client; any spoofing/rogue-DHCP concerns on a
  shared lab network vs. the assumed direct USB-Ethernet link?
- **Power/HID authority:** anyone who can reach the control host can power-cycle and inject HID —
  what bounds that (host access model, per-target ACLs)?
- **Secrets:** LAVA submission tokens, `$FUCHSIA_SSH_KEY`, CIPD/Swarming creds — storage and
  handling.
- **Supply chain:** `paniolo setup` builds/install Rust + Swift + Homebrew components.

---

## 11. Milestones

| Milestone | Contents | Status |
|---|---|---|
| M0 — Analysis & design | gap-analysis, design, this tracker, decisions | ☑ |
| Shipped baseline | §1–§7 capabilities (netboot, serial, power, video, HID, dashboard) | ☑ |
| M1 — Agnostic device-control core | SER-2, SER-4, PWR-1..7, SER-5, SER-1, DEP-1, JTAG-1 (Fuchsia path first) | ☐ (awaiting go-ahead) |
| M2 — Adapters | FX-1..4 (first), then LAVA-1..3 | ☐ |
| M3 — Verify on hardware | FX-3/FX-5, LAVA-4, BOOT-1 | ☐ |
| Security | §10 (SEC-*) | ☐ (to be defined) |
| M4 — Full (deferred) | DEP-3, JTAG-2 | ⤵ |

## 12. Open implementation questions

| ID | Question | Status |
|---|---|---|
| SER-Q1 | Native TCP listener vs. ser2net-on-PTY | ✓ Resolved (D-7): native listener; ser2net fallback |
| SER-Q2 | Write-lock lifetime | ✓ Resolved (D-3): auto-release on disconnect + optional `--lock-timeout` |
| PWR-Q1 | `[power]` shape + `power_cycle_cmd` migration | ✓ Resolved (D-8): clean breaking block, no alias |
