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

# HID injector bring-up

Step-by-step for flashing and provisioning the KB2040 injector, then driving
it with `hidrig` / `paniolo hid`. Linux differs from macOS only in device
paths (`/dev/ttyUSB*` for the adapter, CIRCUITPY under
`/media|/run/media/<user>/`).

See `README.md` for wiring and the protocol; this file is the install
runbook.

## 1. CircuitPython

Match the firmware's target: **CircuitPython 9.x** (10.x unverified). Latest
9.x at time of writing: **9.2.9**.

1. The KB2040's board id is `adafruit_kb2040`. UF2 URL pattern:
   ```
   https://downloads.circuitpython.org/bin/adafruit_kb2040/en_US/adafruit-circuitpython-adafruit_kb2040-en_US-9.2.9.uf2
   ```
2. Enter the UF2 bootloader: **unplug, hold the BOOT button, plug into a dev
   machine**. An `RPI-RP2` drive mounts. Confirm with
   `cat /Volumes/RPI-RP2/INFO_UF2.TXT`.
3. Copy the UF2 onto `RPI-RP2`:
   ```
   cp adafruit-circuitpython-adafruit_kb2040-en_US-9.2.9.uf2 /Volumes/RPI-RP2/
   ```
   On macOS `cp` to this FAT volume exits non-zero with an "extended
   attributes" error — that's benign, the write succeeds and the board
   reboots. Don't retry.
4. The board reboots into CircuitPython and `CIRCUITPY` mounts (~5–10 s).
   `cat /Volumes/CIRCUITPY/boot_out.txt` shows the version + board id.

## 2. adafruit_hid

`circup` reads the board's CP version and installs the matching build:

```
uvx circup --path /Volumes/CIRCUITPY install adafruit_hid
```

## 3. Firmware

```
cp hidrig/firmware/boot.py /Volumes/CIRCUITPY/boot.py
cp hidrig/firmware/code.py /Volumes/CIRCUITPY/code.py
```

`boot.py` only takes effect on a **hard reset** (replug); a code save's soft
reload does not re-run it. After the reset the board is **HID-only**: no
CIRCUITPY drive, no REPL, no serial ports on its USB. That's correct — the
USB now faces the target.

**To get CIRCUITPY back** (firmware updates): jumper `D2` to GND (adjacent
pins on the KB2040 edge), plug into the dev machine, and the drive + REPL
re-enumerate. Remove the jumper and replug for normal operation.

## 4. Wiring

1. Plug the KB2040's USB into the **target** — it enumerates as a USB
   keyboard + mouse and powers the board.
2. Wire the control host's 3.3 V USB-serial adapter to the board:
   adapter **TX -> RX**, adapter **RX -> TX**, **GND -> GND**.

NeoPixel: blinking red until the target enumerates the board, then a green
blip when it starts serving.

## 5. Drive it

Directly:

```
cargo install --path hidrig          # once
hidrig -d /dev/cu.usbserial-XXXX ping
hidrig -d /dev/cu.usbserial-XXXX version     # expect: 1 kb2040-circuitpython/1.0
hidrig -d /dev/cu.usbserial-XXXX type "hello"
```

Through paniolo (lab file is the source of truth):

```
paniolo hid set -t <target> --cmd "hidrig -d /dev/cu.usbserial-XXXX"
paniolo hid send -t <target> type hello
paniolo hid send -t <target> key ENTER
```

## 6. Validate end-to-end without a target

Plug the injector's USB into the same dev machine that drives the UART, then
run the IOKit capture tool (macOS):

```
cd hidrig/host && make
sudo ./hid_seize_reports     # grant Input Monitoring when prompted
```

In a second terminal, `hidrig -d <adapter> type test` — the raw HID reports
print in the first terminal and nothing reaches the focused app.

## Gotchas

- **`boot.py` needs a hard reset** (replug) to take effect; a soft reload
  won't re-run it.
- **No serial ports on the board's USB is normal** — the control path is the
  UART via the adapter. If you need the REPL, use the D2 dev jumper.
- **Crossed wiring:** no reply to `ping` usually means TX/RX not crossed, a
  missing ground, or the target (and therefore the board) is powered off.
- **3.3 V adapters only** — a 5 V-logic adapter can damage the RP2040.
- **FAT32 `cp` error on macOS is benign** (extended attributes); the
  UF2/file copy still succeeds.
- **CircuitPython 9.x**, not 10.x, until the firmware is verified on 10.
- If `CIRCUITPY` won't mount or is read-only in dev mode, replug the board;
  a clean remount restores host write access.
