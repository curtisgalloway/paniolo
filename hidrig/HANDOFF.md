# HANDOFF — KB2040 HID Test Rig

## Context

This repo is a USB keyboard/mouse injector for automated software testing of a
Raspberry Pi. A **control board** (Adafruit KB2040 or USB Trinkey QT2040)
plugged into a test computer receives line-based text commands over a USB CDC
serial channel and relays them over I2C (STEMMA QT) to a **target board**
(KB2040) plugged into the Pi, which replays them as USB HID keyboard/mouse
events.

```
[Test computer] --USB serial--> [Control board] --STEMMA QT / I2C--> [Target board] --USB HID--> [Raspberry Pi]
```

Both boards run **CircuitPython 9.x**. The target uses the built-in
`i2ctarget` core module plus `adafruit_hid`. The control board is the I2C
controller; the target is the I2C peripheral at address `0x41`.

Read `README.md` first — it has the wiring, the command protocol, and the
binary wire protocol tables. The existing code is working and should be
treated as the source of truth for the protocol.

## Repo layout

```
target/code.py     # I2C target -> USB HID (runs on the board into the Pi)
control/boot.py    # enables the usb_cdc data channel
control/code.py    # USB serial -> I2C controller, command parser
host/example.py    # pyserial driver / usage example
README.md          # architecture + protocol reference
```

## Hard constraints — do not break these

1. **Opcode tables in `target/code.py` and `control/code.py` must stay in
   sync.** They are duplicated by design (two separate boards). Any protocol
   change must update both files and the tables in `README.md`.
2. **RP2040 supports only one I2C target address.** Do not add multi-address
   logic to the target.
3. **The target's I2C link is RP2040<->RP2040 and relies on clock stretching;
   that is intentional and fine.** Do not try to "fix" clock stretching or
   move the Pi onto the I2C bus — the Pi must stay on USB (HID).
4. **HID relative mouse movement is int8 per report.** Keep the move-splitting
   logic; do not send raw values outside -127..127 to the target.
5. **Keep I2C write transactions small** (the `MAX_TYPE_CHUNK = 30` cap on
   `TYPE` payloads). Don't send unbounded buffers in a single I2C write.
6. No secrets, no network calls on the boards. CircuitPython only; no external
   pip deps on the boards. The host script may use `pyserial`.

## Tasks (in priority order)

1. **Absolute mouse support.** The default `adafruit_hid.Mouse` is
   relative-only. Add an optional absolute-positioning mode:
   - Add a target-side `target/boot.py` that registers a custom HID device
     with an absolute-axis mouse report descriptor (two 16-bit absolute X/Y
     axes plus buttons), in addition to or replacing the default mouse.
   - Add opcode `OP_MOUSE_MOVE_ABS = 0x14` carrying x (uint16 LE) and
     y (uint16 LE) in a logical 0..32767 coordinate space.
   - Add a `moveabs <x> <y>` host command in `control/code.py` and a wrapper
     in `host/example.py`.
   - Document the new opcode and command in `README.md`.
   - Note: the host OS maps the 0..32767 range across the full screen; callers
     scale pixel coords to that range. Add a helper for that in `host/example.py`.

2. **Protocol robustness.** Add an optional 1-byte sequence number and a
   1-byte XOR checksum to each packet, behind a feature flag so existing
   behavior is preserved by default. On checksum failure the target should
   drop the packet (and, if a status read is implemented, surface an error
   count). Update both boards and the README.

3. **Macros / timing.** Add host-side support (in `host/example.py`, not on
   the boards) for: inter-command delays, key auto-repeat, and loading a
   sequence of commands from a file. Keep the board firmware dumb; sequencing
   and timing live on the host.

4. **Tests.** Add `host/` unit tests that exercise the command-parsing and
   packet-encoding logic without hardware. Factor the encoding logic in
   `control/code.py` into a pure function table if needed so it can be mirrored
   and tested host-side. Mock the serial port.

## Verification

- There is no CI for the on-board firmware (it runs on microcontrollers).
  For board code, the bar is: it imports cleanly under CircuitPython 9.x and
  follows the existing structure.
- For host code, add and run unit tests (pytest). Mock serial; do not require
  hardware to run the suite.
- Manually document a bring-up test plan update in `README.md` for any new
  feature (e.g. how to verify absolute mouse works).

## Style

- Match the existing code style: plain CircuitPython, clear module docstrings,
  opcode constants grouped and commented, no clever metaprogramming.
- Every protocol change touches three places: `target/code.py`,
  `control/code.py`, and `README.md`. Treat that as a checklist.
