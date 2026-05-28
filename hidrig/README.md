# KB2040 HID Test Rig

A USB keyboard/mouse injector for automated software testing of a Raspberry Pi
(or any USB host). A **control board** receives text commands from a test
computer over USB serial and relays them over I2C (STEMMA QT) to a **target
board**, which replays them as USB HID keyboard and mouse events into the Pi.

```
[Test computer] --USB serial--> [Control board: KB2040 / USB Trinkey QT2040]
                                       |
                                 STEMMA QT (I2C)
                                       |
[Raspberry Pi]  <--USB HID-- [Target board: KB2040]
```

## Hardware

- 1x Adafruit KB2040 — **target** (USB to the Pi, STEMMA QT to control board)
- 1x Adafruit KB2040 **or** USB Trinkey QT2040 — **control** (USB to test
  computer, STEMMA QT to target board)
- 1x STEMMA QT / Qwiic cable between the two boards

STEMMA QT is I2C with built-in pull-ups, so no extra resistors are needed.

## Firmware setup

Both boards run **CircuitPython 9.x**.

1. Install CircuitPython 9.x on each board (hold BOOT, copy the UF2).
2. Download the matching CircuitPython library bundle and copy the
   `adafruit_hid` folder into `/lib` on **both** boards' `CIRCUITPY` drives.
   - `i2ctarget` is a built-in core module — no library needed for it.
3. Copy files to the drives:
   - Target board `CIRCUITPY/`: `target/code.py` -> `code.py`
   - Control board `CIRCUITPY/`: `control/boot.py` -> `boot.py`
     and `control/code.py` -> `code.py`
   - `boot.py` only takes effect after a power cycle / hard reset.

## Bring-up checklist

1. Power both boards and connect the STEMMA QT cable.
2. From the control board REPL, confirm the link:
   ```python
   import board
   i2c = board.STEMMA_I2C()
   while not i2c.try_lock():
       pass
   print([hex(a) for a in i2c.scan()])   # expect ['0x41']
   i2c.unlock()
   ```
3. Plug the target board into the Pi; it should enumerate as a USB
   keyboard + mouse.
4. On the test computer, drive the rig with `paniolo hid` (run
   `paniolo hid setup` first to detect/save the control board's data port).

## Command protocol

One command per line, `\n` terminated, sent to the control board's USB CDC
**data** port. The board replies `OK` or `ERR <message>`.

| Command | Example | Effect |
|---|---|---|
| `type <text>` | `type hello world` | Type a string |
| `key <NAME>` | `key ENTER` | Tap (press+release) a key |
| `combo <NAME>...` | `combo LEFT_CONTROL C` | Chord: press all, release all |
| `down <NAME>` | `down LEFT_SHIFT` | Press and hold |
| `up <NAME>` | `up LEFT_SHIFT` | Release a held key |
| `releaseall` | `releaseall` | Release all held keys |
| `move <dx> <dy>` | `move 300 -50` | Relative mouse move (auto-stepped) |
| `click <btn>` | `click left` | Click left/right/middle |
| `mdown <btn>` / `mup <btn>` | `mdown left` | Hold / release a mouse button |
| `scroll <amount>` | `scroll -3` | Scroll wheel |

`<NAME>` values are `adafruit_hid` Keycode names (A-Z, ENTER, TAB, ESCAPE,
LEFT_CONTROL, LEFT_SHIFT, UP_ARROW, F1..F12, etc.).

## Wire protocol (control -> target, over I2C)

Each I2C write is one packet: `[opcode][payload...]`.

| Opcode | Name | Payload |
|---|---|---|
| 0x01 | KEY_PRESS | one or more keycode bytes |
| 0x02 | KEY_RELEASE | one or more keycode bytes |
| 0x03 | KEY_RELEASE_ALL | (none) |
| 0x04 | TYPE | UTF-8 text bytes (<=30 per packet) |
| 0x10 | MOUSE_MOVE | dx (int8), dy (int8) |
| 0x11 | MOUSE_PRESS | button mask (1=L, 2=R, 4=M) |
| 0x12 | MOUSE_RELEASE | button mask |
| 0x13 | MOUSE_SCROLL | amount (int8) |

## Design notes

- The RP2040 `i2ctarget` core module uses I2C clock stretching. That is fine
  here because the I2C bus is RP2040 <-> RP2040. The Raspberry Pi, which is
  poor at clock stretching, sits on USB, not on this I2C bus.
- The RP2040 supports a single I2C target address (`0x41` here).
- HID relative mouse movement is int8 per report; the control board splits
  larger moves into multiple steps automatically.
- The default `adafruit_hid.Mouse` is relative-only. Absolute positioning
  (jump to exact coordinates) needs a custom HID report descriptor in a
  target-side `boot.py`. See HANDOFF.md for the extension task.

## Host testing tool

`host/hid_seize_reports.c` is a macOS IOKit utility that exclusively seizes
the target board's HID interface and prints raw input reports without any
keystroke reaching the focused application. Use it to verify the full pipeline
end-to-end on the same machine the control board is plugged into.

```bash
cd hidrig/host
make
./hid_seize_reports        # grant Input Monitoring in System Settings when prompted
```

In another terminal, drive the rig normally:
```bash
paniolo hid type "hello"
```

The tool prints the raw HID report bytes for every keyboard/mouse event.
The VID/PID are set to 0x239A/0x8106 (KB2040 running CircuitPython).

## Files

```
hidrig/
  target/code.py         # I2C target -> USB HID (runs on the board into the Pi)
  control/boot.py        # enables the usb_cdc data channel
  control/code.py        # USB serial -> I2C controller
  host/hid_seize_reports.c  # macOS IOKit HID capture tool
  host/Makefile
  README.md
  HANDOFF.md             # task brief for the remaining firmware work
```

The host driver lives in paniolo: `src/paniolo/_hid.py` (the `paniolo hid`
command group).
