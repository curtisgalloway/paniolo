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
control/   -> firmware for the board on the CONTROL HOST's USB (I2C controller)
target/    -> firmware for the board on the DUT's USB          (I2C peripheral, HID)
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
  USB). Fine while both are powered for bench bring-up; see design §6 for the
  back-powering caution before this goes near a real DUT power cycle.

## Milestone 1 — prove the link (current)

The control board self-drives a canned HID frame to the target once a second;
the target relays it to `send_report`. No host or daemon yet.

1. Flash both boards with CircuitPython 9.x.
2. Copy `target/boot.py` + `target/code.py` to the **target** board's CIRCUITPY.
   It needs to be in **dev mode** to mount the drive — see [Mode switching](#mode-switching-target-board-dev-vs-hid-only).
3. Copy `control/boot.py` + `control/code.py` to the **control** board's CIRCUITPY.
4. Reset both (boot.py only runs on hard reset). Watch the NeoPixels:
   - **both blink green in lock-step** → I2C link works.
   - **control blinks red** → target not ACKing (pull-ups / wiring / addr / target
     code not running).
   - the default `TEST = "noop"` has no visible HID effect (so it won't hijack
     the cursor if the target is on your own Mac). Flip to `TEST = "mouse"` in
     `control/code.py` for a visible cursor wiggle once the NeoPixels confirm the
     link.

Watch debug prints on either board's REPL console (`tio`, `screen`, or Mu) — both
firmwares print what they send/receive when `DEBUG`/prints are on.

### Sanity check: is `i2ctarget` available?

The target needs CircuitPython's `i2ctarget` module. Confirm at the target's REPL:

```python
>>> import i2ctarget; print(i2ctarget.I2CTarget)
```

If that raises `ImportError`, this CircuitPython build lacks I2C-target support and
the transport decision tips to UART (design §8.1).

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

## Next milestones (not built yet)

- **M2:** control board reads binary frames from `usb_cdc.data` and routes by the
  type byte (relay `0x01` HID frames over I2C; handle `0x02` control frames). A
  host-side test script pushes frames.
- **M3:** the Rust `hidrig serve` daemon gains the composition layer (v1 ASCII →
  HID report bytes → frames), so `paniolo hid send` drives the rig unchanged.
