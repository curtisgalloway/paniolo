<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# VM targets, and RFB as a second transport for video and HID

> **Status: design only — nothing here is built beyond the pty console support
> that shipped in v0.1.16.** Captures a discussion converged on 2026-08-30.
> Answers the deferred question in [console-front-door.md](console-front-door.md)'s
> neighbourhood — specifically the "should the dashboard just be VNC" idea raised
> 2026-08-18 and parked for a separate session.

## What prompted it

A parallel effort (`~/src/windows-vm`) was automating a Windows 11 ARM64 guest
under UTM on Apple Silicon, which raised the question of whether paniolo should
treat a VM as a target at all, or whether that is redundant with what a
hypervisor already gives you. The question was then re-asked for a Windows guest
under **Proxmox**, which turns out to change the answer materially.

One concrete thing shipped out of it: serialcap could not open a pseudo-terminal
on macOS at all, so a VM's console was unreachable. Fixed in v0.1.16 — see
[docs/serial.md](../docs/serial.md) § *Virtual machine consoles*. Everything else
below is undecided.

## The redundancy argument, and where it fails

The first instinct is that VMs are redundant: every paniolo channel exists
because the physical world denies you direct access, and a hypervisor denies you
none of it. Netboot is the clearest case — netbootd exists to reach a boot medium
you cannot touch, and a VM's boot medium is a file you can edit. OCR'ing a
framebuffer you could read losslessly is the sharpest case.

That argument is strong for netboot and weak everywhere else, and it collapses
entirely once the hypervisor is Proxmox rather than UTM. Two objections did most
of the work and neither survives:

