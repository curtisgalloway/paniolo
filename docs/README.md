# Paniolo documentation

Paniolo is an **agent-controlled target-machine wrangler** for low-level software development —
it gives an AI agent (or you) the controls to netboot a target, watch its output, send it input,
and power-cycle it without a person at the bench each iteration. See the root
[`README.md`](https://github.com/curtisgalloway/paniolo/blob/main/README.md) for install and the quick remote-control pattern —
on Debian/Raspberry Pi OS the quickest route is the [apt repository](https://curtisgalloway.github.io/paniolo/apt/) served from this site.

## Start here

| Doc | What it covers |
|---|---|
| [Tested hardware](hardware.md) | The bench hardware each subsystem is verified with, by category, with purchase links. |
| [Demos](demos.md) | Recorded runs from the CI rack — a NUC's BIOS puppeted through the KVM, a live desktop driven by emulated HID, out-of-band power over Intel AMT, a Pi cold-booted through a relay — each capture beside the transcript of what the agent typed. |

## Subsystem guides

| Guide | Commands | Summary |
|---|---|---|
| [Netboot](netboot.md) | `paniolo netboot` | DHCP + TFTP + HTTP over a direct USB-Ethernet link (single-binary Rust `netbootd`), incl. UEFI PXE / HTTP Boot. |
| [Link mode](netif.md) | `paniolo netif` | Atomically switch the link between `netboot`, bare-`link`, `ffx`-over-IPv6, and `off` modes (entering ffx stops netboot and sets up the host `fe80::1`); `down-hard` forces a real carrier drop. |
| [Serial](serial.md) | `paniolo serial` | `serialcap` daemon (timestamped JSONL log + WebSocket terminal) and interactive `tio`. |
| [Power](power.md) | `paniolo power on/off`, `power-cycle`, `power-state`, `serial dtr/reset` | DTR power-button wiring (J2; opt-in per serial interface) and generic shell-command hooks; `cambrionix` hub, `zigplug` Zigbee smart-plug, `shellyplug` Shelly Gen2+ plug/relay (local HTTP RPC), and `amt` Intel AMT/vPro (WS-Man, with true power-state readback) helpers. |
| [Video](video.md) | `paniolo video` | `hdmicap` warm-stream HDMI capture + on-device OCR. |
| [Dashboard](dashboard.md) | `paniolo console` | Combined video + serial web UI. |
| [HID injection](hid.md) | `paniolo hid` | USB keyboard/mouse injection via a generic helper hook; `hidrig` KB2040 injector and `ch9329` CH9329 bridge (Openterface Mini-KVM / KVM-Go, Sipeed NanoKVM-USB); KVM input from the web console. |
| [adb (Android targets)](adb.md) | `paniolo adb` | Drive an Android DUT over adb — console (`adb shell`/`run`), screen (`screencap`), and input (`adb input`); one transport, no capture/HID/serial rig. |

## Distributed control (Phases 0–5 shipped)

| Doc | What it covers |
|---|---|
| [Distributed control: one lab, one file](distributed-control.md) | Driving targets on remote control hosts: a single git-tracked lab file describing hosts + targets, SSH transport with the dev machine as the data-plane hub, per-channel host binding, and a discovery-proposes/human-approves config flow. Shipped: `--lab`, transparent re-exec, tunnelled `console`, remote `setup --host`, `discover`/`configure`. |

## Developer documentation

How paniolo is built and how to extend it — published alongside the user docs,
under [`docs/dev/`](https://github.com/curtisgalloway/paniolo/tree/main/docs/dev).

| Doc | What it covers |
|---|---|
| [**Architecture**](dev/architecture.md) | The whole system in its current state: deployment model, the CLI + per-subsystem daemons, config/state model, data flows, host-OS differences. **Read this first.** |
| [Requirements & progress](dev/requirements.md) | Project-wide requirements tracker (shipped capabilities + planned work + decisions), with status per item. |

### Interfaces

| Doc | What it covers |
|---|---|
| [HID serial protocol](dev/hid-serial-protocol.md) | Normative command vocabulary (v1) — the external interface `hidrig` composes from; the dual-board device wire is in [hid-dual-board-design.md](dev/hid-dual-board-design.md). |
| [OCR helper protocol](dev/ocr.md) | The contract an OCR helper implements, and which engine runs on which platform (Apple Vision on macOS, `Windows.Media.Ocr` on Windows, Tesseract on Linux). |
| [HID dual-board design](dev/hid-dual-board-design.md) | The "dumb pipe" KB2040 rig: I2C1 wire format between the control and target boards, and why the host composes the reports. |

### Extending paniolo

| Doc | What it covers |
|---|---|
| [Adding a power-control helper](dev/adding-power-helpers.md) | Recipe for supporting new power-switching hardware: the hook contract, helper CLI conventions, implementation skeletons (Rust/Python), verification ladder, and PR checklist. |
| [Agent discoverability & usage evals](dev/agent-evals.md) | A no-hardware eval suite measuring how well a naive agent goes from a plain-language goal to the right paniolo command via the self-describing surface (`--help` → `paniolo skill` → docs). |
| [Serial agent benchmark](dev/serial-agent-benchmark.md) | A hardware-in-the-loop, head-to-head eval: does paniolo produce better serial-task outcomes than improvising ("YOLO") or the idiomatic `fx serial`? |

### Hardware-CI integration (in design)

Making paniolo's primitives consumable by hardware-CI orchestrators, without paniolo owning test
orchestration or results.

| Doc | What it covers |
|---|---|
| [Gap analysis](dev/ci-integration/gap-analysis.md) | Per-primitive (power/serial/deploy/boot) × per-ecosystem (KernelCI/LAVA, Fuchsia/botanist) deltas, with the verified contract corrections. |
| [Integration design](dev/ci-integration/design.md) | The ecosystem-agnostic device-control API + LAVA and botanist adapters; minimum-viable vs full paths; verdict. |
| [Related work: paniolo vs. labgrid](dev/ci-integration/related-work.md) | How paniolo compares to the closest existing tool (labgrid) and to Redfish, and why paniolo exists alongside them. |
| [Redfish provider (design sketch)](dev/ci-integration/redfish-provider.md) | Exposing a Redfish API in front of BMC-less boards so Ironic/Metal3/LAVA can drive a paniolo target as a managed server. |

## Notes: design records & bring-up findings

Point-in-time documents — the design a feature was built from, a hardware
bring-up's findings, a plan that has since shipped. They record *what was true
when they were written*, so they are deliberately **not** documentation of the
current state and are never published to this site. They live in
[`notes/`](https://github.com/curtisgalloway/paniolo/tree/main/notes), outside
the docs tree.

| Doc | What it covers |
|---|---|
| [Config redesign: a CLI-managed lab](https://github.com/curtisgalloway/paniolo/blob/main/notes/config-redesign.md) | The lab data model (hosts/targets/per-channel hosts), the CRUD command surface, per-channel dispatch design, and the Python→Rust pivot + staged plan. The CLI + orchestration is rewritten Python→Rust (the `cli/` crate); the lab file is the single, CLI-managed source of truth. |
| [CH9329 driver spec (clean-room)](https://github.com/curtisgalloway/paniolo/blob/main/notes/ch9329-spec.md) | **Implemented** (the [`ch9329`](https://github.com/curtisgalloway/paniolo/blob/main/ch9329/README.md) helper crate): WCH CH9329 serial protocol — frame format, GET_INFO, keyboard report, parameter-config/baud, reset, ACK codes. The helper speaks the [HID serial protocol](dev/hid-serial-protocol.md) surface and plugs into the same `hid` channel; the spec remains the clean-room reference it was built from. |
| [Openterface deep control — findings & testing TODO](https://github.com/curtisgalloway/paniolo/blob/main/notes/openterface-deep-control.md) | **Partially verified (tracker §6.1 OTF-1)**: DTR-driven unplug/replug of the A-port device is confirmed on hardware (asserted DTR = disconnected; opening the control tty replugs it) but is not production-ready — degraded link speed and hub-level instability. The USB-A switch turns out to be *software-monitored*, not switch-driven, and the MS2109 GPIO mux is blocked because ms-tools cannot patch this firmware. EEPROM dumped; the `????????` serial is a RAM descriptor, not an EEPROM field. RTS→CH9329 reset still untested. |
| [Distributed-control implementation plan](https://github.com/curtisgalloway/paniolo/blob/main/notes/distributed-control-plan.md) | The original (Python-era) phased build sequence for [distributed control](distributed-control.md) — Phases 0–5 shipped; superseded by the Rust control plane for mechanism details. |
| [UEFI HTTP Boot design](https://github.com/curtisgalloway/paniolo/blob/main/notes/uefi-http-boot-design.md) | The design netbootd's UEFI PXE / HTTP Boot support was built from (vendor-class dispatch, HTTP serving); the shipped behavior is documented in [netboot.md](netboot.md). |
| [Openterface KVM-Go — architecture and paniolo support](https://github.com/curtisgalloway/paniolo/blob/main/notes/openterface-kvm-go.md) | **Bench-verified 2026-08-24**: the keychain-sized Mini-KVM successor (MS2130S capture + CH32V208 emulating the CH9329 protocol) — both paniolo channels work unmodified; hardware findings and deep-control notes. |
| [Console front door](https://github.com/curtisgalloway/paniolo/blob/main/notes/console-front-door.md) | **Design only — parked**: one stable port with server-side fan-out for the remote dashboard, superseding `?serialws=` stitching. |
| [Pi 4 control host](https://github.com/curtisgalloway/paniolo/blob/main/notes/pi4-control-host.md) | Bring-up plan for a self-contained Pi 4 control host; everything works on Linux/ARM64 today except the net-new USB-HID-gadget backend (design sketch, not implemented). |

## Elsewhere in the repo

- **Bundled agent skills** — paniolo ships agent guides under [`skills/`](https://github.com/curtisgalloway/paniolo/tree/main/skills) (`paniolo` for driving a target, `kvm-puppeting` for GUI puppeting). They install alongside the CLI; `paniolo skill` lists them (with descriptions) and `paniolo skill <name>` prints one's `SKILL.md` — so an agent can discover and read them straight from the CLI, without the harness pre-loading them.
- [`AGENTS.md`](https://github.com/curtisgalloway/paniolo/blob/main/AGENTS.md) — module-by-module internals, source constraints, and how to add a subsystem.
- [`hidrig/README.md`](https://github.com/curtisgalloway/paniolo/blob/main/hidrig/README.md) — HID injector wiring, firmware, and host CLI.

---

*These docs describe paniolo's **current, verified state** and are kept up to date as it changes.
When you change a subsystem, update its guide here and the
[architecture overview](dev/architecture.md); when you change requirements/scope, update the
[tracker](dev/requirements.md). Work in progress, plans, and point-in-time findings belong in
[`notes/`](https://github.com/curtisgalloway/paniolo/tree/main/notes), not here.*
