# Paniolo documentation

Paniolo is an **agent-controlled target-machine wrangler** for low-level software development —
it gives an AI agent (or you) the controls to netboot a target, watch its output, send it input,
and power-cycle it without a person at the bench each iteration. See the root
[`README.md`](../README.md) for install and the quick remote-control pattern.

## Start here

| Doc | What it covers |
|---|---|
| [**Architecture**](architecture.md) | The whole system in its current state: deployment model, the CLI + per-subsystem daemons, config/state model, data flows, host-OS differences. **Read this first.** |
| [Requirements & progress](requirements.md) | Project-wide requirements tracker (shipped capabilities + planned work + decisions), with status per item. |

## Subsystem guides

| Guide | Commands | Summary |
|---|---|---|
| [Netboot](netboot.md) | `paniolo netboot` | Pure-Python DHCP + TFTP over a direct USB-Ethernet link. |
| [Link mode](netif.md) | `paniolo netif` | Atomically switch the link between netboot and ffx-over-IPv6 modes (stops netboot, sets up the host `fe80::1`). |
| [Serial](serial.md) | `paniolo serial` | `serialcap` daemon (timestamped JSONL log + WebSocket terminal) and interactive `tio`. |
| [Power](power.md) | `paniolo power on/off`, `power-cycle`, `power-state`, `serial dtr/reset` | DTR power-button wiring (J2) and generic shell-command hooks; `cambrionix` hub + `zigplug` Zigbee smart-plug helpers. |
| [Video](video.md) | `paniolo video` | `hdmicap` warm-stream HDMI capture + on-device OCR. |
| [Dashboard](dashboard.md) | `paniolo console` | Combined video + serial web UI. |
| [HID injection](hid.md) | `paniolo hid` | USB keyboard/mouse injection via a generic helper hook; `hidrig` KB2040 injector; KVM input from the web console. |
| [HID serial protocol](hid-serial-protocol.md) | — | Normative device-independent spec for HID injectors (v1); implement it on any microcontroller. |

## Distributed control (Phases 0–3 shipped)

| Doc | What it covers |
|---|---|
| [Distributed control: one lab, one file](distributed-control.md) | Driving targets on remote control hosts: a single git-tracked lab file describing hosts + targets, SSH transport with the dev machine as the data-plane hub, per-resource host binding (multi-host-ready), and a discovery-proposes/human-approves config flow. Shipped: `--lab`, transparent re-exec, tunnelled `console`. |
| [Implementation plan](distributed-control-plan.md) | Phased build sequence — Phases 0–3 shipped (SSH transport, lab model, re-exec, console); Phases 4–5 (remote `setup`, discovery-assisted `configure`) and multi-host pending. |

## Rust control-plane rewrite (at command parity)

The CLI + orchestration + device glue is rewritten Python→Rust (the `cli/` crate),
finishing the migration the daemons started. The lab file is the single, CLI-managed
source of truth. All commands are ported and rig-verified locally; remote-host
dispatch and the Python-tree retirement are the remaining steps.

| Doc | What it covers |
|---|---|
| [Config redesign: a CLI-managed lab](config-redesign.md) | The lab data model (hosts/targets/per-channel hosts), the CRUD command surface, per-channel dispatch design, and the Python→Rust pivot + staged plan. |
| [CH9329 driver spec (clean-room)](ch9329-spec.md) | **Deferred** (Openterface HID backend, to revisit): WCH CH9329 serial protocol — frame format, GET_INFO, keyboard report, parameter-config/baud, reset, ACK codes. A CH9329 shim speaking the [HID serial protocol](hid-serial-protocol.md) would plug into the same `hid` channel. |

## Hardware-CI integration (in design)

Making paniolo's primitives consumable by hardware-CI orchestrators, without paniolo owning test
orchestration or results.

| Doc | What it covers |
|---|---|
| [Gap analysis](ci-integration/gap-analysis.md) | Per-primitive (power/serial/deploy/boot) × per-ecosystem (KernelCI/LAVA, Fuchsia/botanist) deltas, with the verified contract corrections. |
| [Integration design](ci-integration/design.md) | The ecosystem-agnostic device-control API + LAVA and botanist adapters; minimum-viable vs full paths; verdict. |
| [Related work: paniolo vs. labgrid](ci-integration/related-work.md) | How paniolo compares to the closest existing tool (labgrid) and to Redfish, and why paniolo exists alongside them. |
| [Redfish provider (design sketch)](ci-integration/redfish-provider.md) | Exposing a Redfish API in front of BMC-less boards so Ironic/Metal3/LAVA can drive a paniolo target as a managed server. |

## For contributors / agents

- [`AGENTS.md`](../AGENTS.md) — module-by-module internals, source constraints, and how to add a subsystem.
- [Adding a power-control helper](adding-power-helpers.md) — recipe for supporting new power-switching hardware: the hook contract, helper CLI conventions, implementation skeletons (Rust/Python), verification ladder, and PR checklist.
- [`hidrig/README.md`](../hidrig/README.md) — HID injector wiring, firmware, and host CLI.

---

*These docs describe paniolo's current state and are kept up to date as it changes. When you
change a subsystem, update its guide here and the [architecture overview](architecture.md); when
you change requirements/scope, update the [tracker](requirements.md).*
