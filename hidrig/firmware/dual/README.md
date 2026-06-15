<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
-->

# Dual-board KB2040 rig — firmware

Two KB2040s, the "dumb pipe" design in
[`docs/hid-dual-board-design.md`](../../../docs/hid-dual-board-design.md). The
**control** board faces the control host (CDC); the **target** board faces the
DUT (USB-HID). They are joined by **I2C1** — `D10 = GP10 = SDA`,
`MOSI = GP19 = SCL` (those are the KB2040 pin labels) — with the target at
address **0x41**.

```
control/      -> firmware for the board on the CONTROL HOST's USB (I2C controller)
target/       -> firmware for the board on the DUT's USB          (I2C peripheral, HID)
host_send.py  -> host-side test driver (push frames to the control board, M2)
```

## Wiring (proto board)

| control board | target board | note |
|---|---|---|
| D10 / GP10 (SDA) | D10 / GP10 (SDA) | **straight, not crossed** (I2C is a bus) |
| MOSI / GP19 (SCL) | MOSI / GP19 (SCL) | straight |
| GND | GND | common ground |

- **Pull-ups are required:** ~4.7 kΩ from SDA→3.3 V and SCL→3.3 V (one set, on
  either board). Without them the link silently fails (control board blinks red).
- The boards sit in **different power domains** (control = host USB, target = DUT
  USB). Fine while both are powered for bench bring-up; see design §7 for the
  back-powering caution before this goes near a real DUT power cycle.

**DUT-side wiring (control board → DUT)** for the serial-console bridge and the
power relay:

| control board | DUT | note |
|---|---|---|
| TX / GP0 | DUT console **RX** | UART0; 3.3 V logic (level-shift true RS-232) |
| RX / GP1 | DUT console **TX** | UART0 |
| GND | DUT GND | common ground for the console |
| D5 / GP5 | relay / load-switch control | switches the DUT's 5 V — a real switch, not the GPIO driving the rail |

## Bring-up

1. Flash both boards with CircuitPython 9.x.
2. Copy `target/boot.py` + `target/code.py` to the **target** board's CIRCUITPY.
   It needs to be in **dev mode** to mount the drive — see [Mode switching](#mode-switching-target-board-dev-vs-hid-only).
3. Copy `control/boot.py` + `control/code.py` to the **control** board's CIRCUITPY.
4. Reset both (boot.py only runs on hard reset).

Watch debug prints on either board's REPL console (`tio`, `screen`, or Mu) — both
firmwares print what they send/receive when `DEBUG` is on. The target's NeoPixel
blips green per frame received; the control's blips green per frame relayed (red
on I2C failure). **Control blinking red** = target not ACKing (pull-ups / wiring /
addr / target code not running).

## Milestone 1 — link proven (historical)

The first control firmware self-drove a canned HID frame to the target once a
second to prove the I2C1 link before any host existed. That is now superseded by
the milestone-2 host-driven control (the self-driving version lives in git
history). The bring-up surfaced three CircuitPython 9.2.9 `i2ctarget` gotchas —
see the comments in `target/code.py` and the minimal `target/min_i2ctarget_test.py`.

### Sanity check: is `i2ctarget` available?

The target needs CircuitPython's `i2ctarget` module. Confirm at the target's REPL:

```python
>>> import i2ctarget; print(i2ctarget.I2CTarget)
```

If that raises `ImportError`, this CircuitPython build lacks I2C-target support and
the transport decision tips to UART (design §9.1).

## Mode switching (target board): dev vs HID-only

The target's `boot.py` configures USB once at reset, so the mode can't change
live — it reads a 1-byte **NVM flag**:

- **dev** (NVM byte 0 ≠ 0, the erased-flash default): CIRCUITPY drive + REPL +
  HID — use this to edit and watch debug prints.
- **HID-only** (NVM byte 0 = 0): only the keyboard + mouse the DUT sees; no
  drive, no console (production).

**Tap the BOOT button (GP11)** while running to flip the flag and reset, so one
press toggles dev ↔ HID-only — no jumper. `code.py` polls the button in both
modes, so it is always an escape from HID-only.

**Hardware fallback:** grounding **D2** at reset forces dev mode regardless of
the flag, so a wedged `code.py` can never strand the board. This is also how you
recover a board running *old* firmware that predates the button logic (e.g. to
install this firmware the first time).

