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
| [Serial](serial.md) | `paniolo serial` | `serialcap` daemon (timestamped JSONL log + WebSocket terminal) and interactive `tio`. |
| [Power](power.md) | `paniolo power-cycle`, `power-state`, `serial dtr/reset` | DTR power-button wiring (J2) and script-based power cycling. |
| [Video](video.md) | `paniolo video` | `hdmicap` warm-stream HDMI capture + on-device OCR. |
| [Dashboard](dashboard.md) | `paniolo console` | Combined video + serial web UI. |
| [HID injection](hid.md) | `paniolo hid` | USB keyboard/mouse injection via the KB2040 rig. |

## Hardware-CI integration (in design)

Making paniolo's primitives consumable by hardware-CI orchestrators, without paniolo owning test
orchestration or results.

| Doc | What it covers |
|---|---|
| [Gap analysis](ci-integration/gap-analysis.md) | Per-primitive (power/serial/deploy/boot) × per-ecosystem (KernelCI/LAVA, Fuchsia/botanist) deltas, with the verified contract corrections. |
| [Integration design](ci-integration/design.md) | The ecosystem-agnostic device-control API + LAVA and botanist adapters; minimum-viable vs full paths; verdict. |

## For contributors / agents

- [`AGENTS.md`](../AGENTS.md) — module-by-module internals, source constraints, and how to add a subsystem.
- [`hidrig/README.md`](../hidrig/README.md) — HID rig wiring, firmware, and wire protocol.

---

*These docs describe paniolo's current state and are kept up to date as it changes. When you
change a subsystem, update its guide here and the [architecture overview](architecture.md); when
you change requirements/scope, update the [tracker](requirements.md).*
