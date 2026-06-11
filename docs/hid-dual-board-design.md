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

# HID rig — dual-board "dumb pipe" design

> **Status:** proposed / converged in design, not yet built.
> **Branch:** `i2c-kb2040-dual-board`. **Date:** 2026-06-09.
> This captures the architecture we converged on for the two-board KB2040 rig so the
> thinking crosses cleanly into implementation. It supersedes the role-based ASCII
> firmware currently on the branch (`hidrig/firmware/code.py`). The **external** paniolo
> HID interface — the `hidrig` CLI and [`hid-serial-protocol.md`](hid-serial-protocol.md)
> v1 — is unchanged; only the daemon↔firmware wire format changes.

---

## 1. Motivation & history

The rig has been two-board before. Per `hidrig/README.md`'s History note, the **first**
version was a USB-CDC control board relaying a **binary I2C protocol** to a USB-HID target
board, and it was collapsed to a single board over UART specifically to eliminate "the
second board, the binary I2C protocol, and **the duplicated opcode tables**." The opcode
duplication — both boards having to understand and act on the command set — is what made
two boards painful.

We are going back to two boards because the control link wants to be the board's own
native USB (a clean CDC endpoint to the control host, no USB-serial dongle) while the
target link stays clean device-mode HID. The KB2040 has one USB port, so you cannot have
both on one board.

**The "dumb pipe" decision is what keeps this from repeating the old trap.** Instead of
both boards parsing an opcode table, *no* board interprets HID semantics:

- the **host** composes the actual HID report bytes (keycodes, US layout, abs/rel mouse
  math) — the composition lives in exactly one place;
- the **control** board routes frames by a one-byte type tag and forwards HID frames
  downstream verbatim;
- the **target** board relays report bytes straight to `send_report` and never inspects
  them.

There is still a binary protocol on the wire, but there are no duplicated opcode tables,
because the only thing that understands the events is the host daemon.

## 2. Topology & power domains

```
[Control host]
      |
      |  USB (CDC) — binary frames           <-- powered by control host
      v
[Control KB2040]  ........ owns: routing, DUT power, (later) target reset
      |
      |  inter-board link (I2C *or* UART) — OPEN DECISION (§8)
      v
[Target KB2040]  --- native USB (device-mode HID) ---> [Target machine / DUT]
                                                         ^
   powered by the DUT's USB  <-----------------------------
```

Power domains matter and are asymmetric:

- **Control board** is powered by the **control host's** USB. It stays alive across a DUT
  power cycle — which is exactly why it is the right place to *own* DUT power and recovery.
- **Target board** is powered by the **DUT's** USB (confirmed). It boots and dies with the
  DUT. Consequence: the inter-board link spans a powered side (control) and an unpowered
  side (target) whenever the DUT is off — see the back-powering caution in §6.

## 3. The dumb-pipe decision (what moves where)

| Concern | Before (role-based ASCII firmware) | After (dumb pipe) |
|---|---|---|
| Keycodes / US layout / `type "string"` | firmware (`adafruit_hid`) | **host daemon (Rust)** |
| Abs/rel mouse math, button state | firmware | **host daemon** |
| Command parsing | both boards | **host daemon** composes frames; boards don't parse |
| Target board's job | parse line → adafruit_hid calls | **relay report bytes → `send_report`** |
| Control board's job | forward every line verbatim | **route by type byte**: relay HID, handle control |
| HID report descriptor | firmware (`boot.py`) | firmware (`boot.py`) — now the host↔rig **contract** |

The target board no longer depends on `adafruit_hid` at all; it needs only the core
`usb_hid` device (the descriptor) and `send_report`.

## 4. Responsibilities

**Host daemon (Rust, `hidrig serve`).** Gains the composition layer: translate the existing
v1 ASCII commands (`type`, `key`, `move`, `moveabs`, `click`, …) into HID report bytes and
binary frames. Everything *above* the daemon is unchanged — `hidrig` one-shots, `paniolo
hid send`, the web console — so this is internal to the daemon and the firmware.