## Milestone 2 — host-driven relay (current)

The control board reads length-prefixed binary frames from `usb_cdc.data` and
routes them by type byte: `0x01` HID frames are relayed **verbatim** over I2C1 to
the target (which injects them as USB-HID); `0x02` control frames are handled
locally (ping, version, power); `0x03` console frames bridge the DUT UART (see
Milestone 4). The host composes the report bytes.

Uniform frame format (byte-stream parseable on both the CDC and I2C legs):

```
[type][b1][len][payload .. len bytes]
  0x01  rid  N    N HID report bytes   (rid 1 = keyboard / 8 B, 2 = abs mouse / 6 B)
  0x02  cmd  N    N arg bytes          (cmd 1 = ping, 2 = version, 3 = power)
  0x03  port N    N raw console bytes  (DUT serial console, both directions)
```

Drive it from the host with `host_send.py`, pointed at the control board's
**data** CDC interface (the *second* `usbmodem` of its pair; the first is the
REPL console):

```bash
uv run --with pyserial python host_send.py --port /dev/cu.usbmodemXXXX ping
uv run --with pyserial python host_send.py --port /dev/cu.usbmodemXXXX mouse 16383 16383
uv run --with pyserial python host_send.py --port /dev/cu.usbmodemXXXX type "hello"
```

`mouse <x> <y>` takes absolute coordinates in `0..32767`; the host OS maps that
range across the full screen, so `mouse 16383 16383` parks the cursor dead
center. Add a button name (`mouse <x> <y> left`) to click at that point.

## Milestone 3 — Rust composition (done)

The Rust `hidrig` CLI/daemon (`hidrig/src/compose.rs`) now owns HID composition:
each command (`type`/`key`/`moveabs`/…) is turned into report bytes and framed in
Rust, then written to the control board's **data CDC endpoint** — no Python in
the loop. `host_send.py` above remains as a dependency-free poke tool, but the
real driver is `hidrig`:

```bash
hidrig -d /dev/cu.usbmodemXXXX moveabs 16383 16383   # cursor to centre
hidrig -d /dev/cu.usbmodemXXXX type "hello"
hidrig -d /dev/cu.usbmodemXXXX serve                  # daemon: holds state, KVM WS
```

`-d` is the control board's data CDC port. `serve` runs the daemon that holds the
composition state (held keys, virtual cursor) and re-exposes the command protocol
over a localhost WebSocket, so `paniolo console` and `paniolo hid send` drive the
rig unchanged. The single-board rig can later adopt the same composition against a
dumb single-board firmware (frames over its UART).

## Milestone 4 — DUT console bridge + power relay (added)

The control board gained two DUT-facing capabilities (design §6–§7), both wired
on the control side (see *Wiring*):

- **Serial console.** Hardware UART0 (`TX = GP0`, `RX = GP1`) bridges the DUT's
  console: inbound `0x03` frames are written to the UART, and UART RX bytes are
  framed back to the host as `0x03`. The `hidrig serve` daemon demuxes that
  stream and re-exports it as a **PTY**, which paniolo's `serial` channel opens
  (`device = /tmp/paniolo-<uid>/hid/console`). The board never interprets console
  bytes — the same dumb-pipe rule as HID.
- **Power.** `D5 = GP5` drives a relay / load-switch on the DUT's 5 V.
  `hidrig power off|on|cycle [secs]` sends a `0x02` power frame; the board acks,
  then acts (`cycle` holds power off for `secs`, default 2 s). The relay pin and
  polarity are `RELAY_PIN` / `RELAY_ACTIVE_HIGH` at the top of `control/code.py`.

```bash
hidrig -d /dev/cu.usbmodemXXXX power cycle            # power-cycle the DUT
hidrig -d /dev/cu.usbmodemXXXX serve                  # also bridges the console as a PTY
paniolo serial watch -t pi5                           # attach to the PTY console
```

> **Status.** The **relay/power** path is hardware-verified — `power off|on|cycle`
> actuate the relay, and the state persists across a control-board reset (NVM).
> The **console** is **not yet** verified: confirm the UART round trip
> (host ↔ DUT) and that `tio`/serialcap behave on the daemon's PTY (design §6),
> and watch the control board's loop headroom when a DUT boot-log streams while
> HID is relaying (design §9.5).
