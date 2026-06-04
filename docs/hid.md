# USB HID injection

paniolo can inject keyboard and mouse events into the target through any
helper tool that drives a USB HID injector — by default the KB2040 rig in
[`hidrig/`](../hidrig/README.md). The integration is a generic per-target
**hid channel**, an opaque command prefix exactly like the power hooks:
paniolo appends arguments to it and runs it, staying agnostic to the device.

---

## Architecture

```
[Control host] --USB-serial adapter--+
                                     | UART (TX/RX/GND, 115200 8N1)
                                     v
                                 [KB2040] --built-in USB (HID device)--> [Target / DUT]
```

The injector implements the [HID serial protocol](hid-serial-protocol.md)
(line-based text commands, `OK`/`ERR` replies). The `hidrig` CLI is the
host-side client; the KB2040 CircuitPython firmware is the reference device
implementation, and anything else that conforms to the spec (another
microcontroller, a CH9329 shim) drops in without touching paniolo.

---

## Setup

```bash
# Build and install the helper (once per control host)
cargo install --path hidrig

# Bind the helper to the target in the lab file
paniolo hid set -t pi5 --cmd "hidrig -d /dev/cu.usbserial-XXXX"

# Channel on a remote control host
paniolo hid set -t pi5 --cmd "hidrig -d /dev/ttyUSB0" --host bench1

# Remove the channel
paniolo hid rm -t pi5
```

`paniolo doctor` checks the channel: an absolute-path helper is probed for
existence on the channel's host; bare names are assumed to be on PATH.

---

## Commands

`paniolo hid send` appends its arguments to the configured command and runs
it (over SSH when the channel lives on a remote control host):

```bash
paniolo hid send -t pi5 type hello world     # type a string
paniolo hid send -t pi5 key ENTER            # tap (press+release) a key
paniolo hid send -t pi5 combo LEFT_CONTROL C # chord: press all, release all
paniolo hid send -t pi5 releaseall           # release any held keys
paniolo hid send -t pi5 click left           # click left/right/middle
paniolo hid send -t pi5 move 300 -50         # relative mouse move
paniolo hid send -t pi5 scroll -3            # scroll wheel (negative = down)
paniolo hid send -t pi5 ping                 # injector liveness check
```

With a single target in the lab, `-t` may be omitted. Everything after
`send` (minus `-t`) is the helper's CLI — see `hidrig --help` for the full
set. Key names are `adafruit_hid` Keycode names: `A`–`Z`, `ENTER`, `TAB`,
`ESCAPE`, `BACKSPACE`, `DELETE`, `UP_ARROW`, `DOWN_ARROW`, `LEFT_ARROW`,
`RIGHT_ARROW`, `LEFT_CONTROL`, `LEFT_SHIFT`, `LEFT_ALT`, `LEFT_GUI`,
`F1`–`F12`, etc.

**Negative arguments:** `move` and `scroll` accept negative values directly
(`paniolo hid send -t pi5 move 50 -30`); put `-t` before the helper
arguments.

---

## Command files

A command file is a plain text file with one protocol command per line.
Blank lines and `# comments` are ignored. Two extra directives are
supported:

```
# boot-sequence.txt
type root
key ENTER
delay 500        # wait 500 ms
type ls /
key ENTER
sleep 1.5        # wait 1.5 seconds
```

Run a sequence (the file must exist on the host that owns the channel):

```bash
hidrig -d /dev/cu.usbserial-XXXX run boot-sequence.txt
hidrig -d /dev/cu.usbserial-XXXX run - < boot-sequence.txt   # via stdin
```

Sequencing and timing live on the host; the firmware stays dumb.

---

## Lab file shape

```toml
[targets.pi5.hid]
cmd = "hidrig -d /dev/cu.usbserial-XXXX"
# host = "bench1"            # if the injector hangs off a remote control host
```

---

## Host testing tool

`hidrig/host/hid_seize_reports.c` is a macOS IOKit utility that exclusively
seizes the injector's HID interface, preventing keystrokes from reaching any
application. Use it to verify the full pipeline end-to-end without a target:

```bash
cd hidrig/host && make
sudo ./hid_seize_reports   # grant Input Monitoring in System Settings first
```

In a second terminal, run `hidrig -d <adapter> type test` and watch the raw
HID report bytes appear.
