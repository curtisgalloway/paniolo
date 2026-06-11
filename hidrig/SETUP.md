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

# Dual-board HID injector bring-up

Step-by-step for flashing and provisioning the **two-board KB2040 rig**, then
driving it with `hidrig` / `paniolo hid`. Linux differs from macOS only in
device paths (CIRCUITPY under `/media|/run/media/<user>/`; the control board's
data port is `/dev/ttyACM*` instead of `/dev/cu.usbmodem*`).

See [`README.md`](README.md) for the topology and wire protocol and
[`firmware/dual/README.md`](firmware/dual/README.md) for the firmware-side
detail; this file is the install runbook. The two boards are the **control**
board (host-facing, USB-CDC, I2C1 controller) and the **target** board
(DUT-facing, USB-HID, I2C1 peripheral).

## 1. CircuitPython (both boards)

Match the firmware's target: **CircuitPython 9.x** (10.x unverified). Latest
9.x at time of writing: **9.2.9**. Do this for each board.

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

The **target** board needs CircuitPython's `i2ctarget` module — confirm at its
REPL: `import i2ctarget; print(i2ctarget.I2CTarget)`. If that raises
`ImportError`, this build lacks I2C-target support (the dumb relay can't run).
The relay uses only core `usb_hid`, so **`adafruit_hid` is not required**.

## 2. Firmware

**Target board** (must be in dev mode to mount CIRCUITPY — see §4):
```
cp hidrig/firmware/dual/target/boot.py /Volumes/CIRCUITPY/boot.py
cp hidrig/firmware/dual/target/code.py /Volumes/CIRCUITPY/code.py
```

**Control board:**
```
cp hidrig/firmware/dual/control/boot.py /Volumes/CIRCUITPY/boot.py
cp hidrig/firmware/dual/control/code.py /Volumes/CIRCUITPY/code.py
```

`boot.py` only takes effect on a **hard reset** (replug); a code save's soft
reload does not re-run it.

## 3. Wiring (I2C1)

Three wires between the boards, **straight, not crossed** (I2C is a bus):

| control board | target board | note |
|---|---|---|
| `D10` / GP10 (SDA) | `D10` / GP10 (SDA) | straight |
| `MOSI` / GP19 (SCL) | `MOSI` / GP19 (SCL) | straight |
| GND | GND | common ground |

**Pull-ups are required:** ~4.7 kΩ from SDA→3.3 V and SCL→3.3 V (one set, on
either board). Without them the control board's controller-mode `busio.I2C`
rejects the bus ("No pull up found") and blinks red. The target peripheral
address is **0x41**.

Then plug each board's native USB into its host:
- **Target** board → the **DUT** (it enumerates as a USB keyboard + mouse and
  is powered by the DUT, so it reboots with the DUT).
- **Control** board → the **control host** (it enumerates as a USB-CDC pair:
  a REPL console and a data endpoint).

## 4. Target mode switching (dev vs HID-only)

The target's `boot.py` configures USB once at reset from a 1-byte **NVM flag**:

- **dev** (the erased-flash default): CIRCUITPY drive + REPL + HID — use this
  to copy firmware and watch debug prints.
- **HID-only**: only the keyboard + mouse the DUT sees; no drive, no console
  (production).

**Tap the BOOT button (GP11)** while running to flip the flag and reset — one
press toggles dev ↔ HID-only. **Hardware fallback:** grounding **D2 at reset**
forces dev mode regardless of the flag, so a wedged `code.py` can never strand
the board (also how to recover a board running pre-button firmware).

## 5. Drive it

The control board's **data** CDC port is the *second* `usbmodem` of its pair
(the first is the REPL console); on Linux it is the higher-numbered
`/dev/ttyACM*`. `hidrig` lives in paniolo's libexec dir (not on PATH — `make
install` puts it there):

```
paniolo helper hidrig -d /dev/cu.usbmodemXXXX ping
paniolo helper hidrig -d /dev/cu.usbmodemXXXX version     # expect: dual-control/1
paniolo helper hidrig -d /dev/cu.usbmodemXXXX type "hello"
```

Through paniolo (lab file is the source of truth):

```
paniolo hid set -t <target> --cmd "hidrig -d /dev/cu.usbmodemXXXX"
paniolo hid send -t <target> type hello
paniolo hid send -t <target> key ENTER
```

## 6. Validate end-to-end without a DUT

Plug the **target** board's USB into the same dev machine that drives the
control link, then run the leak-safe IOKit capture tool (macOS):

```
cd hidrig/host && make
sudo ./hid_capture_usb       # start BEFORE injecting
```

In a second terminal, `hidrig -d /dev/cu.usbmodemXXXX moveabs 16383 16383` —
the raw HID reports print in the first terminal and nothing reaches the focused
app or the real cursor (the device is detached from the macOS HID stack).

## Gotchas

- **`boot.py` needs a hard reset** (replug) to take effect; a soft reload
  won't re-run it.
- **Pull-ups required, and a live target is not proof of them** — `i2ctarget`
  doesn't check for pull-ups, so the target can come up while the control
  board still can't open the bus. If the control board blinks red, suspect
  pull-ups / wiring / address / the target's `code.py` not running.
- **Wiring is straight, not crossed** (I2C bus): SDA→SDA, SCL→SCL.
- **No ASCII on the wire:** `hidrig` composes HID and sends binary frames; the
  control board is USB-CDC, so there is no baud rate to set and no `OK`/`ERR`
  text protocol (only `ping`/`version` control frames draw a reply).
- **HID-only target "vanishes":** in HID-only mode the target drops its
  CIRCUITPY drive and console, so a power blip that hard-resets it can look
  like a dead board — tap BOOT (or ground D2 at reset) to get dev mode back.
- **FAT32 `cp` error on macOS is benign** (extended attributes); the
  UF2/file copy still succeeds.
- **CircuitPython 9.x**, not 10.x, until the firmware is verified on 10.
- If `CIRCUITPY` won't mount or is read-only in dev mode, replug the board;
  a clean remount restores host write access.