- **"It's a local GUI app you'd be puppeting."** True of UTM, which is a sandboxed
  application on a laptop. A Proxmox node is a Debian box you SSH into with guests
  attached — which is *exactly* paniolo's existing control-host model. `[hosts.pve1]
  ssh = "root@pve1"`, channels bound to it, commands re-exec'd by the dispatch that
  already exists. No new architecture.
- **"Video and HID are dead ends."** True of UTM: no screenshot API, and
  `input keystroke` / `input scan code` deliver nothing to a guest with no display
  client attached (verified by the windows-vm session against a UEFI Shell prompt,
  not inferred from the scripting dictionary). Under Proxmox every guest has a VNC
  console, and RFB is a real protocol with real client libraries.

## The reframing: this is not a VM feature

The decision that actually matters is **not** "should paniolo support VMs." It is
"can video and HID come from RFB as well as from capture hardware."

Framed as VM support it justifies one narrow feature. Framed as a transport it
justifies the same code against a NanoKVM, an iDRAC or iLO virtual console, and
any IPMI KVM — all of which sit in front of *physical* machines, some already on
the CI rack. It also composes with the Redfish provider sketched in
[docs/dev/ci-integration/](../docs/dev/ci-integration/redfish-provider.md).

### The two directions share a codec

| Direction | Shape | Serves |
|---|---|---|
| **Server** | HDMI capture + HID rig → RFB out | Physical targets; replaces the dashboard's video pane with any VNC viewer |
| **Client** | RFB in → the `video` and `hid` channels | VMs, BMC virtual consoles, remote KVMs |

Same protocol, same encodings, one codec. **Build the server first** — it is the
one wanted for physical targets regardless — and the client is mostly wiring
afterwards.

## Cost, decomposed

Not uniform across channels, and the cheap parts are cheaper than expected:

- **Power — zero code.** `on_cmd = "qm start 101"` with the channel bound to
  `pve1`, and `qm status` maps onto the `on`/`off` contract in
  [docs/power.md](../docs/power.md).
- **Serial — zero code, as of v0.1.16.** Proxmox exposes `serial0` as a unix
  socket on the node; `socat UNIX-CONNECT:… PTY,link=…` materialises a pty, and
  serialcap can now open one.
- **HID — a helper, no core change.** The `hid` channel is already an opaque
  helper command (`HidChannel { cmd }`), the same generic hook `hidrig` and
  `ch9329` plug into. An RFB helper slots in exactly where `ch9329` does and obeys
  the no-device-specific-code-in-core rule by construction.
- **Video — the only real cost.** `hdmicap` owns a USB capture device and has no
  notion of a network framebuffer source. RFB negotiation, Proxmox's ticket-based
  VNC auth, and enough encodings to decode a framebuffer worth OCR'ing.

## Why the server direction is worth doing on its own

Not primarily to stop maintaining a bespoke web console. `hdmicap` serves `GET /`
as an MJPEG video pane with an xterm.js serial pane below, and MJPEG ships whole
frames on every refresh. RFB sends dirty rectangles. For the screens paniolo
actually looks at — a BIOS menu, a boot log, a mostly-static console — that is a
large difference, and there is an existing latency complaint against the MJPEG
path with a deferred PR behind it. Exporting RFB is a plausible **fix for an open
problem**, not just a tidier UI.

### The tradeoff to decide deliberately

**RFB cannot carry the serial pane.** It carries a framebuffer and input events;
there is no second channel for a text stream. The dashboard's real value is video
and serial in one view on a shared timeline, and "just export VNC" quietly trades
that for two windows. Three options, to be chosen before a codec is written:

1. Keep a thin web page for serial only.
2. Render serial into the framebuffer — worse than a real terminal, breaks
   copy/paste.
3. Accept the split, on the grounds that a VNC viewer beside `paniolo serial watch`
   is what you would have open anyway.

## What actually motivates VM support

The best argument is not any capability comparison. It is that **the lab file is
the single answer to "what can I drive,"** and a machine absent from it costs a
second mental inventory forever. "This VM is another thing in our paniolo lab" is
the whole point.

Note what that does *not* require. It is satisfied by a lab-file entry using
existing channel kinds — power, serial, and eventually video and hid, pointed at
different backends. The uniformity lives in the config and the naming, which is
where paniolo already put it.

**Decision: no `vm` channel kind.** It would add model surface without adding any
of the uniformity that motivates it. The `adb` precedent is instructive — a target
class whose physical channels collapse into a software protocol got its own
`ChannelKind` and its own command tree (`paniolo adb screencap`, not
`paniolo video read`), and did *not* deliver cross-target workflow portability.

### The one exception

**Snapshot and rollback** is the only VM capability with no physical analogue —
lab-nuc-1 cannot be reverted. For agent-driven bring-up that is genuinely
valuable: let an agent do something destructive and undo it. If that is ever
wanted it earns VM-specific verbs on its own merits, independent of everything
above.

## The objection that survives

**The fidelity trap, which gets worse rather than better.** A VNC framebuffer is
synthetic, perfectly rendered text. The HDMI path has scaling artifacts and
scanline noise — which is why `AGENTS.md` documents `1`↔`l`↔`I` and `2`↔`Z`
confusions and why `evals/ocr` measures PP-OCRv6 against Tesseract at all.
Likewise RFB pointer events have none of the 8 ms `moveabs` floor or the ~123/s
bInterval ceiling the physical HID rig imposes.

An agent workflow developed against a VM would be tuned on a world where the hard
parts are not hard, and would be brittle the first time it met a real capture
card. The risk is proportional to how faithfully the emulation succeeds.

## Where this landed

- **Do now, no code:** wire up a Proxmox guest with power and serial, and use it.
  Cheap targets against the CI rack's physical scarcity of one NUC and one Pi.
- **Do not build:** a `vm` channel kind.
- **Open, pending evidence:** the RFB work. Let the power+serial VM tell you
  whether video actually bites. If agents keep losing time to screen-blindness,
  scope it as a **VNC video/hid backend**, not as VM support, and build the server
  half first.

The concrete evidence for the video half so far is one incident, in another repo:
the windows-vm session spent hours inferring which screen a stalled guest was on
from CPU time and qcow2 file size, and reached the wrong conclusion twice — first
"DiskConfiguration is broken", then a suspicion that WinPE could not see a QEMU
NVMe disk. The actual cause was a missing `<ProductKey>` parking Setup on a screen
that appears *before* disk selection. Being unable to see the screen is what made
two wrong answers indistinguishable from the right one. That is a real cost with a
number attached, but it is one incident and not on paniolo hardware.

## Verification status

Verified in this session: serialcap's pty behaviour, `tio` against a live UTM
guest console, `utmctl`'s command surface and status enum. Established on hardware
by the windows-vm session: UTM's inert HID injection, the serial console being a
firmware-level *input* device, the absence of a UTM screenshot API, and the
`shared` vs `emulated` netdev distinction (only the latter implements hostfwd).

**Not verified:** everything about Proxmox. The `qm` verbs and per-guest VNC
console are from general knowledge, not from a node that was touched. The
vncproxy ticket handshake in particular should be checked against a real node
before anyone costs out the video work.
