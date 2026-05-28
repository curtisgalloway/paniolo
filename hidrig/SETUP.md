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

# HID rig bring-up

Step-by-step for flashing and provisioning the two boards, then driving them
with `paniolo hid`. Verified end-to-end on macOS with an **Adafruit QT2040
Trinkey** (control) + **Adafruit KB2040** (target) on CircuitPython 9.2.9: the
full `paniolo` → control → I2C/STEMMA QT → target path was confirmed
(`releaseall` returned `OK` across the link). Linux differs only in device
paths (`/dev/ttyACM*`, CIRCUITPY under `/media|/run/media/<user>/`).

See `README.md` for wiring and the protocol; this file is the install runbook.

## Roles

- **Control board** (KB2040 *or* QT2040 Trinkey): USB → test computer. Parses
  text commands, relays binary packets over I2C. Does **no** HID itself; it
  pulls in `adafruit_hid` only for the `Keycode` name→number table.
- **Target board** (KB2040): USB → the Raspberry Pi (the actual HID keyboard +
  mouse). Acts as the I2C peripheral at `0x41`.

You need **both** boards plus a STEMMA QT cable for real input injection. With
only the control board you can still validate the host→control path (below).

## 1. CircuitPython (both boards)

Match the firmware's target: **CircuitPython 9.x** (the `i2ctarget` /
`adafruit_hid` APIs the target relies on may shift on 10.x — unverified). Latest
9.x at time of writing: **9.2.9**.

1. Find the board id from its CircuitPython page (the QT2040 Trinkey is
   `adafruit_qt2040_trinkey`; the KB2040 is `adafruit_kb2040`). UF2 URL pattern:
   ```
   https://downloads.circuitpython.org/bin/<board_id>/en_US/adafruit-circuitpython-<board_id>-en_US-9.2.9.uf2
   ```
2. Enter the UF2 bootloader: **unplug, hold the BOOT button, plug back in**
   (the QT2040 Trinkey has a BOOT button, not a reset button). An `RPI-RP2`
   drive mounts. Confirm with `cat /Volumes/RPI-RP2/INFO_UF2.TXT`.
3. Copy the UF2 onto `RPI-RP2`:
   ```
   cp adafruit-circuitpython-...-9.2.9.uf2 /Volumes/RPI-RP2/
   ```
   On macOS `cp` to this FAT volume exits non-zero with an "extended attributes"
   error — that's benign, the write succeeds and the board reboots. Don't retry.
4. The board reboots into CircuitPython and `CIRCUITPY` mounts (~5–10 s).
   `cat /Volumes/CIRCUITPY/boot_out.txt` shows the version + board id.

## 2. adafruit_hid (both boards)

`circup` reads the board's CP version and installs the matching build:

```
uvx circup --path /Volumes/CIRCUITPY install adafruit_hid
```

(`i2ctarget` is a built-in core module — no library needed.)

## 3. Control board firmware

```
cp hidrig/control/boot.py /Volumes/CIRCUITPY/boot.py
cp hidrig/control/code.py /Volumes/CIRCUITPY/code.py
```

`boot.py` enables the second USB CDC ("data") channel, and **only takes effect
on a hard reset** — code saves trigger a soft reload, which does *not* re-run
`boot.py`. **Unplug and replug** the board (normal plug, no button). It should
now enumerate **two** serial ports:

```
ls /dev/cu.usbmodem*    # macOS: two nodes appear
```

The **data** port (the one `paniolo hid` uses) is the **higher-numbered** of the
two; the lower one is the REPL console.

## 4. Target board firmware

```
cp hidrig/target/code.py /Volumes/CIRCUITPY/code.py
```

The target needs no `boot.py` today (only the future absolute-mouse descriptor
in HANDOFF.md task 1 would add one). Then:

- Plug the **target** board's USB into the Raspberry Pi — it enumerates as a USB
  keyboard + mouse.
- Connect the **STEMMA QT cable** between the two boards (I2C; built-in
  pull-ups, no resistors needed).

## 5. Configure and drive with paniolo

```
paniolo hid setup --port /dev/cu.usbmodem<DATA>   # save the control board's data port
paniolo hid type "hello"
paniolo hid key ENTER
paniolo hid combo LEFT_CONTROL A
paniolo hid move 300 -50
paniolo hid run sequence.txt                       # file of commands; # comments, delay/sleep
```

`paniolo hid setup` with no `--port` lists candidates and prompts (the data port
is the higher-numbered). `paniolo hid show` reports the saved port.

## 6. Validate

Send a command and watch the reply:

- **No target board attached:** the control board parses the command, tries the
  I2C relay, and replies `ERR [Errno 19] No such device`. That error is the
  **success** signal for the control-only path — it proves the data channel,
  command parser, and OK/ERR protocol all work.
- **Target attached and on the Pi:** the command returns `OK` and the keystroke
  / mouse event appears on the Pi.

## Gotchas

- **BOOT button, not reset** — the QT2040 Trinkey enters the bootloader by
  holding BOOT while plugging in.
- **`boot.py` needs a power cycle** to take effect (soft reload won't do it); if
  you only see one CDC port, you haven't hard-reset since copying `boot.py`.
- **FAT32 `cp` error on macOS is benign** (extended-attributes); the UF2/file
  copy still succeeds.
- **CircuitPython 9.x**, not 10.x, until the target firmware is verified on 10.
- **Data vs console port:** commands only get `OK`/`ERR` replies on the data
  (higher-numbered) port; the console port is the REPL.
- If `CIRCUITPY` won't mount or is read-only, press the board's reset / replug;
  a clean remount restores host write access.
