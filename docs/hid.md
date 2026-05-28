# USB HID injection

paniolo can inject keyboard and mouse events into the target via a two-board
rig built from Adafruit KB2040s. The control board receives text commands from
the test computer over USB serial and relays them as USB HID events to the Pi.

See [`hidrig/README.md`](../hidrig/README.md) for hardware wiring and firmware
setup instructions.

---

## Architecture

```
[Test computer]
  └── USB serial (data CDC) ──► [Control board: KB2040 / Trinkey QT2040]
                                        └── STEMMA QT (I2C) ──► [Target board: KB2040]
                                                                         └── USB HID ──► [Pi / DUT]
```

The control board parses text commands and encodes them as compact binary I2C
packets. The target board replays them as USB HID keyboard and mouse events.

---

## Setup

```bash
# Detect and save the control board's data CDC port
paniolo hid setup [target-machine]

# Show saved HID config
paniolo hid show [target-machine]
```

The data CDC port is the higher-numbered of the two USB serial nodes the
control board exposes. `setup` identifies it automatically.

`pyserial` must be installed for HID commands:

```bash
uv tool install --with pyserial ~/src/paniolo
```

---

## Commands

```bash
paniolo hid type "hello world"        # type a string
paniolo hid key ENTER                 # tap (press+release) a key
paniolo hid combo LEFT_CONTROL C      # chord: press all then release all
paniolo hid releaseall                # release any held keys

paniolo hid click left                # click left/right/middle
paniolo hid move 300 -50              # relative mouse move
paniolo hid scroll -3                 # scroll wheel (negative = down)
```

Key names are `adafruit_hid` Keycode names: `A`–`Z`, `ENTER`, `TAB`,
`ESCAPE`, `BACKSPACE`, `DELETE`, `UP_ARROW`, `DOWN_ARROW`, `LEFT_ARROW`,
`RIGHT_ARROW`, `LEFT_CONTROL`, `LEFT_SHIFT`, `LEFT_ALT`, `LEFT_GUI`,
`F1`–`F12`, etc.

**Negative arguments:** `move` and `scroll` accept negative values directly
(`paniolo hid move 50 -30`) without needing `--`.

---

## Command files

A command file is a plain text file with one command per line. Blank lines and
`# comments` are ignored. Two extra directives are supported:

```
# boot-sequence.txt
type root
key ENTER
delay 500        # wait 500 ms
type ls /
key ENTER
sleep 1.5        # wait 1.5 seconds
```

Run a sequence:

```bash
paniolo hid run boot-sequence.txt [target-machine]
```

---

## Host testing tool

`hidrig/host/hid_seize_reports.c` is a macOS IOKit utility that exclusively
seizes the target board's HID interface, preventing keystrokes from reaching
any application. Use it to verify the full pipeline end-to-end without a Pi:

```bash
cd hidrig/host && make
sudo ./hid_seize_reports   # grant Input Monitoring in System Settings first
```

In a second terminal, run `paniolo hid type "test"` and watch the raw HID
report bytes appear.
