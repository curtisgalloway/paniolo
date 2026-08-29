<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# notes/

Point-in-time documents: the design a feature was built from, a hardware
bring-up's findings, a plan that has since shipped.

**These are records, not documentation.** They describe what was true when they
were written and are not maintained against the current code. Nothing here is
published to the docs site. Documentation of paniolo's verified current state
lives in [`docs/`](../docs/README.md) — user guides at the top level, developer
documentation under [`docs/dev/`](../docs/dev).

If you are looking for how paniolo works *today*, start at
[`docs/README.md`](../docs/README.md).

| Note | What it records |
|---|---|
| [config-redesign.md](config-redesign.md) | The lab data model, CRUD command surface, and per-channel dispatch design, plus the Python→Rust pivot. Shipped. |
| [distributed-control-plan.md](distributed-control-plan.md) | The original Python-era phased build sequence for distributed control. Phases 0–5 shipped. |
| [uefi-http-boot-design.md](uefi-http-boot-design.md) | The design netbootd's UEFI PXE / HTTP Boot support was built from. Shipped behavior is in [docs/netboot.md](../docs/netboot.md). |
| [ch9329-spec.md](ch9329-spec.md) | Clean-room protocol spec for the WCH CH9329 HID bridge, written before the `ch9329` helper. Implemented. |
| [openterface-kvm-go.md](openterface-kvm-go.md) | Bench findings for the Openterface KVM-Go (2026-08-24): both paniolo channels work unmodified. |
| [openterface-deep-control.md](openterface-deep-control.md) | Deep-control findings for the Openterface Mini-KVM, with the parts that remain unverified or blocked. |
| [console-front-door.md](console-front-door.md) | A parked design: one stable port with server-side fan-out for the remote dashboard. Not built. |
| [pi4-control-host.md](pi4-control-host.md) | The original bring-up plan for a self-contained Pi control host. Superseded by the working image; see the `control-host-provisioning` branch. |