**Control board.** Presents a USB CDC interface to the control host. Reads framed input,
switches on the type byte: HID-report frames are relayed downstream over the inter-board
link with no interpretation; control frames (power, version, later reset) are handled
locally and answered. Independently powered, so it is the rig's supervisor.

**Target board.** Device-mode HID to the DUT. Reads frames from the inter-board link and
calls `send_report(payload, report_id)` — nothing else. Holds the HID descriptor that
defines what reports are legal; the host must compose to match it exactly.

**Inter-board link.** Carries the same framed bytes. Transport is an open decision (§8).

## 5. Wire contract

### Frame format (host → control → target)

```
[type][report-id][len][payload .. len bytes]      type 0x01 = HID report
[type][cmd][args ..]                              type 0x02 = control command
```

- **HID report frame** (`0x01`): `report-id` selects the report (1 = keyboard, 2 = absolute
  mouse, matching `boot.py`); `len` + `payload` are the raw report bytes. The relay reads
  `report-id` and `len`, then calls `send_report(payload, report_id)` and never interprets
  `payload`. Carrying an explicit `len` (rather than deriving it from `report-id`) keeps the
  relay descriptor-agnostic and lets the descriptor grow without firmware changes.
- **Control frame** (`0x02`): `cmd` ∈ { power on/off, version, ping, … (reset later) }.
  These are rare and fallible, so they keep a **synchronous request → reply**.

### The descriptor is a shared constant

Because the host composes reports, the host composer must match the target board's HID
descriptor **exactly** (report IDs, field order, the 0..32767 absolute range). The
descriptor lives in `hidrig/firmware/boot.py`. Treat it as a versioned contract: the
`version` control command should let the daemon confirm it is talking to a compatible
descriptor before composing.

### Flow control

- **HID frames are fire-and-forget** — no per-frame ack. The host paces them to the
  downstream HID poll interval (`bInterval`); it must not exceed the rate the target board
  can drain into the DUT, or a bounded buffer backs up. This is what removes the I2C
  reply-synchronization problem from the hot path.
- **Control frames keep request/reply.** Rare, fallible, and not latency-critical, so the
  awkward "poll a buffer, strip `0xFF` fill" dance — if I2C is chosen — only ever runs here,
  never on streaming mouse motion.

## 6. Power & recovery

**DUT power-cycle (the immediate goal).** The control board switches power to the DUT. This
overlaps with paniolo's existing power helpers (zigplug/shellyplug) by design — it is a
*backend swap*, not a new core concept. It surfaces to paniolo as **another power-helper
behind the existing `power` hook**, keeping device-specific logic out of the core. It is
worth doing where it *replaces something flaky or consolidates cabling* — e.g. the pi5
bench currently runs on a Zigbee plug that is finicky (formation fails near USB video,
NVRAM-recovery drama); a load-switch on the pi5's 5 V that the control board owns is a
reliability upgrade and collapses the rig toward one cable.

- **Mechanism — OPEN (§8):** hard cut (inline relay / high-side load switch) vs soft (pulse
  the ATX front-panel header). Soft won't rescue a wedged box.
- **Sizing caveat:** a Pi 5 can pull ~5 A at 5 V, so the switching element is a real power
  load-switch / relay, **not** a bare GPIO.

**Target-board reset (DEFERRED, §9).** A control-board GPIO → target board `RUN` pin would
re-enumerate the HID board *without* rebooting the DUT, and works out-of-band even when that
board is wedged and ignoring the link. Deferred, but noted as useful.

**Recovery hierarchy.** Host owns/recovers the control board; the (independently powered)
control board recovers everything below it — DUT power-cycle for a hung machine, and (later)
target-board reset for a wedged HID link. Two distinct tools for two independent failures.

