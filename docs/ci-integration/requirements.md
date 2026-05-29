# Paniolo hardware-CI integration — Requirements & progress tracker

> Living document. Tracks every requirement for making paniolo a device-control layer under
> **KernelCI/LAVA** and **Fuchsia/Swarming(botanist)**, plus the owner's adjacent goals
> (agent write-to-serial, JTAG). Companion to `gap-analysis.md` (the delta) and `design.md`
> (the how). **Update the Status column as work lands.**
>
> Last updated: 2026-05-29 — *analysis/design complete; implementation not started.*
>
> **Current focus:** single user (the owner) doing a **Fuchsia port** with an agent — no
> existing users, breaking changes are free. M1 leads with the Fuchsia-critical path (PTY +
> power); Adapter B (Fuchsia) is sequenced before Adapter A (LAVA).

## Status legend

| Symbol | Meaning |
|---|---|
| ☐ | Not started |
| ◐ | In progress |
| ☑ | Done (code + tests merged) |
| ⊘ | Out of scope (recorded, not planned) |
| ⤵ | Deferred (planned, later milestone) |

**Source** = where the requirement comes from: `LAVA`, `FX` (Fuchsia/botanist), `OWNER`
(owner's stated goal), `BOTH`. **Pri** = M(ust) / S(hould) / C(ould) for the first useful
milestone.

---

## A. Decisions (locked 2026-05-29)

| ID | Decision | Resolution |
|---|---|---|
| D-1 | KCIDB results path | ⊘ Out of scope — LAVA-lab path only for KernelCI |
| D-2 | Fuchsia serial ownership | PTY proxy; paniolo keeps the physical port (JSONL/dashboard stay live) |
| D-3 | Serial write arbitration | Cooperative last-writer-wins + advisory lock in `/status` + opt-in `--exclusive` |
| D-4 | JTAG in v1 | Extension point only (schema + verb stubs); OpenOCD backend deferred |
| D-5 | CI control-host OS | Linux-only for CI; macOS stays first-class for interactive bringup |
| D-6 | Deploy ownership in CI | Orchestrator owns deploy (LAVA TFTP / botanist pave); paniolo netboot stands down |

---

## B. Agnostic device-control API (the core)

### Power

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| PWR-1 | `paniolo power on` — applies power; DUT begins booting unattended | LAVA | M | ☐ | Maps to `power_on_command`; hard requirement that power-up alone boots |
| PWR-2 | `paniolo power off` — cuts power | LAVA | M | ☐ | DTR long-press (≥3s) or PDU script |
| PWR-3 | `paniolo power reset` — off+delay+on (hard reset) | LAVA | M | ☐ | Reuse `power_cycle_cmd`; fallback off+sleep+on |
| PWR-4 | `paniolo power state` — read on/off | BOTH | S | ☑* | Exists today via sense signal (`/status`); *verify rename only |
| PWR-5 | `[power]` config block w/ `backend = script\|dtr\|pdu\|jtag` + on/off/reset cmds | BOTH | M | ☐ | **Breaking**: replaces flat `power_cycle_cmd` (no alias); update `AGENTS.md` |
| PWR-7 | Update `AGENTS.md` agent guidance for the new `[power]` config + power verbs | OWNER | M | ☐ | Agent reconfigures targets on redeploy |
| PWR-6 | Power commands usable as plain shell cmds (string or list) from a generator | LAVA | M | ☐ | LAVA fields accept string OR list (brief discrepancy #3) |

### Serial (core gap)

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| SER-1 | serialcap exposes a **raw bidirectional TCP listener** (ser2net-equivalent) | LAVA | M | ☐ | Backs `connection_command = telnet host port`; see Q SER-Q1 |
| SER-2 | serialcap exposes a **PTY** whose slave path is a real device file | FX | M | ☐ | Handed to botanist as `DeviceConfig.serial`; botanist opens it |
| SER-3 | New endpoints **tee off existing supervisor** (JSONL + WebSocket + dashboard unaffected) | OWNER | M | ☐ | Must not regress interactive workflow |
| SER-4 | `paniolo serial send <bytes\|->` one-shot write (agent feature) | OWNER | M | ☐ | Same `write_tx` channel; `--enter`/`--hex`/stdin |
| SER-5 | Write arbitration: cooperative last-writer + advisory lock + `--exclusive` | OWNER | M | ☐ | Per D-3; current writer shown in `/status`; exclusive lock auto-releases on client disconnect (+ optional `--lock-timeout`) |
| SER-6 | Stable socket/PTY paths under `$XDG_RUNTIME_DIR/paniolo/<target>/` | BOTH | S | ☐ | Predictable for adapters |
| SER-7 | Existing JSONL log, `/stream` WS, `tio`, `serial log/dtr/reset` unchanged | OWNER | M | ☐ | Regression guard / tests |

### Deploy

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| DEP-1 | netboot **stands down** under CI; no DHCP/TFTP contention with orchestrator | BOTH | M | ☐ | Per D-6; guard `netboot start` when CI attach active |
| DEP-2 | netboot remains available for interactive bringup / non-CI boards | OWNER | M | ☑ | Exists today; just not the CI path |
| DEP-3 | (Full) paniolo-serves-images as a non-standard LAVA deploy method | LAVA | C | ⤵ | Deferred; only if a board can't use LAVA TFTP |

### Boot / detect

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| BOOT-1 | `paniolo serial wait --match <regex> [--timeout]` boot-detect helper | OWNER | S | ⤵ | Not required by either orchestrator; ergonomics + MVP smoke |

### Debug / JTAG (extension point)

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| JTAG-1 | `[jtag]`/`[debug]` config schema + `paniolo debug {halt\|resume\|reset\|gdb}` stubs | OWNER | C | ☐ | Extension point only per D-4 |
| JTAG-2 | OpenOCD backend: reset (SRST/TRST), flash-deploy, GDB `:3333` / Tcl `:6666` sockets | OWNER | C | ⤵ | Deferred |

---

## C. Adapter A — LAVA lab

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| LAVA-1 | Device-dictionary + device-type template generator (`paniolo lava device-dict`) | LAVA | M | ☐ | power_* → `paniolo power …`; connection → telnet |
| LAVA-2 | Generator supports list-valued power commands | LAVA | S | ☐ | Discrepancy #3 |
| LAVA-3 | "First device" onboarding doc (Debian worker, ser2net/TCP wiring, tokens) | LAVA | S | ☐ | Internet-reachable lab; tokens to KernelCI admins |
| LAVA-4 | Verified on a Debian LAVA worker against a real board | LAVA | S | ☐ | macOS unsupported (D-5) |

## D. Adapter B — Fuchsia / botanist

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| FX-1 | botanist device-config emitter (`paniolo botanist device-config`) → PTY path | FX | M | ☐ | `{network,keys,serial}`; serial = PTY (SER-2) |
| FX-2 | Bot-host/recipe **power wrapper** calling `paniolo power {on\|reset\|off}` | FX | M | ☐ | Power is NOT a device-config field (discrepancy #2) |
| FX-3 | `bot_config.py` `get_dimensions()` snippet advertising `device_type:<board>` | FX | S | ☐ | + `bots.cfg` entry, `platforms.gni` entry (upstream coord) |
| FX-4 | Verify `DeviceConfig`/power plumbing against a real Fuchsia checkout | FX | M | ☐ | pkg.go.dev hides unexported fields; confirm `device.go` |
| FX-5 | Acknowledge RFC-0130 Experimental tier (self-hosted CI) in docs | FX | C | ☐ | Community board is not "Supported" tier |

---

## E. Cross-cutting / non-functional

| ID | Requirement | Source | Pri | Status | Notes |
|---|---|---|---|---|---|
| NF-1 | Interactive/agent workflow (dashboard, OCR, HID, `tio`, JSONL) never regresses | OWNER | M | ☐ | Tests + manual check each milestone |
| NF-2 | Changes land as smallest reversible steps, each with tests | OWNER | M | ☐ | |
| NF-3 | Core power/serial path remains functional on both macOS and Linux | OWNER | M | ☐ | CI features Linux-only; interactive cross-platform |
| NF-4 | External contracts re-verified against upstream before relying on them | OWNER | M | ◐ | Verified May 2026; re-check FX `device.go` (FX-4) |

---

## F. Milestones

| Milestone | Contents | Status |
|---|---|---|
| M0 — Analysis & design | gap-analysis, design, this tracker, decisions | ☑ |
| M1 — Minimum-viable core | PWR-1..6, SER-1..7, DEP-1, JTAG-1 stubs | ☐ (awaiting go-ahead) |
| M2 — Adapters | LAVA-1..3, FX-1..4 | ☐ |
| M3 — Verify on hardware | LAVA-4, FX-3/FX-5, BOOT-1 | ☐ |
| M4 — Full (deferred) | DEP-3, JTAG-2 | ⤵ |

## G. Implementation questions (resolved 2026-05-29)

| ID | Question | Resolution |
|---|---|---|
| SER-Q1 | Native Rust TCP listener vs. external ser2net pointed at the PTY (SER-2)? | **Native listener** in serialcap; ser2net-on-PTY documented as LAVA fallback |
| SER-Q2 | Write-arbitration lock lifetime | Exclusive hold is **tied to the client connection — auto-releases on disconnect**; optional `--lock-timeout` safety net |
| PWR-Q1 | `[power]` block shape + migration of `power_cycle_cmd` | **Breaking change accepted** — clean `[power]` block, drop the flat `power_cycle_cmd`; update agent guidance (`AGENTS.md`) so the agent reconfigures targets accordingly |
