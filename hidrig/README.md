# KB2040 HID Injector

A USB keyboard/mouse injector for automated testing of a target machine
(SBC, e.g. a Raspberry Pi). A single **Adafruit KB2040** presents itself to
the target as a plain USB HID keyboard + mouse; the control host drives it
with line-based text commands over a UART on the board's TX/RX pins,
typically through a USB-serial adapter.

```
[Control host] --USB-serial adapter--+
                                     | UART (TX/RX/GND, 115200 8N1, 3.3 V)
                                     v
                                 [KB2040] --built-in USB (HID device)--> [Target]
```

The wire protocol is the **HID serial protocol v1** — see
[`docs/hid-serial-protocol.md`](../docs/hid-serial-protocol.md) for the
normative spec. This directory holds the reference firmware implementation
(CircuitPython) and the host-side CLI (`hidrig`, Rust), which works against
any device implementing the spec.

## Hardware

- 1x Adafruit KB2040 (any CircuitPython-capable RP2040 board with free
  UART pins works with minor pin edits)
- 1x 3.3 V USB-serial adapter (FTDI, CP2102, ...) on the control host
- 3 jumper wires: adapter TX -> board RX, adapter RX -> board TX, GND -> GND

The board is powered by the **target's** USB port, so it reboots with the
target — held keys can never survive a target power cycle, and the UART is
silent while the target is off.

Pins (KB2040): `TX`/`D0` (GPIO0) and `RX`/`D1` (GPIO1), with GND adjacent.
`D2` is the dev-mode jumper (below).

## Firmware setup

The board runs **CircuitPython 9.x**. See [`SETUP.md`](SETUP.md) for the
full runbook; in short:

1. Install CircuitPython 9.x (hold BOOT, copy the UF2).
2. `uvx circup --path /Volumes/CIRCUITPY install adafruit_hid`
3. Copy `firmware/boot.py` and `firmware/code.py` to `CIRCUITPY/`.
4. Hard-reset (replug). The board now enumerates as **HID only** — no
   CIRCUITPY drive, no REPL.

**Dev mode:** jumper `D2` to GND (adjacent pins) and reset — CIRCUITPY and
the REPL come back so you can edit the firmware. Plug into a dev machine
(not the target) for that.

Status NeoPixel: blinking red = waiting for the target's USB to enumerate;
green blip = up and serving; solid red = last command failed.

## Host CLI

`hidrig` (this crate) drives any conforming injector:

```bash
cargo install --path hidrig

hidrig -d /dev/cu.usbserial-XXXX ping            # liveness
hidrig -d /dev/cu.usbserial-XXXX version         # protocol + implementation
hidrig -d /dev/cu.usbserial-XXXX type "hello world"
hidrig -d /dev/cu.usbserial-XXXX key ENTER
hidrig -d /dev/cu.usbserial-XXXX combo LEFT_CONTROL C
hidrig -d /dev/cu.usbserial-XXXX move 300 -50
hidrig -d /dev/cu.usbserial-XXXX click right
hidrig -d /dev/cu.usbserial-XXXX scroll -3
hidrig -d /dev/cu.usbserial-XXXX run boot-seq.txt   # command file; '-' = stdin
```

Command files take one protocol command per line; blank lines and
`# comments` are skipped, and `delay <ms>` / `sleep <seconds>` pause between
commands (sequencing lives on the host; the firmware stays dumb).

Key names are `adafruit_hid` Keycode names (`A`–`Z`, `ENTER`, `TAB`,
`ESCAPE`, `LEFT_CONTROL`, `LEFT_SHIFT`, `UP_ARROW`, `F1`–`F12`, ...).

## paniolo integration

paniolo calls the tool through the generic per-target `hid` channel — an
opaque command prefix, exactly like the power hooks:

```bash
paniolo hid set -t pi5 --cmd "hidrig -d /dev/cu.usbserial-XXXX"
paniolo hid send -t pi5 type hello
paniolo hid send -t pi5 key ENTER
```

`paniolo hid send` appends its arguments to the configured command and runs
it on whichever control host owns the channel (transparently over SSH for
remote hosts). See [`docs/hid.md`](../docs/hid.md).

## Host testing tool

`host/hid_seize_reports.c` is a macOS IOKit utility that exclusively seizes
the injector's HID interface and prints raw input reports without any
keystroke reaching the focused application. Use it to verify the full
pipeline end-to-end by plugging the injector's USB into the same machine
that drives the UART.

```bash
cd hidrig/host
make
sudo ./hid_seize_reports   # grant Input Monitoring in System Settings
```

In another terminal: `hidrig -d /dev/cu.usbserial-XXXX type hello` and watch
the raw HID report bytes. The VID/PID filter is 0x239A/0x8106 (KB2040
running CircuitPython).

## Files

```
hidrig/
  src/                  # `hidrig` host CLI (Rust)
  firmware/boot.py      # USB identity: HID-only (dev-mode jumper on D2)
  firmware/code.py      # UART line protocol -> USB HID (reference impl)
  host/hid_seize_reports.c  # macOS IOKit HID capture tool
  host/Makefile
```

## History

The first version of this rig used two boards (a USB-CDC control board
relaying I2C packets to a USB-HID target board) because the control link was
the board's own USB port. Moving the control link to the UART eliminated the
second board, the binary I2C protocol, and the duplicated opcode tables —
see git history (`hidrig/control/`, `hidrig/target/`) if you need the old
design.