**Cross-power-domain caution.** Because the target board is DUT-powered, any line the
control board drives toward it (the inter-board link today; the `RUN` reset line later) sits
between a powered and an unpowered chip while the DUT is off. A GPIO driven high into an
unpowered RP2040 can leak through its protection diodes and back-power it. Design for this:
drive control→target lines low / tristate them while the DUT is off, and reference the I2C
pull-ups deliberately (control side).

## 7. What's settled

1. **Dumb pipe.** Composition on the host; firmware relays raw report bytes. No duplicated
   opcode tables — this is the answer to why the old two-board design was abandoned.
2. **Two boards, native USB both ends.** Control board vends CDC to the host; target board
   vends device-mode HID to the DUT.
3. **External interface unchanged.** paniolo and `hidrig` keep speaking v1 ASCII to the
   daemon; only the daemon↔rig wire format becomes binary frames.
4. **DUT power is owned by the control board**, integrated as a power-helper backend behind
   the existing hook.
5. **Blast radius is daemon + firmware only** (see §10).

## 8. Open decisions

1. **Inter-board transport — I2C vs UART.** Fire-and-forget makes both workable. UART is
   simpler (native newline/length framing, full-duplex, no reply-poll, no `0xFF` stripping)
   and is the lower-risk default for a point-to-point link. I2C only earns its complexity if
   the rig fans out to **multiple target boards** (addressing). I2C is what's physically
   wired today (A0/A1, addr `0x41`), so "no rewiring" is its only short-term edge.
   *Recommendation: UART unless multidrop is on the roadmap.*
2. **Target-board firmware — CircuitPython vs Embassy.** The dumb-pipe change removes the
   `adafruit_hid` dependency, so the *only* remaining reason to choose Embassy on this board
   is `bInterval` headroom (sub-8 ms HID poll → above ~125 reports/s). **Verify first**
   whether CircuitPython even lets you change `bInterval` (likely not — that's the source of
   the 8 ms floor) and whether the DUT honors `bInterval = 1`, before porting anything.
   *Decision hinges on: is ~125 reports/s enough for the KVM feel?*
3. **Host-side CDC topology — one stream vs two endpoints.** One CDC stream namespaced by
   the frame type byte (simpler; needs a single host owner) vs two CDC endpoints
   (`console` + `data`) — one for HID frames, one for control. **Two endpoints lets DUT
   power be a separate host-side helper** that doesn't contend with the HID daemon for the
   port, which fits paniolo's power-helper model. *Leaning two-endpoint.*
4. **DUT power mechanism — hard cut vs soft button** (see §6).

## 9. Non-goals / deferred

- **Target-board reset line** (control GPIO → `RUN`). Deferred; design the cross-domain
  back-powering protection (§6) when it's added.
- **Multidrop / multiple target boards.** Only if fan-out is needed; would tip the transport
  decision to I2C.
- **Custom vendor USB bulk endpoint.** Not needed and not pursued. CircuitPython's
  `usb_cdc.data` is *already* a bulk endpoint pair — raw binary frames ride it fine; the
  ASCII convention was never a constraint. A true vendor/driverless endpoint would require
  rebuilding CircuitPython in C or moving to Embassy, which the throughput case does not
  justify (the ceiling is the downstream `bInterval`, not the host-side link).

## 10. Blast radius

```
unchanged: paniolo `hid send` · `hidrig` CLI · web console · hid-serial-protocol.md (v1 ASCII)
           |
           v
  CHANGED: host daemon  -- gains composition (keycodes/layout/mouse math) + binary framing
           |
           |  binary frames over CDC  (NEW wire format)
           v
  CHANGED: control firmware  -- routes by type byte (was: verbatim forward)
           |
           |  binary frames over I2C/UART
           v
  CHANGED: target firmware   -- relay -> send_report (was: adafruit_hid command interpreter)
```

Nothing above the daemon moves. The daemon absorbs the protocol translation, so the change
is contained to two layers.
