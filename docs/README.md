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
| [Power](power.md) | `paniolo power-cycle`, `power-state`, `serial dtr/reset` | DTR power-button wiring (J2) and script-based power cycling. |
| [Video](video.md) | `paniolo video` | `hdmicap` warm-stream HDMI capture + on-device OCR. |
| [Dashboard](dashboard.md) | `paniolo console` | Combined video + serial web UI. |
| [HID injection](hid.md) | `paniolo hid` | USB keyboard/mouse injection via the KB2040 rig. |

## Distributed control (Phases 0–3 shipped)

| Doc | What it covers |
|---|---|
| [Distributed control: one lab, one file](distributed-control.md) | Driving targets on remote control hosts: a single git-tracked lab file describing hosts + targets, SSH transport with the dev machine as the data-plane hub, per-resource host binding (multi-host-ready), and a discovery-proposes/human-approves config flow. Shipped: `--lab`, transparent re-exec, tunnelled `console`. |
| [Implementation plan](distributed-control-plan.md) | Phased build sequence — Phases 0–3 shipped (SSH transport, lab model, re-exec, console); Phases 4–5 (remote `setup`, discovery-assisted `configure`) and multi-host pending. |

## Rust control-plane rewrite (in progress)

The CLI + orchestration + device glue is being rewritten Python→Rust (the `cli/` crate),
finishing the migration the daemons started. The lab file becomes the single, CLI-managed
source of truth.

| Doc | What it covers |
|---|---|
| [Config redesign: a CLI-managed lab](config-redesign.md) | The lab data model (hosts/targets/per-channel hosts), the CRUD command surface, per-channel dispatch design, and the Python→Rust pivot + staged plan. |
| [CH9329 driver spec (clean-room)](ch9329-spec.md) | **Deferred** (Openterface HID backend, to revisit): WCH CH9329 serial protocol — frame format, GET_INFO, keyboard report, parameter-config/baud, reset, ACK codes. Reference for when the `hid` channel is reintroduced. |

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
- [`hidrig/README.md`](../hidrig/README.md) — HID rig wiring, firmware, and wire protocol.

---

*These docs describe paniolo's current state and are kept up to date as it changes. When you
change a subsystem, update its guide here and the [architecture overview](architecture.md); when
you change requirements/scope, update the [tracker](requirements.md).*
