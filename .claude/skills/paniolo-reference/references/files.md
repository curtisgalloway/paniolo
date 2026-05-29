# Files

## File: docs/dashboard.md
````markdown
# Combined dashboard

hdmicap's web UI serves a two-pane page: the HDMI video stream on top and an
xterm.js terminal below. The terminal connects over WebSocket to serialcap,
so the two daemons stay decoupled — hdmicap only references serialcap by URL.

---

## Starting the dashboard

```bash
paniolo video watch [target-machine]    # start hdmicap
paniolo serial watch [target-machine]   # start serialcap
paniolo console                  # open in the default browser
```

`paniolo console` verifies that both daemons are running and then opens the
dashboard. The page fetches the serialcap interface list and builds one
terminal pane per interface, displayed side by side in the serial panel (or
stacked in right-panel layout). With a single interface the panel looks the
same as before, with connection status in the top bar.

To open the dashboard pinned to a specific interface (single-pane mode):

```bash
paniolo console -i bmc
```

If a daemon isn't running, `console` prints which one is missing and the
command to start it.

---

## Features

**Live video** — MJPEG stream from the capture card, auto-refreshing.

**Serial terminal** — full xterm.js terminal connected to serialcap via
WebSocket. Keystrokes go to the serial port; output appears in the terminal.
xterm.js is vendored (not CDN) so the dashboard works on an isolated lab
network.

**Interface selector** — when serialcap is running multiple named interfaces,
a dropdown appears in the status bar. Selecting one reconnects the terminal
to that interface. The `?interface=<name>` URL parameter preselects one.

**Layout toggle** — a button in the status bar switches the terminal between
bottom (default, 40 vh) and right-panel (380 px fixed, video fills remaining
width) layouts. The choice persists in `localStorage`.

**OCR button** — triggers `GET /ocr` on the hdmicap daemon, which OCRs the
current frame via Apple Vision and displays the result. Requires
`visionocr` to be installed (`paniolo setup`).

---

## URL parameters

| Parameter | Effect |
|---|---|
| `?serial=<port>` | Connect terminal to serialcap on a non-default port |
| `?serialws=<url>` | Connect terminal to an explicit WebSocket URL |
| `?interface=<name>` | Preselect a named serial interface |

---

## Connecting the daemons

By default hdmicap connects the terminal to `ws://<host>:8724/stream` (the
default serialcap port). The `?serial=` and `?serialws=` parameters let you
point it at a different port or host if serialcap is running elsewhere.
````

## File: docs/hid.md
````markdown
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
````

## File: docs/netboot.md
````markdown
# Netboot

paniolo netboots a target by running a minimal DHCP + TFTP server over a
direct USB-Ethernet link. No router, switch, or upstream DHCP server is involved.

---

## Hardware setup

1. Plug a USB-to-Ethernet adapter into your Mac.
2. Connect an Ethernet cable from the adapter directly to the target's Ethernet
   port (no switch needed — modern adapters handle MDI/MDIX automatically).
3. Find the macOS interface name:

```bash
networksetup -listallhardwareports
```

---

## Target configuration

```bash
# Create or update a target
paniolo target set target-machine \
    --interface en3 \
    --tftp-root ~/src/fuchsia/pxe/tftp-root

# Show all configured targets
paniolo target show

# Show a specific target
paniolo target show target-machine

# Remove a target
paniolo target clear target-machine
```

Target config fields:

| Field | Default | Description |
|---|---|---|
| `--interface` | (required) | USB-Ethernet interface name (e.g. `en3`) |
| `--host-ip` | `192.168.99.1` | Static IP assigned to the interface; also the TFTP server address |
| `--tftp-root` | (none) | Directory whose contents are served over TFTP |
| `--ha-power-entity` | (none) | Home Assistant switch entity for power cycling |
| `--power-serial` | (none) | Serial interface name used for DTR power control |

---

## Starting and stopping

```bash
paniolo netboot start [target-machine]
paniolo netboot stop  [target-machine]
```

`start` assigns a static IP to the interface (`sudo ifconfig`), writes a
dnsmasq config, and launches dnsmasq + tftp-now as background daemons.
`stop` sends SIGTERM to both and clears the state file.

**No root for ports 67/69:** macOS 10.14+ allows binding to `0.0.0.0` on
privileged ports without root. paniolo binds to `0.0.0.0` and uses dnsmasq's
`--interface` flag for filtering. The only step requiring sudo is `ifconfig`
to assign the static IP — configure NOPASSWD sudo on the control Mac for
unattended agent use.

---

## Status and logs

```bash
paniolo netboot status [target-machine]      # running? interface? uptime?
paniolo netboot logs   [target-machine]      # tail the combined dnsmasq + tftp log
paniolo netboot logs -f [target-machine]     # follow
```

---

## Getting the TFTP root path

```bash
paniolo netboot tftp-root [target-machine]
```

Prints the bare TFTP root path, designed for shell substitution:

```bash
TFTP_ROOT=$(ssh control-mac "paniolo netboot tftp-root target-machine")
scp kernel_2712.img control-mac:"${TFTP_ROOT}/kernel_2712.img"
```

---

## Expected TFTP sequence for Raspberry Pi 5

When the Pi 5 EEPROM PXE client boots it walks this file request sequence.
The 404s are normal:

```
404  <serial>/<mac>/start.elf    ← Pi 5 doesn't need it; 404 expected
200  config.txt
200  bcm2712-rpi-5-b.dtb
200  kernel_2712.img              ← your boot shim or kernel
```

The TFTP root must contain at minimum `config.txt`, `bcm2712-rpi-5-b.dtb`,
and `kernel_2712.img`.

---

## dnsmasq configuration notes

paniolo sets both `siaddr` (BOOTP next-server, via `dhcp-boot`) and DHCP
option 66 (TFTP server name). The Pi 5 EEPROM reads option 66 preferentially,
but setting both ensures compatibility with older EEPROM firmware.

DNS is disabled (`port=0`). dnsmasq log output is redirected to the combined
log file at `~/.local/share/paniolo/<name>/netboot.log`.

---

## Runtime paths

| Purpose | Path |
|---|---|
| Generated dnsmasq config | `~/.local/share/paniolo/<name>/dnsmasq.conf` |
| Daemon state (PIDs, uptime) | `~/.local/share/paniolo/<name>/netboot.json` |
| Combined log | `~/.local/share/paniolo/<name>/netboot.log` |
````

## File: docs/power.md
````markdown
# Power control

paniolo provides two power control mechanisms:

- **DTR via FTDI** — drives the target's J2 power button header directly over the
  serial cable. Generic and wiring-based; no external services required.
- **`power_cycle_cmd`** — runs a configurable shell script. Write any script you
  like (HA switch, PDU relay, GPIO, etc.) and paniolo calls it.

---

## DTR power control (FTDI J2 wiring)

### Hardware wiring (Raspberry Pi 5)

```
FTDI DTR  →  1 kΩ  →  Pi J2 Pin 1 (PMIC_POW_BUTTON, pull-up inside DA9091)
FTDI GND  ←─────────  Pi J2 Pin 2
```

Optional power sense — reads whether the Pi is on:

```
Pi 3.3 V (header Pin 1)  →  1 kΩ  →  FTDI CTS# (or DSR#/DCD#/RI#)
                                             │
                                          10 kΩ
                                             │
                                            GND
```

The FTDI adapter should also provide the serial console connection for the
target. The DTR and sense signals share the same USB serial port.

### Setup

```bash
# Add a serial interface with power sense
paniolo serial setup target-machine \
    --name console \
    --device /dev/tty.usbserial-0001 \
    --baud 115200 \
    --power-sense cts       # whichever modem-control input is wired

# Tell the target which interface to use as the default for power commands
paniolo target set target-machine --power-serial console
```

### DTR commands

DTR commands live under `paniolo serial` since the DTR line is part of the
serial interface:

```bash
# Pulse DTR on the default power serial interface (200 ms)
paniolo serial dtr [target-machine]

# Explicit duration — short press signals the OS, long press hard-powers off
paniolo serial dtr --ms 200 [target-machine]   # soft press
paniolo serial dtr --ms 4000 [target-machine]  # hard power-off (PMIC)

# Target a specific interface with -i
paniolo serial dtr -i bmc --ms 200 [target-machine]

# Soft reset (convenience alias for a brief DTR pulse)
paniolo serial reset [target-machine]
paniolo serial reset -i console --ms 500 [target-machine]

# Show whether the target is powered on (requires sense signal + daemon running)
paniolo power-state [target-machine]
```

| Press duration | Effect |
|---|---|
| ≤ 500 ms | Soft power-button event — OS responds (graceful reboot or halt) |
| ≥ 3000 ms | Hard PMIC power-off (equivalent to holding the physical button) |

If no `-i` is given, DTR commands use `power_serial_interface` from the target
config. If that's not set, they fall back to the target's only configured
serial interface (or fail if multiple are configured without an explicit choice).

---

## power_cycle_cmd — script-based power control

For cases where DTR isn't wired (or where you want full mains control), set a
shell script on the target:

```bash
paniolo target set target-machine \
    --power-cycle-cmd /Users/you/.config/paniolo/scripts/power-cycle-target-machine.sh
```

The script can do anything — call a Home Assistant API, drive a PDU relay, toggle
a GPIO, etc. paniolo runs it and reports success or failure based on the exit code.

### Example: Home Assistant script

```bash
#!/usr/bin/env bash
set -euo pipefail
HA_URL="http://homeassistant.local:8123"
ENTITY="switch.pi_power_strip"
TOKEN="${HA_TOKEN:?HA_TOKEN not set}"

curl -sf -X POST "$HA_URL/api/services/switch/turn_off" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"entity_id\": \"$ENTITY\"}"

sleep 10

curl -sf -X POST "$HA_URL/api/services/switch/turn_on" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"entity_id\": \"$ENTITY\"}"
```

The script reads `HA_TOKEN` from the environment — never hardcode it in the
script or the paniolo config. A few ways to inject it at call time:

```bash
# 1Password CLI (op): reads secrets from a .env file or vault and injects them
#    .env file format:  HA_TOKEN=op://vault/item/field
op run --env-file .env -- paniolo power-cycle target-machine

# direnv: place "export HA_TOKEN=..." in an .envrc in your working directory;
#    direnv loads it automatically when you cd there
paniolo power-cycle target-machine   # HA_TOKEN already in environment via direnv

# Inline export (quick/manual use — clears from shell history if prefixed with space)
HA_TOKEN="$(cat ~/.secrets/ha_token)" paniolo power-cycle target-machine

# SSH with env forwarding (when running from a remote agent host)
ssh -o SendEnv=HA_TOKEN control-mac "paniolo power-cycle target-machine"
# (requires AcceptEnv HA_TOKEN in sshd_config on control-mac)
```

### Command

```bash
paniolo power-cycle [target-machine]
```

Runs `power_cycle_cmd` and exits with its return code. No built-in timing or
sense-signal logic — the script is responsible for the full sequence.
````

## File: docs/serial.md
````markdown
# Serial console

paniolo supports two serial console modes: interactive (direct terminal via
`tio`) and daemon-backed (serialcap, with a timestamped rolling log and
WebSocket dashboard terminal).

---

## Setup

Add a serial interface to a target:

```bash
paniolo serial setup target-machine \
    --name console \
    --device /dev/tty.usbserial-0001 \
    --baud 115200

# Optional: also wire a power sense signal on this interface (see power.md)
paniolo serial setup target-machine \
    --name console \
    --device /dev/tty.usbserial-0001 \
    --baud 115200 \
    --power-sense cts
```

A target can have several named interfaces (e.g. `console`, `bmc`). Remove
one with:

```bash
paniolo serial remove target-machine --name console
```

List detected serial devices:

```bash
paniolo serial devices
```

Show a target's configured interfaces:

```bash
paniolo serial show [target-machine]
```

---

## Interactive mode

```bash
paniolo serial connect [-i console] [target-machine]
```

Opens a direct `tio` terminal session (foreground). Exit with Ctrl+T Q.
This mode holds the serial port exclusively — it conflicts with the daemon.

---

## Daemon mode

The serialcap daemon owns all configured interfaces for a target, provides
a WebSocket terminal for the [dashboard](dashboard.md), and writes a
timestamped rolling capture log.

```bash
paniolo serial watch [target-machine]   # start serialcap daemon
paniolo serial stop  [target-machine]   # stop it
```

A target with multiple serial interfaces starts a single daemon that manages
all of them. The daemon's URL is printed on start — it also appears in the
dashboard.

---

## Querying captured output

The capture log persists across daemon restarts. `serialcap log` reads it
directly — no daemon round-trip needed.

```bash
# Tail the last 50 lines from the default interface
paniolo serial log -i console --tail 50 [target-machine]

# Stream new lines as they arrive (poll mode)
paniolo serial log -i console --since [target-machine]

# Specific sequence number range
paniolo serial log -i console --from 1000 --to 1200 [target-machine]

# Keep ANSI escape codes (stripped by default)
paniolo serial log -i console --raw [target-machine]

# JSON output (includes timestamp and sequence number)
paniolo serial log -i console --json [target-machine]
```

Each captured line carries a monotonic sequence number (`seq`, stable across
log rotation) and a UTC timestamp (`ts_ms`). The `--since` flag polls for lines
with `seq` greater than the last seen value — safe to re-run from scripts.

Each interface writes to its own capture directory so logs never conflate:
`$TMPDIR/serialcap/capture/<name>/serial.jsonl`.

---

## Integration with the video dashboard

`paniolo console` opens the combined hdmicap dashboard in a browser. That page
embeds an xterm.js terminal that connects cross-port to serialcap's WebSocket
(`/stream`). For the terminal to work, both daemons must be running:

```bash
paniolo video watch [target-machine]    # hdmicap — serves the page
paniolo serial watch [target-machine]   # serialcap — backs the terminal
paniolo console [-i <interface>] # open in browser
```

When the dashboard page loads, serialcap replays up to 64 KB of scrollback
immediately on WebSocket connect, so the terminal isn't blank mid-session.
Keystrokes typed in the terminal are forwarded to the serial port in real time.

serialcap's HTTP responses carry a permissive CORS header so the cross-port
fetch from the hdmicap page is allowed without any proxy.

When serialcap owns multiple interfaces, the dashboard shows one terminal
pane per interface side by side. Use `paniolo console -i <name>` to open in
single-pane mode pinned to one interface.

See [dashboard.md](dashboard.md) for layout options and other URL parameters.

---

## DTR power control (FTDI wiring)

When an FTDI adapter is wired to the target's J2 power button header, the
same interface used for serial can also drive the power button via the DTR
signal. The DTR commands live under `paniolo serial`:

```bash
paniolo serial dtr [--ms 200] [-i console] [target-machine]   # pulse DTR
paniolo serial reset [-i console] [target-machine]             # soft reset (200 ms)
```

See [power.md](power.md) for wiring diagrams, `power_cycle_cmd` setup, and
a full command reference.

---

## Runtime paths

| Purpose | Path |
|---|---|
| serialcap discovery | `$TMPDIR/serialcap/daemon.json` (`{pid, port, interfaces:[...]}`) |
| serialcap advisory lock | `$TMPDIR/serialcap/daemon.lock` |
| Capture log (per interface) | `$TMPDIR/serialcap/capture/<name>/serial.jsonl(.1..)` |
| Pending (unterminated) line | `$TMPDIR/serialcap/capture/<name>/pending.json` |
````

## File: docs/video.md
````markdown
# Video capture

paniolo drives `hdmicap`, a Rust warm-stream daemon that keeps a USB HDMI
capture device open continuously and serves the current frame over HTTP. This
avoids the multi-second reopen latency you'd get by running ffmpeg per capture.

---

## Hardware

Any USB HDMI capture card that presents as a UVC device (V4L2 / AVFoundation).
Tested with the MS2109-based cards (e.g. generic "USB3.0 HDMI Capture" dongles).

Connect the target's HDMI output to the capture card, then the card to the Mac.

---

## Setup

```bash
# Detect available capture devices
paniolo video devices

# Save which device to use
paniolo video setup
# or specify explicitly:
paniolo video setup --device "USB Video"
```

The device name is saved to `~/.config/paniolo/video.toml` and used by
subsequent commands. When only one non-built-in camera is present, `setup`
auto-selects it.

---

## Starting and stopping the daemon

```bash
paniolo video watch [target-machine]   # start hdmicap daemon for a target
paniolo video stop  [target-machine]   # stop it
paniolo video show  [target-machine]   # show daemon URL and status
```

`watch` starts `hdmicap daemon` detached and polls for startup. The daemon URL
is printed — open it in a browser for the live preview.

---

## Capturing frames

```bash
paniolo video shot [target-machine]           # save a screenshot to a temp file
paniolo video shot [target-machine] -o out.png  # save to a specific path
paniolo video preview [target-machine]        # open the live MJPEG stream in a browser
```

`shot` fetches a single PNG-encoded frame from the running daemon via
`hdmicap shot`.

---

## OCR (Apple Vision)

```bash
paniolo video read [target-machine]
```

Fetches the current frame and runs Apple Vision's `VNRecognizeTextRequest` on
it, printing the recognized text to stdout. On-device — no network, no model
download required.

`paniolo setup` compiles `ocr/visionocr.swift` into `~/.cargo/bin/visionocr`
with `swiftc`. The OCR tool is also accessible from the [web dashboard](dashboard.md)
via the OCR button.

**OCR tuning notes:**
- `.fast` recognition level is used (not `.accurate` — the latter misses small
  console text entirely; it's tuned for natural document text).
- The frame is 2×-upscaled and black-padded before recognition to improve
  accuracy on thin console fonts.
- `minimumTextHeight` is lowered from the default to catch small terminal text.

---

## Runtime paths

| Purpose | Path |
|---|---|
| Video config | `~/.config/paniolo/video.toml` |
| hdmicap discovery | `$TMPDIR/hdmicap/daemon.json` (`{pid, port}`) |
| hdmicap advisory lock | `$TMPDIR/hdmicap/daemon.lock` |
````

## File: hdmicap/assets/xterm-addon-fit.js
````javascript
!function(e,t){"object"==typeof exports&&"object"==typeof module?module.exports=t():"function"==typeof define&&define.amd?define([],t):"object"==typeof exports?exports.FitAddon=t():e.FitAddon=t()}(self,(()=>(()=>{"use strict";var e={};return(()=>{var t=e;Object.defineProperty(t,"__esModule",{value:!0}),t.FitAddon=void 0,t.FitAddon=class{activate(e){this._terminal=e}dispose(){}fit(){const e=this.proposeDimensions();if(!e||!this._terminal||isNaN(e.cols)||isNaN(e.rows))return;const t=this._terminal._core;this._terminal.rows===e.rows&&this._terminal.cols===e.cols||(t._renderService.clear(),this._terminal.resize(e.cols,e.rows))}proposeDimensions(){if(!this._terminal)return;if(!this._terminal.element||!this._terminal.element.parentElement)return;const e=this._terminal._core,t=e._renderService.dimensions;if(0===t.css.cell.width||0===t.css.cell.height)return;const r=0===this._terminal.options.scrollback?0:e.viewport.scrollBarWidth,i=window.getComputedStyle(this._terminal.element.parentElement),o=parseInt(i.getPropertyValue("height")),s=Math.max(0,parseInt(i.getPropertyValue("width"))),n=window.getComputedStyle(this._terminal.element),l=o-(parseInt(n.getPropertyValue("padding-top"))+parseInt(n.getPropertyValue("padding-bottom"))),a=s-(parseInt(n.getPropertyValue("padding-right"))+parseInt(n.getPropertyValue("padding-left")))-r;return{cols:Math.max(2,Math.floor(a/t.css.cell.width)),rows:Math.max(1,Math.floor(l/t.css.cell.height))}}}})(),e})()));
//# sourceMappingURL=xterm-addon-fit.js.map
````

## File: hdmicap/assets/xterm.css
````css
/**
 * Copyright (c) 2014 The xterm.js authors. All rights reserved.
 * Copyright (c) 2012-2013, Christopher Jeffrey (MIT License)
 * https://github.com/chjj/term.js
 * @license MIT
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in
 * all copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
 * THE SOFTWARE.
 *
 * Originally forked from (with the author's permission):
 *   Fabrice Bellard's javascript vt100 for jslinux:
 *   http://bellard.org/jslinux/
 *   Copyright (c) 2011 Fabrice Bellard
 *   The original design remains. The terminal itself
 *   has been extended to include xterm CSI codes, among
 *   other features.
 */

/**
 *  Default styles for xterm.js
 */

.xterm {
    cursor: text;
    position: relative;
    user-select: none;
    -ms-user-select: none;
    -webkit-user-select: none;
}

.xterm.focus,
.xterm:focus {
    outline: none;
}

.xterm .xterm-helpers {
    position: absolute;
    top: 0;
    /**
     * The z-index of the helpers must be higher than the canvases in order for
     * IMEs to appear on top.
     */
    z-index: 5;
}

.xterm .xterm-helper-textarea {
    padding: 0;
    border: 0;
    margin: 0;
    /* Move textarea out of the screen to the far left, so that the cursor is not visible */
    position: absolute;
    opacity: 0;
    left: -9999em;
    top: 0;
    width: 0;
    height: 0;
    z-index: -5;
    /** Prevent wrapping so the IME appears against the textarea at the correct position */
    white-space: nowrap;
    overflow: hidden;
    resize: none;
}

.xterm .composition-view {
    /* TODO: Composition position got messed up somewhere */
    background: #000;
    color: #FFF;
    display: none;
    position: absolute;
    white-space: nowrap;
    z-index: 1;
}

.xterm .composition-view.active {
    display: block;
}

.xterm .xterm-viewport {
    /* On OS X this is required in order for the scroll bar to appear fully opaque */
    background-color: #000;
    overflow-y: scroll;
    cursor: default;
    position: absolute;
    right: 0;
    left: 0;
    top: 0;
    bottom: 0;
}

.xterm .xterm-screen {
    position: relative;
}

.xterm .xterm-screen canvas {
    position: absolute;
    left: 0;
    top: 0;
}

.xterm .xterm-scroll-area {
    visibility: hidden;
}

.xterm-char-measure-element {
    display: inline-block;
    visibility: hidden;
    position: absolute;
    top: 0;
    left: -9999em;
    line-height: normal;
}

.xterm.enable-mouse-events {
    /* When mouse events are enabled (eg. tmux), revert to the standard pointer cursor */
    cursor: default;
}

.xterm.xterm-cursor-pointer,
.xterm .xterm-cursor-pointer {
    cursor: pointer;
}

.xterm.column-select.focus {
    /* Column selection mode */
    cursor: crosshair;
}

.xterm .xterm-accessibility,
.xterm .xterm-message {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    right: 0;
    z-index: 10;
    color: transparent;
    pointer-events: none;
}

.xterm .live-region {
    position: absolute;
    left: -9999px;
    width: 1px;
    height: 1px;
    overflow: hidden;
}

.xterm-dim {
    /* Dim should not apply to background, so the opacity of the foreground color is applied
     * explicitly in the generated class and reset to 1 here */
    opacity: 1 !important;
}

.xterm-underline-1 { text-decoration: underline; }
.xterm-underline-2 { text-decoration: double underline; }
.xterm-underline-3 { text-decoration: wavy underline; }
.xterm-underline-4 { text-decoration: dotted underline; }
.xterm-underline-5 { text-decoration: dashed underline; }

.xterm-overline {
    text-decoration: overline;
}

.xterm-overline.xterm-underline-1 { text-decoration: overline underline; }
.xterm-overline.xterm-underline-2 { text-decoration: overline double underline; }
.xterm-overline.xterm-underline-3 { text-decoration: overline wavy underline; }
.xterm-overline.xterm-underline-4 { text-decoration: overline dotted underline; }
.xterm-overline.xterm-underline-5 { text-decoration: overline dashed underline; }

.xterm-strikethrough {
    text-decoration: line-through;
}

.xterm-screen .xterm-decoration-container .xterm-decoration {
	z-index: 6;
	position: absolute;
}

.xterm-screen .xterm-decoration-container .xterm-decoration.xterm-decoration-top-layer {
	z-index: 7;
}

.xterm-decoration-overview-ruler {
    z-index: 8;
    position: absolute;
    top: 0;
    right: 0;
    pointer-events: none;
}

.xterm-decoration-top {
    z-index: 2;
    position: relative;
}
````

## File: hdmicap/assets/xterm.js
````javascript
!function(e,t){if("object"==typeof exports&&"object"==typeof module)module.exports=t();else if("function"==typeof define&&define.amd)define([],t);else{var i=t();for(var s in i)("object"==typeof exports?exports:e)[s]=i[s]}}(self,(()=>(()=>{"use strict";var e={4567:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.AccessibilityManager=void 0;const n=i(9042),o=i(6114),a=i(9924),h=i(844),c=i(5596),l=i(4725),d=i(3656);let _=t.AccessibilityManager=class extends h.Disposable{constructor(e,t){super(),this._terminal=e,this._renderService=t,this._liveRegionLineCount=0,this._charsToConsume=[],this._charsToAnnounce="",this._accessibilityContainer=document.createElement("div"),this._accessibilityContainer.classList.add("xterm-accessibility"),this._rowContainer=document.createElement("div"),this._rowContainer.setAttribute("role","list"),this._rowContainer.classList.add("xterm-accessibility-tree"),this._rowElements=[];for(let e=0;e<this._terminal.rows;e++)this._rowElements[e]=this._createAccessibilityTreeNode(),this._rowContainer.appendChild(this._rowElements[e]);if(this._topBoundaryFocusListener=e=>this._handleBoundaryFocus(e,0),this._bottomBoundaryFocusListener=e=>this._handleBoundaryFocus(e,1),this._rowElements[0].addEventListener("focus",this._topBoundaryFocusListener),this._rowElements[this._rowElements.length-1].addEventListener("focus",this._bottomBoundaryFocusListener),this._refreshRowsDimensions(),this._accessibilityContainer.appendChild(this._rowContainer),this._liveRegion=document.createElement("div"),this._liveRegion.classList.add("live-region"),this._liveRegion.setAttribute("aria-live","assertive"),this._accessibilityContainer.appendChild(this._liveRegion),this._liveRegionDebouncer=this.register(new a.TimeBasedDebouncer(this._renderRows.bind(this))),!this._terminal.element)throw new Error("Cannot enable accessibility before Terminal.open");this._terminal.element.insertAdjacentElement("afterbegin",this._accessibilityContainer),this.register(this._terminal.onResize((e=>this._handleResize(e.rows)))),this.register(this._terminal.onRender((e=>this._refreshRows(e.start,e.end)))),this.register(this._terminal.onScroll((()=>this._refreshRows()))),this.register(this._terminal.onA11yChar((e=>this._handleChar(e)))),this.register(this._terminal.onLineFeed((()=>this._handleChar("\n")))),this.register(this._terminal.onA11yTab((e=>this._handleTab(e)))),this.register(this._terminal.onKey((e=>this._handleKey(e.key)))),this.register(this._terminal.onBlur((()=>this._clearLiveRegion()))),this.register(this._renderService.onDimensionsChange((()=>this._refreshRowsDimensions()))),this._screenDprMonitor=new c.ScreenDprMonitor(window),this.register(this._screenDprMonitor),this._screenDprMonitor.setListener((()=>this._refreshRowsDimensions())),this.register((0,d.addDisposableDomListener)(window,"resize",(()=>this._refreshRowsDimensions()))),this._refreshRows(),this.register((0,h.toDisposable)((()=>{this._accessibilityContainer.remove(),this._rowElements.length=0})))}_handleTab(e){for(let t=0;t<e;t++)this._handleChar(" ")}_handleChar(e){this._liveRegionLineCount<21&&(this._charsToConsume.length>0?this._charsToConsume.shift()!==e&&(this._charsToAnnounce+=e):this._charsToAnnounce+=e,"\n"===e&&(this._liveRegionLineCount++,21===this._liveRegionLineCount&&(this._liveRegion.textContent+=n.tooMuchOutput)),o.isMac&&this._liveRegion.textContent&&this._liveRegion.textContent.length>0&&!this._liveRegion.parentNode&&setTimeout((()=>{this._accessibilityContainer.appendChild(this._liveRegion)}),0))}_clearLiveRegion(){this._liveRegion.textContent="",this._liveRegionLineCount=0,o.isMac&&this._liveRegion.remove()}_handleKey(e){this._clearLiveRegion(),/\p{Control}/u.test(e)||this._charsToConsume.push(e)}_refreshRows(e,t){this._liveRegionDebouncer.refresh(e,t,this._terminal.rows)}_renderRows(e,t){const i=this._terminal.buffer,s=i.lines.length.toString();for(let r=e;r<=t;r++){const e=i.translateBufferLineToString(i.ydisp+r,!0),t=(i.ydisp+r+1).toString(),n=this._rowElements[r];n&&(0===e.length?n.innerText=" ":n.textContent=e,n.setAttribute("aria-posinset",t),n.setAttribute("aria-setsize",s))}this._announceCharacters()}_announceCharacters(){0!==this._charsToAnnounce.length&&(this._liveRegion.textContent+=this._charsToAnnounce,this._charsToAnnounce="")}_handleBoundaryFocus(e,t){const i=e.target,s=this._rowElements[0===t?1:this._rowElements.length-2];if(i.getAttribute("aria-posinset")===(0===t?"1":`${this._terminal.buffer.lines.length}`))return;if(e.relatedTarget!==s)return;let r,n;if(0===t?(r=i,n=this._rowElements.pop(),this._rowContainer.removeChild(n)):(r=this._rowElements.shift(),n=i,this._rowContainer.removeChild(r)),r.removeEventListener("focus",this._topBoundaryFocusListener),n.removeEventListener("focus",this._bottomBoundaryFocusListener),0===t){const e=this._createAccessibilityTreeNode();this._rowElements.unshift(e),this._rowContainer.insertAdjacentElement("afterbegin",e)}else{const e=this._createAccessibilityTreeNode();this._rowElements.push(e),this._rowContainer.appendChild(e)}this._rowElements[0].addEventListener("focus",this._topBoundaryFocusListener),this._rowElements[this._rowElements.length-1].addEventListener("focus",this._bottomBoundaryFocusListener),this._terminal.scrollLines(0===t?-1:1),this._rowElements[0===t?1:this._rowElements.length-2].focus(),e.preventDefault(),e.stopImmediatePropagation()}_handleResize(e){this._rowElements[this._rowElements.length-1].removeEventListener("focus",this._bottomBoundaryFocusListener);for(let e=this._rowContainer.children.length;e<this._terminal.rows;e++)this._rowElements[e]=this._createAccessibilityTreeNode(),this._rowContainer.appendChild(this._rowElements[e]);for(;this._rowElements.length>e;)this._rowContainer.removeChild(this._rowElements.pop());this._rowElements[this._rowElements.length-1].addEventListener("focus",this._bottomBoundaryFocusListener),this._refreshRowsDimensions()}_createAccessibilityTreeNode(){const e=document.createElement("div");return e.setAttribute("role","listitem"),e.tabIndex=-1,this._refreshRowDimensions(e),e}_refreshRowsDimensions(){if(this._renderService.dimensions.css.cell.height){this._accessibilityContainer.style.width=`${this._renderService.dimensions.css.canvas.width}px`,this._rowElements.length!==this._terminal.rows&&this._handleResize(this._terminal.rows);for(let e=0;e<this._terminal.rows;e++)this._refreshRowDimensions(this._rowElements[e])}}_refreshRowDimensions(e){e.style.height=`${this._renderService.dimensions.css.cell.height}px`}};t.AccessibilityManager=_=s([r(1,l.IRenderService)],_)},3614:(e,t)=>{function i(e){return e.replace(/\r?\n/g,"\r")}function s(e,t){return t?"[200~"+e+"[201~":e}function r(e,t,r,n){e=s(e=i(e),r.decPrivateModes.bracketedPasteMode&&!0!==n.rawOptions.ignoreBracketedPasteMode),r.triggerDataEvent(e,!0),t.value=""}function n(e,t,i){const s=i.getBoundingClientRect(),r=e.clientX-s.left-10,n=e.clientY-s.top-10;t.style.width="20px",t.style.height="20px",t.style.left=`${r}px`,t.style.top=`${n}px`,t.style.zIndex="1000",t.focus()}Object.defineProperty(t,"__esModule",{value:!0}),t.rightClickHandler=t.moveTextAreaUnderMouseCursor=t.paste=t.handlePasteEvent=t.copyHandler=t.bracketTextForPaste=t.prepareTextForTerminal=void 0,t.prepareTextForTerminal=i,t.bracketTextForPaste=s,t.copyHandler=function(e,t){e.clipboardData&&e.clipboardData.setData("text/plain",t.selectionText),e.preventDefault()},t.handlePasteEvent=function(e,t,i,s){e.stopPropagation(),e.clipboardData&&r(e.clipboardData.getData("text/plain"),t,i,s)},t.paste=r,t.moveTextAreaUnderMouseCursor=n,t.rightClickHandler=function(e,t,i,s,r){n(e,t,i),r&&s.rightClickSelect(e),t.value=s.selectionText,t.select()}},7239:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.ColorContrastCache=void 0;const s=i(1505);t.ColorContrastCache=class{constructor(){this._color=new s.TwoKeyMap,this._css=new s.TwoKeyMap}setCss(e,t,i){this._css.set(e,t,i)}getCss(e,t){return this._css.get(e,t)}setColor(e,t,i){this._color.set(e,t,i)}getColor(e,t){return this._color.get(e,t)}clear(){this._color.clear(),this._css.clear()}}},3656:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.addDisposableDomListener=void 0,t.addDisposableDomListener=function(e,t,i,s){e.addEventListener(t,i,s);let r=!1;return{dispose:()=>{r||(r=!0,e.removeEventListener(t,i,s))}}}},6465:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.Linkifier2=void 0;const n=i(3656),o=i(8460),a=i(844),h=i(2585);let c=t.Linkifier2=class extends a.Disposable{get currentLink(){return this._currentLink}constructor(e){super(),this._bufferService=e,this._linkProviders=[],this._linkCacheDisposables=[],this._isMouseOut=!0,this._wasResized=!1,this._activeLine=-1,this._onShowLinkUnderline=this.register(new o.EventEmitter),this.onShowLinkUnderline=this._onShowLinkUnderline.event,this._onHideLinkUnderline=this.register(new o.EventEmitter),this.onHideLinkUnderline=this._onHideLinkUnderline.event,this.register((0,a.getDisposeArrayDisposable)(this._linkCacheDisposables)),this.register((0,a.toDisposable)((()=>{this._lastMouseEvent=void 0}))),this.register(this._bufferService.onResize((()=>{this._clearCurrentLink(),this._wasResized=!0})))}registerLinkProvider(e){return this._linkProviders.push(e),{dispose:()=>{const t=this._linkProviders.indexOf(e);-1!==t&&this._linkProviders.splice(t,1)}}}attachToDom(e,t,i){this._element=e,this._mouseService=t,this._renderService=i,this.register((0,n.addDisposableDomListener)(this._element,"mouseleave",(()=>{this._isMouseOut=!0,this._clearCurrentLink()}))),this.register((0,n.addDisposableDomListener)(this._element,"mousemove",this._handleMouseMove.bind(this))),this.register((0,n.addDisposableDomListener)(this._element,"mousedown",this._handleMouseDown.bind(this))),this.register((0,n.addDisposableDomListener)(this._element,"mouseup",this._handleMouseUp.bind(this)))}_handleMouseMove(e){if(this._lastMouseEvent=e,!this._element||!this._mouseService)return;const t=this._positionFromMouseEvent(e,this._element,this._mouseService);if(!t)return;this._isMouseOut=!1;const i=e.composedPath();for(let e=0;e<i.length;e++){const t=i[e];if(t.classList.contains("xterm"))break;if(t.classList.contains("xterm-hover"))return}this._lastBufferCell&&t.x===this._lastBufferCell.x&&t.y===this._lastBufferCell.y||(this._handleHover(t),this._lastBufferCell=t)}_handleHover(e){if(this._activeLine!==e.y||this._wasResized)return this._clearCurrentLink(),this._askForLink(e,!1),void(this._wasResized=!1);this._currentLink&&this._linkAtPosition(this._currentLink.link,e)||(this._clearCurrentLink(),this._askForLink(e,!0))}_askForLink(e,t){var i,s;this._activeProviderReplies&&t||(null===(i=this._activeProviderReplies)||void 0===i||i.forEach((e=>{null==e||e.forEach((e=>{e.link.dispose&&e.link.dispose()}))})),this._activeProviderReplies=new Map,this._activeLine=e.y);let r=!1;for(const[i,n]of this._linkProviders.entries())t?(null===(s=this._activeProviderReplies)||void 0===s?void 0:s.get(i))&&(r=this._checkLinkProviderResult(i,e,r)):n.provideLinks(e.y,(t=>{var s,n;if(this._isMouseOut)return;const o=null==t?void 0:t.map((e=>({link:e})));null===(s=this._activeProviderReplies)||void 0===s||s.set(i,o),r=this._checkLinkProviderResult(i,e,r),(null===(n=this._activeProviderReplies)||void 0===n?void 0:n.size)===this._linkProviders.length&&this._removeIntersectingLinks(e.y,this._activeProviderReplies)}))}_removeIntersectingLinks(e,t){const i=new Set;for(let s=0;s<t.size;s++){const r=t.get(s);if(r)for(let t=0;t<r.length;t++){const s=r[t],n=s.link.range.start.y<e?0:s.link.range.start.x,o=s.link.range.end.y>e?this._bufferService.cols:s.link.range.end.x;for(let e=n;e<=o;e++){if(i.has(e)){r.splice(t--,1);break}i.add(e)}}}}_checkLinkProviderResult(e,t,i){var s;if(!this._activeProviderReplies)return i;const r=this._activeProviderReplies.get(e);let n=!1;for(let t=0;t<e;t++)this._activeProviderReplies.has(t)&&!this._activeProviderReplies.get(t)||(n=!0);if(!n&&r){const e=r.find((e=>this._linkAtPosition(e.link,t)));e&&(i=!0,this._handleNewLink(e))}if(this._activeProviderReplies.size===this._linkProviders.length&&!i)for(let e=0;e<this._activeProviderReplies.size;e++){const r=null===(s=this._activeProviderReplies.get(e))||void 0===s?void 0:s.find((e=>this._linkAtPosition(e.link,t)));if(r){i=!0,this._handleNewLink(r);break}}return i}_handleMouseDown(){this._mouseDownLink=this._currentLink}_handleMouseUp(e){if(!this._element||!this._mouseService||!this._currentLink)return;const t=this._positionFromMouseEvent(e,this._element,this._mouseService);t&&this._mouseDownLink===this._currentLink&&this._linkAtPosition(this._currentLink.link,t)&&this._currentLink.link.activate(e,this._currentLink.link.text)}_clearCurrentLink(e,t){this._element&&this._currentLink&&this._lastMouseEvent&&(!e||!t||this._currentLink.link.range.start.y>=e&&this._currentLink.link.range.end.y<=t)&&(this._linkLeave(this._element,this._currentLink.link,this._lastMouseEvent),this._currentLink=void 0,(0,a.disposeArray)(this._linkCacheDisposables))}_handleNewLink(e){if(!this._element||!this._lastMouseEvent||!this._mouseService)return;const t=this._positionFromMouseEvent(this._lastMouseEvent,this._element,this._mouseService);t&&this._linkAtPosition(e.link,t)&&(this._currentLink=e,this._currentLink.state={decorations:{underline:void 0===e.link.decorations||e.link.decorations.underline,pointerCursor:void 0===e.link.decorations||e.link.decorations.pointerCursor},isHovered:!0},this._linkHover(this._element,e.link,this._lastMouseEvent),e.link.decorations={},Object.defineProperties(e.link.decorations,{pointerCursor:{get:()=>{var e,t;return null===(t=null===(e=this._currentLink)||void 0===e?void 0:e.state)||void 0===t?void 0:t.decorations.pointerCursor},set:e=>{var t,i;(null===(t=this._currentLink)||void 0===t?void 0:t.state)&&this._currentLink.state.decorations.pointerCursor!==e&&(this._currentLink.state.decorations.pointerCursor=e,this._currentLink.state.isHovered&&(null===(i=this._element)||void 0===i||i.classList.toggle("xterm-cursor-pointer",e)))}},underline:{get:()=>{var e,t;return null===(t=null===(e=this._currentLink)||void 0===e?void 0:e.state)||void 0===t?void 0:t.decorations.underline},set:t=>{var i,s,r;(null===(i=this._currentLink)||void 0===i?void 0:i.state)&&(null===(r=null===(s=this._currentLink)||void 0===s?void 0:s.state)||void 0===r?void 0:r.decorations.underline)!==t&&(this._currentLink.state.decorations.underline=t,this._currentLink.state.isHovered&&this._fireUnderlineEvent(e.link,t))}}}),this._renderService&&this._linkCacheDisposables.push(this._renderService.onRenderedViewportChange((e=>{if(!this._currentLink)return;const t=0===e.start?0:e.start+1+this._bufferService.buffer.ydisp,i=this._bufferService.buffer.ydisp+1+e.end;if(this._currentLink.link.range.start.y>=t&&this._currentLink.link.range.end.y<=i&&(this._clearCurrentLink(t,i),this._lastMouseEvent&&this._element)){const e=this._positionFromMouseEvent(this._lastMouseEvent,this._element,this._mouseService);e&&this._askForLink(e,!1)}}))))}_linkHover(e,t,i){var s;(null===(s=this._currentLink)||void 0===s?void 0:s.state)&&(this._currentLink.state.isHovered=!0,this._currentLink.state.decorations.underline&&this._fireUnderlineEvent(t,!0),this._currentLink.state.decorations.pointerCursor&&e.classList.add("xterm-cursor-pointer")),t.hover&&t.hover(i,t.text)}_fireUnderlineEvent(e,t){const i=e.range,s=this._bufferService.buffer.ydisp,r=this._createLinkUnderlineEvent(i.start.x-1,i.start.y-s-1,i.end.x,i.end.y-s-1,void 0);(t?this._onShowLinkUnderline:this._onHideLinkUnderline).fire(r)}_linkLeave(e,t,i){var s;(null===(s=this._currentLink)||void 0===s?void 0:s.state)&&(this._currentLink.state.isHovered=!1,this._currentLink.state.decorations.underline&&this._fireUnderlineEvent(t,!1),this._currentLink.state.decorations.pointerCursor&&e.classList.remove("xterm-cursor-pointer")),t.leave&&t.leave(i,t.text)}_linkAtPosition(e,t){const i=e.range.start.y*this._bufferService.cols+e.range.start.x,s=e.range.end.y*this._bufferService.cols+e.range.end.x,r=t.y*this._bufferService.cols+t.x;return i<=r&&r<=s}_positionFromMouseEvent(e,t,i){const s=i.getCoords(e,t,this._bufferService.cols,this._bufferService.rows);if(s)return{x:s[0],y:s[1]+this._bufferService.buffer.ydisp}}_createLinkUnderlineEvent(e,t,i,s,r){return{x1:e,y1:t,x2:i,y2:s,cols:this._bufferService.cols,fg:r}}};t.Linkifier2=c=s([r(0,h.IBufferService)],c)},9042:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.tooMuchOutput=t.promptLabel=void 0,t.promptLabel="Terminal input",t.tooMuchOutput="Too much output to announce, navigate to rows manually to read"},3730:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.OscLinkProvider=void 0;const n=i(511),o=i(2585);let a=t.OscLinkProvider=class{constructor(e,t,i){this._bufferService=e,this._optionsService=t,this._oscLinkService=i}provideLinks(e,t){var i;const s=this._bufferService.buffer.lines.get(e-1);if(!s)return void t(void 0);const r=[],o=this._optionsService.rawOptions.linkHandler,a=new n.CellData,c=s.getTrimmedLength();let l=-1,d=-1,_=!1;for(let t=0;t<c;t++)if(-1!==d||s.hasContent(t)){if(s.loadCell(t,a),a.hasExtendedAttrs()&&a.extended.urlId){if(-1===d){d=t,l=a.extended.urlId;continue}_=a.extended.urlId!==l}else-1!==d&&(_=!0);if(_||-1!==d&&t===c-1){const s=null===(i=this._oscLinkService.getLinkData(l))||void 0===i?void 0:i.uri;if(s){const i={start:{x:d+1,y:e},end:{x:t+(_||t!==c-1?0:1),y:e}};let n=!1;if(!(null==o?void 0:o.allowNonHttpProtocols))try{const e=new URL(s);["http:","https:"].includes(e.protocol)||(n=!0)}catch(e){n=!0}n||r.push({text:s,range:i,activate:(e,t)=>o?o.activate(e,t,i):h(0,t),hover:(e,t)=>{var s;return null===(s=null==o?void 0:o.hover)||void 0===s?void 0:s.call(o,e,t,i)},leave:(e,t)=>{var s;return null===(s=null==o?void 0:o.leave)||void 0===s?void 0:s.call(o,e,t,i)}})}_=!1,a.hasExtendedAttrs()&&a.extended.urlId?(d=t,l=a.extended.urlId):(d=-1,l=-1)}}t(r)}};function h(e,t){if(confirm(`Do you want to navigate to ${t}?\n\nWARNING: This link could potentially be dangerous`)){const e=window.open();if(e){try{e.opener=null}catch(e){}e.location.href=t}else console.warn("Opening link blocked as opener could not be cleared")}}t.OscLinkProvider=a=s([r(0,o.IBufferService),r(1,o.IOptionsService),r(2,o.IOscLinkService)],a)},6193:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.RenderDebouncer=void 0,t.RenderDebouncer=class{constructor(e,t){this._parentWindow=e,this._renderCallback=t,this._refreshCallbacks=[]}dispose(){this._animationFrame&&(this._parentWindow.cancelAnimationFrame(this._animationFrame),this._animationFrame=void 0)}addRefreshCallback(e){return this._refreshCallbacks.push(e),this._animationFrame||(this._animationFrame=this._parentWindow.requestAnimationFrame((()=>this._innerRefresh()))),this._animationFrame}refresh(e,t,i){this._rowCount=i,e=void 0!==e?e:0,t=void 0!==t?t:this._rowCount-1,this._rowStart=void 0!==this._rowStart?Math.min(this._rowStart,e):e,this._rowEnd=void 0!==this._rowEnd?Math.max(this._rowEnd,t):t,this._animationFrame||(this._animationFrame=this._parentWindow.requestAnimationFrame((()=>this._innerRefresh())))}_innerRefresh(){if(this._animationFrame=void 0,void 0===this._rowStart||void 0===this._rowEnd||void 0===this._rowCount)return void this._runRefreshCallbacks();const e=Math.max(this._rowStart,0),t=Math.min(this._rowEnd,this._rowCount-1);this._rowStart=void 0,this._rowEnd=void 0,this._renderCallback(e,t),this._runRefreshCallbacks()}_runRefreshCallbacks(){for(const e of this._refreshCallbacks)e(0);this._refreshCallbacks=[]}}},5596:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.ScreenDprMonitor=void 0;const s=i(844);class r extends s.Disposable{constructor(e){super(),this._parentWindow=e,this._currentDevicePixelRatio=this._parentWindow.devicePixelRatio,this.register((0,s.toDisposable)((()=>{this.clearListener()})))}setListener(e){this._listener&&this.clearListener(),this._listener=e,this._outerListener=()=>{this._listener&&(this._listener(this._parentWindow.devicePixelRatio,this._currentDevicePixelRatio),this._updateDpr())},this._updateDpr()}_updateDpr(){var e;this._outerListener&&(null===(e=this._resolutionMediaMatchList)||void 0===e||e.removeListener(this._outerListener),this._currentDevicePixelRatio=this._parentWindow.devicePixelRatio,this._resolutionMediaMatchList=this._parentWindow.matchMedia(`screen and (resolution: ${this._parentWindow.devicePixelRatio}dppx)`),this._resolutionMediaMatchList.addListener(this._outerListener))}clearListener(){this._resolutionMediaMatchList&&this._listener&&this._outerListener&&(this._resolutionMediaMatchList.removeListener(this._outerListener),this._resolutionMediaMatchList=void 0,this._listener=void 0,this._outerListener=void 0)}}t.ScreenDprMonitor=r},3236:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.Terminal=void 0;const s=i(3614),r=i(3656),n=i(6465),o=i(9042),a=i(3730),h=i(1680),c=i(3107),l=i(5744),d=i(2950),_=i(1296),u=i(428),f=i(4269),v=i(5114),p=i(8934),g=i(3230),m=i(9312),S=i(4725),C=i(6731),b=i(8055),y=i(8969),w=i(8460),E=i(844),k=i(6114),L=i(8437),D=i(2584),R=i(7399),x=i(5941),A=i(9074),B=i(2585),T=i(5435),M=i(4567),O="undefined"!=typeof window?window.document:null;class P extends y.CoreTerminal{get onFocus(){return this._onFocus.event}get onBlur(){return this._onBlur.event}get onA11yChar(){return this._onA11yCharEmitter.event}get onA11yTab(){return this._onA11yTabEmitter.event}get onWillOpen(){return this._onWillOpen.event}constructor(e={}){super(e),this.browser=k,this._keyDownHandled=!1,this._keyDownSeen=!1,this._keyPressHandled=!1,this._unprocessedDeadKey=!1,this._accessibilityManager=this.register(new E.MutableDisposable),this._onCursorMove=this.register(new w.EventEmitter),this.onCursorMove=this._onCursorMove.event,this._onKey=this.register(new w.EventEmitter),this.onKey=this._onKey.event,this._onRender=this.register(new w.EventEmitter),this.onRender=this._onRender.event,this._onSelectionChange=this.register(new w.EventEmitter),this.onSelectionChange=this._onSelectionChange.event,this._onTitleChange=this.register(new w.EventEmitter),this.onTitleChange=this._onTitleChange.event,this._onBell=this.register(new w.EventEmitter),this.onBell=this._onBell.event,this._onFocus=this.register(new w.EventEmitter),this._onBlur=this.register(new w.EventEmitter),this._onA11yCharEmitter=this.register(new w.EventEmitter),this._onA11yTabEmitter=this.register(new w.EventEmitter),this._onWillOpen=this.register(new w.EventEmitter),this._setup(),this.linkifier2=this.register(this._instantiationService.createInstance(n.Linkifier2)),this.linkifier2.registerLinkProvider(this._instantiationService.createInstance(a.OscLinkProvider)),this._decorationService=this._instantiationService.createInstance(A.DecorationService),this._instantiationService.setService(B.IDecorationService,this._decorationService),this.register(this._inputHandler.onRequestBell((()=>this._onBell.fire()))),this.register(this._inputHandler.onRequestRefreshRows(((e,t)=>this.refresh(e,t)))),this.register(this._inputHandler.onRequestSendFocus((()=>this._reportFocus()))),this.register(this._inputHandler.onRequestReset((()=>this.reset()))),this.register(this._inputHandler.onRequestWindowsOptionsReport((e=>this._reportWindowsOptions(e)))),this.register(this._inputHandler.onColor((e=>this._handleColorEvent(e)))),this.register((0,w.forwardEvent)(this._inputHandler.onCursorMove,this._onCursorMove)),this.register((0,w.forwardEvent)(this._inputHandler.onTitleChange,this._onTitleChange)),this.register((0,w.forwardEvent)(this._inputHandler.onA11yChar,this._onA11yCharEmitter)),this.register((0,w.forwardEvent)(this._inputHandler.onA11yTab,this._onA11yTabEmitter)),this.register(this._bufferService.onResize((e=>this._afterResize(e.cols,e.rows)))),this.register((0,E.toDisposable)((()=>{var e,t;this._customKeyEventHandler=void 0,null===(t=null===(e=this.element)||void 0===e?void 0:e.parentNode)||void 0===t||t.removeChild(this.element)})))}_handleColorEvent(e){if(this._themeService)for(const t of e){let e,i="";switch(t.index){case 256:e="foreground",i="10";break;case 257:e="background",i="11";break;case 258:e="cursor",i="12";break;default:e="ansi",i="4;"+t.index}switch(t.type){case 0:const s=b.color.toColorRGB("ansi"===e?this._themeService.colors.ansi[t.index]:this._themeService.colors[e]);this.coreService.triggerDataEvent(`${D.C0.ESC}]${i};${(0,x.toRgbString)(s)}${D.C1_ESCAPED.ST}`);break;case 1:if("ansi"===e)this._themeService.modifyColors((e=>e.ansi[t.index]=b.rgba.toColor(...t.color)));else{const i=e;this._themeService.modifyColors((e=>e[i]=b.rgba.toColor(...t.color)))}break;case 2:this._themeService.restoreColor(t.index)}}}_setup(){super._setup(),this._customKeyEventHandler=void 0}get buffer(){return this.buffers.active}focus(){this.textarea&&this.textarea.focus({preventScroll:!0})}_handleScreenReaderModeOptionChange(e){e?!this._accessibilityManager.value&&this._renderService&&(this._accessibilityManager.value=this._instantiationService.createInstance(M.AccessibilityManager,this)):this._accessibilityManager.clear()}_handleTextAreaFocus(e){this.coreService.decPrivateModes.sendFocus&&this.coreService.triggerDataEvent(D.C0.ESC+"[I"),this.updateCursorStyle(e),this.element.classList.add("focus"),this._showCursor(),this._onFocus.fire()}blur(){var e;return null===(e=this.textarea)||void 0===e?void 0:e.blur()}_handleTextAreaBlur(){this.textarea.value="",this.refresh(this.buffer.y,this.buffer.y),this.coreService.decPrivateModes.sendFocus&&this.coreService.triggerDataEvent(D.C0.ESC+"[O"),this.element.classList.remove("focus"),this._onBlur.fire()}_syncTextArea(){if(!this.textarea||!this.buffer.isCursorInViewport||this._compositionHelper.isComposing||!this._renderService)return;const e=this.buffer.ybase+this.buffer.y,t=this.buffer.lines.get(e);if(!t)return;const i=Math.min(this.buffer.x,this.cols-1),s=this._renderService.dimensions.css.cell.height,r=t.getWidth(i),n=this._renderService.dimensions.css.cell.width*r,o=this.buffer.y*this._renderService.dimensions.css.cell.height,a=i*this._renderService.dimensions.css.cell.width;this.textarea.style.left=a+"px",this.textarea.style.top=o+"px",this.textarea.style.width=n+"px",this.textarea.style.height=s+"px",this.textarea.style.lineHeight=s+"px",this.textarea.style.zIndex="-5"}_initGlobal(){this._bindKeys(),this.register((0,r.addDisposableDomListener)(this.element,"copy",(e=>{this.hasSelection()&&(0,s.copyHandler)(e,this._selectionService)})));const e=e=>(0,s.handlePasteEvent)(e,this.textarea,this.coreService,this.optionsService);this.register((0,r.addDisposableDomListener)(this.textarea,"paste",e)),this.register((0,r.addDisposableDomListener)(this.element,"paste",e)),k.isFirefox?this.register((0,r.addDisposableDomListener)(this.element,"mousedown",(e=>{2===e.button&&(0,s.rightClickHandler)(e,this.textarea,this.screenElement,this._selectionService,this.options.rightClickSelectsWord)}))):this.register((0,r.addDisposableDomListener)(this.element,"contextmenu",(e=>{(0,s.rightClickHandler)(e,this.textarea,this.screenElement,this._selectionService,this.options.rightClickSelectsWord)}))),k.isLinux&&this.register((0,r.addDisposableDomListener)(this.element,"auxclick",(e=>{1===e.button&&(0,s.moveTextAreaUnderMouseCursor)(e,this.textarea,this.screenElement)})))}_bindKeys(){this.register((0,r.addDisposableDomListener)(this.textarea,"keyup",(e=>this._keyUp(e)),!0)),this.register((0,r.addDisposableDomListener)(this.textarea,"keydown",(e=>this._keyDown(e)),!0)),this.register((0,r.addDisposableDomListener)(this.textarea,"keypress",(e=>this._keyPress(e)),!0)),this.register((0,r.addDisposableDomListener)(this.textarea,"compositionstart",(()=>this._compositionHelper.compositionstart()))),this.register((0,r.addDisposableDomListener)(this.textarea,"compositionupdate",(e=>this._compositionHelper.compositionupdate(e)))),this.register((0,r.addDisposableDomListener)(this.textarea,"compositionend",(()=>this._compositionHelper.compositionend()))),this.register((0,r.addDisposableDomListener)(this.textarea,"input",(e=>this._inputEvent(e)),!0)),this.register(this.onRender((()=>this._compositionHelper.updateCompositionElements())))}open(e){var t;if(!e)throw new Error("Terminal requires a parent element.");e.isConnected||this._logService.debug("Terminal.open was called on an element that was not attached to the DOM"),this._document=e.ownerDocument,this.element=this._document.createElement("div"),this.element.dir="ltr",this.element.classList.add("terminal"),this.element.classList.add("xterm"),e.appendChild(this.element);const i=O.createDocumentFragment();this._viewportElement=O.createElement("div"),this._viewportElement.classList.add("xterm-viewport"),i.appendChild(this._viewportElement),this._viewportScrollArea=O.createElement("div"),this._viewportScrollArea.classList.add("xterm-scroll-area"),this._viewportElement.appendChild(this._viewportScrollArea),this.screenElement=O.createElement("div"),this.screenElement.classList.add("xterm-screen"),this._helperContainer=O.createElement("div"),this._helperContainer.classList.add("xterm-helpers"),this.screenElement.appendChild(this._helperContainer),i.appendChild(this.screenElement),this.textarea=O.createElement("textarea"),this.textarea.classList.add("xterm-helper-textarea"),this.textarea.setAttribute("aria-label",o.promptLabel),k.isChromeOS||this.textarea.setAttribute("aria-multiline","false"),this.textarea.setAttribute("autocorrect","off"),this.textarea.setAttribute("autocapitalize","off"),this.textarea.setAttribute("spellcheck","false"),this.textarea.tabIndex=0,this._coreBrowserService=this._instantiationService.createInstance(v.CoreBrowserService,this.textarea,null!==(t=this._document.defaultView)&&void 0!==t?t:window),this._instantiationService.setService(S.ICoreBrowserService,this._coreBrowserService),this.register((0,r.addDisposableDomListener)(this.textarea,"focus",(e=>this._handleTextAreaFocus(e)))),this.register((0,r.addDisposableDomListener)(this.textarea,"blur",(()=>this._handleTextAreaBlur()))),this._helperContainer.appendChild(this.textarea),this._charSizeService=this._instantiationService.createInstance(u.CharSizeService,this._document,this._helperContainer),this._instantiationService.setService(S.ICharSizeService,this._charSizeService),this._themeService=this._instantiationService.createInstance(C.ThemeService),this._instantiationService.setService(S.IThemeService,this._themeService),this._characterJoinerService=this._instantiationService.createInstance(f.CharacterJoinerService),this._instantiationService.setService(S.ICharacterJoinerService,this._characterJoinerService),this._renderService=this.register(this._instantiationService.createInstance(g.RenderService,this.rows,this.screenElement)),this._instantiationService.setService(S.IRenderService,this._renderService),this.register(this._renderService.onRenderedViewportChange((e=>this._onRender.fire(e)))),this.onResize((e=>this._renderService.resize(e.cols,e.rows))),this._compositionView=O.createElement("div"),this._compositionView.classList.add("composition-view"),this._compositionHelper=this._instantiationService.createInstance(d.CompositionHelper,this.textarea,this._compositionView),this._helperContainer.appendChild(this._compositionView),this.element.appendChild(i);try{this._onWillOpen.fire(this.element)}catch(e){}this._renderService.hasRenderer()||this._renderService.setRenderer(this._createRenderer()),this._mouseService=this._instantiationService.createInstance(p.MouseService),this._instantiationService.setService(S.IMouseService,this._mouseService),this.viewport=this._instantiationService.createInstance(h.Viewport,this._viewportElement,this._viewportScrollArea),this.viewport.onRequestScrollLines((e=>this.scrollLines(e.amount,e.suppressScrollEvent,1))),this.register(this._inputHandler.onRequestSyncScrollBar((()=>this.viewport.syncScrollArea()))),this.register(this.viewport),this.register(this.onCursorMove((()=>{this._renderService.handleCursorMove(),this._syncTextArea()}))),this.register(this.onResize((()=>this._renderService.handleResize(this.cols,this.rows)))),this.register(this.onBlur((()=>this._renderService.handleBlur()))),this.register(this.onFocus((()=>this._renderService.handleFocus()))),this.register(this._renderService.onDimensionsChange((()=>this.viewport.syncScrollArea()))),this._selectionService=this.register(this._instantiationService.createInstance(m.SelectionService,this.element,this.screenElement,this.linkifier2)),this._instantiationService.setService(S.ISelectionService,this._selectionService),this.register(this._selectionService.onRequestScrollLines((e=>this.scrollLines(e.amount,e.suppressScrollEvent)))),this.register(this._selectionService.onSelectionChange((()=>this._onSelectionChange.fire()))),this.register(this._selectionService.onRequestRedraw((e=>this._renderService.handleSelectionChanged(e.start,e.end,e.columnSelectMode)))),this.register(this._selectionService.onLinuxMouseSelection((e=>{this.textarea.value=e,this.textarea.focus(),this.textarea.select()}))),this.register(this._onScroll.event((e=>{this.viewport.syncScrollArea(),this._selectionService.refresh()}))),this.register((0,r.addDisposableDomListener)(this._viewportElement,"scroll",(()=>this._selectionService.refresh()))),this.linkifier2.attachToDom(this.screenElement,this._mouseService,this._renderService),this.register(this._instantiationService.createInstance(c.BufferDecorationRenderer,this.screenElement)),this.register((0,r.addDisposableDomListener)(this.element,"mousedown",(e=>this._selectionService.handleMouseDown(e)))),this.coreMouseService.areMouseEventsActive?(this._selectionService.disable(),this.element.classList.add("enable-mouse-events")):this._selectionService.enable(),this.options.screenReaderMode&&(this._accessibilityManager.value=this._instantiationService.createInstance(M.AccessibilityManager,this)),this.register(this.optionsService.onSpecificOptionChange("screenReaderMode",(e=>this._handleScreenReaderModeOptionChange(e)))),this.options.overviewRulerWidth&&(this._overviewRulerRenderer=this.register(this._instantiationService.createInstance(l.OverviewRulerRenderer,this._viewportElement,this.screenElement))),this.optionsService.onSpecificOptionChange("overviewRulerWidth",(e=>{!this._overviewRulerRenderer&&e&&this._viewportElement&&this.screenElement&&(this._overviewRulerRenderer=this.register(this._instantiationService.createInstance(l.OverviewRulerRenderer,this._viewportElement,this.screenElement)))})),this._charSizeService.measure(),this.refresh(0,this.rows-1),this._initGlobal(),this.bindMouse()}_createRenderer(){return this._instantiationService.createInstance(_.DomRenderer,this.element,this.screenElement,this._viewportElement,this.linkifier2)}bindMouse(){const e=this,t=this.element;function i(t){const i=e._mouseService.getMouseReportCoords(t,e.screenElement);if(!i)return!1;let s,r;switch(t.overrideType||t.type){case"mousemove":r=32,void 0===t.buttons?(s=3,void 0!==t.button&&(s=t.button<3?t.button:3)):s=1&t.buttons?0:4&t.buttons?1:2&t.buttons?2:3;break;case"mouseup":r=0,s=t.button<3?t.button:3;break;case"mousedown":r=1,s=t.button<3?t.button:3;break;case"wheel":if(0===e.viewport.getLinesScrolled(t))return!1;r=t.deltaY<0?0:1,s=4;break;default:return!1}return!(void 0===r||void 0===s||s>4)&&e.coreMouseService.triggerMouseEvent({col:i.col,row:i.row,x:i.x,y:i.y,button:s,action:r,ctrl:t.ctrlKey,alt:t.altKey,shift:t.shiftKey})}const s={mouseup:null,wheel:null,mousedrag:null,mousemove:null},n={mouseup:e=>(i(e),e.buttons||(this._document.removeEventListener("mouseup",s.mouseup),s.mousedrag&&this._document.removeEventListener("mousemove",s.mousedrag)),this.cancel(e)),wheel:e=>(i(e),this.cancel(e,!0)),mousedrag:e=>{e.buttons&&i(e)},mousemove:e=>{e.buttons||i(e)}};this.register(this.coreMouseService.onProtocolChange((e=>{e?("debug"===this.optionsService.rawOptions.logLevel&&this._logService.debug("Binding to mouse events:",this.coreMouseService.explainEvents(e)),this.element.classList.add("enable-mouse-events"),this._selectionService.disable()):(this._logService.debug("Unbinding from mouse events."),this.element.classList.remove("enable-mouse-events"),this._selectionService.enable()),8&e?s.mousemove||(t.addEventListener("mousemove",n.mousemove),s.mousemove=n.mousemove):(t.removeEventListener("mousemove",s.mousemove),s.mousemove=null),16&e?s.wheel||(t.addEventListener("wheel",n.wheel,{passive:!1}),s.wheel=n.wheel):(t.removeEventListener("wheel",s.wheel),s.wheel=null),2&e?s.mouseup||(t.addEventListener("mouseup",n.mouseup),s.mouseup=n.mouseup):(this._document.removeEventListener("mouseup",s.mouseup),t.removeEventListener("mouseup",s.mouseup),s.mouseup=null),4&e?s.mousedrag||(s.mousedrag=n.mousedrag):(this._document.removeEventListener("mousemove",s.mousedrag),s.mousedrag=null)}))),this.coreMouseService.activeProtocol=this.coreMouseService.activeProtocol,this.register((0,r.addDisposableDomListener)(t,"mousedown",(e=>{if(e.preventDefault(),this.focus(),this.coreMouseService.areMouseEventsActive&&!this._selectionService.shouldForceSelection(e))return i(e),s.mouseup&&this._document.addEventListener("mouseup",s.mouseup),s.mousedrag&&this._document.addEventListener("mousemove",s.mousedrag),this.cancel(e)}))),this.register((0,r.addDisposableDomListener)(t,"wheel",(e=>{if(!s.wheel){if(!this.buffer.hasScrollback){const t=this.viewport.getLinesScrolled(e);if(0===t)return;const i=D.C0.ESC+(this.coreService.decPrivateModes.applicationCursorKeys?"O":"[")+(e.deltaY<0?"A":"B");let s="";for(let e=0;e<Math.abs(t);e++)s+=i;return this.coreService.triggerDataEvent(s,!0),this.cancel(e,!0)}return this.viewport.handleWheel(e)?this.cancel(e):void 0}}),{passive:!1})),this.register((0,r.addDisposableDomListener)(t,"touchstart",(e=>{if(!this.coreMouseService.areMouseEventsActive)return this.viewport.handleTouchStart(e),this.cancel(e)}),{passive:!0})),this.register((0,r.addDisposableDomListener)(t,"touchmove",(e=>{if(!this.coreMouseService.areMouseEventsActive)return this.viewport.handleTouchMove(e)?void 0:this.cancel(e)}),{passive:!1}))}refresh(e,t){var i;null===(i=this._renderService)||void 0===i||i.refreshRows(e,t)}updateCursorStyle(e){var t;(null===(t=this._selectionService)||void 0===t?void 0:t.shouldColumnSelect(e))?this.element.classList.add("column-select"):this.element.classList.remove("column-select")}_showCursor(){this.coreService.isCursorInitialized||(this.coreService.isCursorInitialized=!0,this.refresh(this.buffer.y,this.buffer.y))}scrollLines(e,t,i=0){var s;1===i?(super.scrollLines(e,t,i),this.refresh(0,this.rows-1)):null===(s=this.viewport)||void 0===s||s.scrollLines(e)}paste(e){(0,s.paste)(e,this.textarea,this.coreService,this.optionsService)}attachCustomKeyEventHandler(e){this._customKeyEventHandler=e}registerLinkProvider(e){return this.linkifier2.registerLinkProvider(e)}registerCharacterJoiner(e){if(!this._characterJoinerService)throw new Error("Terminal must be opened first");const t=this._characterJoinerService.register(e);return this.refresh(0,this.rows-1),t}deregisterCharacterJoiner(e){if(!this._characterJoinerService)throw new Error("Terminal must be opened first");this._characterJoinerService.deregister(e)&&this.refresh(0,this.rows-1)}get markers(){return this.buffer.markers}registerMarker(e){return this.buffer.addMarker(this.buffer.ybase+this.buffer.y+e)}registerDecoration(e){return this._decorationService.registerDecoration(e)}hasSelection(){return!!this._selectionService&&this._selectionService.hasSelection}select(e,t,i){this._selectionService.setSelection(e,t,i)}getSelection(){return this._selectionService?this._selectionService.selectionText:""}getSelectionPosition(){if(this._selectionService&&this._selectionService.hasSelection)return{start:{x:this._selectionService.selectionStart[0],y:this._selectionService.selectionStart[1]},end:{x:this._selectionService.selectionEnd[0],y:this._selectionService.selectionEnd[1]}}}clearSelection(){var e;null===(e=this._selectionService)||void 0===e||e.clearSelection()}selectAll(){var e;null===(e=this._selectionService)||void 0===e||e.selectAll()}selectLines(e,t){var i;null===(i=this._selectionService)||void 0===i||i.selectLines(e,t)}_keyDown(e){if(this._keyDownHandled=!1,this._keyDownSeen=!0,this._customKeyEventHandler&&!1===this._customKeyEventHandler(e))return!1;const t=this.browser.isMac&&this.options.macOptionIsMeta&&e.altKey;if(!t&&!this._compositionHelper.keydown(e))return this.options.scrollOnUserInput&&this.buffer.ybase!==this.buffer.ydisp&&this.scrollToBottom(),!1;t||"Dead"!==e.key&&"AltGraph"!==e.key||(this._unprocessedDeadKey=!0);const i=(0,R.evaluateKeyboardEvent)(e,this.coreService.decPrivateModes.applicationCursorKeys,this.browser.isMac,this.options.macOptionIsMeta);if(this.updateCursorStyle(e),3===i.type||2===i.type){const t=this.rows-1;return this.scrollLines(2===i.type?-t:t),this.cancel(e,!0)}return 1===i.type&&this.selectAll(),!!this._isThirdLevelShift(this.browser,e)||(i.cancel&&this.cancel(e,!0),!i.key||!!(e.key&&!e.ctrlKey&&!e.altKey&&!e.metaKey&&1===e.key.length&&e.key.charCodeAt(0)>=65&&e.key.charCodeAt(0)<=90)||(this._unprocessedDeadKey?(this._unprocessedDeadKey=!1,!0):(i.key!==D.C0.ETX&&i.key!==D.C0.CR||(this.textarea.value=""),this._onKey.fire({key:i.key,domEvent:e}),this._showCursor(),this.coreService.triggerDataEvent(i.key,!0),!this.optionsService.rawOptions.screenReaderMode||e.altKey||e.ctrlKey?this.cancel(e,!0):void(this._keyDownHandled=!0))))}_isThirdLevelShift(e,t){const i=e.isMac&&!this.options.macOptionIsMeta&&t.altKey&&!t.ctrlKey&&!t.metaKey||e.isWindows&&t.altKey&&t.ctrlKey&&!t.metaKey||e.isWindows&&t.getModifierState("AltGraph");return"keypress"===t.type?i:i&&(!t.keyCode||t.keyCode>47)}_keyUp(e){this._keyDownSeen=!1,this._customKeyEventHandler&&!1===this._customKeyEventHandler(e)||(function(e){return 16===e.keyCode||17===e.keyCode||18===e.keyCode}(e)||this.focus(),this.updateCursorStyle(e),this._keyPressHandled=!1)}_keyPress(e){let t;if(this._keyPressHandled=!1,this._keyDownHandled)return!1;if(this._customKeyEventHandler&&!1===this._customKeyEventHandler(e))return!1;if(this.cancel(e),e.charCode)t=e.charCode;else if(null===e.which||void 0===e.which)t=e.keyCode;else{if(0===e.which||0===e.charCode)return!1;t=e.which}return!(!t||(e.altKey||e.ctrlKey||e.metaKey)&&!this._isThirdLevelShift(this.browser,e)||(t=String.fromCharCode(t),this._onKey.fire({key:t,domEvent:e}),this._showCursor(),this.coreService.triggerDataEvent(t,!0),this._keyPressHandled=!0,this._unprocessedDeadKey=!1,0))}_inputEvent(e){if(e.data&&"insertText"===e.inputType&&(!e.composed||!this._keyDownSeen)&&!this.optionsService.rawOptions.screenReaderMode){if(this._keyPressHandled)return!1;this._unprocessedDeadKey=!1;const t=e.data;return this.coreService.triggerDataEvent(t,!0),this.cancel(e),!0}return!1}resize(e,t){e!==this.cols||t!==this.rows?super.resize(e,t):this._charSizeService&&!this._charSizeService.hasValidSize&&this._charSizeService.measure()}_afterResize(e,t){var i,s;null===(i=this._charSizeService)||void 0===i||i.measure(),null===(s=this.viewport)||void 0===s||s.syncScrollArea(!0)}clear(){var e;if(0!==this.buffer.ybase||0!==this.buffer.y){this.buffer.clearAllMarkers(),this.buffer.lines.set(0,this.buffer.lines.get(this.buffer.ybase+this.buffer.y)),this.buffer.lines.length=1,this.buffer.ydisp=0,this.buffer.ybase=0,this.buffer.y=0;for(let e=1;e<this.rows;e++)this.buffer.lines.push(this.buffer.getBlankLine(L.DEFAULT_ATTR_DATA));this._onScroll.fire({position:this.buffer.ydisp,source:0}),null===(e=this.viewport)||void 0===e||e.reset(),this.refresh(0,this.rows-1)}}reset(){var e,t;this.options.rows=this.rows,this.options.cols=this.cols;const i=this._customKeyEventHandler;this._setup(),super.reset(),null===(e=this._selectionService)||void 0===e||e.reset(),this._decorationService.reset(),null===(t=this.viewport)||void 0===t||t.reset(),this._customKeyEventHandler=i,this.refresh(0,this.rows-1)}clearTextureAtlas(){var e;null===(e=this._renderService)||void 0===e||e.clearTextureAtlas()}_reportFocus(){var e;(null===(e=this.element)||void 0===e?void 0:e.classList.contains("focus"))?this.coreService.triggerDataEvent(D.C0.ESC+"[I"):this.coreService.triggerDataEvent(D.C0.ESC+"[O")}_reportWindowsOptions(e){if(this._renderService)switch(e){case T.WindowsOptionsReportType.GET_WIN_SIZE_PIXELS:const e=this._renderService.dimensions.css.canvas.width.toFixed(0),t=this._renderService.dimensions.css.canvas.height.toFixed(0);this.coreService.triggerDataEvent(`${D.C0.ESC}[4;${t};${e}t`);break;case T.WindowsOptionsReportType.GET_CELL_SIZE_PIXELS:const i=this._renderService.dimensions.css.cell.width.toFixed(0),s=this._renderService.dimensions.css.cell.height.toFixed(0);this.coreService.triggerDataEvent(`${D.C0.ESC}[6;${s};${i}t`)}}cancel(e,t){if(this.options.cancelEvents||t)return e.preventDefault(),e.stopPropagation(),!1}}t.Terminal=P},9924:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.TimeBasedDebouncer=void 0,t.TimeBasedDebouncer=class{constructor(e,t=1e3){this._renderCallback=e,this._debounceThresholdMS=t,this._lastRefreshMs=0,this._additionalRefreshRequested=!1}dispose(){this._refreshTimeoutID&&clearTimeout(this._refreshTimeoutID)}refresh(e,t,i){this._rowCount=i,e=void 0!==e?e:0,t=void 0!==t?t:this._rowCount-1,this._rowStart=void 0!==this._rowStart?Math.min(this._rowStart,e):e,this._rowEnd=void 0!==this._rowEnd?Math.max(this._rowEnd,t):t;const s=Date.now();if(s-this._lastRefreshMs>=this._debounceThresholdMS)this._lastRefreshMs=s,this._innerRefresh();else if(!this._additionalRefreshRequested){const e=s-this._lastRefreshMs,t=this._debounceThresholdMS-e;this._additionalRefreshRequested=!0,this._refreshTimeoutID=window.setTimeout((()=>{this._lastRefreshMs=Date.now(),this._innerRefresh(),this._additionalRefreshRequested=!1,this._refreshTimeoutID=void 0}),t)}}_innerRefresh(){if(void 0===this._rowStart||void 0===this._rowEnd||void 0===this._rowCount)return;const e=Math.max(this._rowStart,0),t=Math.min(this._rowEnd,this._rowCount-1);this._rowStart=void 0,this._rowEnd=void 0,this._renderCallback(e,t)}}},1680:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.Viewport=void 0;const n=i(3656),o=i(4725),a=i(8460),h=i(844),c=i(2585);let l=t.Viewport=class extends h.Disposable{constructor(e,t,i,s,r,o,h,c){super(),this._viewportElement=e,this._scrollArea=t,this._bufferService=i,this._optionsService=s,this._charSizeService=r,this._renderService=o,this._coreBrowserService=h,this.scrollBarWidth=0,this._currentRowHeight=0,this._currentDeviceCellHeight=0,this._lastRecordedBufferLength=0,this._lastRecordedViewportHeight=0,this._lastRecordedBufferHeight=0,this._lastTouchY=0,this._lastScrollTop=0,this._wheelPartialScroll=0,this._refreshAnimationFrame=null,this._ignoreNextScrollEvent=!1,this._smoothScrollState={startTime:0,origin:-1,target:-1},this._onRequestScrollLines=this.register(new a.EventEmitter),this.onRequestScrollLines=this._onRequestScrollLines.event,this.scrollBarWidth=this._viewportElement.offsetWidth-this._scrollArea.offsetWidth||15,this.register((0,n.addDisposableDomListener)(this._viewportElement,"scroll",this._handleScroll.bind(this))),this._activeBuffer=this._bufferService.buffer,this.register(this._bufferService.buffers.onBufferActivate((e=>this._activeBuffer=e.activeBuffer))),this._renderDimensions=this._renderService.dimensions,this.register(this._renderService.onDimensionsChange((e=>this._renderDimensions=e))),this._handleThemeChange(c.colors),this.register(c.onChangeColors((e=>this._handleThemeChange(e)))),this.register(this._optionsService.onSpecificOptionChange("scrollback",(()=>this.syncScrollArea()))),setTimeout((()=>this.syncScrollArea()))}_handleThemeChange(e){this._viewportElement.style.backgroundColor=e.background.css}reset(){this._currentRowHeight=0,this._currentDeviceCellHeight=0,this._lastRecordedBufferLength=0,this._lastRecordedViewportHeight=0,this._lastRecordedBufferHeight=0,this._lastTouchY=0,this._lastScrollTop=0,this._coreBrowserService.window.requestAnimationFrame((()=>this.syncScrollArea()))}_refresh(e){if(e)return this._innerRefresh(),void(null!==this._refreshAnimationFrame&&this._coreBrowserService.window.cancelAnimationFrame(this._refreshAnimationFrame));null===this._refreshAnimationFrame&&(this._refreshAnimationFrame=this._coreBrowserService.window.requestAnimationFrame((()=>this._innerRefresh())))}_innerRefresh(){if(this._charSizeService.height>0){this._currentRowHeight=this._renderService.dimensions.device.cell.height/this._coreBrowserService.dpr,this._currentDeviceCellHeight=this._renderService.dimensions.device.cell.height,this._lastRecordedViewportHeight=this._viewportElement.offsetHeight;const e=Math.round(this._currentRowHeight*this._lastRecordedBufferLength)+(this._lastRecordedViewportHeight-this._renderService.dimensions.css.canvas.height);this._lastRecordedBufferHeight!==e&&(this._lastRecordedBufferHeight=e,this._scrollArea.style.height=this._lastRecordedBufferHeight+"px")}const e=this._bufferService.buffer.ydisp*this._currentRowHeight;this._viewportElement.scrollTop!==e&&(this._ignoreNextScrollEvent=!0,this._viewportElement.scrollTop=e),this._refreshAnimationFrame=null}syncScrollArea(e=!1){if(this._lastRecordedBufferLength!==this._bufferService.buffer.lines.length)return this._lastRecordedBufferLength=this._bufferService.buffer.lines.length,void this._refresh(e);this._lastRecordedViewportHeight===this._renderService.dimensions.css.canvas.height&&this._lastScrollTop===this._activeBuffer.ydisp*this._currentRowHeight&&this._renderDimensions.device.cell.height===this._currentDeviceCellHeight||this._refresh(e)}_handleScroll(e){if(this._lastScrollTop=this._viewportElement.scrollTop,!this._viewportElement.offsetParent)return;if(this._ignoreNextScrollEvent)return this._ignoreNextScrollEvent=!1,void this._onRequestScrollLines.fire({amount:0,suppressScrollEvent:!0});const t=Math.round(this._lastScrollTop/this._currentRowHeight)-this._bufferService.buffer.ydisp;this._onRequestScrollLines.fire({amount:t,suppressScrollEvent:!0})}_smoothScroll(){if(this._isDisposed||-1===this._smoothScrollState.origin||-1===this._smoothScrollState.target)return;const e=this._smoothScrollPercent();this._viewportElement.scrollTop=this._smoothScrollState.origin+Math.round(e*(this._smoothScrollState.target-this._smoothScrollState.origin)),e<1?this._coreBrowserService.window.requestAnimationFrame((()=>this._smoothScroll())):this._clearSmoothScrollState()}_smoothScrollPercent(){return this._optionsService.rawOptions.smoothScrollDuration&&this._smoothScrollState.startTime?Math.max(Math.min((Date.now()-this._smoothScrollState.startTime)/this._optionsService.rawOptions.smoothScrollDuration,1),0):1}_clearSmoothScrollState(){this._smoothScrollState.startTime=0,this._smoothScrollState.origin=-1,this._smoothScrollState.target=-1}_bubbleScroll(e,t){const i=this._viewportElement.scrollTop+this._lastRecordedViewportHeight;return!(t<0&&0!==this._viewportElement.scrollTop||t>0&&i<this._lastRecordedBufferHeight)||(e.cancelable&&e.preventDefault(),!1)}handleWheel(e){const t=this._getPixelsScrolled(e);return 0!==t&&(this._optionsService.rawOptions.smoothScrollDuration?(this._smoothScrollState.startTime=Date.now(),this._smoothScrollPercent()<1?(this._smoothScrollState.origin=this._viewportElement.scrollTop,-1===this._smoothScrollState.target?this._smoothScrollState.target=this._viewportElement.scrollTop+t:this._smoothScrollState.target+=t,this._smoothScrollState.target=Math.max(Math.min(this._smoothScrollState.target,this._viewportElement.scrollHeight),0),this._smoothScroll()):this._clearSmoothScrollState()):this._viewportElement.scrollTop+=t,this._bubbleScroll(e,t))}scrollLines(e){if(0!==e)if(this._optionsService.rawOptions.smoothScrollDuration){const t=e*this._currentRowHeight;this._smoothScrollState.startTime=Date.now(),this._smoothScrollPercent()<1?(this._smoothScrollState.origin=this._viewportElement.scrollTop,this._smoothScrollState.target=this._smoothScrollState.origin+t,this._smoothScrollState.target=Math.max(Math.min(this._smoothScrollState.target,this._viewportElement.scrollHeight),0),this._smoothScroll()):this._clearSmoothScrollState()}else this._onRequestScrollLines.fire({amount:e,suppressScrollEvent:!1})}_getPixelsScrolled(e){if(0===e.deltaY||e.shiftKey)return 0;let t=this._applyScrollModifier(e.deltaY,e);return e.deltaMode===WheelEvent.DOM_DELTA_LINE?t*=this._currentRowHeight:e.deltaMode===WheelEvent.DOM_DELTA_PAGE&&(t*=this._currentRowHeight*this._bufferService.rows),t}getBufferElements(e,t){var i;let s,r="";const n=[],o=null!=t?t:this._bufferService.buffer.lines.length,a=this._bufferService.buffer.lines;for(let t=e;t<o;t++){const e=a.get(t);if(!e)continue;const o=null===(i=a.get(t+1))||void 0===i?void 0:i.isWrapped;if(r+=e.translateToString(!o),!o||t===a.length-1){const e=document.createElement("div");e.textContent=r,n.push(e),r.length>0&&(s=e),r=""}}return{bufferElements:n,cursorElement:s}}getLinesScrolled(e){if(0===e.deltaY||e.shiftKey)return 0;let t=this._applyScrollModifier(e.deltaY,e);return e.deltaMode===WheelEvent.DOM_DELTA_PIXEL?(t/=this._currentRowHeight+0,this._wheelPartialScroll+=t,t=Math.floor(Math.abs(this._wheelPartialScroll))*(this._wheelPartialScroll>0?1:-1),this._wheelPartialScroll%=1):e.deltaMode===WheelEvent.DOM_DELTA_PAGE&&(t*=this._bufferService.rows),t}_applyScrollModifier(e,t){const i=this._optionsService.rawOptions.fastScrollModifier;return"alt"===i&&t.altKey||"ctrl"===i&&t.ctrlKey||"shift"===i&&t.shiftKey?e*this._optionsService.rawOptions.fastScrollSensitivity*this._optionsService.rawOptions.scrollSensitivity:e*this._optionsService.rawOptions.scrollSensitivity}handleTouchStart(e){this._lastTouchY=e.touches[0].pageY}handleTouchMove(e){const t=this._lastTouchY-e.touches[0].pageY;return this._lastTouchY=e.touches[0].pageY,0!==t&&(this._viewportElement.scrollTop+=t,this._bubbleScroll(e,t))}};t.Viewport=l=s([r(2,c.IBufferService),r(3,c.IOptionsService),r(4,o.ICharSizeService),r(5,o.IRenderService),r(6,o.ICoreBrowserService),r(7,o.IThemeService)],l)},3107:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.BufferDecorationRenderer=void 0;const n=i(3656),o=i(4725),a=i(844),h=i(2585);let c=t.BufferDecorationRenderer=class extends a.Disposable{constructor(e,t,i,s){super(),this._screenElement=e,this._bufferService=t,this._decorationService=i,this._renderService=s,this._decorationElements=new Map,this._altBufferIsActive=!1,this._dimensionsChanged=!1,this._container=document.createElement("div"),this._container.classList.add("xterm-decoration-container"),this._screenElement.appendChild(this._container),this.register(this._renderService.onRenderedViewportChange((()=>this._doRefreshDecorations()))),this.register(this._renderService.onDimensionsChange((()=>{this._dimensionsChanged=!0,this._queueRefresh()}))),this.register((0,n.addDisposableDomListener)(window,"resize",(()=>this._queueRefresh()))),this.register(this._bufferService.buffers.onBufferActivate((()=>{this._altBufferIsActive=this._bufferService.buffer===this._bufferService.buffers.alt}))),this.register(this._decorationService.onDecorationRegistered((()=>this._queueRefresh()))),this.register(this._decorationService.onDecorationRemoved((e=>this._removeDecoration(e)))),this.register((0,a.toDisposable)((()=>{this._container.remove(),this._decorationElements.clear()})))}_queueRefresh(){void 0===this._animationFrame&&(this._animationFrame=this._renderService.addRefreshCallback((()=>{this._doRefreshDecorations(),this._animationFrame=void 0})))}_doRefreshDecorations(){for(const e of this._decorationService.decorations)this._renderDecoration(e);this._dimensionsChanged=!1}_renderDecoration(e){this._refreshStyle(e),this._dimensionsChanged&&this._refreshXPosition(e)}_createElement(e){var t,i;const s=document.createElement("div");s.classList.add("xterm-decoration"),s.classList.toggle("xterm-decoration-top-layer","top"===(null===(t=null==e?void 0:e.options)||void 0===t?void 0:t.layer)),s.style.width=`${Math.round((e.options.width||1)*this._renderService.dimensions.css.cell.width)}px`,s.style.height=(e.options.height||1)*this._renderService.dimensions.css.cell.height+"px",s.style.top=(e.marker.line-this._bufferService.buffers.active.ydisp)*this._renderService.dimensions.css.cell.height+"px",s.style.lineHeight=`${this._renderService.dimensions.css.cell.height}px`;const r=null!==(i=e.options.x)&&void 0!==i?i:0;return r&&r>this._bufferService.cols&&(s.style.display="none"),this._refreshXPosition(e,s),s}_refreshStyle(e){const t=e.marker.line-this._bufferService.buffers.active.ydisp;if(t<0||t>=this._bufferService.rows)e.element&&(e.element.style.display="none",e.onRenderEmitter.fire(e.element));else{let i=this._decorationElements.get(e);i||(i=this._createElement(e),e.element=i,this._decorationElements.set(e,i),this._container.appendChild(i),e.onDispose((()=>{this._decorationElements.delete(e),i.remove()}))),i.style.top=t*this._renderService.dimensions.css.cell.height+"px",i.style.display=this._altBufferIsActive?"none":"block",e.onRenderEmitter.fire(i)}}_refreshXPosition(e,t=e.element){var i;if(!t)return;const s=null!==(i=e.options.x)&&void 0!==i?i:0;"right"===(e.options.anchor||"left")?t.style.right=s?s*this._renderService.dimensions.css.cell.width+"px":"":t.style.left=s?s*this._renderService.dimensions.css.cell.width+"px":""}_removeDecoration(e){var t;null===(t=this._decorationElements.get(e))||void 0===t||t.remove(),this._decorationElements.delete(e),e.dispose()}};t.BufferDecorationRenderer=c=s([r(1,h.IBufferService),r(2,h.IDecorationService),r(3,o.IRenderService)],c)},5871:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.ColorZoneStore=void 0,t.ColorZoneStore=class{constructor(){this._zones=[],this._zonePool=[],this._zonePoolIndex=0,this._linePadding={full:0,left:0,center:0,right:0}}get zones(){return this._zonePool.length=Math.min(this._zonePool.length,this._zones.length),this._zones}clear(){this._zones.length=0,this._zonePoolIndex=0}addDecoration(e){if(e.options.overviewRulerOptions){for(const t of this._zones)if(t.color===e.options.overviewRulerOptions.color&&t.position===e.options.overviewRulerOptions.position){if(this._lineIntersectsZone(t,e.marker.line))return;if(this._lineAdjacentToZone(t,e.marker.line,e.options.overviewRulerOptions.position))return void this._addLineToZone(t,e.marker.line)}if(this._zonePoolIndex<this._zonePool.length)return this._zonePool[this._zonePoolIndex].color=e.options.overviewRulerOptions.color,this._zonePool[this._zonePoolIndex].position=e.options.overviewRulerOptions.position,this._zonePool[this._zonePoolIndex].startBufferLine=e.marker.line,this._zonePool[this._zonePoolIndex].endBufferLine=e.marker.line,void this._zones.push(this._zonePool[this._zonePoolIndex++]);this._zones.push({color:e.options.overviewRulerOptions.color,position:e.options.overviewRulerOptions.position,startBufferLine:e.marker.line,endBufferLine:e.marker.line}),this._zonePool.push(this._zones[this._zones.length-1]),this._zonePoolIndex++}}setPadding(e){this._linePadding=e}_lineIntersectsZone(e,t){return t>=e.startBufferLine&&t<=e.endBufferLine}_lineAdjacentToZone(e,t,i){return t>=e.startBufferLine-this._linePadding[i||"full"]&&t<=e.endBufferLine+this._linePadding[i||"full"]}_addLineToZone(e,t){e.startBufferLine=Math.min(e.startBufferLine,t),e.endBufferLine=Math.max(e.endBufferLine,t)}}},5744:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.OverviewRulerRenderer=void 0;const n=i(5871),o=i(3656),a=i(4725),h=i(844),c=i(2585),l={full:0,left:0,center:0,right:0},d={full:0,left:0,center:0,right:0},_={full:0,left:0,center:0,right:0};let u=t.OverviewRulerRenderer=class extends h.Disposable{get _width(){return this._optionsService.options.overviewRulerWidth||0}constructor(e,t,i,s,r,o,a){var c;super(),this._viewportElement=e,this._screenElement=t,this._bufferService=i,this._decorationService=s,this._renderService=r,this._optionsService=o,this._coreBrowseService=a,this._colorZoneStore=new n.ColorZoneStore,this._shouldUpdateDimensions=!0,this._shouldUpdateAnchor=!0,this._lastKnownBufferLength=0,this._canvas=document.createElement("canvas"),this._canvas.classList.add("xterm-decoration-overview-ruler"),this._refreshCanvasDimensions(),null===(c=this._viewportElement.parentElement)||void 0===c||c.insertBefore(this._canvas,this._viewportElement);const l=this._canvas.getContext("2d");if(!l)throw new Error("Ctx cannot be null");this._ctx=l,this._registerDecorationListeners(),this._registerBufferChangeListeners(),this._registerDimensionChangeListeners(),this.register((0,h.toDisposable)((()=>{var e;null===(e=this._canvas)||void 0===e||e.remove()})))}_registerDecorationListeners(){this.register(this._decorationService.onDecorationRegistered((()=>this._queueRefresh(void 0,!0)))),this.register(this._decorationService.onDecorationRemoved((()=>this._queueRefresh(void 0,!0))))}_registerBufferChangeListeners(){this.register(this._renderService.onRenderedViewportChange((()=>this._queueRefresh()))),this.register(this._bufferService.buffers.onBufferActivate((()=>{this._canvas.style.display=this._bufferService.buffer===this._bufferService.buffers.alt?"none":"block"}))),this.register(this._bufferService.onScroll((()=>{this._lastKnownBufferLength!==this._bufferService.buffers.normal.lines.length&&(this._refreshDrawHeightConstants(),this._refreshColorZonePadding())})))}_registerDimensionChangeListeners(){this.register(this._renderService.onRender((()=>{this._containerHeight&&this._containerHeight===this._screenElement.clientHeight||(this._queueRefresh(!0),this._containerHeight=this._screenElement.clientHeight)}))),this.register(this._optionsService.onSpecificOptionChange("overviewRulerWidth",(()=>this._queueRefresh(!0)))),this.register((0,o.addDisposableDomListener)(this._coreBrowseService.window,"resize",(()=>this._queueRefresh(!0)))),this._queueRefresh(!0)}_refreshDrawConstants(){const e=Math.floor(this._canvas.width/3),t=Math.ceil(this._canvas.width/3);d.full=this._canvas.width,d.left=e,d.center=t,d.right=e,this._refreshDrawHeightConstants(),_.full=0,_.left=0,_.center=d.left,_.right=d.left+d.center}_refreshDrawHeightConstants(){l.full=Math.round(2*this._coreBrowseService.dpr);const e=this._canvas.height/this._bufferService.buffer.lines.length,t=Math.round(Math.max(Math.min(e,12),6)*this._coreBrowseService.dpr);l.left=t,l.center=t,l.right=t}_refreshColorZonePadding(){this._colorZoneStore.setPadding({full:Math.floor(this._bufferService.buffers.active.lines.length/(this._canvas.height-1)*l.full),left:Math.floor(this._bufferService.buffers.active.lines.length/(this._canvas.height-1)*l.left),center:Math.floor(this._bufferService.buffers.active.lines.length/(this._canvas.height-1)*l.center),right:Math.floor(this._bufferService.buffers.active.lines.length/(this._canvas.height-1)*l.right)}),this._lastKnownBufferLength=this._bufferService.buffers.normal.lines.length}_refreshCanvasDimensions(){this._canvas.style.width=`${this._width}px`,this._canvas.width=Math.round(this._width*this._coreBrowseService.dpr),this._canvas.style.height=`${this._screenElement.clientHeight}px`,this._canvas.height=Math.round(this._screenElement.clientHeight*this._coreBrowseService.dpr),this._refreshDrawConstants(),this._refreshColorZonePadding()}_refreshDecorations(){this._shouldUpdateDimensions&&this._refreshCanvasDimensions(),this._ctx.clearRect(0,0,this._canvas.width,this._canvas.height),this._colorZoneStore.clear();for(const e of this._decorationService.decorations)this._colorZoneStore.addDecoration(e);this._ctx.lineWidth=1;const e=this._colorZoneStore.zones;for(const t of e)"full"!==t.position&&this._renderColorZone(t);for(const t of e)"full"===t.position&&this._renderColorZone(t);this._shouldUpdateDimensions=!1,this._shouldUpdateAnchor=!1}_renderColorZone(e){this._ctx.fillStyle=e.color,this._ctx.fillRect(_[e.position||"full"],Math.round((this._canvas.height-1)*(e.startBufferLine/this._bufferService.buffers.active.lines.length)-l[e.position||"full"]/2),d[e.position||"full"],Math.round((this._canvas.height-1)*((e.endBufferLine-e.startBufferLine)/this._bufferService.buffers.active.lines.length)+l[e.position||"full"]))}_queueRefresh(e,t){this._shouldUpdateDimensions=e||this._shouldUpdateDimensions,this._shouldUpdateAnchor=t||this._shouldUpdateAnchor,void 0===this._animationFrame&&(this._animationFrame=this._coreBrowseService.window.requestAnimationFrame((()=>{this._refreshDecorations(),this._animationFrame=void 0})))}};t.OverviewRulerRenderer=u=s([r(2,c.IBufferService),r(3,c.IDecorationService),r(4,a.IRenderService),r(5,c.IOptionsService),r(6,a.ICoreBrowserService)],u)},2950:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.CompositionHelper=void 0;const n=i(4725),o=i(2585),a=i(2584);let h=t.CompositionHelper=class{get isComposing(){return this._isComposing}constructor(e,t,i,s,r,n){this._textarea=e,this._compositionView=t,this._bufferService=i,this._optionsService=s,this._coreService=r,this._renderService=n,this._isComposing=!1,this._isSendingComposition=!1,this._compositionPosition={start:0,end:0},this._dataAlreadySent=""}compositionstart(){this._isComposing=!0,this._compositionPosition.start=this._textarea.value.length,this._compositionView.textContent="",this._dataAlreadySent="",this._compositionView.classList.add("active")}compositionupdate(e){this._compositionView.textContent=e.data,this.updateCompositionElements(),setTimeout((()=>{this._compositionPosition.end=this._textarea.value.length}),0)}compositionend(){this._finalizeComposition(!0)}keydown(e){if(this._isComposing||this._isSendingComposition){if(229===e.keyCode)return!1;if(16===e.keyCode||17===e.keyCode||18===e.keyCode)return!1;this._finalizeComposition(!1)}return 229!==e.keyCode||(this._handleAnyTextareaChanges(),!1)}_finalizeComposition(e){if(this._compositionView.classList.remove("active"),this._isComposing=!1,e){const e={start:this._compositionPosition.start,end:this._compositionPosition.end};this._isSendingComposition=!0,setTimeout((()=>{if(this._isSendingComposition){let t;this._isSendingComposition=!1,e.start+=this._dataAlreadySent.length,t=this._isComposing?this._textarea.value.substring(e.start,e.end):this._textarea.value.substring(e.start),t.length>0&&this._coreService.triggerDataEvent(t,!0)}}),0)}else{this._isSendingComposition=!1;const e=this._textarea.value.substring(this._compositionPosition.start,this._compositionPosition.end);this._coreService.triggerDataEvent(e,!0)}}_handleAnyTextareaChanges(){const e=this._textarea.value;setTimeout((()=>{if(!this._isComposing){const t=this._textarea.value,i=t.replace(e,"");this._dataAlreadySent=i,t.length>e.length?this._coreService.triggerDataEvent(i,!0):t.length<e.length?this._coreService.triggerDataEvent(`${a.C0.DEL}`,!0):t.length===e.length&&t!==e&&this._coreService.triggerDataEvent(t,!0)}}),0)}updateCompositionElements(e){if(this._isComposing){if(this._bufferService.buffer.isCursorInViewport){const e=Math.min(this._bufferService.buffer.x,this._bufferService.cols-1),t=this._renderService.dimensions.css.cell.height,i=this._bufferService.buffer.y*this._renderService.dimensions.css.cell.height,s=e*this._renderService.dimensions.css.cell.width;this._compositionView.style.left=s+"px",this._compositionView.style.top=i+"px",this._compositionView.style.height=t+"px",this._compositionView.style.lineHeight=t+"px",this._compositionView.style.fontFamily=this._optionsService.rawOptions.fontFamily,this._compositionView.style.fontSize=this._optionsService.rawOptions.fontSize+"px";const r=this._compositionView.getBoundingClientRect();this._textarea.style.left=s+"px",this._textarea.style.top=i+"px",this._textarea.style.width=Math.max(r.width,1)+"px",this._textarea.style.height=Math.max(r.height,1)+"px",this._textarea.style.lineHeight=r.height+"px"}e||setTimeout((()=>this.updateCompositionElements(!0)),0)}}};t.CompositionHelper=h=s([r(2,o.IBufferService),r(3,o.IOptionsService),r(4,o.ICoreService),r(5,n.IRenderService)],h)},9806:(e,t)=>{function i(e,t,i){const s=i.getBoundingClientRect(),r=e.getComputedStyle(i),n=parseInt(r.getPropertyValue("padding-left")),o=parseInt(r.getPropertyValue("padding-top"));return[t.clientX-s.left-n,t.clientY-s.top-o]}Object.defineProperty(t,"__esModule",{value:!0}),t.getCoords=t.getCoordsRelativeToElement=void 0,t.getCoordsRelativeToElement=i,t.getCoords=function(e,t,s,r,n,o,a,h,c){if(!o)return;const l=i(e,t,s);return l?(l[0]=Math.ceil((l[0]+(c?a/2:0))/a),l[1]=Math.ceil(l[1]/h),l[0]=Math.min(Math.max(l[0],1),r+(c?1:0)),l[1]=Math.min(Math.max(l[1],1),n),l):void 0}},9504:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.moveToCellSequence=void 0;const s=i(2584);function r(e,t,i,s){const r=e-n(e,i),a=t-n(t,i),l=Math.abs(r-a)-function(e,t,i){let s=0;const r=e-n(e,i),a=t-n(t,i);for(let n=0;n<Math.abs(r-a);n++){const a="A"===o(e,t)?-1:1,h=i.buffer.lines.get(r+a*n);(null==h?void 0:h.isWrapped)&&s++}return s}(e,t,i);return c(l,h(o(e,t),s))}function n(e,t){let i=0,s=t.buffer.lines.get(e),r=null==s?void 0:s.isWrapped;for(;r&&e>=0&&e<t.rows;)i++,s=t.buffer.lines.get(--e),r=null==s?void 0:s.isWrapped;return i}function o(e,t){return e>t?"A":"B"}function a(e,t,i,s,r,n){let o=e,a=t,h="";for(;o!==i||a!==s;)o+=r?1:-1,r&&o>n.cols-1?(h+=n.buffer.translateBufferLineToString(a,!1,e,o),o=0,e=0,a++):!r&&o<0&&(h+=n.buffer.translateBufferLineToString(a,!1,0,e+1),o=n.cols-1,e=o,a--);return h+n.buffer.translateBufferLineToString(a,!1,e,o)}function h(e,t){const i=t?"O":"[";return s.C0.ESC+i+e}function c(e,t){e=Math.floor(e);let i="";for(let s=0;s<e;s++)i+=t;return i}t.moveToCellSequence=function(e,t,i,s){const o=i.buffer.x,l=i.buffer.y;if(!i.buffer.hasScrollback)return function(e,t,i,s,o,l){return 0===r(t,s,o,l).length?"":c(a(e,t,e,t-n(t,o),!1,o).length,h("D",l))}(o,l,0,t,i,s)+r(l,t,i,s)+function(e,t,i,s,o,l){let d;d=r(t,s,o,l).length>0?s-n(s,o):t;const _=s,u=function(e,t,i,s,o,a){let h;return h=r(i,s,o,a).length>0?s-n(s,o):t,e<i&&h<=s||e>=i&&h<s?"C":"D"}(e,t,i,s,o,l);return c(a(e,d,i,_,"C"===u,o).length,h(u,l))}(o,l,e,t,i,s);let d;if(l===t)return d=o>e?"D":"C",c(Math.abs(o-e),h(d,s));d=l>t?"D":"C";const _=Math.abs(l-t);return c(function(e,t){return t.cols-e}(l>t?e:o,i)+(_-1)*i.cols+1+((l>t?o:e)-1),h(d,s))}},1296:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.DomRenderer=void 0;const n=i(3787),o=i(2550),a=i(2223),h=i(6171),c=i(4725),l=i(8055),d=i(8460),_=i(844),u=i(2585),f="xterm-dom-renderer-owner-",v="xterm-rows",p="xterm-fg-",g="xterm-bg-",m="xterm-focus",S="xterm-selection";let C=1,b=t.DomRenderer=class extends _.Disposable{constructor(e,t,i,s,r,a,c,l,u,p){super(),this._element=e,this._screenElement=t,this._viewportElement=i,this._linkifier2=s,this._charSizeService=a,this._optionsService=c,this._bufferService=l,this._coreBrowserService=u,this._themeService=p,this._terminalClass=C++,this._rowElements=[],this.onRequestRedraw=this.register(new d.EventEmitter).event,this._rowContainer=document.createElement("div"),this._rowContainer.classList.add(v),this._rowContainer.style.lineHeight="normal",this._rowContainer.setAttribute("aria-hidden","true"),this._refreshRowElements(this._bufferService.cols,this._bufferService.rows),this._selectionContainer=document.createElement("div"),this._selectionContainer.classList.add(S),this._selectionContainer.setAttribute("aria-hidden","true"),this.dimensions=(0,h.createRenderDimensions)(),this._updateDimensions(),this.register(this._optionsService.onOptionChange((()=>this._handleOptionsChanged()))),this.register(this._themeService.onChangeColors((e=>this._injectCss(e)))),this._injectCss(this._themeService.colors),this._rowFactory=r.createInstance(n.DomRendererRowFactory,document),this._element.classList.add(f+this._terminalClass),this._screenElement.appendChild(this._rowContainer),this._screenElement.appendChild(this._selectionContainer),this.register(this._linkifier2.onShowLinkUnderline((e=>this._handleLinkHover(e)))),this.register(this._linkifier2.onHideLinkUnderline((e=>this._handleLinkLeave(e)))),this.register((0,_.toDisposable)((()=>{this._element.classList.remove(f+this._terminalClass),this._rowContainer.remove(),this._selectionContainer.remove(),this._widthCache.dispose(),this._themeStyleElement.remove(),this._dimensionsStyleElement.remove()}))),this._widthCache=new o.WidthCache(document),this._widthCache.setFont(this._optionsService.rawOptions.fontFamily,this._optionsService.rawOptions.fontSize,this._optionsService.rawOptions.fontWeight,this._optionsService.rawOptions.fontWeightBold),this._setDefaultSpacing()}_updateDimensions(){const e=this._coreBrowserService.dpr;this.dimensions.device.char.width=this._charSizeService.width*e,this.dimensions.device.char.height=Math.ceil(this._charSizeService.height*e),this.dimensions.device.cell.width=this.dimensions.device.char.width+Math.round(this._optionsService.rawOptions.letterSpacing),this.dimensions.device.cell.height=Math.floor(this.dimensions.device.char.height*this._optionsService.rawOptions.lineHeight),this.dimensions.device.char.left=0,this.dimensions.device.char.top=0,this.dimensions.device.canvas.width=this.dimensions.device.cell.width*this._bufferService.cols,this.dimensions.device.canvas.height=this.dimensions.device.cell.height*this._bufferService.rows,this.dimensions.css.canvas.width=Math.round(this.dimensions.device.canvas.width/e),this.dimensions.css.canvas.height=Math.round(this.dimensions.device.canvas.height/e),this.dimensions.css.cell.width=this.dimensions.css.canvas.width/this._bufferService.cols,this.dimensions.css.cell.height=this.dimensions.css.canvas.height/this._bufferService.rows;for(const e of this._rowElements)e.style.width=`${this.dimensions.css.canvas.width}px`,e.style.height=`${this.dimensions.css.cell.height}px`,e.style.lineHeight=`${this.dimensions.css.cell.height}px`,e.style.overflow="hidden";this._dimensionsStyleElement||(this._dimensionsStyleElement=document.createElement("style"),this._screenElement.appendChild(this._dimensionsStyleElement));const t=`${this._terminalSelector} .${v} span { display: inline-block; height: 100%; vertical-align: top;}`;this._dimensionsStyleElement.textContent=t,this._selectionContainer.style.height=this._viewportElement.style.height,this._screenElement.style.width=`${this.dimensions.css.canvas.width}px`,this._screenElement.style.height=`${this.dimensions.css.canvas.height}px`}_injectCss(e){this._themeStyleElement||(this._themeStyleElement=document.createElement("style"),this._screenElement.appendChild(this._themeStyleElement));let t=`${this._terminalSelector} .${v} { color: ${e.foreground.css}; font-family: ${this._optionsService.rawOptions.fontFamily}; font-size: ${this._optionsService.rawOptions.fontSize}px; font-kerning: none; white-space: pre}`;t+=`${this._terminalSelector} .${v} .xterm-dim { color: ${l.color.multiplyOpacity(e.foreground,.5).css};}`,t+=`${this._terminalSelector} span:not(.xterm-bold) { font-weight: ${this._optionsService.rawOptions.fontWeight};}${this._terminalSelector} span.xterm-bold { font-weight: ${this._optionsService.rawOptions.fontWeightBold};}${this._terminalSelector} span.xterm-italic { font-style: italic;}`,t+="@keyframes blink_box_shadow_"+this._terminalClass+" { 50% {  border-bottom-style: hidden; }}",t+="@keyframes blink_block_"+this._terminalClass+" { 0% {"+`  background-color: ${e.cursor.css};`+`  color: ${e.cursorAccent.css}; } 50% {  background-color: inherit;`+`  color: ${e.cursor.css}; }}`,t+=`${this._terminalSelector} .${v}.${m} .xterm-cursor.xterm-cursor-blink:not(.xterm-cursor-block) { animation: blink_box_shadow_`+this._terminalClass+" 1s step-end infinite;}"+`${this._terminalSelector} .${v}.${m} .xterm-cursor.xterm-cursor-blink.xterm-cursor-block { animation: blink_block_`+this._terminalClass+" 1s step-end infinite;}"+`${this._terminalSelector} .${v} .xterm-cursor.xterm-cursor-block {`+` background-color: ${e.cursor.css};`+` color: ${e.cursorAccent.css};}`+`${this._terminalSelector} .${v} .xterm-cursor.xterm-cursor-outline {`+` outline: 1px solid ${e.cursor.css}; outline-offset: -1px;}`+`${this._terminalSelector} .${v} .xterm-cursor.xterm-cursor-bar {`+` box-shadow: ${this._optionsService.rawOptions.cursorWidth}px 0 0 ${e.cursor.css} inset;}`+`${this._terminalSelector} .${v} .xterm-cursor.xterm-cursor-underline {`+` border-bottom: 1px ${e.cursor.css}; border-bottom-style: solid; height: calc(100% - 1px);}`,t+=`${this._terminalSelector} .${S} { position: absolute; top: 0; left: 0; z-index: 1; pointer-events: none;}${this._terminalSelector}.focus .${S} div { position: absolute; background-color: ${e.selectionBackgroundOpaque.css};}${this._terminalSelector} .${S} div { position: absolute; background-color: ${e.selectionInactiveBackgroundOpaque.css};}`;for(const[i,s]of e.ansi.entries())t+=`${this._terminalSelector} .${p}${i} { color: ${s.css}; }${this._terminalSelector} .${p}${i}.xterm-dim { color: ${l.color.multiplyOpacity(s,.5).css}; }${this._terminalSelector} .${g}${i} { background-color: ${s.css}; }`;t+=`${this._terminalSelector} .${p}${a.INVERTED_DEFAULT_COLOR} { color: ${l.color.opaque(e.background).css}; }${this._terminalSelector} .${p}${a.INVERTED_DEFAULT_COLOR}.xterm-dim { color: ${l.color.multiplyOpacity(l.color.opaque(e.background),.5).css}; }${this._terminalSelector} .${g}${a.INVERTED_DEFAULT_COLOR} { background-color: ${e.foreground.css}; }`,this._themeStyleElement.textContent=t}_setDefaultSpacing(){const e=this.dimensions.css.cell.width-this._widthCache.get("W",!1,!1);this._rowContainer.style.letterSpacing=`${e}px`,this._rowFactory.defaultSpacing=e}handleDevicePixelRatioChange(){this._updateDimensions(),this._widthCache.clear(),this._setDefaultSpacing()}_refreshRowElements(e,t){for(let e=this._rowElements.length;e<=t;e++){const e=document.createElement("div");this._rowContainer.appendChild(e),this._rowElements.push(e)}for(;this._rowElements.length>t;)this._rowContainer.removeChild(this._rowElements.pop())}handleResize(e,t){this._refreshRowElements(e,t),this._updateDimensions()}handleCharSizeChanged(){this._updateDimensions(),this._widthCache.clear(),this._setDefaultSpacing()}handleBlur(){this._rowContainer.classList.remove(m)}handleFocus(){this._rowContainer.classList.add(m),this.renderRows(this._bufferService.buffer.y,this._bufferService.buffer.y)}handleSelectionChanged(e,t,i){if(this._selectionContainer.replaceChildren(),this._rowFactory.handleSelectionChanged(e,t,i),this.renderRows(0,this._bufferService.rows-1),!e||!t)return;const s=e[1]-this._bufferService.buffer.ydisp,r=t[1]-this._bufferService.buffer.ydisp,n=Math.max(s,0),o=Math.min(r,this._bufferService.rows-1);if(n>=this._bufferService.rows||o<0)return;const a=document.createDocumentFragment();if(i){const i=e[0]>t[0];a.appendChild(this._createSelectionElement(n,i?t[0]:e[0],i?e[0]:t[0],o-n+1))}else{const i=s===n?e[0]:0,h=n===r?t[0]:this._bufferService.cols;a.appendChild(this._createSelectionElement(n,i,h));const c=o-n-1;if(a.appendChild(this._createSelectionElement(n+1,0,this._bufferService.cols,c)),n!==o){const e=r===o?t[0]:this._bufferService.cols;a.appendChild(this._createSelectionElement(o,0,e))}}this._selectionContainer.appendChild(a)}_createSelectionElement(e,t,i,s=1){const r=document.createElement("div");return r.style.height=s*this.dimensions.css.cell.height+"px",r.style.top=e*this.dimensions.css.cell.height+"px",r.style.left=t*this.dimensions.css.cell.width+"px",r.style.width=this.dimensions.css.cell.width*(i-t)+"px",r}handleCursorMove(){}_handleOptionsChanged(){this._updateDimensions(),this._injectCss(this._themeService.colors),this._widthCache.setFont(this._optionsService.rawOptions.fontFamily,this._optionsService.rawOptions.fontSize,this._optionsService.rawOptions.fontWeight,this._optionsService.rawOptions.fontWeightBold),this._setDefaultSpacing()}clear(){for(const e of this._rowElements)e.replaceChildren()}renderRows(e,t){const i=this._bufferService.buffer,s=i.ybase+i.y,r=Math.min(i.x,this._bufferService.cols-1),n=this._optionsService.rawOptions.cursorBlink,o=this._optionsService.rawOptions.cursorStyle,a=this._optionsService.rawOptions.cursorInactiveStyle;for(let h=e;h<=t;h++){const e=h+i.ydisp,t=this._rowElements[h],c=i.lines.get(e);if(!t||!c)break;t.replaceChildren(...this._rowFactory.createRow(c,e,e===s,o,a,r,n,this.dimensions.css.cell.width,this._widthCache,-1,-1))}}get _terminalSelector(){return`.${f}${this._terminalClass}`}_handleLinkHover(e){this._setCellUnderline(e.x1,e.x2,e.y1,e.y2,e.cols,!0)}_handleLinkLeave(e){this._setCellUnderline(e.x1,e.x2,e.y1,e.y2,e.cols,!1)}_setCellUnderline(e,t,i,s,r,n){i<0&&(e=0),s<0&&(t=0);const o=this._bufferService.rows-1;i=Math.max(Math.min(i,o),0),s=Math.max(Math.min(s,o),0),r=Math.min(r,this._bufferService.cols);const a=this._bufferService.buffer,h=a.ybase+a.y,c=Math.min(a.x,r-1),l=this._optionsService.rawOptions.cursorBlink,d=this._optionsService.rawOptions.cursorStyle,_=this._optionsService.rawOptions.cursorInactiveStyle;for(let o=i;o<=s;++o){const u=o+a.ydisp,f=this._rowElements[o],v=a.lines.get(u);if(!f||!v)break;f.replaceChildren(...this._rowFactory.createRow(v,u,u===h,d,_,c,l,this.dimensions.css.cell.width,this._widthCache,n?o===i?e:0:-1,n?(o===s?t:r)-1:-1))}}};t.DomRenderer=b=s([r(4,u.IInstantiationService),r(5,c.ICharSizeService),r(6,u.IOptionsService),r(7,u.IBufferService),r(8,c.ICoreBrowserService),r(9,c.IThemeService)],b)},3787:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.DomRendererRowFactory=void 0;const n=i(2223),o=i(643),a=i(511),h=i(2585),c=i(8055),l=i(4725),d=i(4269),_=i(6171),u=i(3734);let f=t.DomRendererRowFactory=class{constructor(e,t,i,s,r,n,o){this._document=e,this._characterJoinerService=t,this._optionsService=i,this._coreBrowserService=s,this._coreService=r,this._decorationService=n,this._themeService=o,this._workCell=new a.CellData,this._columnSelectMode=!1,this.defaultSpacing=0}handleSelectionChanged(e,t,i){this._selectionStart=e,this._selectionEnd=t,this._columnSelectMode=i}createRow(e,t,i,s,r,a,h,l,_,f,p){const g=[],m=this._characterJoinerService.getJoinedCharacters(t),S=this._themeService.colors;let C,b=e.getNoBgTrimmedLength();i&&b<a+1&&(b=a+1);let y=0,w="",E=0,k=0,L=0,D=!1,R=0,x=!1,A=0;const B=[],T=-1!==f&&-1!==p;for(let M=0;M<b;M++){e.loadCell(M,this._workCell);let b=this._workCell.getWidth();if(0===b)continue;let O=!1,P=M,I=this._workCell;if(m.length>0&&M===m[0][0]){O=!0;const t=m.shift();I=new d.JoinedCellData(this._workCell,e.translateToString(!0,t[0],t[1]),t[1]-t[0]),P=t[1]-1,b=I.getWidth()}const H=this._isCellInSelection(M,t),F=i&&M===a,W=T&&M>=f&&M<=p;let U=!1;this._decorationService.forEachDecorationAtCell(M,t,void 0,(e=>{U=!0}));let N=I.getChars()||o.WHITESPACE_CELL_CHAR;if(" "===N&&(I.isUnderline()||I.isOverline())&&(N=" "),A=b*l-_.get(N,I.isBold(),I.isItalic()),C){if(y&&(H&&x||!H&&!x&&I.bg===E)&&(H&&x&&S.selectionForeground||I.fg===k)&&I.extended.ext===L&&W===D&&A===R&&!F&&!O&&!U){w+=N,y++;continue}y&&(C.textContent=w),C=this._document.createElement("span"),y=0,w=""}else C=this._document.createElement("span");if(E=I.bg,k=I.fg,L=I.extended.ext,D=W,R=A,x=H,O&&a>=M&&a<=P&&(a=M),!this._coreService.isCursorHidden&&F)if(B.push("xterm-cursor"),this._coreBrowserService.isFocused)h&&B.push("xterm-cursor-blink"),B.push("bar"===s?"xterm-cursor-bar":"underline"===s?"xterm-cursor-underline":"xterm-cursor-block");else if(r)switch(r){case"outline":B.push("xterm-cursor-outline");break;case"block":B.push("xterm-cursor-block");break;case"bar":B.push("xterm-cursor-bar");break;case"underline":B.push("xterm-cursor-underline")}if(I.isBold()&&B.push("xterm-bold"),I.isItalic()&&B.push("xterm-italic"),I.isDim()&&B.push("xterm-dim"),w=I.isInvisible()?o.WHITESPACE_CELL_CHAR:I.getChars()||o.WHITESPACE_CELL_CHAR,I.isUnderline()&&(B.push(`xterm-underline-${I.extended.underlineStyle}`)," "===w&&(w=" "),!I.isUnderlineColorDefault()))if(I.isUnderlineColorRGB())C.style.textDecorationColor=`rgb(${u.AttributeData.toColorRGB(I.getUnderlineColor()).join(",")})`;else{let e=I.getUnderlineColor();this._optionsService.rawOptions.drawBoldTextInBrightColors&&I.isBold()&&e<8&&(e+=8),C.style.textDecorationColor=S.ansi[e].css}I.isOverline()&&(B.push("xterm-overline")," "===w&&(w=" ")),I.isStrikethrough()&&B.push("xterm-strikethrough"),W&&(C.style.textDecoration="underline");let $=I.getFgColor(),j=I.getFgColorMode(),z=I.getBgColor(),K=I.getBgColorMode();const q=!!I.isInverse();if(q){const e=$;$=z,z=e;const t=j;j=K,K=t}let V,G,X,J=!1;switch(this._decorationService.forEachDecorationAtCell(M,t,void 0,(e=>{"top"!==e.options.layer&&J||(e.backgroundColorRGB&&(K=50331648,z=e.backgroundColorRGB.rgba>>8&16777215,V=e.backgroundColorRGB),e.foregroundColorRGB&&(j=50331648,$=e.foregroundColorRGB.rgba>>8&16777215,G=e.foregroundColorRGB),J="top"===e.options.layer)})),!J&&H&&(V=this._coreBrowserService.isFocused?S.selectionBackgroundOpaque:S.selectionInactiveBackgroundOpaque,z=V.rgba>>8&16777215,K=50331648,J=!0,S.selectionForeground&&(j=50331648,$=S.selectionForeground.rgba>>8&16777215,G=S.selectionForeground)),J&&B.push("xterm-decoration-top"),K){case 16777216:case 33554432:X=S.ansi[z],B.push(`xterm-bg-${z}`);break;case 50331648:X=c.rgba.toColor(z>>16,z>>8&255,255&z),this._addStyle(C,`background-color:#${v((z>>>0).toString(16),"0",6)}`);break;default:q?(X=S.foreground,B.push(`xterm-bg-${n.INVERTED_DEFAULT_COLOR}`)):X=S.background}switch(V||I.isDim()&&(V=c.color.multiplyOpacity(X,.5)),j){case 16777216:case 33554432:I.isBold()&&$<8&&this._optionsService.rawOptions.drawBoldTextInBrightColors&&($+=8),this._applyMinimumContrast(C,X,S.ansi[$],I,V,void 0)||B.push(`xterm-fg-${$}`);break;case 50331648:const e=c.rgba.toColor($>>16&255,$>>8&255,255&$);this._applyMinimumContrast(C,X,e,I,V,G)||this._addStyle(C,`color:#${v($.toString(16),"0",6)}`);break;default:this._applyMinimumContrast(C,X,S.foreground,I,V,void 0)||q&&B.push(`xterm-fg-${n.INVERTED_DEFAULT_COLOR}`)}B.length&&(C.className=B.join(" "),B.length=0),F||O||U?C.textContent=w:y++,A!==this.defaultSpacing&&(C.style.letterSpacing=`${A}px`),g.push(C),M=P}return C&&y&&(C.textContent=w),g}_applyMinimumContrast(e,t,i,s,r,n){if(1===this._optionsService.rawOptions.minimumContrastRatio||(0,_.excludeFromContrastRatioDemands)(s.getCode()))return!1;const o=this._getContrastCache(s);let a;if(r||n||(a=o.getColor(t.rgba,i.rgba)),void 0===a){const e=this._optionsService.rawOptions.minimumContrastRatio/(s.isDim()?2:1);a=c.color.ensureContrastRatio(r||t,n||i,e),o.setColor((r||t).rgba,(n||i).rgba,null!=a?a:null)}return!!a&&(this._addStyle(e,`color:${a.css}`),!0)}_getContrastCache(e){return e.isDim()?this._themeService.colors.halfContrastCache:this._themeService.colors.contrastCache}_addStyle(e,t){e.setAttribute("style",`${e.getAttribute("style")||""}${t};`)}_isCellInSelection(e,t){const i=this._selectionStart,s=this._selectionEnd;return!(!i||!s)&&(this._columnSelectMode?i[0]<=s[0]?e>=i[0]&&t>=i[1]&&e<s[0]&&t<=s[1]:e<i[0]&&t>=i[1]&&e>=s[0]&&t<=s[1]:t>i[1]&&t<s[1]||i[1]===s[1]&&t===i[1]&&e>=i[0]&&e<s[0]||i[1]<s[1]&&t===s[1]&&e<s[0]||i[1]<s[1]&&t===i[1]&&e>=i[0])}};function v(e,t,i){for(;e.length<i;)e=t+e;return e}t.DomRendererRowFactory=f=s([r(1,l.ICharacterJoinerService),r(2,h.IOptionsService),r(3,l.ICoreBrowserService),r(4,h.ICoreService),r(5,h.IDecorationService),r(6,l.IThemeService)],f)},2550:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.WidthCache=void 0,t.WidthCache=class{constructor(e){this._flat=new Float32Array(256),this._font="",this._fontSize=0,this._weight="normal",this._weightBold="bold",this._measureElements=[],this._container=e.createElement("div"),this._container.style.position="absolute",this._container.style.top="-50000px",this._container.style.width="50000px",this._container.style.whiteSpace="pre",this._container.style.fontKerning="none";const t=e.createElement("span"),i=e.createElement("span");i.style.fontWeight="bold";const s=e.createElement("span");s.style.fontStyle="italic";const r=e.createElement("span");r.style.fontWeight="bold",r.style.fontStyle="italic",this._measureElements=[t,i,s,r],this._container.appendChild(t),this._container.appendChild(i),this._container.appendChild(s),this._container.appendChild(r),e.body.appendChild(this._container),this.clear()}dispose(){this._container.remove(),this._measureElements.length=0,this._holey=void 0}clear(){this._flat.fill(-9999),this._holey=new Map}setFont(e,t,i,s){e===this._font&&t===this._fontSize&&i===this._weight&&s===this._weightBold||(this._font=e,this._fontSize=t,this._weight=i,this._weightBold=s,this._container.style.fontFamily=this._font,this._container.style.fontSize=`${this._fontSize}px`,this._measureElements[0].style.fontWeight=`${i}`,this._measureElements[1].style.fontWeight=`${s}`,this._measureElements[2].style.fontWeight=`${i}`,this._measureElements[3].style.fontWeight=`${s}`,this.clear())}get(e,t,i){let s=0;if(!t&&!i&&1===e.length&&(s=e.charCodeAt(0))<256)return-9999!==this._flat[s]?this._flat[s]:this._flat[s]=this._measure(e,0);let r=e;t&&(r+="B"),i&&(r+="I");let n=this._holey.get(r);if(void 0===n){let s=0;t&&(s|=1),i&&(s|=2),n=this._measure(e,s),this._holey.set(r,n)}return n}_measure(e,t){const i=this._measureElements[t];return i.textContent=e.repeat(32),i.offsetWidth/32}}},2223:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.TEXT_BASELINE=t.DIM_OPACITY=t.INVERTED_DEFAULT_COLOR=void 0;const s=i(6114);t.INVERTED_DEFAULT_COLOR=257,t.DIM_OPACITY=.5,t.TEXT_BASELINE=s.isFirefox||s.isLegacyEdge?"bottom":"ideographic"},6171:(e,t)=>{function i(e){return 57508<=e&&e<=57558}Object.defineProperty(t,"__esModule",{value:!0}),t.createRenderDimensions=t.excludeFromContrastRatioDemands=t.isRestrictedPowerlineGlyph=t.isPowerlineGlyph=t.throwIfFalsy=void 0,t.throwIfFalsy=function(e){if(!e)throw new Error("value must not be falsy");return e},t.isPowerlineGlyph=i,t.isRestrictedPowerlineGlyph=function(e){return 57520<=e&&e<=57527},t.excludeFromContrastRatioDemands=function(e){return i(e)||function(e){return 9472<=e&&e<=9631}(e)},t.createRenderDimensions=function(){return{css:{canvas:{width:0,height:0},cell:{width:0,height:0}},device:{canvas:{width:0,height:0},cell:{width:0,height:0},char:{width:0,height:0,left:0,top:0}}}}},456:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.SelectionModel=void 0,t.SelectionModel=class{constructor(e){this._bufferService=e,this.isSelectAllActive=!1,this.selectionStartLength=0}clearSelection(){this.selectionStart=void 0,this.selectionEnd=void 0,this.isSelectAllActive=!1,this.selectionStartLength=0}get finalSelectionStart(){return this.isSelectAllActive?[0,0]:this.selectionEnd&&this.selectionStart&&this.areSelectionValuesReversed()?this.selectionEnd:this.selectionStart}get finalSelectionEnd(){if(this.isSelectAllActive)return[this._bufferService.cols,this._bufferService.buffer.ybase+this._bufferService.rows-1];if(this.selectionStart){if(!this.selectionEnd||this.areSelectionValuesReversed()){const e=this.selectionStart[0]+this.selectionStartLength;return e>this._bufferService.cols?e%this._bufferService.cols==0?[this._bufferService.cols,this.selectionStart[1]+Math.floor(e/this._bufferService.cols)-1]:[e%this._bufferService.cols,this.selectionStart[1]+Math.floor(e/this._bufferService.cols)]:[e,this.selectionStart[1]]}if(this.selectionStartLength&&this.selectionEnd[1]===this.selectionStart[1]){const e=this.selectionStart[0]+this.selectionStartLength;return e>this._bufferService.cols?[e%this._bufferService.cols,this.selectionStart[1]+Math.floor(e/this._bufferService.cols)]:[Math.max(e,this.selectionEnd[0]),this.selectionEnd[1]]}return this.selectionEnd}}areSelectionValuesReversed(){const e=this.selectionStart,t=this.selectionEnd;return!(!e||!t)&&(e[1]>t[1]||e[1]===t[1]&&e[0]>t[0])}handleTrim(e){return this.selectionStart&&(this.selectionStart[1]-=e),this.selectionEnd&&(this.selectionEnd[1]-=e),this.selectionEnd&&this.selectionEnd[1]<0?(this.clearSelection(),!0):(this.selectionStart&&this.selectionStart[1]<0&&(this.selectionStart[1]=0),!1)}}},428:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.CharSizeService=void 0;const n=i(2585),o=i(8460),a=i(844);let h=t.CharSizeService=class extends a.Disposable{get hasValidSize(){return this.width>0&&this.height>0}constructor(e,t,i){super(),this._optionsService=i,this.width=0,this.height=0,this._onCharSizeChange=this.register(new o.EventEmitter),this.onCharSizeChange=this._onCharSizeChange.event,this._measureStrategy=new c(e,t,this._optionsService),this.register(this._optionsService.onMultipleOptionChange(["fontFamily","fontSize"],(()=>this.measure())))}measure(){const e=this._measureStrategy.measure();e.width===this.width&&e.height===this.height||(this.width=e.width,this.height=e.height,this._onCharSizeChange.fire())}};t.CharSizeService=h=s([r(2,n.IOptionsService)],h);class c{constructor(e,t,i){this._document=e,this._parentElement=t,this._optionsService=i,this._result={width:0,height:0},this._measureElement=this._document.createElement("span"),this._measureElement.classList.add("xterm-char-measure-element"),this._measureElement.textContent="W".repeat(32),this._measureElement.setAttribute("aria-hidden","true"),this._measureElement.style.whiteSpace="pre",this._measureElement.style.fontKerning="none",this._parentElement.appendChild(this._measureElement)}measure(){this._measureElement.style.fontFamily=this._optionsService.rawOptions.fontFamily,this._measureElement.style.fontSize=`${this._optionsService.rawOptions.fontSize}px`;const e={height:Number(this._measureElement.offsetHeight),width:Number(this._measureElement.offsetWidth)};return 0!==e.width&&0!==e.height&&(this._result.width=e.width/32,this._result.height=Math.ceil(e.height)),this._result}}},4269:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.CharacterJoinerService=t.JoinedCellData=void 0;const n=i(3734),o=i(643),a=i(511),h=i(2585);class c extends n.AttributeData{constructor(e,t,i){super(),this.content=0,this.combinedData="",this.fg=e.fg,this.bg=e.bg,this.combinedData=t,this._width=i}isCombined(){return 2097152}getWidth(){return this._width}getChars(){return this.combinedData}getCode(){return 2097151}setFromCharData(e){throw new Error("not implemented")}getAsCharData(){return[this.fg,this.getChars(),this.getWidth(),this.getCode()]}}t.JoinedCellData=c;let l=t.CharacterJoinerService=class e{constructor(e){this._bufferService=e,this._characterJoiners=[],this._nextCharacterJoinerId=0,this._workCell=new a.CellData}register(e){const t={id:this._nextCharacterJoinerId++,handler:e};return this._characterJoiners.push(t),t.id}deregister(e){for(let t=0;t<this._characterJoiners.length;t++)if(this._characterJoiners[t].id===e)return this._characterJoiners.splice(t,1),!0;return!1}getJoinedCharacters(e){if(0===this._characterJoiners.length)return[];const t=this._bufferService.buffer.lines.get(e);if(!t||0===t.length)return[];const i=[],s=t.translateToString(!0);let r=0,n=0,a=0,h=t.getFg(0),c=t.getBg(0);for(let e=0;e<t.getTrimmedLength();e++)if(t.loadCell(e,this._workCell),0!==this._workCell.getWidth()){if(this._workCell.fg!==h||this._workCell.bg!==c){if(e-r>1){const e=this._getJoinedRanges(s,a,n,t,r);for(let t=0;t<e.length;t++)i.push(e[t])}r=e,a=n,h=this._workCell.fg,c=this._workCell.bg}n+=this._workCell.getChars().length||o.WHITESPACE_CELL_CHAR.length}if(this._bufferService.cols-r>1){const e=this._getJoinedRanges(s,a,n,t,r);for(let t=0;t<e.length;t++)i.push(e[t])}return i}_getJoinedRanges(t,i,s,r,n){const o=t.substring(i,s);let a=[];try{a=this._characterJoiners[0].handler(o)}catch(e){console.error(e)}for(let t=1;t<this._characterJoiners.length;t++)try{const i=this._characterJoiners[t].handler(o);for(let t=0;t<i.length;t++)e._mergeRanges(a,i[t])}catch(e){console.error(e)}return this._stringRangesToCellRanges(a,r,n),a}_stringRangesToCellRanges(e,t,i){let s=0,r=!1,n=0,a=e[s];if(a){for(let h=i;h<this._bufferService.cols;h++){const i=t.getWidth(h),c=t.getString(h).length||o.WHITESPACE_CELL_CHAR.length;if(0!==i){if(!r&&a[0]<=n&&(a[0]=h,r=!0),a[1]<=n){if(a[1]=h,a=e[++s],!a)break;a[0]<=n?(a[0]=h,r=!0):r=!1}n+=c}}a&&(a[1]=this._bufferService.cols)}}static _mergeRanges(e,t){let i=!1;for(let s=0;s<e.length;s++){const r=e[s];if(i){if(t[1]<=r[0])return e[s-1][1]=t[1],e;if(t[1]<=r[1])return e[s-1][1]=Math.max(t[1],r[1]),e.splice(s,1),e;e.splice(s,1),s--}else{if(t[1]<=r[0])return e.splice(s,0,t),e;if(t[1]<=r[1])return r[0]=Math.min(t[0],r[0]),e;t[0]<r[1]&&(r[0]=Math.min(t[0],r[0]),i=!0)}}return i?e[e.length-1][1]=t[1]:e.push(t),e}};t.CharacterJoinerService=l=s([r(0,h.IBufferService)],l)},5114:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.CoreBrowserService=void 0,t.CoreBrowserService=class{constructor(e,t){this._textarea=e,this.window=t,this._isFocused=!1,this._cachedIsFocused=void 0,this._textarea.addEventListener("focus",(()=>this._isFocused=!0)),this._textarea.addEventListener("blur",(()=>this._isFocused=!1))}get dpr(){return this.window.devicePixelRatio}get isFocused(){return void 0===this._cachedIsFocused&&(this._cachedIsFocused=this._isFocused&&this._textarea.ownerDocument.hasFocus(),queueMicrotask((()=>this._cachedIsFocused=void 0))),this._cachedIsFocused}}},8934:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.MouseService=void 0;const n=i(4725),o=i(9806);let a=t.MouseService=class{constructor(e,t){this._renderService=e,this._charSizeService=t}getCoords(e,t,i,s,r){return(0,o.getCoords)(window,e,t,i,s,this._charSizeService.hasValidSize,this._renderService.dimensions.css.cell.width,this._renderService.dimensions.css.cell.height,r)}getMouseReportCoords(e,t){const i=(0,o.getCoordsRelativeToElement)(window,e,t);if(this._charSizeService.hasValidSize)return i[0]=Math.min(Math.max(i[0],0),this._renderService.dimensions.css.canvas.width-1),i[1]=Math.min(Math.max(i[1],0),this._renderService.dimensions.css.canvas.height-1),{col:Math.floor(i[0]/this._renderService.dimensions.css.cell.width),row:Math.floor(i[1]/this._renderService.dimensions.css.cell.height),x:Math.floor(i[0]),y:Math.floor(i[1])}}};t.MouseService=a=s([r(0,n.IRenderService),r(1,n.ICharSizeService)],a)},3230:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.RenderService=void 0;const n=i(3656),o=i(6193),a=i(5596),h=i(4725),c=i(8460),l=i(844),d=i(7226),_=i(2585);let u=t.RenderService=class extends l.Disposable{get dimensions(){return this._renderer.value.dimensions}constructor(e,t,i,s,r,h,_,u){if(super(),this._rowCount=e,this._charSizeService=s,this._renderer=this.register(new l.MutableDisposable),this._pausedResizeTask=new d.DebouncedIdleTask,this._isPaused=!1,this._needsFullRefresh=!1,this._isNextRenderRedrawOnly=!0,this._needsSelectionRefresh=!1,this._canvasWidth=0,this._canvasHeight=0,this._selectionState={start:void 0,end:void 0,columnSelectMode:!1},this._onDimensionsChange=this.register(new c.EventEmitter),this.onDimensionsChange=this._onDimensionsChange.event,this._onRenderedViewportChange=this.register(new c.EventEmitter),this.onRenderedViewportChange=this._onRenderedViewportChange.event,this._onRender=this.register(new c.EventEmitter),this.onRender=this._onRender.event,this._onRefreshRequest=this.register(new c.EventEmitter),this.onRefreshRequest=this._onRefreshRequest.event,this._renderDebouncer=new o.RenderDebouncer(_.window,((e,t)=>this._renderRows(e,t))),this.register(this._renderDebouncer),this._screenDprMonitor=new a.ScreenDprMonitor(_.window),this._screenDprMonitor.setListener((()=>this.handleDevicePixelRatioChange())),this.register(this._screenDprMonitor),this.register(h.onResize((()=>this._fullRefresh()))),this.register(h.buffers.onBufferActivate((()=>{var e;return null===(e=this._renderer.value)||void 0===e?void 0:e.clear()}))),this.register(i.onOptionChange((()=>this._handleOptionsChanged()))),this.register(this._charSizeService.onCharSizeChange((()=>this.handleCharSizeChanged()))),this.register(r.onDecorationRegistered((()=>this._fullRefresh()))),this.register(r.onDecorationRemoved((()=>this._fullRefresh()))),this.register(i.onMultipleOptionChange(["customGlyphs","drawBoldTextInBrightColors","letterSpacing","lineHeight","fontFamily","fontSize","fontWeight","fontWeightBold","minimumContrastRatio"],(()=>{this.clear(),this.handleResize(h.cols,h.rows),this._fullRefresh()}))),this.register(i.onMultipleOptionChange(["cursorBlink","cursorStyle"],(()=>this.refreshRows(h.buffer.y,h.buffer.y,!0)))),this.register((0,n.addDisposableDomListener)(_.window,"resize",(()=>this.handleDevicePixelRatioChange()))),this.register(u.onChangeColors((()=>this._fullRefresh()))),"IntersectionObserver"in _.window){const e=new _.window.IntersectionObserver((e=>this._handleIntersectionChange(e[e.length-1])),{threshold:0});e.observe(t),this.register({dispose:()=>e.disconnect()})}}_handleIntersectionChange(e){this._isPaused=void 0===e.isIntersecting?0===e.intersectionRatio:!e.isIntersecting,this._isPaused||this._charSizeService.hasValidSize||this._charSizeService.measure(),!this._isPaused&&this._needsFullRefresh&&(this._pausedResizeTask.flush(),this.refreshRows(0,this._rowCount-1),this._needsFullRefresh=!1)}refreshRows(e,t,i=!1){this._isPaused?this._needsFullRefresh=!0:(i||(this._isNextRenderRedrawOnly=!1),this._renderDebouncer.refresh(e,t,this._rowCount))}_renderRows(e,t){this._renderer.value&&(e=Math.min(e,this._rowCount-1),t=Math.min(t,this._rowCount-1),this._renderer.value.renderRows(e,t),this._needsSelectionRefresh&&(this._renderer.value.handleSelectionChanged(this._selectionState.start,this._selectionState.end,this._selectionState.columnSelectMode),this._needsSelectionRefresh=!1),this._isNextRenderRedrawOnly||this._onRenderedViewportChange.fire({start:e,end:t}),this._onRender.fire({start:e,end:t}),this._isNextRenderRedrawOnly=!0)}resize(e,t){this._rowCount=t,this._fireOnCanvasResize()}_handleOptionsChanged(){this._renderer.value&&(this.refreshRows(0,this._rowCount-1),this._fireOnCanvasResize())}_fireOnCanvasResize(){this._renderer.value&&(this._renderer.value.dimensions.css.canvas.width===this._canvasWidth&&this._renderer.value.dimensions.css.canvas.height===this._canvasHeight||this._onDimensionsChange.fire(this._renderer.value.dimensions))}hasRenderer(){return!!this._renderer.value}setRenderer(e){this._renderer.value=e,this._renderer.value.onRequestRedraw((e=>this.refreshRows(e.start,e.end,!0))),this._needsSelectionRefresh=!0,this._fullRefresh()}addRefreshCallback(e){return this._renderDebouncer.addRefreshCallback(e)}_fullRefresh(){this._isPaused?this._needsFullRefresh=!0:this.refreshRows(0,this._rowCount-1)}clearTextureAtlas(){var e,t;this._renderer.value&&(null===(t=(e=this._renderer.value).clearTextureAtlas)||void 0===t||t.call(e),this._fullRefresh())}handleDevicePixelRatioChange(){this._charSizeService.measure(),this._renderer.value&&(this._renderer.value.handleDevicePixelRatioChange(),this.refreshRows(0,this._rowCount-1))}handleResize(e,t){this._renderer.value&&(this._isPaused?this._pausedResizeTask.set((()=>this._renderer.value.handleResize(e,t))):this._renderer.value.handleResize(e,t),this._fullRefresh())}handleCharSizeChanged(){var e;null===(e=this._renderer.value)||void 0===e||e.handleCharSizeChanged()}handleBlur(){var e;null===(e=this._renderer.value)||void 0===e||e.handleBlur()}handleFocus(){var e;null===(e=this._renderer.value)||void 0===e||e.handleFocus()}handleSelectionChanged(e,t,i){var s;this._selectionState.start=e,this._selectionState.end=t,this._selectionState.columnSelectMode=i,null===(s=this._renderer.value)||void 0===s||s.handleSelectionChanged(e,t,i)}handleCursorMove(){var e;null===(e=this._renderer.value)||void 0===e||e.handleCursorMove()}clear(){var e;null===(e=this._renderer.value)||void 0===e||e.clear()}};t.RenderService=u=s([r(2,_.IOptionsService),r(3,h.ICharSizeService),r(4,_.IDecorationService),r(5,_.IBufferService),r(6,h.ICoreBrowserService),r(7,h.IThemeService)],u)},9312:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.SelectionService=void 0;const n=i(9806),o=i(9504),a=i(456),h=i(4725),c=i(8460),l=i(844),d=i(6114),_=i(4841),u=i(511),f=i(2585),v=String.fromCharCode(160),p=new RegExp(v,"g");let g=t.SelectionService=class extends l.Disposable{constructor(e,t,i,s,r,n,o,h,d){super(),this._element=e,this._screenElement=t,this._linkifier=i,this._bufferService=s,this._coreService=r,this._mouseService=n,this._optionsService=o,this._renderService=h,this._coreBrowserService=d,this._dragScrollAmount=0,this._enabled=!0,this._workCell=new u.CellData,this._mouseDownTimeStamp=0,this._oldHasSelection=!1,this._oldSelectionStart=void 0,this._oldSelectionEnd=void 0,this._onLinuxMouseSelection=this.register(new c.EventEmitter),this.onLinuxMouseSelection=this._onLinuxMouseSelection.event,this._onRedrawRequest=this.register(new c.EventEmitter),this.onRequestRedraw=this._onRedrawRequest.event,this._onSelectionChange=this.register(new c.EventEmitter),this.onSelectionChange=this._onSelectionChange.event,this._onRequestScrollLines=this.register(new c.EventEmitter),this.onRequestScrollLines=this._onRequestScrollLines.event,this._mouseMoveListener=e=>this._handleMouseMove(e),this._mouseUpListener=e=>this._handleMouseUp(e),this._coreService.onUserInput((()=>{this.hasSelection&&this.clearSelection()})),this._trimListener=this._bufferService.buffer.lines.onTrim((e=>this._handleTrim(e))),this.register(this._bufferService.buffers.onBufferActivate((e=>this._handleBufferActivate(e)))),this.enable(),this._model=new a.SelectionModel(this._bufferService),this._activeSelectionMode=0,this.register((0,l.toDisposable)((()=>{this._removeMouseDownListeners()})))}reset(){this.clearSelection()}disable(){this.clearSelection(),this._enabled=!1}enable(){this._enabled=!0}get selectionStart(){return this._model.finalSelectionStart}get selectionEnd(){return this._model.finalSelectionEnd}get hasSelection(){const e=this._model.finalSelectionStart,t=this._model.finalSelectionEnd;return!(!e||!t||e[0]===t[0]&&e[1]===t[1])}get selectionText(){const e=this._model.finalSelectionStart,t=this._model.finalSelectionEnd;if(!e||!t)return"";const i=this._bufferService.buffer,s=[];if(3===this._activeSelectionMode){if(e[0]===t[0])return"";const r=e[0]<t[0]?e[0]:t[0],n=e[0]<t[0]?t[0]:e[0];for(let o=e[1];o<=t[1];o++){const e=i.translateBufferLineToString(o,!0,r,n);s.push(e)}}else{const r=e[1]===t[1]?t[0]:void 0;s.push(i.translateBufferLineToString(e[1],!0,e[0],r));for(let r=e[1]+1;r<=t[1]-1;r++){const e=i.lines.get(r),t=i.translateBufferLineToString(r,!0);(null==e?void 0:e.isWrapped)?s[s.length-1]+=t:s.push(t)}if(e[1]!==t[1]){const e=i.lines.get(t[1]),r=i.translateBufferLineToString(t[1],!0,0,t[0]);e&&e.isWrapped?s[s.length-1]+=r:s.push(r)}}return s.map((e=>e.replace(p," "))).join(d.isWindows?"\r\n":"\n")}clearSelection(){this._model.clearSelection(),this._removeMouseDownListeners(),this.refresh(),this._onSelectionChange.fire()}refresh(e){this._refreshAnimationFrame||(this._refreshAnimationFrame=this._coreBrowserService.window.requestAnimationFrame((()=>this._refresh()))),d.isLinux&&e&&this.selectionText.length&&this._onLinuxMouseSelection.fire(this.selectionText)}_refresh(){this._refreshAnimationFrame=void 0,this._onRedrawRequest.fire({start:this._model.finalSelectionStart,end:this._model.finalSelectionEnd,columnSelectMode:3===this._activeSelectionMode})}_isClickInSelection(e){const t=this._getMouseBufferCoords(e),i=this._model.finalSelectionStart,s=this._model.finalSelectionEnd;return!!(i&&s&&t)&&this._areCoordsInSelection(t,i,s)}isCellInSelection(e,t){const i=this._model.finalSelectionStart,s=this._model.finalSelectionEnd;return!(!i||!s)&&this._areCoordsInSelection([e,t],i,s)}_areCoordsInSelection(e,t,i){return e[1]>t[1]&&e[1]<i[1]||t[1]===i[1]&&e[1]===t[1]&&e[0]>=t[0]&&e[0]<i[0]||t[1]<i[1]&&e[1]===i[1]&&e[0]<i[0]||t[1]<i[1]&&e[1]===t[1]&&e[0]>=t[0]}_selectWordAtCursor(e,t){var i,s;const r=null===(s=null===(i=this._linkifier.currentLink)||void 0===i?void 0:i.link)||void 0===s?void 0:s.range;if(r)return this._model.selectionStart=[r.start.x-1,r.start.y-1],this._model.selectionStartLength=(0,_.getRangeLength)(r,this._bufferService.cols),this._model.selectionEnd=void 0,!0;const n=this._getMouseBufferCoords(e);return!!n&&(this._selectWordAt(n,t),this._model.selectionEnd=void 0,!0)}selectAll(){this._model.isSelectAllActive=!0,this.refresh(),this._onSelectionChange.fire()}selectLines(e,t){this._model.clearSelection(),e=Math.max(e,0),t=Math.min(t,this._bufferService.buffer.lines.length-1),this._model.selectionStart=[0,e],this._model.selectionEnd=[this._bufferService.cols,t],this.refresh(),this._onSelectionChange.fire()}_handleTrim(e){this._model.handleTrim(e)&&this.refresh()}_getMouseBufferCoords(e){const t=this._mouseService.getCoords(e,this._screenElement,this._bufferService.cols,this._bufferService.rows,!0);if(t)return t[0]--,t[1]--,t[1]+=this._bufferService.buffer.ydisp,t}_getMouseEventScrollAmount(e){let t=(0,n.getCoordsRelativeToElement)(this._coreBrowserService.window,e,this._screenElement)[1];const i=this._renderService.dimensions.css.canvas.height;return t>=0&&t<=i?0:(t>i&&(t-=i),t=Math.min(Math.max(t,-50),50),t/=50,t/Math.abs(t)+Math.round(14*t))}shouldForceSelection(e){return d.isMac?e.altKey&&this._optionsService.rawOptions.macOptionClickForcesSelection:e.shiftKey}handleMouseDown(e){if(this._mouseDownTimeStamp=e.timeStamp,(2!==e.button||!this.hasSelection)&&0===e.button){if(!this._enabled){if(!this.shouldForceSelection(e))return;e.stopPropagation()}e.preventDefault(),this._dragScrollAmount=0,this._enabled&&e.shiftKey?this._handleIncrementalClick(e):1===e.detail?this._handleSingleClick(e):2===e.detail?this._handleDoubleClick(e):3===e.detail&&this._handleTripleClick(e),this._addMouseDownListeners(),this.refresh(!0)}}_addMouseDownListeners(){this._screenElement.ownerDocument&&(this._screenElement.ownerDocument.addEventListener("mousemove",this._mouseMoveListener),this._screenElement.ownerDocument.addEventListener("mouseup",this._mouseUpListener)),this._dragScrollIntervalTimer=this._coreBrowserService.window.setInterval((()=>this._dragScroll()),50)}_removeMouseDownListeners(){this._screenElement.ownerDocument&&(this._screenElement.ownerDocument.removeEventListener("mousemove",this._mouseMoveListener),this._screenElement.ownerDocument.removeEventListener("mouseup",this._mouseUpListener)),this._coreBrowserService.window.clearInterval(this._dragScrollIntervalTimer),this._dragScrollIntervalTimer=void 0}_handleIncrementalClick(e){this._model.selectionStart&&(this._model.selectionEnd=this._getMouseBufferCoords(e))}_handleSingleClick(e){if(this._model.selectionStartLength=0,this._model.isSelectAllActive=!1,this._activeSelectionMode=this.shouldColumnSelect(e)?3:0,this._model.selectionStart=this._getMouseBufferCoords(e),!this._model.selectionStart)return;this._model.selectionEnd=void 0;const t=this._bufferService.buffer.lines.get(this._model.selectionStart[1]);t&&t.length!==this._model.selectionStart[0]&&0===t.hasWidth(this._model.selectionStart[0])&&this._model.selectionStart[0]++}_handleDoubleClick(e){this._selectWordAtCursor(e,!0)&&(this._activeSelectionMode=1)}_handleTripleClick(e){const t=this._getMouseBufferCoords(e);t&&(this._activeSelectionMode=2,this._selectLineAt(t[1]))}shouldColumnSelect(e){return e.altKey&&!(d.isMac&&this._optionsService.rawOptions.macOptionClickForcesSelection)}_handleMouseMove(e){if(e.stopImmediatePropagation(),!this._model.selectionStart)return;const t=this._model.selectionEnd?[this._model.selectionEnd[0],this._model.selectionEnd[1]]:null;if(this._model.selectionEnd=this._getMouseBufferCoords(e),!this._model.selectionEnd)return void this.refresh(!0);2===this._activeSelectionMode?this._model.selectionEnd[1]<this._model.selectionStart[1]?this._model.selectionEnd[0]=0:this._model.selectionEnd[0]=this._bufferService.cols:1===this._activeSelectionMode&&this._selectToWordAt(this._model.selectionEnd),this._dragScrollAmount=this._getMouseEventScrollAmount(e),3!==this._activeSelectionMode&&(this._dragScrollAmount>0?this._model.selectionEnd[0]=this._bufferService.cols:this._dragScrollAmount<0&&(this._model.selectionEnd[0]=0));const i=this._bufferService.buffer;if(this._model.selectionEnd[1]<i.lines.length){const e=i.lines.get(this._model.selectionEnd[1]);e&&0===e.hasWidth(this._model.selectionEnd[0])&&this._model.selectionEnd[0]++}t&&t[0]===this._model.selectionEnd[0]&&t[1]===this._model.selectionEnd[1]||this.refresh(!0)}_dragScroll(){if(this._model.selectionEnd&&this._model.selectionStart&&this._dragScrollAmount){this._onRequestScrollLines.fire({amount:this._dragScrollAmount,suppressScrollEvent:!1});const e=this._bufferService.buffer;this._dragScrollAmount>0?(3!==this._activeSelectionMode&&(this._model.selectionEnd[0]=this._bufferService.cols),this._model.selectionEnd[1]=Math.min(e.ydisp+this._bufferService.rows,e.lines.length-1)):(3!==this._activeSelectionMode&&(this._model.selectionEnd[0]=0),this._model.selectionEnd[1]=e.ydisp),this.refresh()}}_handleMouseUp(e){const t=e.timeStamp-this._mouseDownTimeStamp;if(this._removeMouseDownListeners(),this.selectionText.length<=1&&t<500&&e.altKey&&this._optionsService.rawOptions.altClickMovesCursor){if(this._bufferService.buffer.ybase===this._bufferService.buffer.ydisp){const t=this._mouseService.getCoords(e,this._element,this._bufferService.cols,this._bufferService.rows,!1);if(t&&void 0!==t[0]&&void 0!==t[1]){const e=(0,o.moveToCellSequence)(t[0]-1,t[1]-1,this._bufferService,this._coreService.decPrivateModes.applicationCursorKeys);this._coreService.triggerDataEvent(e,!0)}}}else this._fireEventIfSelectionChanged()}_fireEventIfSelectionChanged(){const e=this._model.finalSelectionStart,t=this._model.finalSelectionEnd,i=!(!e||!t||e[0]===t[0]&&e[1]===t[1]);i?e&&t&&(this._oldSelectionStart&&this._oldSelectionEnd&&e[0]===this._oldSelectionStart[0]&&e[1]===this._oldSelectionStart[1]&&t[0]===this._oldSelectionEnd[0]&&t[1]===this._oldSelectionEnd[1]||this._fireOnSelectionChange(e,t,i)):this._oldHasSelection&&this._fireOnSelectionChange(e,t,i)}_fireOnSelectionChange(e,t,i){this._oldSelectionStart=e,this._oldSelectionEnd=t,this._oldHasSelection=i,this._onSelectionChange.fire()}_handleBufferActivate(e){this.clearSelection(),this._trimListener.dispose(),this._trimListener=e.activeBuffer.lines.onTrim((e=>this._handleTrim(e)))}_convertViewportColToCharacterIndex(e,t){let i=t;for(let s=0;t>=s;s++){const r=e.loadCell(s,this._workCell).getChars().length;0===this._workCell.getWidth()?i--:r>1&&t!==s&&(i+=r-1)}return i}setSelection(e,t,i){this._model.clearSelection(),this._removeMouseDownListeners(),this._model.selectionStart=[e,t],this._model.selectionStartLength=i,this.refresh(),this._fireEventIfSelectionChanged()}rightClickSelect(e){this._isClickInSelection(e)||(this._selectWordAtCursor(e,!1)&&this.refresh(!0),this._fireEventIfSelectionChanged())}_getWordAt(e,t,i=!0,s=!0){if(e[0]>=this._bufferService.cols)return;const r=this._bufferService.buffer,n=r.lines.get(e[1]);if(!n)return;const o=r.translateBufferLineToString(e[1],!1);let a=this._convertViewportColToCharacterIndex(n,e[0]),h=a;const c=e[0]-a;let l=0,d=0,_=0,u=0;if(" "===o.charAt(a)){for(;a>0&&" "===o.charAt(a-1);)a--;for(;h<o.length&&" "===o.charAt(h+1);)h++}else{let t=e[0],i=e[0];0===n.getWidth(t)&&(l++,t--),2===n.getWidth(i)&&(d++,i++);const s=n.getString(i).length;for(s>1&&(u+=s-1,h+=s-1);t>0&&a>0&&!this._isCharWordSeparator(n.loadCell(t-1,this._workCell));){n.loadCell(t-1,this._workCell);const e=this._workCell.getChars().length;0===this._workCell.getWidth()?(l++,t--):e>1&&(_+=e-1,a-=e-1),a--,t--}for(;i<n.length&&h+1<o.length&&!this._isCharWordSeparator(n.loadCell(i+1,this._workCell));){n.loadCell(i+1,this._workCell);const e=this._workCell.getChars().length;2===this._workCell.getWidth()?(d++,i++):e>1&&(u+=e-1,h+=e-1),h++,i++}}h++;let f=a+c-l+_,v=Math.min(this._bufferService.cols,h-a+l+d-_-u);if(t||""!==o.slice(a,h).trim()){if(i&&0===f&&32!==n.getCodePoint(0)){const t=r.lines.get(e[1]-1);if(t&&n.isWrapped&&32!==t.getCodePoint(this._bufferService.cols-1)){const t=this._getWordAt([this._bufferService.cols-1,e[1]-1],!1,!0,!1);if(t){const e=this._bufferService.cols-t.start;f-=e,v+=e}}}if(s&&f+v===this._bufferService.cols&&32!==n.getCodePoint(this._bufferService.cols-1)){const t=r.lines.get(e[1]+1);if((null==t?void 0:t.isWrapped)&&32!==t.getCodePoint(0)){const t=this._getWordAt([0,e[1]+1],!1,!1,!0);t&&(v+=t.length)}}return{start:f,length:v}}}_selectWordAt(e,t){const i=this._getWordAt(e,t);if(i){for(;i.start<0;)i.start+=this._bufferService.cols,e[1]--;this._model.selectionStart=[i.start,e[1]],this._model.selectionStartLength=i.length}}_selectToWordAt(e){const t=this._getWordAt(e,!0);if(t){let i=e[1];for(;t.start<0;)t.start+=this._bufferService.cols,i--;if(!this._model.areSelectionValuesReversed())for(;t.start+t.length>this._bufferService.cols;)t.length-=this._bufferService.cols,i++;this._model.selectionEnd=[this._model.areSelectionValuesReversed()?t.start:t.start+t.length,i]}}_isCharWordSeparator(e){return 0!==e.getWidth()&&this._optionsService.rawOptions.wordSeparator.indexOf(e.getChars())>=0}_selectLineAt(e){const t=this._bufferService.buffer.getWrappedRangeForLine(e),i={start:{x:0,y:t.first},end:{x:this._bufferService.cols-1,y:t.last}};this._model.selectionStart=[0,t.first],this._model.selectionEnd=void 0,this._model.selectionStartLength=(0,_.getRangeLength)(i,this._bufferService.cols)}};t.SelectionService=g=s([r(3,f.IBufferService),r(4,f.ICoreService),r(5,h.IMouseService),r(6,f.IOptionsService),r(7,h.IRenderService),r(8,h.ICoreBrowserService)],g)},4725:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.IThemeService=t.ICharacterJoinerService=t.ISelectionService=t.IRenderService=t.IMouseService=t.ICoreBrowserService=t.ICharSizeService=void 0;const s=i(8343);t.ICharSizeService=(0,s.createDecorator)("CharSizeService"),t.ICoreBrowserService=(0,s.createDecorator)("CoreBrowserService"),t.IMouseService=(0,s.createDecorator)("MouseService"),t.IRenderService=(0,s.createDecorator)("RenderService"),t.ISelectionService=(0,s.createDecorator)("SelectionService"),t.ICharacterJoinerService=(0,s.createDecorator)("CharacterJoinerService"),t.IThemeService=(0,s.createDecorator)("ThemeService")},6731:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.ThemeService=t.DEFAULT_ANSI_COLORS=void 0;const n=i(7239),o=i(8055),a=i(8460),h=i(844),c=i(2585),l=o.css.toColor("#ffffff"),d=o.css.toColor("#000000"),_=o.css.toColor("#ffffff"),u=o.css.toColor("#000000"),f={css:"rgba(255, 255, 255, 0.3)",rgba:4294967117};t.DEFAULT_ANSI_COLORS=Object.freeze((()=>{const e=[o.css.toColor("#2e3436"),o.css.toColor("#cc0000"),o.css.toColor("#4e9a06"),o.css.toColor("#c4a000"),o.css.toColor("#3465a4"),o.css.toColor("#75507b"),o.css.toColor("#06989a"),o.css.toColor("#d3d7cf"),o.css.toColor("#555753"),o.css.toColor("#ef2929"),o.css.toColor("#8ae234"),o.css.toColor("#fce94f"),o.css.toColor("#729fcf"),o.css.toColor("#ad7fa8"),o.css.toColor("#34e2e2"),o.css.toColor("#eeeeec")],t=[0,95,135,175,215,255];for(let i=0;i<216;i++){const s=t[i/36%6|0],r=t[i/6%6|0],n=t[i%6];e.push({css:o.channels.toCss(s,r,n),rgba:o.channels.toRgba(s,r,n)})}for(let t=0;t<24;t++){const i=8+10*t;e.push({css:o.channels.toCss(i,i,i),rgba:o.channels.toRgba(i,i,i)})}return e})());let v=t.ThemeService=class extends h.Disposable{get colors(){return this._colors}constructor(e){super(),this._optionsService=e,this._contrastCache=new n.ColorContrastCache,this._halfContrastCache=new n.ColorContrastCache,this._onChangeColors=this.register(new a.EventEmitter),this.onChangeColors=this._onChangeColors.event,this._colors={foreground:l,background:d,cursor:_,cursorAccent:u,selectionForeground:void 0,selectionBackgroundTransparent:f,selectionBackgroundOpaque:o.color.blend(d,f),selectionInactiveBackgroundTransparent:f,selectionInactiveBackgroundOpaque:o.color.blend(d,f),ansi:t.DEFAULT_ANSI_COLORS.slice(),contrastCache:this._contrastCache,halfContrastCache:this._halfContrastCache},this._updateRestoreColors(),this._setTheme(this._optionsService.rawOptions.theme),this.register(this._optionsService.onSpecificOptionChange("minimumContrastRatio",(()=>this._contrastCache.clear()))),this.register(this._optionsService.onSpecificOptionChange("theme",(()=>this._setTheme(this._optionsService.rawOptions.theme))))}_setTheme(e={}){const i=this._colors;if(i.foreground=p(e.foreground,l),i.background=p(e.background,d),i.cursor=p(e.cursor,_),i.cursorAccent=p(e.cursorAccent,u),i.selectionBackgroundTransparent=p(e.selectionBackground,f),i.selectionBackgroundOpaque=o.color.blend(i.background,i.selectionBackgroundTransparent),i.selectionInactiveBackgroundTransparent=p(e.selectionInactiveBackground,i.selectionBackgroundTransparent),i.selectionInactiveBackgroundOpaque=o.color.blend(i.background,i.selectionInactiveBackgroundTransparent),i.selectionForeground=e.selectionForeground?p(e.selectionForeground,o.NULL_COLOR):void 0,i.selectionForeground===o.NULL_COLOR&&(i.selectionForeground=void 0),o.color.isOpaque(i.selectionBackgroundTransparent)){const e=.3;i.selectionBackgroundTransparent=o.color.opacity(i.selectionBackgroundTransparent,e)}if(o.color.isOpaque(i.selectionInactiveBackgroundTransparent)){const e=.3;i.selectionInactiveBackgroundTransparent=o.color.opacity(i.selectionInactiveBackgroundTransparent,e)}if(i.ansi=t.DEFAULT_ANSI_COLORS.slice(),i.ansi[0]=p(e.black,t.DEFAULT_ANSI_COLORS[0]),i.ansi[1]=p(e.red,t.DEFAULT_ANSI_COLORS[1]),i.ansi[2]=p(e.green,t.DEFAULT_ANSI_COLORS[2]),i.ansi[3]=p(e.yellow,t.DEFAULT_ANSI_COLORS[3]),i.ansi[4]=p(e.blue,t.DEFAULT_ANSI_COLORS[4]),i.ansi[5]=p(e.magenta,t.DEFAULT_ANSI_COLORS[5]),i.ansi[6]=p(e.cyan,t.DEFAULT_ANSI_COLORS[6]),i.ansi[7]=p(e.white,t.DEFAULT_ANSI_COLORS[7]),i.ansi[8]=p(e.brightBlack,t.DEFAULT_ANSI_COLORS[8]),i.ansi[9]=p(e.brightRed,t.DEFAULT_ANSI_COLORS[9]),i.ansi[10]=p(e.brightGreen,t.DEFAULT_ANSI_COLORS[10]),i.ansi[11]=p(e.brightYellow,t.DEFAULT_ANSI_COLORS[11]),i.ansi[12]=p(e.brightBlue,t.DEFAULT_ANSI_COLORS[12]),i.ansi[13]=p(e.brightMagenta,t.DEFAULT_ANSI_COLORS[13]),i.ansi[14]=p(e.brightCyan,t.DEFAULT_ANSI_COLORS[14]),i.ansi[15]=p(e.brightWhite,t.DEFAULT_ANSI_COLORS[15]),e.extendedAnsi){const s=Math.min(i.ansi.length-16,e.extendedAnsi.length);for(let r=0;r<s;r++)i.ansi[r+16]=p(e.extendedAnsi[r],t.DEFAULT_ANSI_COLORS[r+16])}this._contrastCache.clear(),this._halfContrastCache.clear(),this._updateRestoreColors(),this._onChangeColors.fire(this.colors)}restoreColor(e){this._restoreColor(e),this._onChangeColors.fire(this.colors)}_restoreColor(e){if(void 0!==e)switch(e){case 256:this._colors.foreground=this._restoreColors.foreground;break;case 257:this._colors.background=this._restoreColors.background;break;case 258:this._colors.cursor=this._restoreColors.cursor;break;default:this._colors.ansi[e]=this._restoreColors.ansi[e]}else for(let e=0;e<this._restoreColors.ansi.length;++e)this._colors.ansi[e]=this._restoreColors.ansi[e]}modifyColors(e){e(this._colors),this._onChangeColors.fire(this.colors)}_updateRestoreColors(){this._restoreColors={foreground:this._colors.foreground,background:this._colors.background,cursor:this._colors.cursor,ansi:this._colors.ansi.slice()}}};function p(e,t){if(void 0!==e)try{return o.css.toColor(e)}catch(e){}return t}t.ThemeService=v=s([r(0,c.IOptionsService)],v)},6349:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.CircularList=void 0;const s=i(8460),r=i(844);class n extends r.Disposable{constructor(e){super(),this._maxLength=e,this.onDeleteEmitter=this.register(new s.EventEmitter),this.onDelete=this.onDeleteEmitter.event,this.onInsertEmitter=this.register(new s.EventEmitter),this.onInsert=this.onInsertEmitter.event,this.onTrimEmitter=this.register(new s.EventEmitter),this.onTrim=this.onTrimEmitter.event,this._array=new Array(this._maxLength),this._startIndex=0,this._length=0}get maxLength(){return this._maxLength}set maxLength(e){if(this._maxLength===e)return;const t=new Array(e);for(let i=0;i<Math.min(e,this.length);i++)t[i]=this._array[this._getCyclicIndex(i)];this._array=t,this._maxLength=e,this._startIndex=0}get length(){return this._length}set length(e){if(e>this._length)for(let t=this._length;t<e;t++)this._array[t]=void 0;this._length=e}get(e){return this._array[this._getCyclicIndex(e)]}set(e,t){this._array[this._getCyclicIndex(e)]=t}push(e){this._array[this._getCyclicIndex(this._length)]=e,this._length===this._maxLength?(this._startIndex=++this._startIndex%this._maxLength,this.onTrimEmitter.fire(1)):this._length++}recycle(){if(this._length!==this._maxLength)throw new Error("Can only recycle when the buffer is full");return this._startIndex=++this._startIndex%this._maxLength,this.onTrimEmitter.fire(1),this._array[this._getCyclicIndex(this._length-1)]}get isFull(){return this._length===this._maxLength}pop(){return this._array[this._getCyclicIndex(this._length---1)]}splice(e,t,...i){if(t){for(let i=e;i<this._length-t;i++)this._array[this._getCyclicIndex(i)]=this._array[this._getCyclicIndex(i+t)];this._length-=t,this.onDeleteEmitter.fire({index:e,amount:t})}for(let t=this._length-1;t>=e;t--)this._array[this._getCyclicIndex(t+i.length)]=this._array[this._getCyclicIndex(t)];for(let t=0;t<i.length;t++)this._array[this._getCyclicIndex(e+t)]=i[t];if(i.length&&this.onInsertEmitter.fire({index:e,amount:i.length}),this._length+i.length>this._maxLength){const e=this._length+i.length-this._maxLength;this._startIndex+=e,this._length=this._maxLength,this.onTrimEmitter.fire(e)}else this._length+=i.length}trimStart(e){e>this._length&&(e=this._length),this._startIndex+=e,this._length-=e,this.onTrimEmitter.fire(e)}shiftElements(e,t,i){if(!(t<=0)){if(e<0||e>=this._length)throw new Error("start argument out of range");if(e+i<0)throw new Error("Cannot shift elements in list beyond index 0");if(i>0){for(let s=t-1;s>=0;s--)this.set(e+s+i,this.get(e+s));const s=e+t+i-this._length;if(s>0)for(this._length+=s;this._length>this._maxLength;)this._length--,this._startIndex++,this.onTrimEmitter.fire(1)}else for(let s=0;s<t;s++)this.set(e+s+i,this.get(e+s))}}_getCyclicIndex(e){return(this._startIndex+e)%this._maxLength}}t.CircularList=n},1439:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.clone=void 0,t.clone=function e(t,i=5){if("object"!=typeof t)return t;const s=Array.isArray(t)?[]:{};for(const r in t)s[r]=i<=1?t[r]:t[r]&&e(t[r],i-1);return s}},8055:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.contrastRatio=t.toPaddedHex=t.rgba=t.rgb=t.css=t.color=t.channels=t.NULL_COLOR=void 0;const s=i(6114);let r=0,n=0,o=0,a=0;var h,c,l,d,_;function u(e){const t=e.toString(16);return t.length<2?"0"+t:t}function f(e,t){return e<t?(t+.05)/(e+.05):(e+.05)/(t+.05)}t.NULL_COLOR={css:"#00000000",rgba:0},function(e){e.toCss=function(e,t,i,s){return void 0!==s?`#${u(e)}${u(t)}${u(i)}${u(s)}`:`#${u(e)}${u(t)}${u(i)}`},e.toRgba=function(e,t,i,s=255){return(e<<24|t<<16|i<<8|s)>>>0}}(h||(t.channels=h={})),function(e){function t(e,t){return a=Math.round(255*t),[r,n,o]=_.toChannels(e.rgba),{css:h.toCss(r,n,o,a),rgba:h.toRgba(r,n,o,a)}}e.blend=function(e,t){if(a=(255&t.rgba)/255,1===a)return{css:t.css,rgba:t.rgba};const i=t.rgba>>24&255,s=t.rgba>>16&255,c=t.rgba>>8&255,l=e.rgba>>24&255,d=e.rgba>>16&255,_=e.rgba>>8&255;return r=l+Math.round((i-l)*a),n=d+Math.round((s-d)*a),o=_+Math.round((c-_)*a),{css:h.toCss(r,n,o),rgba:h.toRgba(r,n,o)}},e.isOpaque=function(e){return 255==(255&e.rgba)},e.ensureContrastRatio=function(e,t,i){const s=_.ensureContrastRatio(e.rgba,t.rgba,i);if(s)return _.toColor(s>>24&255,s>>16&255,s>>8&255)},e.opaque=function(e){const t=(255|e.rgba)>>>0;return[r,n,o]=_.toChannels(t),{css:h.toCss(r,n,o),rgba:t}},e.opacity=t,e.multiplyOpacity=function(e,i){return a=255&e.rgba,t(e,a*i/255)},e.toColorRGB=function(e){return[e.rgba>>24&255,e.rgba>>16&255,e.rgba>>8&255]}}(c||(t.color=c={})),function(e){let t,i;if(!s.isNode){const e=document.createElement("canvas");e.width=1,e.height=1;const s=e.getContext("2d",{willReadFrequently:!0});s&&(t=s,t.globalCompositeOperation="copy",i=t.createLinearGradient(0,0,1,1))}e.toColor=function(e){if(e.match(/#[\da-f]{3,8}/i))switch(e.length){case 4:return r=parseInt(e.slice(1,2).repeat(2),16),n=parseInt(e.slice(2,3).repeat(2),16),o=parseInt(e.slice(3,4).repeat(2),16),_.toColor(r,n,o);case 5:return r=parseInt(e.slice(1,2).repeat(2),16),n=parseInt(e.slice(2,3).repeat(2),16),o=parseInt(e.slice(3,4).repeat(2),16),a=parseInt(e.slice(4,5).repeat(2),16),_.toColor(r,n,o,a);case 7:return{css:e,rgba:(parseInt(e.slice(1),16)<<8|255)>>>0};case 9:return{css:e,rgba:parseInt(e.slice(1),16)>>>0}}const s=e.match(/rgba?\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*(,\s*(0|1|\d?\.(\d+))\s*)?\)/);if(s)return r=parseInt(s[1]),n=parseInt(s[2]),o=parseInt(s[3]),a=Math.round(255*(void 0===s[5]?1:parseFloat(s[5]))),_.toColor(r,n,o,a);if(!t||!i)throw new Error("css.toColor: Unsupported css format");if(t.fillStyle=i,t.fillStyle=e,"string"!=typeof t.fillStyle)throw new Error("css.toColor: Unsupported css format");if(t.fillRect(0,0,1,1),[r,n,o,a]=t.getImageData(0,0,1,1).data,255!==a)throw new Error("css.toColor: Unsupported css format");return{rgba:h.toRgba(r,n,o,a),css:e}}}(l||(t.css=l={})),function(e){function t(e,t,i){const s=e/255,r=t/255,n=i/255;return.2126*(s<=.03928?s/12.92:Math.pow((s+.055)/1.055,2.4))+.7152*(r<=.03928?r/12.92:Math.pow((r+.055)/1.055,2.4))+.0722*(n<=.03928?n/12.92:Math.pow((n+.055)/1.055,2.4))}e.relativeLuminance=function(e){return t(e>>16&255,e>>8&255,255&e)},e.relativeLuminance2=t}(d||(t.rgb=d={})),function(e){function t(e,t,i){const s=e>>24&255,r=e>>16&255,n=e>>8&255;let o=t>>24&255,a=t>>16&255,h=t>>8&255,c=f(d.relativeLuminance2(o,a,h),d.relativeLuminance2(s,r,n));for(;c<i&&(o>0||a>0||h>0);)o-=Math.max(0,Math.ceil(.1*o)),a-=Math.max(0,Math.ceil(.1*a)),h-=Math.max(0,Math.ceil(.1*h)),c=f(d.relativeLuminance2(o,a,h),d.relativeLuminance2(s,r,n));return(o<<24|a<<16|h<<8|255)>>>0}function i(e,t,i){const s=e>>24&255,r=e>>16&255,n=e>>8&255;let o=t>>24&255,a=t>>16&255,h=t>>8&255,c=f(d.relativeLuminance2(o,a,h),d.relativeLuminance2(s,r,n));for(;c<i&&(o<255||a<255||h<255);)o=Math.min(255,o+Math.ceil(.1*(255-o))),a=Math.min(255,a+Math.ceil(.1*(255-a))),h=Math.min(255,h+Math.ceil(.1*(255-h))),c=f(d.relativeLuminance2(o,a,h),d.relativeLuminance2(s,r,n));return(o<<24|a<<16|h<<8|255)>>>0}e.ensureContrastRatio=function(e,s,r){const n=d.relativeLuminance(e>>8),o=d.relativeLuminance(s>>8);if(f(n,o)<r){if(o<n){const o=t(e,s,r),a=f(n,d.relativeLuminance(o>>8));if(a<r){const t=i(e,s,r);return a>f(n,d.relativeLuminance(t>>8))?o:t}return o}const a=i(e,s,r),h=f(n,d.relativeLuminance(a>>8));if(h<r){const i=t(e,s,r);return h>f(n,d.relativeLuminance(i>>8))?a:i}return a}},e.reduceLuminance=t,e.increaseLuminance=i,e.toChannels=function(e){return[e>>24&255,e>>16&255,e>>8&255,255&e]},e.toColor=function(e,t,i,s){return{css:h.toCss(e,t,i,s),rgba:h.toRgba(e,t,i,s)}}}(_||(t.rgba=_={})),t.toPaddedHex=u,t.contrastRatio=f},8969:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.CoreTerminal=void 0;const s=i(844),r=i(2585),n=i(4348),o=i(7866),a=i(744),h=i(7302),c=i(6975),l=i(8460),d=i(1753),_=i(1480),u=i(7994),f=i(9282),v=i(5435),p=i(5981),g=i(2660);let m=!1;class S extends s.Disposable{get onScroll(){return this._onScrollApi||(this._onScrollApi=this.register(new l.EventEmitter),this._onScroll.event((e=>{var t;null===(t=this._onScrollApi)||void 0===t||t.fire(e.position)}))),this._onScrollApi.event}get cols(){return this._bufferService.cols}get rows(){return this._bufferService.rows}get buffers(){return this._bufferService.buffers}get options(){return this.optionsService.options}set options(e){for(const t in e)this.optionsService.options[t]=e[t]}constructor(e){super(),this._windowsWrappingHeuristics=this.register(new s.MutableDisposable),this._onBinary=this.register(new l.EventEmitter),this.onBinary=this._onBinary.event,this._onData=this.register(new l.EventEmitter),this.onData=this._onData.event,this._onLineFeed=this.register(new l.EventEmitter),this.onLineFeed=this._onLineFeed.event,this._onResize=this.register(new l.EventEmitter),this.onResize=this._onResize.event,this._onWriteParsed=this.register(new l.EventEmitter),this.onWriteParsed=this._onWriteParsed.event,this._onScroll=this.register(new l.EventEmitter),this._instantiationService=new n.InstantiationService,this.optionsService=this.register(new h.OptionsService(e)),this._instantiationService.setService(r.IOptionsService,this.optionsService),this._bufferService=this.register(this._instantiationService.createInstance(a.BufferService)),this._instantiationService.setService(r.IBufferService,this._bufferService),this._logService=this.register(this._instantiationService.createInstance(o.LogService)),this._instantiationService.setService(r.ILogService,this._logService),this.coreService=this.register(this._instantiationService.createInstance(c.CoreService)),this._instantiationService.setService(r.ICoreService,this.coreService),this.coreMouseService=this.register(this._instantiationService.createInstance(d.CoreMouseService)),this._instantiationService.setService(r.ICoreMouseService,this.coreMouseService),this.unicodeService=this.register(this._instantiationService.createInstance(_.UnicodeService)),this._instantiationService.setService(r.IUnicodeService,this.unicodeService),this._charsetService=this._instantiationService.createInstance(u.CharsetService),this._instantiationService.setService(r.ICharsetService,this._charsetService),this._oscLinkService=this._instantiationService.createInstance(g.OscLinkService),this._instantiationService.setService(r.IOscLinkService,this._oscLinkService),this._inputHandler=this.register(new v.InputHandler(this._bufferService,this._charsetService,this.coreService,this._logService,this.optionsService,this._oscLinkService,this.coreMouseService,this.unicodeService)),this.register((0,l.forwardEvent)(this._inputHandler.onLineFeed,this._onLineFeed)),this.register(this._inputHandler),this.register((0,l.forwardEvent)(this._bufferService.onResize,this._onResize)),this.register((0,l.forwardEvent)(this.coreService.onData,this._onData)),this.register((0,l.forwardEvent)(this.coreService.onBinary,this._onBinary)),this.register(this.coreService.onRequestScrollToBottom((()=>this.scrollToBottom()))),this.register(this.coreService.onUserInput((()=>this._writeBuffer.handleUserInput()))),this.register(this.optionsService.onMultipleOptionChange(["windowsMode","windowsPty"],(()=>this._handleWindowsPtyOptionChange()))),this.register(this._bufferService.onScroll((e=>{this._onScroll.fire({position:this._bufferService.buffer.ydisp,source:0}),this._inputHandler.markRangeDirty(this._bufferService.buffer.scrollTop,this._bufferService.buffer.scrollBottom)}))),this.register(this._inputHandler.onScroll((e=>{this._onScroll.fire({position:this._bufferService.buffer.ydisp,source:0}),this._inputHandler.markRangeDirty(this._bufferService.buffer.scrollTop,this._bufferService.buffer.scrollBottom)}))),this._writeBuffer=this.register(new p.WriteBuffer(((e,t)=>this._inputHandler.parse(e,t)))),this.register((0,l.forwardEvent)(this._writeBuffer.onWriteParsed,this._onWriteParsed))}write(e,t){this._writeBuffer.write(e,t)}writeSync(e,t){this._logService.logLevel<=r.LogLevelEnum.WARN&&!m&&(this._logService.warn("writeSync is unreliable and will be removed soon."),m=!0),this._writeBuffer.writeSync(e,t)}resize(e,t){isNaN(e)||isNaN(t)||(e=Math.max(e,a.MINIMUM_COLS),t=Math.max(t,a.MINIMUM_ROWS),this._bufferService.resize(e,t))}scroll(e,t=!1){this._bufferService.scroll(e,t)}scrollLines(e,t,i){this._bufferService.scrollLines(e,t,i)}scrollPages(e){this.scrollLines(e*(this.rows-1))}scrollToTop(){this.scrollLines(-this._bufferService.buffer.ydisp)}scrollToBottom(){this.scrollLines(this._bufferService.buffer.ybase-this._bufferService.buffer.ydisp)}scrollToLine(e){const t=e-this._bufferService.buffer.ydisp;0!==t&&this.scrollLines(t)}registerEscHandler(e,t){return this._inputHandler.registerEscHandler(e,t)}registerDcsHandler(e,t){return this._inputHandler.registerDcsHandler(e,t)}registerCsiHandler(e,t){return this._inputHandler.registerCsiHandler(e,t)}registerOscHandler(e,t){return this._inputHandler.registerOscHandler(e,t)}_setup(){this._handleWindowsPtyOptionChange()}reset(){this._inputHandler.reset(),this._bufferService.reset(),this._charsetService.reset(),this.coreService.reset(),this.coreMouseService.reset()}_handleWindowsPtyOptionChange(){let e=!1;const t=this.optionsService.rawOptions.windowsPty;t&&void 0!==t.buildNumber&&void 0!==t.buildNumber?e=!!("conpty"===t.backend&&t.buildNumber<21376):this.optionsService.rawOptions.windowsMode&&(e=!0),e?this._enableWindowsWrappingHeuristics():this._windowsWrappingHeuristics.clear()}_enableWindowsWrappingHeuristics(){if(!this._windowsWrappingHeuristics.value){const e=[];e.push(this.onLineFeed(f.updateWindowsModeWrappedState.bind(null,this._bufferService))),e.push(this.registerCsiHandler({final:"H"},(()=>((0,f.updateWindowsModeWrappedState)(this._bufferService),!1)))),this._windowsWrappingHeuristics.value=(0,s.toDisposable)((()=>{for(const t of e)t.dispose()}))}}}t.CoreTerminal=S},8460:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.forwardEvent=t.EventEmitter=void 0,t.EventEmitter=class{constructor(){this._listeners=[],this._disposed=!1}get event(){return this._event||(this._event=e=>(this._listeners.push(e),{dispose:()=>{if(!this._disposed)for(let t=0;t<this._listeners.length;t++)if(this._listeners[t]===e)return void this._listeners.splice(t,1)}})),this._event}fire(e,t){const i=[];for(let e=0;e<this._listeners.length;e++)i.push(this._listeners[e]);for(let s=0;s<i.length;s++)i[s].call(void 0,e,t)}dispose(){this.clearListeners(),this._disposed=!0}clearListeners(){this._listeners&&(this._listeners.length=0)}},t.forwardEvent=function(e,t){return e((e=>t.fire(e)))}},5435:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.InputHandler=t.WindowsOptionsReportType=void 0;const n=i(2584),o=i(7116),a=i(2015),h=i(844),c=i(482),l=i(8437),d=i(8460),_=i(643),u=i(511),f=i(3734),v=i(2585),p=i(6242),g=i(6351),m=i(5941),S={"(":0,")":1,"*":2,"+":3,"-":1,".":2},C=131072;function b(e,t){if(e>24)return t.setWinLines||!1;switch(e){case 1:return!!t.restoreWin;case 2:return!!t.minimizeWin;case 3:return!!t.setWinPosition;case 4:return!!t.setWinSizePixels;case 5:return!!t.raiseWin;case 6:return!!t.lowerWin;case 7:return!!t.refreshWin;case 8:return!!t.setWinSizeChars;case 9:return!!t.maximizeWin;case 10:return!!t.fullscreenWin;case 11:return!!t.getWinState;case 13:return!!t.getWinPosition;case 14:return!!t.getWinSizePixels;case 15:return!!t.getScreenSizePixels;case 16:return!!t.getCellSizePixels;case 18:return!!t.getWinSizeChars;case 19:return!!t.getScreenSizeChars;case 20:return!!t.getIconTitle;case 21:return!!t.getWinTitle;case 22:return!!t.pushTitle;case 23:return!!t.popTitle;case 24:return!!t.setWinLines}return!1}var y;!function(e){e[e.GET_WIN_SIZE_PIXELS=0]="GET_WIN_SIZE_PIXELS",e[e.GET_CELL_SIZE_PIXELS=1]="GET_CELL_SIZE_PIXELS"}(y||(t.WindowsOptionsReportType=y={}));let w=0;class E extends h.Disposable{getAttrData(){return this._curAttrData}constructor(e,t,i,s,r,h,_,f,v=new a.EscapeSequenceParser){super(),this._bufferService=e,this._charsetService=t,this._coreService=i,this._logService=s,this._optionsService=r,this._oscLinkService=h,this._coreMouseService=_,this._unicodeService=f,this._parser=v,this._parseBuffer=new Uint32Array(4096),this._stringDecoder=new c.StringToUtf32,this._utf8Decoder=new c.Utf8ToUtf32,this._workCell=new u.CellData,this._windowTitle="",this._iconName="",this._windowTitleStack=[],this._iconNameStack=[],this._curAttrData=l.DEFAULT_ATTR_DATA.clone(),this._eraseAttrDataInternal=l.DEFAULT_ATTR_DATA.clone(),this._onRequestBell=this.register(new d.EventEmitter),this.onRequestBell=this._onRequestBell.event,this._onRequestRefreshRows=this.register(new d.EventEmitter),this.onRequestRefreshRows=this._onRequestRefreshRows.event,this._onRequestReset=this.register(new d.EventEmitter),this.onRequestReset=this._onRequestReset.event,this._onRequestSendFocus=this.register(new d.EventEmitter),this.onRequestSendFocus=this._onRequestSendFocus.event,this._onRequestSyncScrollBar=this.register(new d.EventEmitter),this.onRequestSyncScrollBar=this._onRequestSyncScrollBar.event,this._onRequestWindowsOptionsReport=this.register(new d.EventEmitter),this.onRequestWindowsOptionsReport=this._onRequestWindowsOptionsReport.event,this._onA11yChar=this.register(new d.EventEmitter),this.onA11yChar=this._onA11yChar.event,this._onA11yTab=this.register(new d.EventEmitter),this.onA11yTab=this._onA11yTab.event,this._onCursorMove=this.register(new d.EventEmitter),this.onCursorMove=this._onCursorMove.event,this._onLineFeed=this.register(new d.EventEmitter),this.onLineFeed=this._onLineFeed.event,this._onScroll=this.register(new d.EventEmitter),this.onScroll=this._onScroll.event,this._onTitleChange=this.register(new d.EventEmitter),this.onTitleChange=this._onTitleChange.event,this._onColor=this.register(new d.EventEmitter),this.onColor=this._onColor.event,this._parseStack={paused:!1,cursorStartX:0,cursorStartY:0,decodedLength:0,position:0},this._specialColors=[256,257,258],this.register(this._parser),this._dirtyRowTracker=new k(this._bufferService),this._activeBuffer=this._bufferService.buffer,this.register(this._bufferService.buffers.onBufferActivate((e=>this._activeBuffer=e.activeBuffer))),this._parser.setCsiHandlerFallback(((e,t)=>{this._logService.debug("Unknown CSI code: ",{identifier:this._parser.identToString(e),params:t.toArray()})})),this._parser.setEscHandlerFallback((e=>{this._logService.debug("Unknown ESC code: ",{identifier:this._parser.identToString(e)})})),this._parser.setExecuteHandlerFallback((e=>{this._logService.debug("Unknown EXECUTE code: ",{code:e})})),this._parser.setOscHandlerFallback(((e,t,i)=>{this._logService.debug("Unknown OSC code: ",{identifier:e,action:t,data:i})})),this._parser.setDcsHandlerFallback(((e,t,i)=>{"HOOK"===t&&(i=i.toArray()),this._logService.debug("Unknown DCS code: ",{identifier:this._parser.identToString(e),action:t,payload:i})})),this._parser.setPrintHandler(((e,t,i)=>this.print(e,t,i))),this._parser.registerCsiHandler({final:"@"},(e=>this.insertChars(e))),this._parser.registerCsiHandler({intermediates:" ",final:"@"},(e=>this.scrollLeft(e))),this._parser.registerCsiHandler({final:"A"},(e=>this.cursorUp(e))),this._parser.registerCsiHandler({intermediates:" ",final:"A"},(e=>this.scrollRight(e))),this._parser.registerCsiHandler({final:"B"},(e=>this.cursorDown(e))),this._parser.registerCsiHandler({final:"C"},(e=>this.cursorForward(e))),this._parser.registerCsiHandler({final:"D"},(e=>this.cursorBackward(e))),this._parser.registerCsiHandler({final:"E"},(e=>this.cursorNextLine(e))),this._parser.registerCsiHandler({final:"F"},(e=>this.cursorPrecedingLine(e))),this._parser.registerCsiHandler({final:"G"},(e=>this.cursorCharAbsolute(e))),this._parser.registerCsiHandler({final:"H"},(e=>this.cursorPosition(e))),this._parser.registerCsiHandler({final:"I"},(e=>this.cursorForwardTab(e))),this._parser.registerCsiHandler({final:"J"},(e=>this.eraseInDisplay(e,!1))),this._parser.registerCsiHandler({prefix:"?",final:"J"},(e=>this.eraseInDisplay(e,!0))),this._parser.registerCsiHandler({final:"K"},(e=>this.eraseInLine(e,!1))),this._parser.registerCsiHandler({prefix:"?",final:"K"},(e=>this.eraseInLine(e,!0))),this._parser.registerCsiHandler({final:"L"},(e=>this.insertLines(e))),this._parser.registerCsiHandler({final:"M"},(e=>this.deleteLines(e))),this._parser.registerCsiHandler({final:"P"},(e=>this.deleteChars(e))),this._parser.registerCsiHandler({final:"S"},(e=>this.scrollUp(e))),this._parser.registerCsiHandler({final:"T"},(e=>this.scrollDown(e))),this._parser.registerCsiHandler({final:"X"},(e=>this.eraseChars(e))),this._parser.registerCsiHandler({final:"Z"},(e=>this.cursorBackwardTab(e))),this._parser.registerCsiHandler({final:"`"},(e=>this.charPosAbsolute(e))),this._parser.registerCsiHandler({final:"a"},(e=>this.hPositionRelative(e))),this._parser.registerCsiHandler({final:"b"},(e=>this.repeatPrecedingCharacter(e))),this._parser.registerCsiHandler({final:"c"},(e=>this.sendDeviceAttributesPrimary(e))),this._parser.registerCsiHandler({prefix:">",final:"c"},(e=>this.sendDeviceAttributesSecondary(e))),this._parser.registerCsiHandler({final:"d"},(e=>this.linePosAbsolute(e))),this._parser.registerCsiHandler({final:"e"},(e=>this.vPositionRelative(e))),this._parser.registerCsiHandler({final:"f"},(e=>this.hVPosition(e))),this._parser.registerCsiHandler({final:"g"},(e=>this.tabClear(e))),this._parser.registerCsiHandler({final:"h"},(e=>this.setMode(e))),this._parser.registerCsiHandler({prefix:"?",final:"h"},(e=>this.setModePrivate(e))),this._parser.registerCsiHandler({final:"l"},(e=>this.resetMode(e))),this._parser.registerCsiHandler({prefix:"?",final:"l"},(e=>this.resetModePrivate(e))),this._parser.registerCsiHandler({final:"m"},(e=>this.charAttributes(e))),this._parser.registerCsiHandler({final:"n"},(e=>this.deviceStatus(e))),this._parser.registerCsiHandler({prefix:"?",final:"n"},(e=>this.deviceStatusPrivate(e))),this._parser.registerCsiHandler({intermediates:"!",final:"p"},(e=>this.softReset(e))),this._parser.registerCsiHandler({intermediates:" ",final:"q"},(e=>this.setCursorStyle(e))),this._parser.registerCsiHandler({final:"r"},(e=>this.setScrollRegion(e))),this._parser.registerCsiHandler({final:"s"},(e=>this.saveCursor(e))),this._parser.registerCsiHandler({final:"t"},(e=>this.windowOptions(e))),this._parser.registerCsiHandler({final:"u"},(e=>this.restoreCursor(e))),this._parser.registerCsiHandler({intermediates:"'",final:"}"},(e=>this.insertColumns(e))),this._parser.registerCsiHandler({intermediates:"'",final:"~"},(e=>this.deleteColumns(e))),this._parser.registerCsiHandler({intermediates:'"',final:"q"},(e=>this.selectProtected(e))),this._parser.registerCsiHandler({intermediates:"$",final:"p"},(e=>this.requestMode(e,!0))),this._parser.registerCsiHandler({prefix:"?",intermediates:"$",final:"p"},(e=>this.requestMode(e,!1))),this._parser.setExecuteHandler(n.C0.BEL,(()=>this.bell())),this._parser.setExecuteHandler(n.C0.LF,(()=>this.lineFeed())),this._parser.setExecuteHandler(n.C0.VT,(()=>this.lineFeed())),this._parser.setExecuteHandler(n.C0.FF,(()=>this.lineFeed())),this._parser.setExecuteHandler(n.C0.CR,(()=>this.carriageReturn())),this._parser.setExecuteHandler(n.C0.BS,(()=>this.backspace())),this._parser.setExecuteHandler(n.C0.HT,(()=>this.tab())),this._parser.setExecuteHandler(n.C0.SO,(()=>this.shiftOut())),this._parser.setExecuteHandler(n.C0.SI,(()=>this.shiftIn())),this._parser.setExecuteHandler(n.C1.IND,(()=>this.index())),this._parser.setExecuteHandler(n.C1.NEL,(()=>this.nextLine())),this._parser.setExecuteHandler(n.C1.HTS,(()=>this.tabSet())),this._parser.registerOscHandler(0,new p.OscHandler((e=>(this.setTitle(e),this.setIconName(e),!0)))),this._parser.registerOscHandler(1,new p.OscHandler((e=>this.setIconName(e)))),this._parser.registerOscHandler(2,new p.OscHandler((e=>this.setTitle(e)))),this._parser.registerOscHandler(4,new p.OscHandler((e=>this.setOrReportIndexedColor(e)))),this._parser.registerOscHandler(8,new p.OscHandler((e=>this.setHyperlink(e)))),this._parser.registerOscHandler(10,new p.OscHandler((e=>this.setOrReportFgColor(e)))),this._parser.registerOscHandler(11,new p.OscHandler((e=>this.setOrReportBgColor(e)))),this._parser.registerOscHandler(12,new p.OscHandler((e=>this.setOrReportCursorColor(e)))),this._parser.registerOscHandler(104,new p.OscHandler((e=>this.restoreIndexedColor(e)))),this._parser.registerOscHandler(110,new p.OscHandler((e=>this.restoreFgColor(e)))),this._parser.registerOscHandler(111,new p.OscHandler((e=>this.restoreBgColor(e)))),this._parser.registerOscHandler(112,new p.OscHandler((e=>this.restoreCursorColor(e)))),this._parser.registerEscHandler({final:"7"},(()=>this.saveCursor())),this._parser.registerEscHandler({final:"8"},(()=>this.restoreCursor())),this._parser.registerEscHandler({final:"D"},(()=>this.index())),this._parser.registerEscHandler({final:"E"},(()=>this.nextLine())),this._parser.registerEscHandler({final:"H"},(()=>this.tabSet())),this._parser.registerEscHandler({final:"M"},(()=>this.reverseIndex())),this._parser.registerEscHandler({final:"="},(()=>this.keypadApplicationMode())),this._parser.registerEscHandler({final:">"},(()=>this.keypadNumericMode())),this._parser.registerEscHandler({final:"c"},(()=>this.fullReset())),this._parser.registerEscHandler({final:"n"},(()=>this.setgLevel(2))),this._parser.registerEscHandler({final:"o"},(()=>this.setgLevel(3))),this._parser.registerEscHandler({final:"|"},(()=>this.setgLevel(3))),this._parser.registerEscHandler({final:"}"},(()=>this.setgLevel(2))),this._parser.registerEscHandler({final:"~"},(()=>this.setgLevel(1))),this._parser.registerEscHandler({intermediates:"%",final:"@"},(()=>this.selectDefaultCharset())),this._parser.registerEscHandler({intermediates:"%",final:"G"},(()=>this.selectDefaultCharset()));for(const e in o.CHARSETS)this._parser.registerEscHandler({intermediates:"(",final:e},(()=>this.selectCharset("("+e))),this._parser.registerEscHandler({intermediates:")",final:e},(()=>this.selectCharset(")"+e))),this._parser.registerEscHandler({intermediates:"*",final:e},(()=>this.selectCharset("*"+e))),this._parser.registerEscHandler({intermediates:"+",final:e},(()=>this.selectCharset("+"+e))),this._parser.registerEscHandler({intermediates:"-",final:e},(()=>this.selectCharset("-"+e))),this._parser.registerEscHandler({intermediates:".",final:e},(()=>this.selectCharset("."+e))),this._parser.registerEscHandler({intermediates:"/",final:e},(()=>this.selectCharset("/"+e)));this._parser.registerEscHandler({intermediates:"#",final:"8"},(()=>this.screenAlignmentPattern())),this._parser.setErrorHandler((e=>(this._logService.error("Parsing error: ",e),e))),this._parser.registerDcsHandler({intermediates:"$",final:"q"},new g.DcsHandler(((e,t)=>this.requestStatusString(e,t))))}_preserveStack(e,t,i,s){this._parseStack.paused=!0,this._parseStack.cursorStartX=e,this._parseStack.cursorStartY=t,this._parseStack.decodedLength=i,this._parseStack.position=s}_logSlowResolvingAsync(e){this._logService.logLevel<=v.LogLevelEnum.WARN&&Promise.race([e,new Promise(((e,t)=>setTimeout((()=>t("#SLOW_TIMEOUT")),5e3)))]).catch((e=>{if("#SLOW_TIMEOUT"!==e)throw e;console.warn("async parser handler taking longer than 5000 ms")}))}_getCurrentLinkId(){return this._curAttrData.extended.urlId}parse(e,t){let i,s=this._activeBuffer.x,r=this._activeBuffer.y,n=0;const o=this._parseStack.paused;if(o){if(i=this._parser.parse(this._parseBuffer,this._parseStack.decodedLength,t))return this._logSlowResolvingAsync(i),i;s=this._parseStack.cursorStartX,r=this._parseStack.cursorStartY,this._parseStack.paused=!1,e.length>C&&(n=this._parseStack.position+C)}if(this._logService.logLevel<=v.LogLevelEnum.DEBUG&&this._logService.debug("parsing data"+("string"==typeof e?` "${e}"`:` "${Array.prototype.map.call(e,(e=>String.fromCharCode(e))).join("")}"`),"string"==typeof e?e.split("").map((e=>e.charCodeAt(0))):e),this._parseBuffer.length<e.length&&this._parseBuffer.length<C&&(this._parseBuffer=new Uint32Array(Math.min(e.length,C))),o||this._dirtyRowTracker.clearRange(),e.length>C)for(let t=n;t<e.length;t+=C){const n=t+C<e.length?t+C:e.length,o="string"==typeof e?this._stringDecoder.decode(e.substring(t,n),this._parseBuffer):this._utf8Decoder.decode(e.subarray(t,n),this._parseBuffer);if(i=this._parser.parse(this._parseBuffer,o))return this._preserveStack(s,r,o,t),this._logSlowResolvingAsync(i),i}else if(!o){const t="string"==typeof e?this._stringDecoder.decode(e,this._parseBuffer):this._utf8Decoder.decode(e,this._parseBuffer);if(i=this._parser.parse(this._parseBuffer,t))return this._preserveStack(s,r,t,0),this._logSlowResolvingAsync(i),i}this._activeBuffer.x===s&&this._activeBuffer.y===r||this._onCursorMove.fire(),this._onRequestRefreshRows.fire(this._dirtyRowTracker.start,this._dirtyRowTracker.end)}print(e,t,i){let s,r;const n=this._charsetService.charset,o=this._optionsService.rawOptions.screenReaderMode,a=this._bufferService.cols,h=this._coreService.decPrivateModes.wraparound,l=this._coreService.modes.insertMode,d=this._curAttrData;let u=this._activeBuffer.lines.get(this._activeBuffer.ybase+this._activeBuffer.y);this._dirtyRowTracker.markDirty(this._activeBuffer.y),this._activeBuffer.x&&i-t>0&&2===u.getWidth(this._activeBuffer.x-1)&&u.setCellFromCodePoint(this._activeBuffer.x-1,0,1,d.fg,d.bg,d.extended);for(let f=t;f<i;++f){if(s=e[f],r=this._unicodeService.wcwidth(s),s<127&&n){const e=n[String.fromCharCode(s)];e&&(s=e.charCodeAt(0))}if(o&&this._onA11yChar.fire((0,c.stringFromCodePoint)(s)),this._getCurrentLinkId()&&this._oscLinkService.addLineToLink(this._getCurrentLinkId(),this._activeBuffer.ybase+this._activeBuffer.y),r||!this._activeBuffer.x){if(this._activeBuffer.x+r-1>=a)if(h){for(;this._activeBuffer.x<a;)u.setCellFromCodePoint(this._activeBuffer.x++,0,1,d.fg,d.bg,d.extended);this._activeBuffer.x=0,this._activeBuffer.y++,this._activeBuffer.y===this._activeBuffer.scrollBottom+1?(this._activeBuffer.y--,this._bufferService.scroll(this._eraseAttrData(),!0)):(this._activeBuffer.y>=this._bufferService.rows&&(this._activeBuffer.y=this._bufferService.rows-1),this._activeBuffer.lines.get(this._activeBuffer.ybase+this._activeBuffer.y).isWrapped=!0),u=this._activeBuffer.lines.get(this._activeBuffer.ybase+this._activeBuffer.y)}else if(this._activeBuffer.x=a-1,2===r)continue;if(l&&(u.insertCells(this._activeBuffer.x,r,this._activeBuffer.getNullCell(d),d),2===u.getWidth(a-1)&&u.setCellFromCodePoint(a-1,_.NULL_CELL_CODE,_.NULL_CELL_WIDTH,d.fg,d.bg,d.extended)),u.setCellFromCodePoint(this._activeBuffer.x++,s,r,d.fg,d.bg,d.extended),r>0)for(;--r;)u.setCellFromCodePoint(this._activeBuffer.x++,0,0,d.fg,d.bg,d.extended)}else u.getWidth(this._activeBuffer.x-1)?u.addCodepointToCell(this._activeBuffer.x-1,s):u.addCodepointToCell(this._activeBuffer.x-2,s)}i-t>0&&(u.loadCell(this._activeBuffer.x-1,this._workCell),2===this._workCell.getWidth()||this._workCell.getCode()>65535?this._parser.precedingCodepoint=0:this._workCell.isCombined()?this._parser.precedingCodepoint=this._workCell.getChars().charCodeAt(0):this._parser.precedingCodepoint=this._workCell.content),this._activeBuffer.x<a&&i-t>0&&0===u.getWidth(this._activeBuffer.x)&&!u.hasContent(this._activeBuffer.x)&&u.setCellFromCodePoint(this._activeBuffer.x,0,1,d.fg,d.bg,d.extended),this._dirtyRowTracker.markDirty(this._activeBuffer.y)}registerCsiHandler(e,t){return"t"!==e.final||e.prefix||e.intermediates?this._parser.registerCsiHandler(e,t):this._parser.registerCsiHandler(e,(e=>!b(e.params[0],this._optionsService.rawOptions.windowOptions)||t(e)))}registerDcsHandler(e,t){return this._parser.registerDcsHandler(e,new g.DcsHandler(t))}registerEscHandler(e,t){return this._parser.registerEscHandler(e,t)}registerOscHandler(e,t){return this._parser.registerOscHandler(e,new p.OscHandler(t))}bell(){return this._onRequestBell.fire(),!0}lineFeed(){return this._dirtyRowTracker.markDirty(this._activeBuffer.y),this._optionsService.rawOptions.convertEol&&(this._activeBuffer.x=0),this._activeBuffer.y++,this._activeBuffer.y===this._activeBuffer.scrollBottom+1?(this._activeBuffer.y--,this._bufferService.scroll(this._eraseAttrData())):this._activeBuffer.y>=this._bufferService.rows?this._activeBuffer.y=this._bufferService.rows-1:this._activeBuffer.lines.get(this._activeBuffer.ybase+this._activeBuffer.y).isWrapped=!1,this._activeBuffer.x>=this._bufferService.cols&&this._activeBuffer.x--,this._dirtyRowTracker.markDirty(this._activeBuffer.y),this._onLineFeed.fire(),!0}carriageReturn(){return this._activeBuffer.x=0,!0}backspace(){var e;if(!this._coreService.decPrivateModes.reverseWraparound)return this._restrictCursor(),this._activeBuffer.x>0&&this._activeBuffer.x--,!0;if(this._restrictCursor(this._bufferService.cols),this._activeBuffer.x>0)this._activeBuffer.x--;else if(0===this._activeBuffer.x&&this._activeBuffer.y>this._activeBuffer.scrollTop&&this._activeBuffer.y<=this._activeBuffer.scrollBottom&&(null===(e=this._activeBuffer.lines.get(this._activeBuffer.ybase+this._activeBuffer.y))||void 0===e?void 0:e.isWrapped)){this._activeBuffer.lines.get(this._activeBuffer.ybase+this._activeBuffer.y).isWrapped=!1,this._activeBuffer.y--,this._activeBuffer.x=this._bufferService.cols-1;const e=this._activeBuffer.lines.get(this._activeBuffer.ybase+this._activeBuffer.y);e.hasWidth(this._activeBuffer.x)&&!e.hasContent(this._activeBuffer.x)&&this._activeBuffer.x--}return this._restrictCursor(),!0}tab(){if(this._activeBuffer.x>=this._bufferService.cols)return!0;const e=this._activeBuffer.x;return this._activeBuffer.x=this._activeBuffer.nextStop(),this._optionsService.rawOptions.screenReaderMode&&this._onA11yTab.fire(this._activeBuffer.x-e),!0}shiftOut(){return this._charsetService.setgLevel(1),!0}shiftIn(){return this._charsetService.setgLevel(0),!0}_restrictCursor(e=this._bufferService.cols-1){this._activeBuffer.x=Math.min(e,Math.max(0,this._activeBuffer.x)),this._activeBuffer.y=this._coreService.decPrivateModes.origin?Math.min(this._activeBuffer.scrollBottom,Math.max(this._activeBuffer.scrollTop,this._activeBuffer.y)):Math.min(this._bufferService.rows-1,Math.max(0,this._activeBuffer.y)),this._dirtyRowTracker.markDirty(this._activeBuffer.y)}_setCursor(e,t){this._dirtyRowTracker.markDirty(this._activeBuffer.y),this._coreService.decPrivateModes.origin?(this._activeBuffer.x=e,this._activeBuffer.y=this._activeBuffer.scrollTop+t):(this._activeBuffer.x=e,this._activeBuffer.y=t),this._restrictCursor(),this._dirtyRowTracker.markDirty(this._activeBuffer.y)}_moveCursor(e,t){this._restrictCursor(),this._setCursor(this._activeBuffer.x+e,this._activeBuffer.y+t)}cursorUp(e){const t=this._activeBuffer.y-this._activeBuffer.scrollTop;return t>=0?this._moveCursor(0,-Math.min(t,e.params[0]||1)):this._moveCursor(0,-(e.params[0]||1)),!0}cursorDown(e){const t=this._activeBuffer.scrollBottom-this._activeBuffer.y;return t>=0?this._moveCursor(0,Math.min(t,e.params[0]||1)):this._moveCursor(0,e.params[0]||1),!0}cursorForward(e){return this._moveCursor(e.params[0]||1,0),!0}cursorBackward(e){return this._moveCursor(-(e.params[0]||1),0),!0}cursorNextLine(e){return this.cursorDown(e),this._activeBuffer.x=0,!0}cursorPrecedingLine(e){return this.cursorUp(e),this._activeBuffer.x=0,!0}cursorCharAbsolute(e){return this._setCursor((e.params[0]||1)-1,this._activeBuffer.y),!0}cursorPosition(e){return this._setCursor(e.length>=2?(e.params[1]||1)-1:0,(e.params[0]||1)-1),!0}charPosAbsolute(e){return this._setCursor((e.params[0]||1)-1,this._activeBuffer.y),!0}hPositionRelative(e){return this._moveCursor(e.params[0]||1,0),!0}linePosAbsolute(e){return this._setCursor(this._activeBuffer.x,(e.params[0]||1)-1),!0}vPositionRelative(e){return this._moveCursor(0,e.params[0]||1),!0}hVPosition(e){return this.cursorPosition(e),!0}tabClear(e){const t=e.params[0];return 0===t?delete this._activeBuffer.tabs[this._activeBuffer.x]:3===t&&(this._activeBuffer.tabs={}),!0}cursorForwardTab(e){if(this._activeBuffer.x>=this._bufferService.cols)return!0;let t=e.params[0]||1;for(;t--;)this._activeBuffer.x=this._activeBuffer.nextStop();return!0}cursorBackwardTab(e){if(this._activeBuffer.x>=this._bufferService.cols)return!0;let t=e.params[0]||1;for(;t--;)this._activeBuffer.x=this._activeBuffer.prevStop();return!0}selectProtected(e){const t=e.params[0];return 1===t&&(this._curAttrData.bg|=536870912),2!==t&&0!==t||(this._curAttrData.bg&=-536870913),!0}_eraseInBufferLine(e,t,i,s=!1,r=!1){const n=this._activeBuffer.lines.get(this._activeBuffer.ybase+e);n.replaceCells(t,i,this._activeBuffer.getNullCell(this._eraseAttrData()),this._eraseAttrData(),r),s&&(n.isWrapped=!1)}_resetBufferLine(e,t=!1){const i=this._activeBuffer.lines.get(this._activeBuffer.ybase+e);i&&(i.fill(this._activeBuffer.getNullCell(this._eraseAttrData()),t),this._bufferService.buffer.clearMarkers(this._activeBuffer.ybase+e),i.isWrapped=!1)}eraseInDisplay(e,t=!1){let i;switch(this._restrictCursor(this._bufferService.cols),e.params[0]){case 0:for(i=this._activeBuffer.y,this._dirtyRowTracker.markDirty(i),this._eraseInBufferLine(i++,this._activeBuffer.x,this._bufferService.cols,0===this._activeBuffer.x,t);i<this._bufferService.rows;i++)this._resetBufferLine(i,t);this._dirtyRowTracker.markDirty(i);break;case 1:for(i=this._activeBuffer.y,this._dirtyRowTracker.markDirty(i),this._eraseInBufferLine(i,0,this._activeBuffer.x+1,!0,t),this._activeBuffer.x+1>=this._bufferService.cols&&(this._activeBuffer.lines.get(i+1).isWrapped=!1);i--;)this._resetBufferLine(i,t);this._dirtyRowTracker.markDirty(0);break;case 2:for(i=this._bufferService.rows,this._dirtyRowTracker.markDirty(i-1);i--;)this._resetBufferLine(i,t);this._dirtyRowTracker.markDirty(0);break;case 3:const e=this._activeBuffer.lines.length-this._bufferService.rows;e>0&&(this._activeBuffer.lines.trimStart(e),this._activeBuffer.ybase=Math.max(this._activeBuffer.ybase-e,0),this._activeBuffer.ydisp=Math.max(this._activeBuffer.ydisp-e,0),this._onScroll.fire(0))}return!0}eraseInLine(e,t=!1){switch(this._restrictCursor(this._bufferService.cols),e.params[0]){case 0:this._eraseInBufferLine(this._activeBuffer.y,this._activeBuffer.x,this._bufferService.cols,0===this._activeBuffer.x,t);break;case 1:this._eraseInBufferLine(this._activeBuffer.y,0,this._activeBuffer.x+1,!1,t);break;case 2:this._eraseInBufferLine(this._activeBuffer.y,0,this._bufferService.cols,!0,t)}return this._dirtyRowTracker.markDirty(this._activeBuffer.y),!0}insertLines(e){this._restrictCursor();let t=e.params[0]||1;if(this._activeBuffer.y>this._activeBuffer.scrollBottom||this._activeBuffer.y<this._activeBuffer.scrollTop)return!0;const i=this._activeBuffer.ybase+this._activeBuffer.y,s=this._bufferService.rows-1-this._activeBuffer.scrollBottom,r=this._bufferService.rows-1+this._activeBuffer.ybase-s+1;for(;t--;)this._activeBuffer.lines.splice(r-1,1),this._activeBuffer.lines.splice(i,0,this._activeBuffer.getBlankLine(this._eraseAttrData()));return this._dirtyRowTracker.markRangeDirty(this._activeBuffer.y,this._activeBuffer.scrollBottom),this._activeBuffer.x=0,!0}deleteLines(e){this._restrictCursor();let t=e.params[0]||1;if(this._activeBuffer.y>this._activeBuffer.scrollBottom||this._activeBuffer.y<this._activeBuffer.scrollTop)return!0;const i=this._activeBuffer.ybase+this._activeBuffer.y;let s;for(s=this._bufferService.rows-1-this._activeBuffer.scrollBottom,s=this._bufferService.rows-1+this._activeBuffer.ybase-s;t--;)this._activeBuffer.lines.splice(i,1),this._activeBuffer.lines.splice(s,0,this._activeBuffer.getBlankLine(this._eraseAttrData()));return this._dirtyRowTracker.markRangeDirty(this._activeBuffer.y,this._activeBuffer.scrollBottom),this._activeBuffer.x=0,!0}insertChars(e){this._restrictCursor();const t=this._activeBuffer.lines.get(this._activeBuffer.ybase+this._activeBuffer.y);return t&&(t.insertCells(this._activeBuffer.x,e.params[0]||1,this._activeBuffer.getNullCell(this._eraseAttrData()),this._eraseAttrData()),this._dirtyRowTracker.markDirty(this._activeBuffer.y)),!0}deleteChars(e){this._restrictCursor();const t=this._activeBuffer.lines.get(this._activeBuffer.ybase+this._activeBuffer.y);return t&&(t.deleteCells(this._activeBuffer.x,e.params[0]||1,this._activeBuffer.getNullCell(this._eraseAttrData()),this._eraseAttrData()),this._dirtyRowTracker.markDirty(this._activeBuffer.y)),!0}scrollUp(e){let t=e.params[0]||1;for(;t--;)this._activeBuffer.lines.splice(this._activeBuffer.ybase+this._activeBuffer.scrollTop,1),this._activeBuffer.lines.splice(this._activeBuffer.ybase+this._activeBuffer.scrollBottom,0,this._activeBuffer.getBlankLine(this._eraseAttrData()));return this._dirtyRowTracker.markRangeDirty(this._activeBuffer.scrollTop,this._activeBuffer.scrollBottom),!0}scrollDown(e){let t=e.params[0]||1;for(;t--;)this._activeBuffer.lines.splice(this._activeBuffer.ybase+this._activeBuffer.scrollBottom,1),this._activeBuffer.lines.splice(this._activeBuffer.ybase+this._activeBuffer.scrollTop,0,this._activeBuffer.getBlankLine(l.DEFAULT_ATTR_DATA));return this._dirtyRowTracker.markRangeDirty(this._activeBuffer.scrollTop,this._activeBuffer.scrollBottom),!0}scrollLeft(e){if(this._activeBuffer.y>this._activeBuffer.scrollBottom||this._activeBuffer.y<this._activeBuffer.scrollTop)return!0;const t=e.params[0]||1;for(let e=this._activeBuffer.scrollTop;e<=this._activeBuffer.scrollBottom;++e){const i=this._activeBuffer.lines.get(this._activeBuffer.ybase+e);i.deleteCells(0,t,this._activeBuffer.getNullCell(this._eraseAttrData()),this._eraseAttrData()),i.isWrapped=!1}return this._dirtyRowTracker.markRangeDirty(this._activeBuffer.scrollTop,this._activeBuffer.scrollBottom),!0}scrollRight(e){if(this._activeBuffer.y>this._activeBuffer.scrollBottom||this._activeBuffer.y<this._activeBuffer.scrollTop)return!0;const t=e.params[0]||1;for(let e=this._activeBuffer.scrollTop;e<=this._activeBuffer.scrollBottom;++e){const i=this._activeBuffer.lines.get(this._activeBuffer.ybase+e);i.insertCells(0,t,this._activeBuffer.getNullCell(this._eraseAttrData()),this._eraseAttrData()),i.isWrapped=!1}return this._dirtyRowTracker.markRangeDirty(this._activeBuffer.scrollTop,this._activeBuffer.scrollBottom),!0}insertColumns(e){if(this._activeBuffer.y>this._activeBuffer.scrollBottom||this._activeBuffer.y<this._activeBuffer.scrollTop)return!0;const t=e.params[0]||1;for(let e=this._activeBuffer.scrollTop;e<=this._activeBuffer.scrollBottom;++e){const i=this._activeBuffer.lines.get(this._activeBuffer.ybase+e);i.insertCells(this._activeBuffer.x,t,this._activeBuffer.getNullCell(this._eraseAttrData()),this._eraseAttrData()),i.isWrapped=!1}return this._dirtyRowTracker.markRangeDirty(this._activeBuffer.scrollTop,this._activeBuffer.scrollBottom),!0}deleteColumns(e){if(this._activeBuffer.y>this._activeBuffer.scrollBottom||this._activeBuffer.y<this._activeBuffer.scrollTop)return!0;const t=e.params[0]||1;for(let e=this._activeBuffer.scrollTop;e<=this._activeBuffer.scrollBottom;++e){const i=this._activeBuffer.lines.get(this._activeBuffer.ybase+e);i.deleteCells(this._activeBuffer.x,t,this._activeBuffer.getNullCell(this._eraseAttrData()),this._eraseAttrData()),i.isWrapped=!1}return this._dirtyRowTracker.markRangeDirty(this._activeBuffer.scrollTop,this._activeBuffer.scrollBottom),!0}eraseChars(e){this._restrictCursor();const t=this._activeBuffer.lines.get(this._activeBuffer.ybase+this._activeBuffer.y);return t&&(t.replaceCells(this._activeBuffer.x,this._activeBuffer.x+(e.params[0]||1),this._activeBuffer.getNullCell(this._eraseAttrData()),this._eraseAttrData()),this._dirtyRowTracker.markDirty(this._activeBuffer.y)),!0}repeatPrecedingCharacter(e){if(!this._parser.precedingCodepoint)return!0;const t=e.params[0]||1,i=new Uint32Array(t);for(let e=0;e<t;++e)i[e]=this._parser.precedingCodepoint;return this.print(i,0,i.length),!0}sendDeviceAttributesPrimary(e){return e.params[0]>0||(this._is("xterm")||this._is("rxvt-unicode")||this._is("screen")?this._coreService.triggerDataEvent(n.C0.ESC+"[?1;2c"):this._is("linux")&&this._coreService.triggerDataEvent(n.C0.ESC+"[?6c")),!0}sendDeviceAttributesSecondary(e){return e.params[0]>0||(this._is("xterm")?this._coreService.triggerDataEvent(n.C0.ESC+"[>0;276;0c"):this._is("rxvt-unicode")?this._coreService.triggerDataEvent(n.C0.ESC+"[>85;95;0c"):this._is("linux")?this._coreService.triggerDataEvent(e.params[0]+"c"):this._is("screen")&&this._coreService.triggerDataEvent(n.C0.ESC+"[>83;40003;0c")),!0}_is(e){return 0===(this._optionsService.rawOptions.termName+"").indexOf(e)}setMode(e){for(let t=0;t<e.length;t++)switch(e.params[t]){case 4:this._coreService.modes.insertMode=!0;break;case 20:this._optionsService.options.convertEol=!0}return!0}setModePrivate(e){for(let t=0;t<e.length;t++)switch(e.params[t]){case 1:this._coreService.decPrivateModes.applicationCursorKeys=!0;break;case 2:this._charsetService.setgCharset(0,o.DEFAULT_CHARSET),this._charsetService.setgCharset(1,o.DEFAULT_CHARSET),this._charsetService.setgCharset(2,o.DEFAULT_CHARSET),this._charsetService.setgCharset(3,o.DEFAULT_CHARSET);break;case 3:this._optionsService.rawOptions.windowOptions.setWinLines&&(this._bufferService.resize(132,this._bufferService.rows),this._onRequestReset.fire());break;case 6:this._coreService.decPrivateModes.origin=!0,this._setCursor(0,0);break;case 7:this._coreService.decPrivateModes.wraparound=!0;break;case 12:this._optionsService.options.cursorBlink=!0;break;case 45:this._coreService.decPrivateModes.reverseWraparound=!0;break;case 66:this._logService.debug("Serial port requested application keypad."),this._coreService.decPrivateModes.applicationKeypad=!0,this._onRequestSyncScrollBar.fire();break;case 9:this._coreMouseService.activeProtocol="X10";break;case 1e3:this._coreMouseService.activeProtocol="VT200";break;case 1002:this._coreMouseService.activeProtocol="DRAG";break;case 1003:this._coreMouseService.activeProtocol="ANY";break;case 1004:this._coreService.decPrivateModes.sendFocus=!0,this._onRequestSendFocus.fire();break;case 1005:this._logService.debug("DECSET 1005 not supported (see #2507)");break;case 1006:this._coreMouseService.activeEncoding="SGR";break;case 1015:this._logService.debug("DECSET 1015 not supported (see #2507)");break;case 1016:this._coreMouseService.activeEncoding="SGR_PIXELS";break;case 25:this._coreService.isCursorHidden=!1;break;case 1048:this.saveCursor();break;case 1049:this.saveCursor();case 47:case 1047:this._bufferService.buffers.activateAltBuffer(this._eraseAttrData()),this._coreService.isCursorInitialized=!0,this._onRequestRefreshRows.fire(0,this._bufferService.rows-1),this._onRequestSyncScrollBar.fire();break;case 2004:this._coreService.decPrivateModes.bracketedPasteMode=!0}return!0}resetMode(e){for(let t=0;t<e.length;t++)switch(e.params[t]){case 4:this._coreService.modes.insertMode=!1;break;case 20:this._optionsService.options.convertEol=!1}return!0}resetModePrivate(e){for(let t=0;t<e.length;t++)switch(e.params[t]){case 1:this._coreService.decPrivateModes.applicationCursorKeys=!1;break;case 3:this._optionsService.rawOptions.windowOptions.setWinLines&&(this._bufferService.resize(80,this._bufferService.rows),this._onRequestReset.fire());break;case 6:this._coreService.decPrivateModes.origin=!1,this._setCursor(0,0);break;case 7:this._coreService.decPrivateModes.wraparound=!1;break;case 12:this._optionsService.options.cursorBlink=!1;break;case 45:this._coreService.decPrivateModes.reverseWraparound=!1;break;case 66:this._logService.debug("Switching back to normal keypad."),this._coreService.decPrivateModes.applicationKeypad=!1,this._onRequestSyncScrollBar.fire();break;case 9:case 1e3:case 1002:case 1003:this._coreMouseService.activeProtocol="NONE";break;case 1004:this._coreService.decPrivateModes.sendFocus=!1;break;case 1005:this._logService.debug("DECRST 1005 not supported (see #2507)");break;case 1006:case 1016:this._coreMouseService.activeEncoding="DEFAULT";break;case 1015:this._logService.debug("DECRST 1015 not supported (see #2507)");break;case 25:this._coreService.isCursorHidden=!0;break;case 1048:this.restoreCursor();break;case 1049:case 47:case 1047:this._bufferService.buffers.activateNormalBuffer(),1049===e.params[t]&&this.restoreCursor(),this._coreService.isCursorInitialized=!0,this._onRequestRefreshRows.fire(0,this._bufferService.rows-1),this._onRequestSyncScrollBar.fire();break;case 2004:this._coreService.decPrivateModes.bracketedPasteMode=!1}return!0}requestMode(e,t){const i=this._coreService.decPrivateModes,{activeProtocol:s,activeEncoding:r}=this._coreMouseService,o=this._coreService,{buffers:a,cols:h}=this._bufferService,{active:c,alt:l}=a,d=this._optionsService.rawOptions,_=e=>e?1:2,u=e.params[0];return f=u,v=t?2===u?4:4===u?_(o.modes.insertMode):12===u?3:20===u?_(d.convertEol):0:1===u?_(i.applicationCursorKeys):3===u?d.windowOptions.setWinLines?80===h?2:132===h?1:0:0:6===u?_(i.origin):7===u?_(i.wraparound):8===u?3:9===u?_("X10"===s):12===u?_(d.cursorBlink):25===u?_(!o.isCursorHidden):45===u?_(i.reverseWraparound):66===u?_(i.applicationKeypad):67===u?4:1e3===u?_("VT200"===s):1002===u?_("DRAG"===s):1003===u?_("ANY"===s):1004===u?_(i.sendFocus):1005===u?4:1006===u?_("SGR"===r):1015===u?4:1016===u?_("SGR_PIXELS"===r):1048===u?1:47===u||1047===u||1049===u?_(c===l):2004===u?_(i.bracketedPasteMode):0,o.triggerDataEvent(`${n.C0.ESC}[${t?"":"?"}${f};${v}$y`),!0;var f,v}_updateAttrColor(e,t,i,s,r){return 2===t?(e|=50331648,e&=-16777216,e|=f.AttributeData.fromColorRGB([i,s,r])):5===t&&(e&=-50331904,e|=33554432|255&i),e}_extractColor(e,t,i){const s=[0,0,-1,0,0,0];let r=0,n=0;do{if(s[n+r]=e.params[t+n],e.hasSubParams(t+n)){const i=e.getSubParams(t+n);let o=0;do{5===s[1]&&(r=1),s[n+o+1+r]=i[o]}while(++o<i.length&&o+n+1+r<s.length);break}if(5===s[1]&&n+r>=2||2===s[1]&&n+r>=5)break;s[1]&&(r=1)}while(++n+t<e.length&&n+r<s.length);for(let e=2;e<s.length;++e)-1===s[e]&&(s[e]=0);switch(s[0]){case 38:i.fg=this._updateAttrColor(i.fg,s[1],s[3],s[4],s[5]);break;case 48:i.bg=this._updateAttrColor(i.bg,s[1],s[3],s[4],s[5]);break;case 58:i.extended=i.extended.clone(),i.extended.underlineColor=this._updateAttrColor(i.extended.underlineColor,s[1],s[3],s[4],s[5])}return n}_processUnderline(e,t){t.extended=t.extended.clone(),(!~e||e>5)&&(e=1),t.extended.underlineStyle=e,t.fg|=268435456,0===e&&(t.fg&=-268435457),t.updateExtended()}_processSGR0(e){e.fg=l.DEFAULT_ATTR_DATA.fg,e.bg=l.DEFAULT_ATTR_DATA.bg,e.extended=e.extended.clone(),e.extended.underlineStyle=0,e.extended.underlineColor&=-67108864,e.updateExtended()}charAttributes(e){if(1===e.length&&0===e.params[0])return this._processSGR0(this._curAttrData),!0;const t=e.length;let i;const s=this._curAttrData;for(let r=0;r<t;r++)i=e.params[r],i>=30&&i<=37?(s.fg&=-50331904,s.fg|=16777216|i-30):i>=40&&i<=47?(s.bg&=-50331904,s.bg|=16777216|i-40):i>=90&&i<=97?(s.fg&=-50331904,s.fg|=16777224|i-90):i>=100&&i<=107?(s.bg&=-50331904,s.bg|=16777224|i-100):0===i?this._processSGR0(s):1===i?s.fg|=134217728:3===i?s.bg|=67108864:4===i?(s.fg|=268435456,this._processUnderline(e.hasSubParams(r)?e.getSubParams(r)[0]:1,s)):5===i?s.fg|=536870912:7===i?s.fg|=67108864:8===i?s.fg|=1073741824:9===i?s.fg|=2147483648:2===i?s.bg|=134217728:21===i?this._processUnderline(2,s):22===i?(s.fg&=-134217729,s.bg&=-134217729):23===i?s.bg&=-67108865:24===i?(s.fg&=-268435457,this._processUnderline(0,s)):25===i?s.fg&=-536870913:27===i?s.fg&=-67108865:28===i?s.fg&=-1073741825:29===i?s.fg&=2147483647:39===i?(s.fg&=-67108864,s.fg|=16777215&l.DEFAULT_ATTR_DATA.fg):49===i?(s.bg&=-67108864,s.bg|=16777215&l.DEFAULT_ATTR_DATA.bg):38===i||48===i||58===i?r+=this._extractColor(e,r,s):53===i?s.bg|=1073741824:55===i?s.bg&=-1073741825:59===i?(s.extended=s.extended.clone(),s.extended.underlineColor=-1,s.updateExtended()):100===i?(s.fg&=-67108864,s.fg|=16777215&l.DEFAULT_ATTR_DATA.fg,s.bg&=-67108864,s.bg|=16777215&l.DEFAULT_ATTR_DATA.bg):this._logService.debug("Unknown SGR attribute: %d.",i);return!0}deviceStatus(e){switch(e.params[0]){case 5:this._coreService.triggerDataEvent(`${n.C0.ESC}[0n`);break;case 6:const e=this._activeBuffer.y+1,t=this._activeBuffer.x+1;this._coreService.triggerDataEvent(`${n.C0.ESC}[${e};${t}R`)}return!0}deviceStatusPrivate(e){if(6===e.params[0]){const e=this._activeBuffer.y+1,t=this._activeBuffer.x+1;this._coreService.triggerDataEvent(`${n.C0.ESC}[?${e};${t}R`)}return!0}softReset(e){return this._coreService.isCursorHidden=!1,this._onRequestSyncScrollBar.fire(),this._activeBuffer.scrollTop=0,this._activeBuffer.scrollBottom=this._bufferService.rows-1,this._curAttrData=l.DEFAULT_ATTR_DATA.clone(),this._coreService.reset(),this._charsetService.reset(),this._activeBuffer.savedX=0,this._activeBuffer.savedY=this._activeBuffer.ybase,this._activeBuffer.savedCurAttrData.fg=this._curAttrData.fg,this._activeBuffer.savedCurAttrData.bg=this._curAttrData.bg,this._activeBuffer.savedCharset=this._charsetService.charset,this._coreService.decPrivateModes.origin=!1,!0}setCursorStyle(e){const t=e.params[0]||1;switch(t){case 1:case 2:this._optionsService.options.cursorStyle="block";break;case 3:case 4:this._optionsService.options.cursorStyle="underline";break;case 5:case 6:this._optionsService.options.cursorStyle="bar"}const i=t%2==1;return this._optionsService.options.cursorBlink=i,!0}setScrollRegion(e){const t=e.params[0]||1;let i;return(e.length<2||(i=e.params[1])>this._bufferService.rows||0===i)&&(i=this._bufferService.rows),i>t&&(this._activeBuffer.scrollTop=t-1,this._activeBuffer.scrollBottom=i-1,this._setCursor(0,0)),!0}windowOptions(e){if(!b(e.params[0],this._optionsService.rawOptions.windowOptions))return!0;const t=e.length>1?e.params[1]:0;switch(e.params[0]){case 14:2!==t&&this._onRequestWindowsOptionsReport.fire(y.GET_WIN_SIZE_PIXELS);break;case 16:this._onRequestWindowsOptionsReport.fire(y.GET_CELL_SIZE_PIXELS);break;case 18:this._bufferService&&this._coreService.triggerDataEvent(`${n.C0.ESC}[8;${this._bufferService.rows};${this._bufferService.cols}t`);break;case 22:0!==t&&2!==t||(this._windowTitleStack.push(this._windowTitle),this._windowTitleStack.length>10&&this._windowTitleStack.shift()),0!==t&&1!==t||(this._iconNameStack.push(this._iconName),this._iconNameStack.length>10&&this._iconNameStack.shift());break;case 23:0!==t&&2!==t||this._windowTitleStack.length&&this.setTitle(this._windowTitleStack.pop()),0!==t&&1!==t||this._iconNameStack.length&&this.setIconName(this._iconNameStack.pop())}return!0}saveCursor(e){return this._activeBuffer.savedX=this._activeBuffer.x,this._activeBuffer.savedY=this._activeBuffer.ybase+this._activeBuffer.y,this._activeBuffer.savedCurAttrData.fg=this._curAttrData.fg,this._activeBuffer.savedCurAttrData.bg=this._curAttrData.bg,this._activeBuffer.savedCharset=this._charsetService.charset,!0}restoreCursor(e){return this._activeBuffer.x=this._activeBuffer.savedX||0,this._activeBuffer.y=Math.max(this._activeBuffer.savedY-this._activeBuffer.ybase,0),this._curAttrData.fg=this._activeBuffer.savedCurAttrData.fg,this._curAttrData.bg=this._activeBuffer.savedCurAttrData.bg,this._charsetService.charset=this._savedCharset,this._activeBuffer.savedCharset&&(this._charsetService.charset=this._activeBuffer.savedCharset),this._restrictCursor(),!0}setTitle(e){return this._windowTitle=e,this._onTitleChange.fire(e),!0}setIconName(e){return this._iconName=e,!0}setOrReportIndexedColor(e){const t=[],i=e.split(";");for(;i.length>1;){const e=i.shift(),s=i.shift();if(/^\d+$/.exec(e)){const i=parseInt(e);if(L(i))if("?"===s)t.push({type:0,index:i});else{const e=(0,m.parseColor)(s);e&&t.push({type:1,index:i,color:e})}}}return t.length&&this._onColor.fire(t),!0}setHyperlink(e){const t=e.split(";");return!(t.length<2)&&(t[1]?this._createHyperlink(t[0],t[1]):!t[0]&&this._finishHyperlink())}_createHyperlink(e,t){this._getCurrentLinkId()&&this._finishHyperlink();const i=e.split(":");let s;const r=i.findIndex((e=>e.startsWith("id=")));return-1!==r&&(s=i[r].slice(3)||void 0),this._curAttrData.extended=this._curAttrData.extended.clone(),this._curAttrData.extended.urlId=this._oscLinkService.registerLink({id:s,uri:t}),this._curAttrData.updateExtended(),!0}_finishHyperlink(){return this._curAttrData.extended=this._curAttrData.extended.clone(),this._curAttrData.extended.urlId=0,this._curAttrData.updateExtended(),!0}_setOrReportSpecialColor(e,t){const i=e.split(";");for(let e=0;e<i.length&&!(t>=this._specialColors.length);++e,++t)if("?"===i[e])this._onColor.fire([{type:0,index:this._specialColors[t]}]);else{const s=(0,m.parseColor)(i[e]);s&&this._onColor.fire([{type:1,index:this._specialColors[t],color:s}])}return!0}setOrReportFgColor(e){return this._setOrReportSpecialColor(e,0)}setOrReportBgColor(e){return this._setOrReportSpecialColor(e,1)}setOrReportCursorColor(e){return this._setOrReportSpecialColor(e,2)}restoreIndexedColor(e){if(!e)return this._onColor.fire([{type:2}]),!0;const t=[],i=e.split(";");for(let e=0;e<i.length;++e)if(/^\d+$/.exec(i[e])){const s=parseInt(i[e]);L(s)&&t.push({type:2,index:s})}return t.length&&this._onColor.fire(t),!0}restoreFgColor(e){return this._onColor.fire([{type:2,index:256}]),!0}restoreBgColor(e){return this._onColor.fire([{type:2,index:257}]),!0}restoreCursorColor(e){return this._onColor.fire([{type:2,index:258}]),!0}nextLine(){return this._activeBuffer.x=0,this.index(),!0}keypadApplicationMode(){return this._logService.debug("Serial port requested application keypad."),this._coreService.decPrivateModes.applicationKeypad=!0,this._onRequestSyncScrollBar.fire(),!0}keypadNumericMode(){return this._logService.debug("Switching back to normal keypad."),this._coreService.decPrivateModes.applicationKeypad=!1,this._onRequestSyncScrollBar.fire(),!0}selectDefaultCharset(){return this._charsetService.setgLevel(0),this._charsetService.setgCharset(0,o.DEFAULT_CHARSET),!0}selectCharset(e){return 2!==e.length?(this.selectDefaultCharset(),!0):("/"===e[0]||this._charsetService.setgCharset(S[e[0]],o.CHARSETS[e[1]]||o.DEFAULT_CHARSET),!0)}index(){return this._restrictCursor(),this._activeBuffer.y++,this._activeBuffer.y===this._activeBuffer.scrollBottom+1?(this._activeBuffer.y--,this._bufferService.scroll(this._eraseAttrData())):this._activeBuffer.y>=this._bufferService.rows&&(this._activeBuffer.y=this._bufferService.rows-1),this._restrictCursor(),!0}tabSet(){return this._activeBuffer.tabs[this._activeBuffer.x]=!0,!0}reverseIndex(){if(this._restrictCursor(),this._activeBuffer.y===this._activeBuffer.scrollTop){const e=this._activeBuffer.scrollBottom-this._activeBuffer.scrollTop;this._activeBuffer.lines.shiftElements(this._activeBuffer.ybase+this._activeBuffer.y,e,1),this._activeBuffer.lines.set(this._activeBuffer.ybase+this._activeBuffer.y,this._activeBuffer.getBlankLine(this._eraseAttrData())),this._dirtyRowTracker.markRangeDirty(this._activeBuffer.scrollTop,this._activeBuffer.scrollBottom)}else this._activeBuffer.y--,this._restrictCursor();return!0}fullReset(){return this._parser.reset(),this._onRequestReset.fire(),!0}reset(){this._curAttrData=l.DEFAULT_ATTR_DATA.clone(),this._eraseAttrDataInternal=l.DEFAULT_ATTR_DATA.clone()}_eraseAttrData(){return this._eraseAttrDataInternal.bg&=-67108864,this._eraseAttrDataInternal.bg|=67108863&this._curAttrData.bg,this._eraseAttrDataInternal}setgLevel(e){return this._charsetService.setgLevel(e),!0}screenAlignmentPattern(){const e=new u.CellData;e.content=1<<22|"E".charCodeAt(0),e.fg=this._curAttrData.fg,e.bg=this._curAttrData.bg,this._setCursor(0,0);for(let t=0;t<this._bufferService.rows;++t){const i=this._activeBuffer.ybase+this._activeBuffer.y+t,s=this._activeBuffer.lines.get(i);s&&(s.fill(e),s.isWrapped=!1)}return this._dirtyRowTracker.markAllDirty(),this._setCursor(0,0),!0}requestStatusString(e,t){const i=this._bufferService.buffer,s=this._optionsService.rawOptions;return(e=>(this._coreService.triggerDataEvent(`${n.C0.ESC}${e}${n.C0.ESC}\\`),!0))('"q'===e?`P1$r${this._curAttrData.isProtected()?1:0}"q`:'"p'===e?'P1$r61;1"p':"r"===e?`P1$r${i.scrollTop+1};${i.scrollBottom+1}r`:"m"===e?"P1$r0m":" q"===e?`P1$r${{block:2,underline:4,bar:6}[s.cursorStyle]-(s.cursorBlink?1:0)} q`:"P0$r")}markRangeDirty(e,t){this._dirtyRowTracker.markRangeDirty(e,t)}}t.InputHandler=E;let k=class{constructor(e){this._bufferService=e,this.clearRange()}clearRange(){this.start=this._bufferService.buffer.y,this.end=this._bufferService.buffer.y}markDirty(e){e<this.start?this.start=e:e>this.end&&(this.end=e)}markRangeDirty(e,t){e>t&&(w=e,e=t,t=w),e<this.start&&(this.start=e),t>this.end&&(this.end=t)}markAllDirty(){this.markRangeDirty(0,this._bufferService.rows-1)}};function L(e){return 0<=e&&e<256}k=s([r(0,v.IBufferService)],k)},844:(e,t)=>{function i(e){for(const t of e)t.dispose();e.length=0}Object.defineProperty(t,"__esModule",{value:!0}),t.getDisposeArrayDisposable=t.disposeArray=t.toDisposable=t.MutableDisposable=t.Disposable=void 0,t.Disposable=class{constructor(){this._disposables=[],this._isDisposed=!1}dispose(){this._isDisposed=!0;for(const e of this._disposables)e.dispose();this._disposables.length=0}register(e){return this._disposables.push(e),e}unregister(e){const t=this._disposables.indexOf(e);-1!==t&&this._disposables.splice(t,1)}},t.MutableDisposable=class{constructor(){this._isDisposed=!1}get value(){return this._isDisposed?void 0:this._value}set value(e){var t;this._isDisposed||e===this._value||(null===(t=this._value)||void 0===t||t.dispose(),this._value=e)}clear(){this.value=void 0}dispose(){var e;this._isDisposed=!0,null===(e=this._value)||void 0===e||e.dispose(),this._value=void 0}},t.toDisposable=function(e){return{dispose:e}},t.disposeArray=i,t.getDisposeArrayDisposable=function(e){return{dispose:()=>i(e)}}},1505:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.FourKeyMap=t.TwoKeyMap=void 0;class i{constructor(){this._data={}}set(e,t,i){this._data[e]||(this._data[e]={}),this._data[e][t]=i}get(e,t){return this._data[e]?this._data[e][t]:void 0}clear(){this._data={}}}t.TwoKeyMap=i,t.FourKeyMap=class{constructor(){this._data=new i}set(e,t,s,r,n){this._data.get(e,t)||this._data.set(e,t,new i),this._data.get(e,t).set(s,r,n)}get(e,t,i,s){var r;return null===(r=this._data.get(e,t))||void 0===r?void 0:r.get(i,s)}clear(){this._data.clear()}}},6114:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.isChromeOS=t.isLinux=t.isWindows=t.isIphone=t.isIpad=t.isMac=t.getSafariVersion=t.isSafari=t.isLegacyEdge=t.isFirefox=t.isNode=void 0,t.isNode="undefined"==typeof navigator;const i=t.isNode?"node":navigator.userAgent,s=t.isNode?"node":navigator.platform;t.isFirefox=i.includes("Firefox"),t.isLegacyEdge=i.includes("Edge"),t.isSafari=/^((?!chrome|android).)*safari/i.test(i),t.getSafariVersion=function(){if(!t.isSafari)return 0;const e=i.match(/Version\/(\d+)/);return null===e||e.length<2?0:parseInt(e[1])},t.isMac=["Macintosh","MacIntel","MacPPC","Mac68K"].includes(s),t.isIpad="iPad"===s,t.isIphone="iPhone"===s,t.isWindows=["Windows","Win16","Win32","WinCE"].includes(s),t.isLinux=s.indexOf("Linux")>=0,t.isChromeOS=/\bCrOS\b/.test(i)},6106:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.SortedList=void 0;let i=0;t.SortedList=class{constructor(e){this._getKey=e,this._array=[]}clear(){this._array.length=0}insert(e){0!==this._array.length?(i=this._search(this._getKey(e)),this._array.splice(i,0,e)):this._array.push(e)}delete(e){if(0===this._array.length)return!1;const t=this._getKey(e);if(void 0===t)return!1;if(i=this._search(t),-1===i)return!1;if(this._getKey(this._array[i])!==t)return!1;do{if(this._array[i]===e)return this._array.splice(i,1),!0}while(++i<this._array.length&&this._getKey(this._array[i])===t);return!1}*getKeyIterator(e){if(0!==this._array.length&&(i=this._search(e),!(i<0||i>=this._array.length)&&this._getKey(this._array[i])===e))do{yield this._array[i]}while(++i<this._array.length&&this._getKey(this._array[i])===e)}forEachByKey(e,t){if(0!==this._array.length&&(i=this._search(e),!(i<0||i>=this._array.length)&&this._getKey(this._array[i])===e))do{t(this._array[i])}while(++i<this._array.length&&this._getKey(this._array[i])===e)}values(){return[...this._array].values()}_search(e){let t=0,i=this._array.length-1;for(;i>=t;){let s=t+i>>1;const r=this._getKey(this._array[s]);if(r>e)i=s-1;else{if(!(r<e)){for(;s>0&&this._getKey(this._array[s-1])===e;)s--;return s}t=s+1}}return t}}},7226:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.DebouncedIdleTask=t.IdleTaskQueue=t.PriorityTaskQueue=void 0;const s=i(6114);class r{constructor(){this._tasks=[],this._i=0}enqueue(e){this._tasks.push(e),this._start()}flush(){for(;this._i<this._tasks.length;)this._tasks[this._i]()||this._i++;this.clear()}clear(){this._idleCallback&&(this._cancelCallback(this._idleCallback),this._idleCallback=void 0),this._i=0,this._tasks.length=0}_start(){this._idleCallback||(this._idleCallback=this._requestCallback(this._process.bind(this)))}_process(e){this._idleCallback=void 0;let t=0,i=0,s=e.timeRemaining(),r=0;for(;this._i<this._tasks.length;){if(t=Date.now(),this._tasks[this._i]()||this._i++,t=Math.max(1,Date.now()-t),i=Math.max(t,i),r=e.timeRemaining(),1.5*i>r)return s-t<-20&&console.warn(`task queue exceeded allotted deadline by ${Math.abs(Math.round(s-t))}ms`),void this._start();s=r}this.clear()}}class n extends r{_requestCallback(e){return setTimeout((()=>e(this._createDeadline(16))))}_cancelCallback(e){clearTimeout(e)}_createDeadline(e){const t=Date.now()+e;return{timeRemaining:()=>Math.max(0,t-Date.now())}}}t.PriorityTaskQueue=n,t.IdleTaskQueue=!s.isNode&&"requestIdleCallback"in window?class extends r{_requestCallback(e){return requestIdleCallback(e)}_cancelCallback(e){cancelIdleCallback(e)}}:n,t.DebouncedIdleTask=class{constructor(){this._queue=new t.IdleTaskQueue}set(e){this._queue.clear(),this._queue.enqueue(e)}flush(){this._queue.flush()}}},9282:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.updateWindowsModeWrappedState=void 0;const s=i(643);t.updateWindowsModeWrappedState=function(e){const t=e.buffer.lines.get(e.buffer.ybase+e.buffer.y-1),i=null==t?void 0:t.get(e.cols-1),r=e.buffer.lines.get(e.buffer.ybase+e.buffer.y);r&&i&&(r.isWrapped=i[s.CHAR_DATA_CODE_INDEX]!==s.NULL_CELL_CODE&&i[s.CHAR_DATA_CODE_INDEX]!==s.WHITESPACE_CELL_CODE)}},3734:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.ExtendedAttrs=t.AttributeData=void 0;class i{constructor(){this.fg=0,this.bg=0,this.extended=new s}static toColorRGB(e){return[e>>>16&255,e>>>8&255,255&e]}static fromColorRGB(e){return(255&e[0])<<16|(255&e[1])<<8|255&e[2]}clone(){const e=new i;return e.fg=this.fg,e.bg=this.bg,e.extended=this.extended.clone(),e}isInverse(){return 67108864&this.fg}isBold(){return 134217728&this.fg}isUnderline(){return this.hasExtendedAttrs()&&0!==this.extended.underlineStyle?1:268435456&this.fg}isBlink(){return 536870912&this.fg}isInvisible(){return 1073741824&this.fg}isItalic(){return 67108864&this.bg}isDim(){return 134217728&this.bg}isStrikethrough(){return 2147483648&this.fg}isProtected(){return 536870912&this.bg}isOverline(){return 1073741824&this.bg}getFgColorMode(){return 50331648&this.fg}getBgColorMode(){return 50331648&this.bg}isFgRGB(){return 50331648==(50331648&this.fg)}isBgRGB(){return 50331648==(50331648&this.bg)}isFgPalette(){return 16777216==(50331648&this.fg)||33554432==(50331648&this.fg)}isBgPalette(){return 16777216==(50331648&this.bg)||33554432==(50331648&this.bg)}isFgDefault(){return 0==(50331648&this.fg)}isBgDefault(){return 0==(50331648&this.bg)}isAttributeDefault(){return 0===this.fg&&0===this.bg}getFgColor(){switch(50331648&this.fg){case 16777216:case 33554432:return 255&this.fg;case 50331648:return 16777215&this.fg;default:return-1}}getBgColor(){switch(50331648&this.bg){case 16777216:case 33554432:return 255&this.bg;case 50331648:return 16777215&this.bg;default:return-1}}hasExtendedAttrs(){return 268435456&this.bg}updateExtended(){this.extended.isEmpty()?this.bg&=-268435457:this.bg|=268435456}getUnderlineColor(){if(268435456&this.bg&&~this.extended.underlineColor)switch(50331648&this.extended.underlineColor){case 16777216:case 33554432:return 255&this.extended.underlineColor;case 50331648:return 16777215&this.extended.underlineColor;default:return this.getFgColor()}return this.getFgColor()}getUnderlineColorMode(){return 268435456&this.bg&&~this.extended.underlineColor?50331648&this.extended.underlineColor:this.getFgColorMode()}isUnderlineColorRGB(){return 268435456&this.bg&&~this.extended.underlineColor?50331648==(50331648&this.extended.underlineColor):this.isFgRGB()}isUnderlineColorPalette(){return 268435456&this.bg&&~this.extended.underlineColor?16777216==(50331648&this.extended.underlineColor)||33554432==(50331648&this.extended.underlineColor):this.isFgPalette()}isUnderlineColorDefault(){return 268435456&this.bg&&~this.extended.underlineColor?0==(50331648&this.extended.underlineColor):this.isFgDefault()}getUnderlineStyle(){return 268435456&this.fg?268435456&this.bg?this.extended.underlineStyle:1:0}}t.AttributeData=i;class s{get ext(){return this._urlId?-469762049&this._ext|this.underlineStyle<<26:this._ext}set ext(e){this._ext=e}get underlineStyle(){return this._urlId?5:(469762048&this._ext)>>26}set underlineStyle(e){this._ext&=-469762049,this._ext|=e<<26&469762048}get underlineColor(){return 67108863&this._ext}set underlineColor(e){this._ext&=-67108864,this._ext|=67108863&e}get urlId(){return this._urlId}set urlId(e){this._urlId=e}constructor(e=0,t=0){this._ext=0,this._urlId=0,this._ext=e,this._urlId=t}clone(){return new s(this._ext,this._urlId)}isEmpty(){return 0===this.underlineStyle&&0===this._urlId}}t.ExtendedAttrs=s},9092:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.Buffer=t.MAX_BUFFER_SIZE=void 0;const s=i(6349),r=i(7226),n=i(3734),o=i(8437),a=i(4634),h=i(511),c=i(643),l=i(4863),d=i(7116);t.MAX_BUFFER_SIZE=4294967295,t.Buffer=class{constructor(e,t,i){this._hasScrollback=e,this._optionsService=t,this._bufferService=i,this.ydisp=0,this.ybase=0,this.y=0,this.x=0,this.tabs={},this.savedY=0,this.savedX=0,this.savedCurAttrData=o.DEFAULT_ATTR_DATA.clone(),this.savedCharset=d.DEFAULT_CHARSET,this.markers=[],this._nullCell=h.CellData.fromCharData([0,c.NULL_CELL_CHAR,c.NULL_CELL_WIDTH,c.NULL_CELL_CODE]),this._whitespaceCell=h.CellData.fromCharData([0,c.WHITESPACE_CELL_CHAR,c.WHITESPACE_CELL_WIDTH,c.WHITESPACE_CELL_CODE]),this._isClearing=!1,this._memoryCleanupQueue=new r.IdleTaskQueue,this._memoryCleanupPosition=0,this._cols=this._bufferService.cols,this._rows=this._bufferService.rows,this.lines=new s.CircularList(this._getCorrectBufferLength(this._rows)),this.scrollTop=0,this.scrollBottom=this._rows-1,this.setupTabStops()}getNullCell(e){return e?(this._nullCell.fg=e.fg,this._nullCell.bg=e.bg,this._nullCell.extended=e.extended):(this._nullCell.fg=0,this._nullCell.bg=0,this._nullCell.extended=new n.ExtendedAttrs),this._nullCell}getWhitespaceCell(e){return e?(this._whitespaceCell.fg=e.fg,this._whitespaceCell.bg=e.bg,this._whitespaceCell.extended=e.extended):(this._whitespaceCell.fg=0,this._whitespaceCell.bg=0,this._whitespaceCell.extended=new n.ExtendedAttrs),this._whitespaceCell}getBlankLine(e,t){return new o.BufferLine(this._bufferService.cols,this.getNullCell(e),t)}get hasScrollback(){return this._hasScrollback&&this.lines.maxLength>this._rows}get isCursorInViewport(){const e=this.ybase+this.y-this.ydisp;return e>=0&&e<this._rows}_getCorrectBufferLength(e){if(!this._hasScrollback)return e;const i=e+this._optionsService.rawOptions.scrollback;return i>t.MAX_BUFFER_SIZE?t.MAX_BUFFER_SIZE:i}fillViewportRows(e){if(0===this.lines.length){void 0===e&&(e=o.DEFAULT_ATTR_DATA);let t=this._rows;for(;t--;)this.lines.push(this.getBlankLine(e))}}clear(){this.ydisp=0,this.ybase=0,this.y=0,this.x=0,this.lines=new s.CircularList(this._getCorrectBufferLength(this._rows)),this.scrollTop=0,this.scrollBottom=this._rows-1,this.setupTabStops()}resize(e,t){const i=this.getNullCell(o.DEFAULT_ATTR_DATA);let s=0;const r=this._getCorrectBufferLength(t);if(r>this.lines.maxLength&&(this.lines.maxLength=r),this.lines.length>0){if(this._cols<e)for(let t=0;t<this.lines.length;t++)s+=+this.lines.get(t).resize(e,i);let n=0;if(this._rows<t)for(let s=this._rows;s<t;s++)this.lines.length<t+this.ybase&&(this._optionsService.rawOptions.windowsMode||void 0!==this._optionsService.rawOptions.windowsPty.backend||void 0!==this._optionsService.rawOptions.windowsPty.buildNumber?this.lines.push(new o.BufferLine(e,i)):this.ybase>0&&this.lines.length<=this.ybase+this.y+n+1?(this.ybase--,n++,this.ydisp>0&&this.ydisp--):this.lines.push(new o.BufferLine(e,i)));else for(let e=this._rows;e>t;e--)this.lines.length>t+this.ybase&&(this.lines.length>this.ybase+this.y+1?this.lines.pop():(this.ybase++,this.ydisp++));if(r<this.lines.maxLength){const e=this.lines.length-r;e>0&&(this.lines.trimStart(e),this.ybase=Math.max(this.ybase-e,0),this.ydisp=Math.max(this.ydisp-e,0),this.savedY=Math.max(this.savedY-e,0)),this.lines.maxLength=r}this.x=Math.min(this.x,e-1),this.y=Math.min(this.y,t-1),n&&(this.y+=n),this.savedX=Math.min(this.savedX,e-1),this.scrollTop=0}if(this.scrollBottom=t-1,this._isReflowEnabled&&(this._reflow(e,t),this._cols>e))for(let t=0;t<this.lines.length;t++)s+=+this.lines.get(t).resize(e,i);this._cols=e,this._rows=t,this._memoryCleanupQueue.clear(),s>.1*this.lines.length&&(this._memoryCleanupPosition=0,this._memoryCleanupQueue.enqueue((()=>this._batchedMemoryCleanup())))}_batchedMemoryCleanup(){let e=!0;this._memoryCleanupPosition>=this.lines.length&&(this._memoryCleanupPosition=0,e=!1);let t=0;for(;this._memoryCleanupPosition<this.lines.length;)if(t+=this.lines.get(this._memoryCleanupPosition++).cleanupMemory(),t>100)return!0;return e}get _isReflowEnabled(){const e=this._optionsService.rawOptions.windowsPty;return e&&e.buildNumber?this._hasScrollback&&"conpty"===e.backend&&e.buildNumber>=21376:this._hasScrollback&&!this._optionsService.rawOptions.windowsMode}_reflow(e,t){this._cols!==e&&(e>this._cols?this._reflowLarger(e,t):this._reflowSmaller(e,t))}_reflowLarger(e,t){const i=(0,a.reflowLargerGetLinesToRemove)(this.lines,this._cols,e,this.ybase+this.y,this.getNullCell(o.DEFAULT_ATTR_DATA));if(i.length>0){const s=(0,a.reflowLargerCreateNewLayout)(this.lines,i);(0,a.reflowLargerApplyNewLayout)(this.lines,s.layout),this._reflowLargerAdjustViewport(e,t,s.countRemoved)}}_reflowLargerAdjustViewport(e,t,i){const s=this.getNullCell(o.DEFAULT_ATTR_DATA);let r=i;for(;r-- >0;)0===this.ybase?(this.y>0&&this.y--,this.lines.length<t&&this.lines.push(new o.BufferLine(e,s))):(this.ydisp===this.ybase&&this.ydisp--,this.ybase--);this.savedY=Math.max(this.savedY-i,0)}_reflowSmaller(e,t){const i=this.getNullCell(o.DEFAULT_ATTR_DATA),s=[];let r=0;for(let n=this.lines.length-1;n>=0;n--){let h=this.lines.get(n);if(!h||!h.isWrapped&&h.getTrimmedLength()<=e)continue;const c=[h];for(;h.isWrapped&&n>0;)h=this.lines.get(--n),c.unshift(h);const l=this.ybase+this.y;if(l>=n&&l<n+c.length)continue;const d=c[c.length-1].getTrimmedLength(),_=(0,a.reflowSmallerGetNewLineLengths)(c,this._cols,e),u=_.length-c.length;let f;f=0===this.ybase&&this.y!==this.lines.length-1?Math.max(0,this.y-this.lines.maxLength+u):Math.max(0,this.lines.length-this.lines.maxLength+u);const v=[];for(let e=0;e<u;e++){const e=this.getBlankLine(o.DEFAULT_ATTR_DATA,!0);v.push(e)}v.length>0&&(s.push({start:n+c.length+r,newLines:v}),r+=v.length),c.push(...v);let p=_.length-1,g=_[p];0===g&&(p--,g=_[p]);let m=c.length-u-1,S=d;for(;m>=0;){const e=Math.min(S,g);if(void 0===c[p])break;if(c[p].copyCellsFrom(c[m],S-e,g-e,e,!0),g-=e,0===g&&(p--,g=_[p]),S-=e,0===S){m--;const e=Math.max(m,0);S=(0,a.getWrappedLineTrimmedLength)(c,e,this._cols)}}for(let t=0;t<c.length;t++)_[t]<e&&c[t].setCell(_[t],i);let C=u-f;for(;C-- >0;)0===this.ybase?this.y<t-1?(this.y++,this.lines.pop()):(this.ybase++,this.ydisp++):this.ybase<Math.min(this.lines.maxLength,this.lines.length+r)-t&&(this.ybase===this.ydisp&&this.ydisp++,this.ybase++);this.savedY=Math.min(this.savedY+u,this.ybase+t-1)}if(s.length>0){const e=[],t=[];for(let e=0;e<this.lines.length;e++)t.push(this.lines.get(e));const i=this.lines.length;let n=i-1,o=0,a=s[o];this.lines.length=Math.min(this.lines.maxLength,this.lines.length+r);let h=0;for(let c=Math.min(this.lines.maxLength-1,i+r-1);c>=0;c--)if(a&&a.start>n+h){for(let e=a.newLines.length-1;e>=0;e--)this.lines.set(c--,a.newLines[e]);c++,e.push({index:n+1,amount:a.newLines.length}),h+=a.newLines.length,a=s[++o]}else this.lines.set(c,t[n--]);let c=0;for(let t=e.length-1;t>=0;t--)e[t].index+=c,this.lines.onInsertEmitter.fire(e[t]),c+=e[t].amount;const l=Math.max(0,i+r-this.lines.maxLength);l>0&&this.lines.onTrimEmitter.fire(l)}}translateBufferLineToString(e,t,i=0,s){const r=this.lines.get(e);return r?r.translateToString(t,i,s):""}getWrappedRangeForLine(e){let t=e,i=e;for(;t>0&&this.lines.get(t).isWrapped;)t--;for(;i+1<this.lines.length&&this.lines.get(i+1).isWrapped;)i++;return{first:t,last:i}}setupTabStops(e){for(null!=e?this.tabs[e]||(e=this.prevStop(e)):(this.tabs={},e=0);e<this._cols;e+=this._optionsService.rawOptions.tabStopWidth)this.tabs[e]=!0}prevStop(e){for(null==e&&(e=this.x);!this.tabs[--e]&&e>0;);return e>=this._cols?this._cols-1:e<0?0:e}nextStop(e){for(null==e&&(e=this.x);!this.tabs[++e]&&e<this._cols;);return e>=this._cols?this._cols-1:e<0?0:e}clearMarkers(e){this._isClearing=!0;for(let t=0;t<this.markers.length;t++)this.markers[t].line===e&&(this.markers[t].dispose(),this.markers.splice(t--,1));this._isClearing=!1}clearAllMarkers(){this._isClearing=!0;for(let e=0;e<this.markers.length;e++)this.markers[e].dispose(),this.markers.splice(e--,1);this._isClearing=!1}addMarker(e){const t=new l.Marker(e);return this.markers.push(t),t.register(this.lines.onTrim((e=>{t.line-=e,t.line<0&&t.dispose()}))),t.register(this.lines.onInsert((e=>{t.line>=e.index&&(t.line+=e.amount)}))),t.register(this.lines.onDelete((e=>{t.line>=e.index&&t.line<e.index+e.amount&&t.dispose(),t.line>e.index&&(t.line-=e.amount)}))),t.register(t.onDispose((()=>this._removeMarker(t)))),t}_removeMarker(e){this._isClearing||this.markers.splice(this.markers.indexOf(e),1)}}},8437:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.BufferLine=t.DEFAULT_ATTR_DATA=void 0;const s=i(3734),r=i(511),n=i(643),o=i(482);t.DEFAULT_ATTR_DATA=Object.freeze(new s.AttributeData);let a=0;class h{constructor(e,t,i=!1){this.isWrapped=i,this._combined={},this._extendedAttrs={},this._data=new Uint32Array(3*e);const s=t||r.CellData.fromCharData([0,n.NULL_CELL_CHAR,n.NULL_CELL_WIDTH,n.NULL_CELL_CODE]);for(let t=0;t<e;++t)this.setCell(t,s);this.length=e}get(e){const t=this._data[3*e+0],i=2097151&t;return[this._data[3*e+1],2097152&t?this._combined[e]:i?(0,o.stringFromCodePoint)(i):"",t>>22,2097152&t?this._combined[e].charCodeAt(this._combined[e].length-1):i]}set(e,t){this._data[3*e+1]=t[n.CHAR_DATA_ATTR_INDEX],t[n.CHAR_DATA_CHAR_INDEX].length>1?(this._combined[e]=t[1],this._data[3*e+0]=2097152|e|t[n.CHAR_DATA_WIDTH_INDEX]<<22):this._data[3*e+0]=t[n.CHAR_DATA_CHAR_INDEX].charCodeAt(0)|t[n.CHAR_DATA_WIDTH_INDEX]<<22}getWidth(e){return this._data[3*e+0]>>22}hasWidth(e){return 12582912&this._data[3*e+0]}getFg(e){return this._data[3*e+1]}getBg(e){return this._data[3*e+2]}hasContent(e){return 4194303&this._data[3*e+0]}getCodePoint(e){const t=this._data[3*e+0];return 2097152&t?this._combined[e].charCodeAt(this._combined[e].length-1):2097151&t}isCombined(e){return 2097152&this._data[3*e+0]}getString(e){const t=this._data[3*e+0];return 2097152&t?this._combined[e]:2097151&t?(0,o.stringFromCodePoint)(2097151&t):""}isProtected(e){return 536870912&this._data[3*e+2]}loadCell(e,t){return a=3*e,t.content=this._data[a+0],t.fg=this._data[a+1],t.bg=this._data[a+2],2097152&t.content&&(t.combinedData=this._combined[e]),268435456&t.bg&&(t.extended=this._extendedAttrs[e]),t}setCell(e,t){2097152&t.content&&(this._combined[e]=t.combinedData),268435456&t.bg&&(this._extendedAttrs[e]=t.extended),this._data[3*e+0]=t.content,this._data[3*e+1]=t.fg,this._data[3*e+2]=t.bg}setCellFromCodePoint(e,t,i,s,r,n){268435456&r&&(this._extendedAttrs[e]=n),this._data[3*e+0]=t|i<<22,this._data[3*e+1]=s,this._data[3*e+2]=r}addCodepointToCell(e,t){let i=this._data[3*e+0];2097152&i?this._combined[e]+=(0,o.stringFromCodePoint)(t):(2097151&i?(this._combined[e]=(0,o.stringFromCodePoint)(2097151&i)+(0,o.stringFromCodePoint)(t),i&=-2097152,i|=2097152):i=t|1<<22,this._data[3*e+0]=i)}insertCells(e,t,i,n){if((e%=this.length)&&2===this.getWidth(e-1)&&this.setCellFromCodePoint(e-1,0,1,(null==n?void 0:n.fg)||0,(null==n?void 0:n.bg)||0,(null==n?void 0:n.extended)||new s.ExtendedAttrs),t<this.length-e){const s=new r.CellData;for(let i=this.length-e-t-1;i>=0;--i)this.setCell(e+t+i,this.loadCell(e+i,s));for(let s=0;s<t;++s)this.setCell(e+s,i)}else for(let t=e;t<this.length;++t)this.setCell(t,i);2===this.getWidth(this.length-1)&&this.setCellFromCodePoint(this.length-1,0,1,(null==n?void 0:n.fg)||0,(null==n?void 0:n.bg)||0,(null==n?void 0:n.extended)||new s.ExtendedAttrs)}deleteCells(e,t,i,n){if(e%=this.length,t<this.length-e){const s=new r.CellData;for(let i=0;i<this.length-e-t;++i)this.setCell(e+i,this.loadCell(e+t+i,s));for(let e=this.length-t;e<this.length;++e)this.setCell(e,i)}else for(let t=e;t<this.length;++t)this.setCell(t,i);e&&2===this.getWidth(e-1)&&this.setCellFromCodePoint(e-1,0,1,(null==n?void 0:n.fg)||0,(null==n?void 0:n.bg)||0,(null==n?void 0:n.extended)||new s.ExtendedAttrs),0!==this.getWidth(e)||this.hasContent(e)||this.setCellFromCodePoint(e,0,1,(null==n?void 0:n.fg)||0,(null==n?void 0:n.bg)||0,(null==n?void 0:n.extended)||new s.ExtendedAttrs)}replaceCells(e,t,i,r,n=!1){if(n)for(e&&2===this.getWidth(e-1)&&!this.isProtected(e-1)&&this.setCellFromCodePoint(e-1,0,1,(null==r?void 0:r.fg)||0,(null==r?void 0:r.bg)||0,(null==r?void 0:r.extended)||new s.ExtendedAttrs),t<this.length&&2===this.getWidth(t-1)&&!this.isProtected(t)&&this.setCellFromCodePoint(t,0,1,(null==r?void 0:r.fg)||0,(null==r?void 0:r.bg)||0,(null==r?void 0:r.extended)||new s.ExtendedAttrs);e<t&&e<this.length;)this.isProtected(e)||this.setCell(e,i),e++;else for(e&&2===this.getWidth(e-1)&&this.setCellFromCodePoint(e-1,0,1,(null==r?void 0:r.fg)||0,(null==r?void 0:r.bg)||0,(null==r?void 0:r.extended)||new s.ExtendedAttrs),t<this.length&&2===this.getWidth(t-1)&&this.setCellFromCodePoint(t,0,1,(null==r?void 0:r.fg)||0,(null==r?void 0:r.bg)||0,(null==r?void 0:r.extended)||new s.ExtendedAttrs);e<t&&e<this.length;)this.setCell(e++,i)}resize(e,t){if(e===this.length)return 4*this._data.length*2<this._data.buffer.byteLength;const i=3*e;if(e>this.length){if(this._data.buffer.byteLength>=4*i)this._data=new Uint32Array(this._data.buffer,0,i);else{const e=new Uint32Array(i);e.set(this._data),this._data=e}for(let i=this.length;i<e;++i)this.setCell(i,t)}else{this._data=this._data.subarray(0,i);const t=Object.keys(this._combined);for(let i=0;i<t.length;i++){const s=parseInt(t[i],10);s>=e&&delete this._combined[s]}const s=Object.keys(this._extendedAttrs);for(let t=0;t<s.length;t++){const i=parseInt(s[t],10);i>=e&&delete this._extendedAttrs[i]}}return this.length=e,4*i*2<this._data.buffer.byteLength}cleanupMemory(){if(4*this._data.length*2<this._data.buffer.byteLength){const e=new Uint32Array(this._data.length);return e.set(this._data),this._data=e,1}return 0}fill(e,t=!1){if(t)for(let t=0;t<this.length;++t)this.isProtected(t)||this.setCell(t,e);else{this._combined={},this._extendedAttrs={};for(let t=0;t<this.length;++t)this.setCell(t,e)}}copyFrom(e){this.length!==e.length?this._data=new Uint32Array(e._data):this._data.set(e._data),this.length=e.length,this._combined={};for(const t in e._combined)this._combined[t]=e._combined[t];this._extendedAttrs={};for(const t in e._extendedAttrs)this._extendedAttrs[t]=e._extendedAttrs[t];this.isWrapped=e.isWrapped}clone(){const e=new h(0);e._data=new Uint32Array(this._data),e.length=this.length;for(const t in this._combined)e._combined[t]=this._combined[t];for(const t in this._extendedAttrs)e._extendedAttrs[t]=this._extendedAttrs[t];return e.isWrapped=this.isWrapped,e}getTrimmedLength(){for(let e=this.length-1;e>=0;--e)if(4194303&this._data[3*e+0])return e+(this._data[3*e+0]>>22);return 0}getNoBgTrimmedLength(){for(let e=this.length-1;e>=0;--e)if(4194303&this._data[3*e+0]||50331648&this._data[3*e+2])return e+(this._data[3*e+0]>>22);return 0}copyCellsFrom(e,t,i,s,r){const n=e._data;if(r)for(let r=s-1;r>=0;r--){for(let e=0;e<3;e++)this._data[3*(i+r)+e]=n[3*(t+r)+e];268435456&n[3*(t+r)+2]&&(this._extendedAttrs[i+r]=e._extendedAttrs[t+r])}else for(let r=0;r<s;r++){for(let e=0;e<3;e++)this._data[3*(i+r)+e]=n[3*(t+r)+e];268435456&n[3*(t+r)+2]&&(this._extendedAttrs[i+r]=e._extendedAttrs[t+r])}const o=Object.keys(e._combined);for(let s=0;s<o.length;s++){const r=parseInt(o[s],10);r>=t&&(this._combined[r-t+i]=e._combined[r])}}translateToString(e=!1,t=0,i=this.length){e&&(i=Math.min(i,this.getTrimmedLength()));let s="";for(;t<i;){const e=this._data[3*t+0],i=2097151&e;s+=2097152&e?this._combined[t]:i?(0,o.stringFromCodePoint)(i):n.WHITESPACE_CELL_CHAR,t+=e>>22||1}return s}}t.BufferLine=h},4841:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.getRangeLength=void 0,t.getRangeLength=function(e,t){if(e.start.y>e.end.y)throw new Error(`Buffer range end (${e.end.x}, ${e.end.y}) cannot be before start (${e.start.x}, ${e.start.y})`);return t*(e.end.y-e.start.y)+(e.end.x-e.start.x+1)}},4634:(e,t)=>{function i(e,t,i){if(t===e.length-1)return e[t].getTrimmedLength();const s=!e[t].hasContent(i-1)&&1===e[t].getWidth(i-1),r=2===e[t+1].getWidth(0);return s&&r?i-1:i}Object.defineProperty(t,"__esModule",{value:!0}),t.getWrappedLineTrimmedLength=t.reflowSmallerGetNewLineLengths=t.reflowLargerApplyNewLayout=t.reflowLargerCreateNewLayout=t.reflowLargerGetLinesToRemove=void 0,t.reflowLargerGetLinesToRemove=function(e,t,s,r,n){const o=[];for(let a=0;a<e.length-1;a++){let h=a,c=e.get(++h);if(!c.isWrapped)continue;const l=[e.get(a)];for(;h<e.length&&c.isWrapped;)l.push(c),c=e.get(++h);if(r>=a&&r<h){a+=l.length-1;continue}let d=0,_=i(l,d,t),u=1,f=0;for(;u<l.length;){const e=i(l,u,t),r=e-f,o=s-_,a=Math.min(r,o);l[d].copyCellsFrom(l[u],f,_,a,!1),_+=a,_===s&&(d++,_=0),f+=a,f===e&&(u++,f=0),0===_&&0!==d&&2===l[d-1].getWidth(s-1)&&(l[d].copyCellsFrom(l[d-1],s-1,_++,1,!1),l[d-1].setCell(s-1,n))}l[d].replaceCells(_,s,n);let v=0;for(let e=l.length-1;e>0&&(e>d||0===l[e].getTrimmedLength());e--)v++;v>0&&(o.push(a+l.length-v),o.push(v)),a+=l.length-1}return o},t.reflowLargerCreateNewLayout=function(e,t){const i=[];let s=0,r=t[s],n=0;for(let o=0;o<e.length;o++)if(r===o){const i=t[++s];e.onDeleteEmitter.fire({index:o-n,amount:i}),o+=i-1,n+=i,r=t[++s]}else i.push(o);return{layout:i,countRemoved:n}},t.reflowLargerApplyNewLayout=function(e,t){const i=[];for(let s=0;s<t.length;s++)i.push(e.get(t[s]));for(let t=0;t<i.length;t++)e.set(t,i[t]);e.length=t.length},t.reflowSmallerGetNewLineLengths=function(e,t,s){const r=[],n=e.map(((s,r)=>i(e,r,t))).reduce(((e,t)=>e+t));let o=0,a=0,h=0;for(;h<n;){if(n-h<s){r.push(n-h);break}o+=s;const c=i(e,a,t);o>c&&(o-=c,a++);const l=2===e[a].getWidth(o-1);l&&o--;const d=l?s-1:s;r.push(d),h+=d}return r},t.getWrappedLineTrimmedLength=i},5295:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.BufferSet=void 0;const s=i(8460),r=i(844),n=i(9092);class o extends r.Disposable{constructor(e,t){super(),this._optionsService=e,this._bufferService=t,this._onBufferActivate=this.register(new s.EventEmitter),this.onBufferActivate=this._onBufferActivate.event,this.reset(),this.register(this._optionsService.onSpecificOptionChange("scrollback",(()=>this.resize(this._bufferService.cols,this._bufferService.rows)))),this.register(this._optionsService.onSpecificOptionChange("tabStopWidth",(()=>this.setupTabStops())))}reset(){this._normal=new n.Buffer(!0,this._optionsService,this._bufferService),this._normal.fillViewportRows(),this._alt=new n.Buffer(!1,this._optionsService,this._bufferService),this._activeBuffer=this._normal,this._onBufferActivate.fire({activeBuffer:this._normal,inactiveBuffer:this._alt}),this.setupTabStops()}get alt(){return this._alt}get active(){return this._activeBuffer}get normal(){return this._normal}activateNormalBuffer(){this._activeBuffer!==this._normal&&(this._normal.x=this._alt.x,this._normal.y=this._alt.y,this._alt.clearAllMarkers(),this._alt.clear(),this._activeBuffer=this._normal,this._onBufferActivate.fire({activeBuffer:this._normal,inactiveBuffer:this._alt}))}activateAltBuffer(e){this._activeBuffer!==this._alt&&(this._alt.fillViewportRows(e),this._alt.x=this._normal.x,this._alt.y=this._normal.y,this._activeBuffer=this._alt,this._onBufferActivate.fire({activeBuffer:this._alt,inactiveBuffer:this._normal}))}resize(e,t){this._normal.resize(e,t),this._alt.resize(e,t),this.setupTabStops(e)}setupTabStops(e){this._normal.setupTabStops(e),this._alt.setupTabStops(e)}}t.BufferSet=o},511:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.CellData=void 0;const s=i(482),r=i(643),n=i(3734);class o extends n.AttributeData{constructor(){super(...arguments),this.content=0,this.fg=0,this.bg=0,this.extended=new n.ExtendedAttrs,this.combinedData=""}static fromCharData(e){const t=new o;return t.setFromCharData(e),t}isCombined(){return 2097152&this.content}getWidth(){return this.content>>22}getChars(){return 2097152&this.content?this.combinedData:2097151&this.content?(0,s.stringFromCodePoint)(2097151&this.content):""}getCode(){return this.isCombined()?this.combinedData.charCodeAt(this.combinedData.length-1):2097151&this.content}setFromCharData(e){this.fg=e[r.CHAR_DATA_ATTR_INDEX],this.bg=0;let t=!1;if(e[r.CHAR_DATA_CHAR_INDEX].length>2)t=!0;else if(2===e[r.CHAR_DATA_CHAR_INDEX].length){const i=e[r.CHAR_DATA_CHAR_INDEX].charCodeAt(0);if(55296<=i&&i<=56319){const s=e[r.CHAR_DATA_CHAR_INDEX].charCodeAt(1);56320<=s&&s<=57343?this.content=1024*(i-55296)+s-56320+65536|e[r.CHAR_DATA_WIDTH_INDEX]<<22:t=!0}else t=!0}else this.content=e[r.CHAR_DATA_CHAR_INDEX].charCodeAt(0)|e[r.CHAR_DATA_WIDTH_INDEX]<<22;t&&(this.combinedData=e[r.CHAR_DATA_CHAR_INDEX],this.content=2097152|e[r.CHAR_DATA_WIDTH_INDEX]<<22)}getAsCharData(){return[this.fg,this.getChars(),this.getWidth(),this.getCode()]}}t.CellData=o},643:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.WHITESPACE_CELL_CODE=t.WHITESPACE_CELL_WIDTH=t.WHITESPACE_CELL_CHAR=t.NULL_CELL_CODE=t.NULL_CELL_WIDTH=t.NULL_CELL_CHAR=t.CHAR_DATA_CODE_INDEX=t.CHAR_DATA_WIDTH_INDEX=t.CHAR_DATA_CHAR_INDEX=t.CHAR_DATA_ATTR_INDEX=t.DEFAULT_EXT=t.DEFAULT_ATTR=t.DEFAULT_COLOR=void 0,t.DEFAULT_COLOR=0,t.DEFAULT_ATTR=256|t.DEFAULT_COLOR<<9,t.DEFAULT_EXT=0,t.CHAR_DATA_ATTR_INDEX=0,t.CHAR_DATA_CHAR_INDEX=1,t.CHAR_DATA_WIDTH_INDEX=2,t.CHAR_DATA_CODE_INDEX=3,t.NULL_CELL_CHAR="",t.NULL_CELL_WIDTH=1,t.NULL_CELL_CODE=0,t.WHITESPACE_CELL_CHAR=" ",t.WHITESPACE_CELL_WIDTH=1,t.WHITESPACE_CELL_CODE=32},4863:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.Marker=void 0;const s=i(8460),r=i(844);class n{get id(){return this._id}constructor(e){this.line=e,this.isDisposed=!1,this._disposables=[],this._id=n._nextId++,this._onDispose=this.register(new s.EventEmitter),this.onDispose=this._onDispose.event}dispose(){this.isDisposed||(this.isDisposed=!0,this.line=-1,this._onDispose.fire(),(0,r.disposeArray)(this._disposables),this._disposables.length=0)}register(e){return this._disposables.push(e),e}}t.Marker=n,n._nextId=1},7116:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.DEFAULT_CHARSET=t.CHARSETS=void 0,t.CHARSETS={},t.DEFAULT_CHARSET=t.CHARSETS.B,t.CHARSETS[0]={"`":"◆",a:"▒",b:"␉",c:"␌",d:"␍",e:"␊",f:"°",g:"±",h:"␤",i:"␋",j:"┘",k:"┐",l:"┌",m:"└",n:"┼",o:"⎺",p:"⎻",q:"─",r:"⎼",s:"⎽",t:"├",u:"┤",v:"┴",w:"┬",x:"│",y:"≤",z:"≥","{":"π","|":"≠","}":"£","~":"·"},t.CHARSETS.A={"#":"£"},t.CHARSETS.B=void 0,t.CHARSETS[4]={"#":"£","@":"¾","[":"ij","\\":"½","]":"|","{":"¨","|":"f","}":"¼","~":"´"},t.CHARSETS.C=t.CHARSETS[5]={"[":"Ä","\\":"Ö","]":"Å","^":"Ü","`":"é","{":"ä","|":"ö","}":"å","~":"ü"},t.CHARSETS.R={"#":"£","@":"à","[":"°","\\":"ç","]":"§","{":"é","|":"ù","}":"è","~":"¨"},t.CHARSETS.Q={"@":"à","[":"â","\\":"ç","]":"ê","^":"î","`":"ô","{":"é","|":"ù","}":"è","~":"û"},t.CHARSETS.K={"@":"§","[":"Ä","\\":"Ö","]":"Ü","{":"ä","|":"ö","}":"ü","~":"ß"},t.CHARSETS.Y={"#":"£","@":"§","[":"°","\\":"ç","]":"é","`":"ù","{":"à","|":"ò","}":"è","~":"ì"},t.CHARSETS.E=t.CHARSETS[6]={"@":"Ä","[":"Æ","\\":"Ø","]":"Å","^":"Ü","`":"ä","{":"æ","|":"ø","}":"å","~":"ü"},t.CHARSETS.Z={"#":"£","@":"§","[":"¡","\\":"Ñ","]":"¿","{":"°","|":"ñ","}":"ç"},t.CHARSETS.H=t.CHARSETS[7]={"@":"É","[":"Ä","\\":"Ö","]":"Å","^":"Ü","`":"é","{":"ä","|":"ö","}":"å","~":"ü"},t.CHARSETS["="]={"#":"ù","@":"à","[":"é","\\":"ç","]":"ê","^":"î",_:"è","`":"ô","{":"ä","|":"ö","}":"ü","~":"û"}},2584:(e,t)=>{var i,s,r;Object.defineProperty(t,"__esModule",{value:!0}),t.C1_ESCAPED=t.C1=t.C0=void 0,function(e){e.NUL="\0",e.SOH="",e.STX="",e.ETX="",e.EOT="",e.ENQ="",e.ACK="",e.BEL="",e.BS="\b",e.HT="\t",e.LF="\n",e.VT="\v",e.FF="\f",e.CR="\r",e.SO="",e.SI="",e.DLE="",e.DC1="",e.DC2="",e.DC3="",e.DC4="",e.NAK="",e.SYN="",e.ETB="",e.CAN="",e.EM="",e.SUB="",e.ESC="",e.FS="",e.GS="",e.RS="",e.US="",e.SP=" ",e.DEL=""}(i||(t.C0=i={})),function(e){e.PAD="",e.HOP="",e.BPH="",e.NBH="",e.IND="",e.NEL="",e.SSA="",e.ESA="",e.HTS="",e.HTJ="",e.VTS="",e.PLD="",e.PLU="",e.RI="",e.SS2="",e.SS3="",e.DCS="",e.PU1="",e.PU2="",e.STS="",e.CCH="",e.MW="",e.SPA="",e.EPA="",e.SOS="",e.SGCI="",e.SCI="",e.CSI="",e.ST="",e.OSC="",e.PM="",e.APC=""}(s||(t.C1=s={})),function(e){e.ST=`${i.ESC}\\`}(r||(t.C1_ESCAPED=r={}))},7399:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.evaluateKeyboardEvent=void 0;const s=i(2584),r={48:["0",")"],49:["1","!"],50:["2","@"],51:["3","#"],52:["4","$"],53:["5","%"],54:["6","^"],55:["7","&"],56:["8","*"],57:["9","("],186:[";",":"],187:["=","+"],188:[",","<"],189:["-","_"],190:[".",">"],191:["/","?"],192:["`","~"],219:["[","{"],220:["\\","|"],221:["]","}"],222:["'",'"']};t.evaluateKeyboardEvent=function(e,t,i,n){const o={type:0,cancel:!1,key:void 0},a=(e.shiftKey?1:0)|(e.altKey?2:0)|(e.ctrlKey?4:0)|(e.metaKey?8:0);switch(e.keyCode){case 0:"UIKeyInputUpArrow"===e.key?o.key=t?s.C0.ESC+"OA":s.C0.ESC+"[A":"UIKeyInputLeftArrow"===e.key?o.key=t?s.C0.ESC+"OD":s.C0.ESC+"[D":"UIKeyInputRightArrow"===e.key?o.key=t?s.C0.ESC+"OC":s.C0.ESC+"[C":"UIKeyInputDownArrow"===e.key&&(o.key=t?s.C0.ESC+"OB":s.C0.ESC+"[B");break;case 8:if(e.altKey){o.key=s.C0.ESC+s.C0.DEL;break}o.key=s.C0.DEL;break;case 9:if(e.shiftKey){o.key=s.C0.ESC+"[Z";break}o.key=s.C0.HT,o.cancel=!0;break;case 13:o.key=e.altKey?s.C0.ESC+s.C0.CR:s.C0.CR,o.cancel=!0;break;case 27:o.key=s.C0.ESC,e.altKey&&(o.key=s.C0.ESC+s.C0.ESC),o.cancel=!0;break;case 37:if(e.metaKey)break;a?(o.key=s.C0.ESC+"[1;"+(a+1)+"D",o.key===s.C0.ESC+"[1;3D"&&(o.key=s.C0.ESC+(i?"b":"[1;5D"))):o.key=t?s.C0.ESC+"OD":s.C0.ESC+"[D";break;case 39:if(e.metaKey)break;a?(o.key=s.C0.ESC+"[1;"+(a+1)+"C",o.key===s.C0.ESC+"[1;3C"&&(o.key=s.C0.ESC+(i?"f":"[1;5C"))):o.key=t?s.C0.ESC+"OC":s.C0.ESC+"[C";break;case 38:if(e.metaKey)break;a?(o.key=s.C0.ESC+"[1;"+(a+1)+"A",i||o.key!==s.C0.ESC+"[1;3A"||(o.key=s.C0.ESC+"[1;5A")):o.key=t?s.C0.ESC+"OA":s.C0.ESC+"[A";break;case 40:if(e.metaKey)break;a?(o.key=s.C0.ESC+"[1;"+(a+1)+"B",i||o.key!==s.C0.ESC+"[1;3B"||(o.key=s.C0.ESC+"[1;5B")):o.key=t?s.C0.ESC+"OB":s.C0.ESC+"[B";break;case 45:e.shiftKey||e.ctrlKey||(o.key=s.C0.ESC+"[2~");break;case 46:o.key=a?s.C0.ESC+"[3;"+(a+1)+"~":s.C0.ESC+"[3~";break;case 36:o.key=a?s.C0.ESC+"[1;"+(a+1)+"H":t?s.C0.ESC+"OH":s.C0.ESC+"[H";break;case 35:o.key=a?s.C0.ESC+"[1;"+(a+1)+"F":t?s.C0.ESC+"OF":s.C0.ESC+"[F";break;case 33:e.shiftKey?o.type=2:e.ctrlKey?o.key=s.C0.ESC+"[5;"+(a+1)+"~":o.key=s.C0.ESC+"[5~";break;case 34:e.shiftKey?o.type=3:e.ctrlKey?o.key=s.C0.ESC+"[6;"+(a+1)+"~":o.key=s.C0.ESC+"[6~";break;case 112:o.key=a?s.C0.ESC+"[1;"+(a+1)+"P":s.C0.ESC+"OP";break;case 113:o.key=a?s.C0.ESC+"[1;"+(a+1)+"Q":s.C0.ESC+"OQ";break;case 114:o.key=a?s.C0.ESC+"[1;"+(a+1)+"R":s.C0.ESC+"OR";break;case 115:o.key=a?s.C0.ESC+"[1;"+(a+1)+"S":s.C0.ESC+"OS";break;case 116:o.key=a?s.C0.ESC+"[15;"+(a+1)+"~":s.C0.ESC+"[15~";break;case 117:o.key=a?s.C0.ESC+"[17;"+(a+1)+"~":s.C0.ESC+"[17~";break;case 118:o.key=a?s.C0.ESC+"[18;"+(a+1)+"~":s.C0.ESC+"[18~";break;case 119:o.key=a?s.C0.ESC+"[19;"+(a+1)+"~":s.C0.ESC+"[19~";break;case 120:o.key=a?s.C0.ESC+"[20;"+(a+1)+"~":s.C0.ESC+"[20~";break;case 121:o.key=a?s.C0.ESC+"[21;"+(a+1)+"~":s.C0.ESC+"[21~";break;case 122:o.key=a?s.C0.ESC+"[23;"+(a+1)+"~":s.C0.ESC+"[23~";break;case 123:o.key=a?s.C0.ESC+"[24;"+(a+1)+"~":s.C0.ESC+"[24~";break;default:if(!e.ctrlKey||e.shiftKey||e.altKey||e.metaKey)if(i&&!n||!e.altKey||e.metaKey)!i||e.altKey||e.ctrlKey||e.shiftKey||!e.metaKey?e.key&&!e.ctrlKey&&!e.altKey&&!e.metaKey&&e.keyCode>=48&&1===e.key.length?o.key=e.key:e.key&&e.ctrlKey&&("_"===e.key&&(o.key=s.C0.US),"@"===e.key&&(o.key=s.C0.NUL)):65===e.keyCode&&(o.type=1);else{const t=r[e.keyCode],i=null==t?void 0:t[e.shiftKey?1:0];if(i)o.key=s.C0.ESC+i;else if(e.keyCode>=65&&e.keyCode<=90){const t=e.ctrlKey?e.keyCode-64:e.keyCode+32;let i=String.fromCharCode(t);e.shiftKey&&(i=i.toUpperCase()),o.key=s.C0.ESC+i}else if(32===e.keyCode)o.key=s.C0.ESC+(e.ctrlKey?s.C0.NUL:" ");else if("Dead"===e.key&&e.code.startsWith("Key")){let t=e.code.slice(3,4);e.shiftKey||(t=t.toLowerCase()),o.key=s.C0.ESC+t,o.cancel=!0}}else e.keyCode>=65&&e.keyCode<=90?o.key=String.fromCharCode(e.keyCode-64):32===e.keyCode?o.key=s.C0.NUL:e.keyCode>=51&&e.keyCode<=55?o.key=String.fromCharCode(e.keyCode-51+27):56===e.keyCode?o.key=s.C0.DEL:219===e.keyCode?o.key=s.C0.ESC:220===e.keyCode?o.key=s.C0.FS:221===e.keyCode&&(o.key=s.C0.GS)}return o}},482:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.Utf8ToUtf32=t.StringToUtf32=t.utf32ToString=t.stringFromCodePoint=void 0,t.stringFromCodePoint=function(e){return e>65535?(e-=65536,String.fromCharCode(55296+(e>>10))+String.fromCharCode(e%1024+56320)):String.fromCharCode(e)},t.utf32ToString=function(e,t=0,i=e.length){let s="";for(let r=t;r<i;++r){let t=e[r];t>65535?(t-=65536,s+=String.fromCharCode(55296+(t>>10))+String.fromCharCode(t%1024+56320)):s+=String.fromCharCode(t)}return s},t.StringToUtf32=class{constructor(){this._interim=0}clear(){this._interim=0}decode(e,t){const i=e.length;if(!i)return 0;let s=0,r=0;if(this._interim){const i=e.charCodeAt(r++);56320<=i&&i<=57343?t[s++]=1024*(this._interim-55296)+i-56320+65536:(t[s++]=this._interim,t[s++]=i),this._interim=0}for(let n=r;n<i;++n){const r=e.charCodeAt(n);if(55296<=r&&r<=56319){if(++n>=i)return this._interim=r,s;const o=e.charCodeAt(n);56320<=o&&o<=57343?t[s++]=1024*(r-55296)+o-56320+65536:(t[s++]=r,t[s++]=o)}else 65279!==r&&(t[s++]=r)}return s}},t.Utf8ToUtf32=class{constructor(){this.interim=new Uint8Array(3)}clear(){this.interim.fill(0)}decode(e,t){const i=e.length;if(!i)return 0;let s,r,n,o,a=0,h=0,c=0;if(this.interim[0]){let s=!1,r=this.interim[0];r&=192==(224&r)?31:224==(240&r)?15:7;let n,o=0;for(;(n=63&this.interim[++o])&&o<4;)r<<=6,r|=n;const h=192==(224&this.interim[0])?2:224==(240&this.interim[0])?3:4,l=h-o;for(;c<l;){if(c>=i)return 0;if(n=e[c++],128!=(192&n)){c--,s=!0;break}this.interim[o++]=n,r<<=6,r|=63&n}s||(2===h?r<128?c--:t[a++]=r:3===h?r<2048||r>=55296&&r<=57343||65279===r||(t[a++]=r):r<65536||r>1114111||(t[a++]=r)),this.interim.fill(0)}const l=i-4;let d=c;for(;d<i;){for(;!(!(d<l)||128&(s=e[d])||128&(r=e[d+1])||128&(n=e[d+2])||128&(o=e[d+3]));)t[a++]=s,t[a++]=r,t[a++]=n,t[a++]=o,d+=4;if(s=e[d++],s<128)t[a++]=s;else if(192==(224&s)){if(d>=i)return this.interim[0]=s,a;if(r=e[d++],128!=(192&r)){d--;continue}if(h=(31&s)<<6|63&r,h<128){d--;continue}t[a++]=h}else if(224==(240&s)){if(d>=i)return this.interim[0]=s,a;if(r=e[d++],128!=(192&r)){d--;continue}if(d>=i)return this.interim[0]=s,this.interim[1]=r,a;if(n=e[d++],128!=(192&n)){d--;continue}if(h=(15&s)<<12|(63&r)<<6|63&n,h<2048||h>=55296&&h<=57343||65279===h)continue;t[a++]=h}else if(240==(248&s)){if(d>=i)return this.interim[0]=s,a;if(r=e[d++],128!=(192&r)){d--;continue}if(d>=i)return this.interim[0]=s,this.interim[1]=r,a;if(n=e[d++],128!=(192&n)){d--;continue}if(d>=i)return this.interim[0]=s,this.interim[1]=r,this.interim[2]=n,a;if(o=e[d++],128!=(192&o)){d--;continue}if(h=(7&s)<<18|(63&r)<<12|(63&n)<<6|63&o,h<65536||h>1114111)continue;t[a++]=h}}return a}}},225:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.UnicodeV6=void 0;const i=[[768,879],[1155,1158],[1160,1161],[1425,1469],[1471,1471],[1473,1474],[1476,1477],[1479,1479],[1536,1539],[1552,1557],[1611,1630],[1648,1648],[1750,1764],[1767,1768],[1770,1773],[1807,1807],[1809,1809],[1840,1866],[1958,1968],[2027,2035],[2305,2306],[2364,2364],[2369,2376],[2381,2381],[2385,2388],[2402,2403],[2433,2433],[2492,2492],[2497,2500],[2509,2509],[2530,2531],[2561,2562],[2620,2620],[2625,2626],[2631,2632],[2635,2637],[2672,2673],[2689,2690],[2748,2748],[2753,2757],[2759,2760],[2765,2765],[2786,2787],[2817,2817],[2876,2876],[2879,2879],[2881,2883],[2893,2893],[2902,2902],[2946,2946],[3008,3008],[3021,3021],[3134,3136],[3142,3144],[3146,3149],[3157,3158],[3260,3260],[3263,3263],[3270,3270],[3276,3277],[3298,3299],[3393,3395],[3405,3405],[3530,3530],[3538,3540],[3542,3542],[3633,3633],[3636,3642],[3655,3662],[3761,3761],[3764,3769],[3771,3772],[3784,3789],[3864,3865],[3893,3893],[3895,3895],[3897,3897],[3953,3966],[3968,3972],[3974,3975],[3984,3991],[3993,4028],[4038,4038],[4141,4144],[4146,4146],[4150,4151],[4153,4153],[4184,4185],[4448,4607],[4959,4959],[5906,5908],[5938,5940],[5970,5971],[6002,6003],[6068,6069],[6071,6077],[6086,6086],[6089,6099],[6109,6109],[6155,6157],[6313,6313],[6432,6434],[6439,6440],[6450,6450],[6457,6459],[6679,6680],[6912,6915],[6964,6964],[6966,6970],[6972,6972],[6978,6978],[7019,7027],[7616,7626],[7678,7679],[8203,8207],[8234,8238],[8288,8291],[8298,8303],[8400,8431],[12330,12335],[12441,12442],[43014,43014],[43019,43019],[43045,43046],[64286,64286],[65024,65039],[65056,65059],[65279,65279],[65529,65531]],s=[[68097,68099],[68101,68102],[68108,68111],[68152,68154],[68159,68159],[119143,119145],[119155,119170],[119173,119179],[119210,119213],[119362,119364],[917505,917505],[917536,917631],[917760,917999]];let r;t.UnicodeV6=class{constructor(){if(this.version="6",!r){r=new Uint8Array(65536),r.fill(1),r[0]=0,r.fill(0,1,32),r.fill(0,127,160),r.fill(2,4352,4448),r[9001]=2,r[9002]=2,r.fill(2,11904,42192),r[12351]=1,r.fill(2,44032,55204),r.fill(2,63744,64256),r.fill(2,65040,65050),r.fill(2,65072,65136),r.fill(2,65280,65377),r.fill(2,65504,65511);for(let e=0;e<i.length;++e)r.fill(0,i[e][0],i[e][1]+1)}}wcwidth(e){return e<32?0:e<127?1:e<65536?r[e]:function(e,t){let i,s=0,r=t.length-1;if(e<t[0][0]||e>t[r][1])return!1;for(;r>=s;)if(i=s+r>>1,e>t[i][1])s=i+1;else{if(!(e<t[i][0]))return!0;r=i-1}return!1}(e,s)?0:e>=131072&&e<=196605||e>=196608&&e<=262141?2:1}}},5981:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.WriteBuffer=void 0;const s=i(8460),r=i(844);class n extends r.Disposable{constructor(e){super(),this._action=e,this._writeBuffer=[],this._callbacks=[],this._pendingData=0,this._bufferOffset=0,this._isSyncWriting=!1,this._syncCalls=0,this._didUserInput=!1,this._onWriteParsed=this.register(new s.EventEmitter),this.onWriteParsed=this._onWriteParsed.event}handleUserInput(){this._didUserInput=!0}writeSync(e,t){if(void 0!==t&&this._syncCalls>t)return void(this._syncCalls=0);if(this._pendingData+=e.length,this._writeBuffer.push(e),this._callbacks.push(void 0),this._syncCalls++,this._isSyncWriting)return;let i;for(this._isSyncWriting=!0;i=this._writeBuffer.shift();){this._action(i);const e=this._callbacks.shift();e&&e()}this._pendingData=0,this._bufferOffset=2147483647,this._isSyncWriting=!1,this._syncCalls=0}write(e,t){if(this._pendingData>5e7)throw new Error("write data discarded, use flow control to avoid losing data");if(!this._writeBuffer.length){if(this._bufferOffset=0,this._didUserInput)return this._didUserInput=!1,this._pendingData+=e.length,this._writeBuffer.push(e),this._callbacks.push(t),void this._innerWrite();setTimeout((()=>this._innerWrite()))}this._pendingData+=e.length,this._writeBuffer.push(e),this._callbacks.push(t)}_innerWrite(e=0,t=!0){const i=e||Date.now();for(;this._writeBuffer.length>this._bufferOffset;){const e=this._writeBuffer[this._bufferOffset],s=this._action(e,t);if(s){const e=e=>Date.now()-i>=12?setTimeout((()=>this._innerWrite(0,e))):this._innerWrite(i,e);return void s.catch((e=>(queueMicrotask((()=>{throw e})),Promise.resolve(!1)))).then(e)}const r=this._callbacks[this._bufferOffset];if(r&&r(),this._bufferOffset++,this._pendingData-=e.length,Date.now()-i>=12)break}this._writeBuffer.length>this._bufferOffset?(this._bufferOffset>50&&(this._writeBuffer=this._writeBuffer.slice(this._bufferOffset),this._callbacks=this._callbacks.slice(this._bufferOffset),this._bufferOffset=0),setTimeout((()=>this._innerWrite()))):(this._writeBuffer.length=0,this._callbacks.length=0,this._pendingData=0,this._bufferOffset=0),this._onWriteParsed.fire()}}t.WriteBuffer=n},5941:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.toRgbString=t.parseColor=void 0;const i=/^([\da-f])\/([\da-f])\/([\da-f])$|^([\da-f]{2})\/([\da-f]{2})\/([\da-f]{2})$|^([\da-f]{3})\/([\da-f]{3})\/([\da-f]{3})$|^([\da-f]{4})\/([\da-f]{4})\/([\da-f]{4})$/,s=/^[\da-f]+$/;function r(e,t){const i=e.toString(16),s=i.length<2?"0"+i:i;switch(t){case 4:return i[0];case 8:return s;case 12:return(s+s).slice(0,3);default:return s+s}}t.parseColor=function(e){if(!e)return;let t=e.toLowerCase();if(0===t.indexOf("rgb:")){t=t.slice(4);const e=i.exec(t);if(e){const t=e[1]?15:e[4]?255:e[7]?4095:65535;return[Math.round(parseInt(e[1]||e[4]||e[7]||e[10],16)/t*255),Math.round(parseInt(e[2]||e[5]||e[8]||e[11],16)/t*255),Math.round(parseInt(e[3]||e[6]||e[9]||e[12],16)/t*255)]}}else if(0===t.indexOf("#")&&(t=t.slice(1),s.exec(t)&&[3,6,9,12].includes(t.length))){const e=t.length/3,i=[0,0,0];for(let s=0;s<3;++s){const r=parseInt(t.slice(e*s,e*s+e),16);i[s]=1===e?r<<4:2===e?r:3===e?r>>4:r>>8}return i}},t.toRgbString=function(e,t=16){const[i,s,n]=e;return`rgb:${r(i,t)}/${r(s,t)}/${r(n,t)}`}},5770:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.PAYLOAD_LIMIT=void 0,t.PAYLOAD_LIMIT=1e7},6351:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.DcsHandler=t.DcsParser=void 0;const s=i(482),r=i(8742),n=i(5770),o=[];t.DcsParser=class{constructor(){this._handlers=Object.create(null),this._active=o,this._ident=0,this._handlerFb=()=>{},this._stack={paused:!1,loopPosition:0,fallThrough:!1}}dispose(){this._handlers=Object.create(null),this._handlerFb=()=>{},this._active=o}registerHandler(e,t){void 0===this._handlers[e]&&(this._handlers[e]=[]);const i=this._handlers[e];return i.push(t),{dispose:()=>{const e=i.indexOf(t);-1!==e&&i.splice(e,1)}}}clearHandler(e){this._handlers[e]&&delete this._handlers[e]}setHandlerFallback(e){this._handlerFb=e}reset(){if(this._active.length)for(let e=this._stack.paused?this._stack.loopPosition-1:this._active.length-1;e>=0;--e)this._active[e].unhook(!1);this._stack.paused=!1,this._active=o,this._ident=0}hook(e,t){if(this.reset(),this._ident=e,this._active=this._handlers[e]||o,this._active.length)for(let e=this._active.length-1;e>=0;e--)this._active[e].hook(t);else this._handlerFb(this._ident,"HOOK",t)}put(e,t,i){if(this._active.length)for(let s=this._active.length-1;s>=0;s--)this._active[s].put(e,t,i);else this._handlerFb(this._ident,"PUT",(0,s.utf32ToString)(e,t,i))}unhook(e,t=!0){if(this._active.length){let i=!1,s=this._active.length-1,r=!1;if(this._stack.paused&&(s=this._stack.loopPosition-1,i=t,r=this._stack.fallThrough,this._stack.paused=!1),!r&&!1===i){for(;s>=0&&(i=this._active[s].unhook(e),!0!==i);s--)if(i instanceof Promise)return this._stack.paused=!0,this._stack.loopPosition=s,this._stack.fallThrough=!1,i;s--}for(;s>=0;s--)if(i=this._active[s].unhook(!1),i instanceof Promise)return this._stack.paused=!0,this._stack.loopPosition=s,this._stack.fallThrough=!0,i}else this._handlerFb(this._ident,"UNHOOK",e);this._active=o,this._ident=0}};const a=new r.Params;a.addParam(0),t.DcsHandler=class{constructor(e){this._handler=e,this._data="",this._params=a,this._hitLimit=!1}hook(e){this._params=e.length>1||e.params[0]?e.clone():a,this._data="",this._hitLimit=!1}put(e,t,i){this._hitLimit||(this._data+=(0,s.utf32ToString)(e,t,i),this._data.length>n.PAYLOAD_LIMIT&&(this._data="",this._hitLimit=!0))}unhook(e){let t=!1;if(this._hitLimit)t=!1;else if(e&&(t=this._handler(this._data,this._params),t instanceof Promise))return t.then((e=>(this._params=a,this._data="",this._hitLimit=!1,e)));return this._params=a,this._data="",this._hitLimit=!1,t}}},2015:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.EscapeSequenceParser=t.VT500_TRANSITION_TABLE=t.TransitionTable=void 0;const s=i(844),r=i(8742),n=i(6242),o=i(6351);class a{constructor(e){this.table=new Uint8Array(e)}setDefault(e,t){this.table.fill(e<<4|t)}add(e,t,i,s){this.table[t<<8|e]=i<<4|s}addMany(e,t,i,s){for(let r=0;r<e.length;r++)this.table[t<<8|e[r]]=i<<4|s}}t.TransitionTable=a;const h=160;t.VT500_TRANSITION_TABLE=function(){const e=new a(4095),t=Array.apply(null,Array(256)).map(((e,t)=>t)),i=(e,i)=>t.slice(e,i),s=i(32,127),r=i(0,24);r.push(25),r.push.apply(r,i(28,32));const n=i(0,14);let o;for(o in e.setDefault(1,0),e.addMany(s,0,2,0),n)e.addMany([24,26,153,154],o,3,0),e.addMany(i(128,144),o,3,0),e.addMany(i(144,152),o,3,0),e.add(156,o,0,0),e.add(27,o,11,1),e.add(157,o,4,8),e.addMany([152,158,159],o,0,7),e.add(155,o,11,3),e.add(144,o,11,9);return e.addMany(r,0,3,0),e.addMany(r,1,3,1),e.add(127,1,0,1),e.addMany(r,8,0,8),e.addMany(r,3,3,3),e.add(127,3,0,3),e.addMany(r,4,3,4),e.add(127,4,0,4),e.addMany(r,6,3,6),e.addMany(r,5,3,5),e.add(127,5,0,5),e.addMany(r,2,3,2),e.add(127,2,0,2),e.add(93,1,4,8),e.addMany(s,8,5,8),e.add(127,8,5,8),e.addMany([156,27,24,26,7],8,6,0),e.addMany(i(28,32),8,0,8),e.addMany([88,94,95],1,0,7),e.addMany(s,7,0,7),e.addMany(r,7,0,7),e.add(156,7,0,0),e.add(127,7,0,7),e.add(91,1,11,3),e.addMany(i(64,127),3,7,0),e.addMany(i(48,60),3,8,4),e.addMany([60,61,62,63],3,9,4),e.addMany(i(48,60),4,8,4),e.addMany(i(64,127),4,7,0),e.addMany([60,61,62,63],4,0,6),e.addMany(i(32,64),6,0,6),e.add(127,6,0,6),e.addMany(i(64,127),6,0,0),e.addMany(i(32,48),3,9,5),e.addMany(i(32,48),5,9,5),e.addMany(i(48,64),5,0,6),e.addMany(i(64,127),5,7,0),e.addMany(i(32,48),4,9,5),e.addMany(i(32,48),1,9,2),e.addMany(i(32,48),2,9,2),e.addMany(i(48,127),2,10,0),e.addMany(i(48,80),1,10,0),e.addMany(i(81,88),1,10,0),e.addMany([89,90,92],1,10,0),e.addMany(i(96,127),1,10,0),e.add(80,1,11,9),e.addMany(r,9,0,9),e.add(127,9,0,9),e.addMany(i(28,32),9,0,9),e.addMany(i(32,48),9,9,12),e.addMany(i(48,60),9,8,10),e.addMany([60,61,62,63],9,9,10),e.addMany(r,11,0,11),e.addMany(i(32,128),11,0,11),e.addMany(i(28,32),11,0,11),e.addMany(r,10,0,10),e.add(127,10,0,10),e.addMany(i(28,32),10,0,10),e.addMany(i(48,60),10,8,10),e.addMany([60,61,62,63],10,0,11),e.addMany(i(32,48),10,9,12),e.addMany(r,12,0,12),e.add(127,12,0,12),e.addMany(i(28,32),12,0,12),e.addMany(i(32,48),12,9,12),e.addMany(i(48,64),12,0,11),e.addMany(i(64,127),12,12,13),e.addMany(i(64,127),10,12,13),e.addMany(i(64,127),9,12,13),e.addMany(r,13,13,13),e.addMany(s,13,13,13),e.add(127,13,0,13),e.addMany([27,156,24,26],13,14,0),e.add(h,0,2,0),e.add(h,8,5,8),e.add(h,6,0,6),e.add(h,11,0,11),e.add(h,13,13,13),e}();class c extends s.Disposable{constructor(e=t.VT500_TRANSITION_TABLE){super(),this._transitions=e,this._parseStack={state:0,handlers:[],handlerPos:0,transition:0,chunkPos:0},this.initialState=0,this.currentState=this.initialState,this._params=new r.Params,this._params.addParam(0),this._collect=0,this.precedingCodepoint=0,this._printHandlerFb=(e,t,i)=>{},this._executeHandlerFb=e=>{},this._csiHandlerFb=(e,t)=>{},this._escHandlerFb=e=>{},this._errorHandlerFb=e=>e,this._printHandler=this._printHandlerFb,this._executeHandlers=Object.create(null),this._csiHandlers=Object.create(null),this._escHandlers=Object.create(null),this.register((0,s.toDisposable)((()=>{this._csiHandlers=Object.create(null),this._executeHandlers=Object.create(null),this._escHandlers=Object.create(null)}))),this._oscParser=this.register(new n.OscParser),this._dcsParser=this.register(new o.DcsParser),this._errorHandler=this._errorHandlerFb,this.registerEscHandler({final:"\\"},(()=>!0))}_identifier(e,t=[64,126]){let i=0;if(e.prefix){if(e.prefix.length>1)throw new Error("only one byte as prefix supported");if(i=e.prefix.charCodeAt(0),i&&60>i||i>63)throw new Error("prefix must be in range 0x3c .. 0x3f")}if(e.intermediates){if(e.intermediates.length>2)throw new Error("only two bytes as intermediates are supported");for(let t=0;t<e.intermediates.length;++t){const s=e.intermediates.charCodeAt(t);if(32>s||s>47)throw new Error("intermediate must be in range 0x20 .. 0x2f");i<<=8,i|=s}}if(1!==e.final.length)throw new Error("final must be a single byte");const s=e.final.charCodeAt(0);if(t[0]>s||s>t[1])throw new Error(`final must be in range ${t[0]} .. ${t[1]}`);return i<<=8,i|=s,i}identToString(e){const t=[];for(;e;)t.push(String.fromCharCode(255&e)),e>>=8;return t.reverse().join("")}setPrintHandler(e){this._printHandler=e}clearPrintHandler(){this._printHandler=this._printHandlerFb}registerEscHandler(e,t){const i=this._identifier(e,[48,126]);void 0===this._escHandlers[i]&&(this._escHandlers[i]=[]);const s=this._escHandlers[i];return s.push(t),{dispose:()=>{const e=s.indexOf(t);-1!==e&&s.splice(e,1)}}}clearEscHandler(e){this._escHandlers[this._identifier(e,[48,126])]&&delete this._escHandlers[this._identifier(e,[48,126])]}setEscHandlerFallback(e){this._escHandlerFb=e}setExecuteHandler(e,t){this._executeHandlers[e.charCodeAt(0)]=t}clearExecuteHandler(e){this._executeHandlers[e.charCodeAt(0)]&&delete this._executeHandlers[e.charCodeAt(0)]}setExecuteHandlerFallback(e){this._executeHandlerFb=e}registerCsiHandler(e,t){const i=this._identifier(e);void 0===this._csiHandlers[i]&&(this._csiHandlers[i]=[]);const s=this._csiHandlers[i];return s.push(t),{dispose:()=>{const e=s.indexOf(t);-1!==e&&s.splice(e,1)}}}clearCsiHandler(e){this._csiHandlers[this._identifier(e)]&&delete this._csiHandlers[this._identifier(e)]}setCsiHandlerFallback(e){this._csiHandlerFb=e}registerDcsHandler(e,t){return this._dcsParser.registerHandler(this._identifier(e),t)}clearDcsHandler(e){this._dcsParser.clearHandler(this._identifier(e))}setDcsHandlerFallback(e){this._dcsParser.setHandlerFallback(e)}registerOscHandler(e,t){return this._oscParser.registerHandler(e,t)}clearOscHandler(e){this._oscParser.clearHandler(e)}setOscHandlerFallback(e){this._oscParser.setHandlerFallback(e)}setErrorHandler(e){this._errorHandler=e}clearErrorHandler(){this._errorHandler=this._errorHandlerFb}reset(){this.currentState=this.initialState,this._oscParser.reset(),this._dcsParser.reset(),this._params.reset(),this._params.addParam(0),this._collect=0,this.precedingCodepoint=0,0!==this._parseStack.state&&(this._parseStack.state=2,this._parseStack.handlers=[])}_preserveStack(e,t,i,s,r){this._parseStack.state=e,this._parseStack.handlers=t,this._parseStack.handlerPos=i,this._parseStack.transition=s,this._parseStack.chunkPos=r}parse(e,t,i){let s,r=0,n=0,o=0;if(this._parseStack.state)if(2===this._parseStack.state)this._parseStack.state=0,o=this._parseStack.chunkPos+1;else{if(void 0===i||1===this._parseStack.state)throw this._parseStack.state=1,new Error("improper continuation due to previous async handler, giving up parsing");const t=this._parseStack.handlers;let n=this._parseStack.handlerPos-1;switch(this._parseStack.state){case 3:if(!1===i&&n>-1)for(;n>=0&&(s=t[n](this._params),!0!==s);n--)if(s instanceof Promise)return this._parseStack.handlerPos=n,s;this._parseStack.handlers=[];break;case 4:if(!1===i&&n>-1)for(;n>=0&&(s=t[n](),!0!==s);n--)if(s instanceof Promise)return this._parseStack.handlerPos=n,s;this._parseStack.handlers=[];break;case 6:if(r=e[this._parseStack.chunkPos],s=this._dcsParser.unhook(24!==r&&26!==r,i),s)return s;27===r&&(this._parseStack.transition|=1),this._params.reset(),this._params.addParam(0),this._collect=0;break;case 5:if(r=e[this._parseStack.chunkPos],s=this._oscParser.end(24!==r&&26!==r,i),s)return s;27===r&&(this._parseStack.transition|=1),this._params.reset(),this._params.addParam(0),this._collect=0}this._parseStack.state=0,o=this._parseStack.chunkPos+1,this.precedingCodepoint=0,this.currentState=15&this._parseStack.transition}for(let i=o;i<t;++i){switch(r=e[i],n=this._transitions.table[this.currentState<<8|(r<160?r:h)],n>>4){case 2:for(let s=i+1;;++s){if(s>=t||(r=e[s])<32||r>126&&r<h){this._printHandler(e,i,s),i=s-1;break}if(++s>=t||(r=e[s])<32||r>126&&r<h){this._printHandler(e,i,s),i=s-1;break}if(++s>=t||(r=e[s])<32||r>126&&r<h){this._printHandler(e,i,s),i=s-1;break}if(++s>=t||(r=e[s])<32||r>126&&r<h){this._printHandler(e,i,s),i=s-1;break}}break;case 3:this._executeHandlers[r]?this._executeHandlers[r]():this._executeHandlerFb(r),this.precedingCodepoint=0;break;case 0:break;case 1:if(this._errorHandler({position:i,code:r,currentState:this.currentState,collect:this._collect,params:this._params,abort:!1}).abort)return;break;case 7:const o=this._csiHandlers[this._collect<<8|r];let a=o?o.length-1:-1;for(;a>=0&&(s=o[a](this._params),!0!==s);a--)if(s instanceof Promise)return this._preserveStack(3,o,a,n,i),s;a<0&&this._csiHandlerFb(this._collect<<8|r,this._params),this.precedingCodepoint=0;break;case 8:do{switch(r){case 59:this._params.addParam(0);break;case 58:this._params.addSubParam(-1);break;default:this._params.addDigit(r-48)}}while(++i<t&&(r=e[i])>47&&r<60);i--;break;case 9:this._collect<<=8,this._collect|=r;break;case 10:const c=this._escHandlers[this._collect<<8|r];let l=c?c.length-1:-1;for(;l>=0&&(s=c[l](),!0!==s);l--)if(s instanceof Promise)return this._preserveStack(4,c,l,n,i),s;l<0&&this._escHandlerFb(this._collect<<8|r),this.precedingCodepoint=0;break;case 11:this._params.reset(),this._params.addParam(0),this._collect=0;break;case 12:this._dcsParser.hook(this._collect<<8|r,this._params);break;case 13:for(let s=i+1;;++s)if(s>=t||24===(r=e[s])||26===r||27===r||r>127&&r<h){this._dcsParser.put(e,i,s),i=s-1;break}break;case 14:if(s=this._dcsParser.unhook(24!==r&&26!==r),s)return this._preserveStack(6,[],0,n,i),s;27===r&&(n|=1),this._params.reset(),this._params.addParam(0),this._collect=0,this.precedingCodepoint=0;break;case 4:this._oscParser.start();break;case 5:for(let s=i+1;;s++)if(s>=t||(r=e[s])<32||r>127&&r<h){this._oscParser.put(e,i,s),i=s-1;break}break;case 6:if(s=this._oscParser.end(24!==r&&26!==r),s)return this._preserveStack(5,[],0,n,i),s;27===r&&(n|=1),this._params.reset(),this._params.addParam(0),this._collect=0,this.precedingCodepoint=0}this.currentState=15&n}}}t.EscapeSequenceParser=c},6242:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.OscHandler=t.OscParser=void 0;const s=i(5770),r=i(482),n=[];t.OscParser=class{constructor(){this._state=0,this._active=n,this._id=-1,this._handlers=Object.create(null),this._handlerFb=()=>{},this._stack={paused:!1,loopPosition:0,fallThrough:!1}}registerHandler(e,t){void 0===this._handlers[e]&&(this._handlers[e]=[]);const i=this._handlers[e];return i.push(t),{dispose:()=>{const e=i.indexOf(t);-1!==e&&i.splice(e,1)}}}clearHandler(e){this._handlers[e]&&delete this._handlers[e]}setHandlerFallback(e){this._handlerFb=e}dispose(){this._handlers=Object.create(null),this._handlerFb=()=>{},this._active=n}reset(){if(2===this._state)for(let e=this._stack.paused?this._stack.loopPosition-1:this._active.length-1;e>=0;--e)this._active[e].end(!1);this._stack.paused=!1,this._active=n,this._id=-1,this._state=0}_start(){if(this._active=this._handlers[this._id]||n,this._active.length)for(let e=this._active.length-1;e>=0;e--)this._active[e].start();else this._handlerFb(this._id,"START")}_put(e,t,i){if(this._active.length)for(let s=this._active.length-1;s>=0;s--)this._active[s].put(e,t,i);else this._handlerFb(this._id,"PUT",(0,r.utf32ToString)(e,t,i))}start(){this.reset(),this._state=1}put(e,t,i){if(3!==this._state){if(1===this._state)for(;t<i;){const i=e[t++];if(59===i){this._state=2,this._start();break}if(i<48||57<i)return void(this._state=3);-1===this._id&&(this._id=0),this._id=10*this._id+i-48}2===this._state&&i-t>0&&this._put(e,t,i)}}end(e,t=!0){if(0!==this._state){if(3!==this._state)if(1===this._state&&this._start(),this._active.length){let i=!1,s=this._active.length-1,r=!1;if(this._stack.paused&&(s=this._stack.loopPosition-1,i=t,r=this._stack.fallThrough,this._stack.paused=!1),!r&&!1===i){for(;s>=0&&(i=this._active[s].end(e),!0!==i);s--)if(i instanceof Promise)return this._stack.paused=!0,this._stack.loopPosition=s,this._stack.fallThrough=!1,i;s--}for(;s>=0;s--)if(i=this._active[s].end(!1),i instanceof Promise)return this._stack.paused=!0,this._stack.loopPosition=s,this._stack.fallThrough=!0,i}else this._handlerFb(this._id,"END",e);this._active=n,this._id=-1,this._state=0}}},t.OscHandler=class{constructor(e){this._handler=e,this._data="",this._hitLimit=!1}start(){this._data="",this._hitLimit=!1}put(e,t,i){this._hitLimit||(this._data+=(0,r.utf32ToString)(e,t,i),this._data.length>s.PAYLOAD_LIMIT&&(this._data="",this._hitLimit=!0))}end(e){let t=!1;if(this._hitLimit)t=!1;else if(e&&(t=this._handler(this._data),t instanceof Promise))return t.then((e=>(this._data="",this._hitLimit=!1,e)));return this._data="",this._hitLimit=!1,t}}},8742:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.Params=void 0;const i=2147483647;class s{static fromArray(e){const t=new s;if(!e.length)return t;for(let i=Array.isArray(e[0])?1:0;i<e.length;++i){const s=e[i];if(Array.isArray(s))for(let e=0;e<s.length;++e)t.addSubParam(s[e]);else t.addParam(s)}return t}constructor(e=32,t=32){if(this.maxLength=e,this.maxSubParamsLength=t,t>256)throw new Error("maxSubParamsLength must not be greater than 256");this.params=new Int32Array(e),this.length=0,this._subParams=new Int32Array(t),this._subParamsLength=0,this._subParamsIdx=new Uint16Array(e),this._rejectDigits=!1,this._rejectSubDigits=!1,this._digitIsSub=!1}clone(){const e=new s(this.maxLength,this.maxSubParamsLength);return e.params.set(this.params),e.length=this.length,e._subParams.set(this._subParams),e._subParamsLength=this._subParamsLength,e._subParamsIdx.set(this._subParamsIdx),e._rejectDigits=this._rejectDigits,e._rejectSubDigits=this._rejectSubDigits,e._digitIsSub=this._digitIsSub,e}toArray(){const e=[];for(let t=0;t<this.length;++t){e.push(this.params[t]);const i=this._subParamsIdx[t]>>8,s=255&this._subParamsIdx[t];s-i>0&&e.push(Array.prototype.slice.call(this._subParams,i,s))}return e}reset(){this.length=0,this._subParamsLength=0,this._rejectDigits=!1,this._rejectSubDigits=!1,this._digitIsSub=!1}addParam(e){if(this._digitIsSub=!1,this.length>=this.maxLength)this._rejectDigits=!0;else{if(e<-1)throw new Error("values lesser than -1 are not allowed");this._subParamsIdx[this.length]=this._subParamsLength<<8|this._subParamsLength,this.params[this.length++]=e>i?i:e}}addSubParam(e){if(this._digitIsSub=!0,this.length)if(this._rejectDigits||this._subParamsLength>=this.maxSubParamsLength)this._rejectSubDigits=!0;else{if(e<-1)throw new Error("values lesser than -1 are not allowed");this._subParams[this._subParamsLength++]=e>i?i:e,this._subParamsIdx[this.length-1]++}}hasSubParams(e){return(255&this._subParamsIdx[e])-(this._subParamsIdx[e]>>8)>0}getSubParams(e){const t=this._subParamsIdx[e]>>8,i=255&this._subParamsIdx[e];return i-t>0?this._subParams.subarray(t,i):null}getSubParamsAll(){const e={};for(let t=0;t<this.length;++t){const i=this._subParamsIdx[t]>>8,s=255&this._subParamsIdx[t];s-i>0&&(e[t]=this._subParams.slice(i,s))}return e}addDigit(e){let t;if(this._rejectDigits||!(t=this._digitIsSub?this._subParamsLength:this.length)||this._digitIsSub&&this._rejectSubDigits)return;const s=this._digitIsSub?this._subParams:this.params,r=s[t-1];s[t-1]=~r?Math.min(10*r+e,i):e}}t.Params=s},5741:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.AddonManager=void 0,t.AddonManager=class{constructor(){this._addons=[]}dispose(){for(let e=this._addons.length-1;e>=0;e--)this._addons[e].instance.dispose()}loadAddon(e,t){const i={instance:t,dispose:t.dispose,isDisposed:!1};this._addons.push(i),t.dispose=()=>this._wrappedAddonDispose(i),t.activate(e)}_wrappedAddonDispose(e){if(e.isDisposed)return;let t=-1;for(let i=0;i<this._addons.length;i++)if(this._addons[i]===e){t=i;break}if(-1===t)throw new Error("Could not dispose an addon that has not been loaded");e.isDisposed=!0,e.dispose.apply(e.instance),this._addons.splice(t,1)}}},8771:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.BufferApiView=void 0;const s=i(3785),r=i(511);t.BufferApiView=class{constructor(e,t){this._buffer=e,this.type=t}init(e){return this._buffer=e,this}get cursorY(){return this._buffer.y}get cursorX(){return this._buffer.x}get viewportY(){return this._buffer.ydisp}get baseY(){return this._buffer.ybase}get length(){return this._buffer.lines.length}getLine(e){const t=this._buffer.lines.get(e);if(t)return new s.BufferLineApiView(t)}getNullCell(){return new r.CellData}}},3785:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.BufferLineApiView=void 0;const s=i(511);t.BufferLineApiView=class{constructor(e){this._line=e}get isWrapped(){return this._line.isWrapped}get length(){return this._line.length}getCell(e,t){if(!(e<0||e>=this._line.length))return t?(this._line.loadCell(e,t),t):this._line.loadCell(e,new s.CellData)}translateToString(e,t,i){return this._line.translateToString(e,t,i)}}},8285:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.BufferNamespaceApi=void 0;const s=i(8771),r=i(8460),n=i(844);class o extends n.Disposable{constructor(e){super(),this._core=e,this._onBufferChange=this.register(new r.EventEmitter),this.onBufferChange=this._onBufferChange.event,this._normal=new s.BufferApiView(this._core.buffers.normal,"normal"),this._alternate=new s.BufferApiView(this._core.buffers.alt,"alternate"),this._core.buffers.onBufferActivate((()=>this._onBufferChange.fire(this.active)))}get active(){if(this._core.buffers.active===this._core.buffers.normal)return this.normal;if(this._core.buffers.active===this._core.buffers.alt)return this.alternate;throw new Error("Active buffer is neither normal nor alternate")}get normal(){return this._normal.init(this._core.buffers.normal)}get alternate(){return this._alternate.init(this._core.buffers.alt)}}t.BufferNamespaceApi=o},7975:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.ParserApi=void 0,t.ParserApi=class{constructor(e){this._core=e}registerCsiHandler(e,t){return this._core.registerCsiHandler(e,(e=>t(e.toArray())))}addCsiHandler(e,t){return this.registerCsiHandler(e,t)}registerDcsHandler(e,t){return this._core.registerDcsHandler(e,((e,i)=>t(e,i.toArray())))}addDcsHandler(e,t){return this.registerDcsHandler(e,t)}registerEscHandler(e,t){return this._core.registerEscHandler(e,t)}addEscHandler(e,t){return this.registerEscHandler(e,t)}registerOscHandler(e,t){return this._core.registerOscHandler(e,t)}addOscHandler(e,t){return this.registerOscHandler(e,t)}}},7090:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.UnicodeApi=void 0,t.UnicodeApi=class{constructor(e){this._core=e}register(e){this._core.unicodeService.register(e)}get versions(){return this._core.unicodeService.versions}get activeVersion(){return this._core.unicodeService.activeVersion}set activeVersion(e){this._core.unicodeService.activeVersion=e}}},744:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.BufferService=t.MINIMUM_ROWS=t.MINIMUM_COLS=void 0;const n=i(8460),o=i(844),a=i(5295),h=i(2585);t.MINIMUM_COLS=2,t.MINIMUM_ROWS=1;let c=t.BufferService=class extends o.Disposable{get buffer(){return this.buffers.active}constructor(e){super(),this.isUserScrolling=!1,this._onResize=this.register(new n.EventEmitter),this.onResize=this._onResize.event,this._onScroll=this.register(new n.EventEmitter),this.onScroll=this._onScroll.event,this.cols=Math.max(e.rawOptions.cols||0,t.MINIMUM_COLS),this.rows=Math.max(e.rawOptions.rows||0,t.MINIMUM_ROWS),this.buffers=this.register(new a.BufferSet(e,this))}resize(e,t){this.cols=e,this.rows=t,this.buffers.resize(e,t),this._onResize.fire({cols:e,rows:t})}reset(){this.buffers.reset(),this.isUserScrolling=!1}scroll(e,t=!1){const i=this.buffer;let s;s=this._cachedBlankLine,s&&s.length===this.cols&&s.getFg(0)===e.fg&&s.getBg(0)===e.bg||(s=i.getBlankLine(e,t),this._cachedBlankLine=s),s.isWrapped=t;const r=i.ybase+i.scrollTop,n=i.ybase+i.scrollBottom;if(0===i.scrollTop){const e=i.lines.isFull;n===i.lines.length-1?e?i.lines.recycle().copyFrom(s):i.lines.push(s.clone()):i.lines.splice(n+1,0,s.clone()),e?this.isUserScrolling&&(i.ydisp=Math.max(i.ydisp-1,0)):(i.ybase++,this.isUserScrolling||i.ydisp++)}else{const e=n-r+1;i.lines.shiftElements(r+1,e-1,-1),i.lines.set(n,s.clone())}this.isUserScrolling||(i.ydisp=i.ybase),this._onScroll.fire(i.ydisp)}scrollLines(e,t,i){const s=this.buffer;if(e<0){if(0===s.ydisp)return;this.isUserScrolling=!0}else e+s.ydisp>=s.ybase&&(this.isUserScrolling=!1);const r=s.ydisp;s.ydisp=Math.max(Math.min(s.ydisp+e,s.ybase),0),r!==s.ydisp&&(t||this._onScroll.fire(s.ydisp))}};t.BufferService=c=s([r(0,h.IOptionsService)],c)},7994:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.CharsetService=void 0,t.CharsetService=class{constructor(){this.glevel=0,this._charsets=[]}reset(){this.charset=void 0,this._charsets=[],this.glevel=0}setgLevel(e){this.glevel=e,this.charset=this._charsets[e]}setgCharset(e,t){this._charsets[e]=t,this.glevel===e&&(this.charset=t)}}},1753:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.CoreMouseService=void 0;const n=i(2585),o=i(8460),a=i(844),h={NONE:{events:0,restrict:()=>!1},X10:{events:1,restrict:e=>4!==e.button&&1===e.action&&(e.ctrl=!1,e.alt=!1,e.shift=!1,!0)},VT200:{events:19,restrict:e=>32!==e.action},DRAG:{events:23,restrict:e=>32!==e.action||3!==e.button},ANY:{events:31,restrict:e=>!0}};function c(e,t){let i=(e.ctrl?16:0)|(e.shift?4:0)|(e.alt?8:0);return 4===e.button?(i|=64,i|=e.action):(i|=3&e.button,4&e.button&&(i|=64),8&e.button&&(i|=128),32===e.action?i|=32:0!==e.action||t||(i|=3)),i}const l=String.fromCharCode,d={DEFAULT:e=>{const t=[c(e,!1)+32,e.col+32,e.row+32];return t[0]>255||t[1]>255||t[2]>255?"":`[M${l(t[0])}${l(t[1])}${l(t[2])}`},SGR:e=>{const t=0===e.action&&4!==e.button?"m":"M";return`[<${c(e,!0)};${e.col};${e.row}${t}`},SGR_PIXELS:e=>{const t=0===e.action&&4!==e.button?"m":"M";return`[<${c(e,!0)};${e.x};${e.y}${t}`}};let _=t.CoreMouseService=class extends a.Disposable{constructor(e,t){super(),this._bufferService=e,this._coreService=t,this._protocols={},this._encodings={},this._activeProtocol="",this._activeEncoding="",this._lastEvent=null,this._onProtocolChange=this.register(new o.EventEmitter),this.onProtocolChange=this._onProtocolChange.event;for(const e of Object.keys(h))this.addProtocol(e,h[e]);for(const e of Object.keys(d))this.addEncoding(e,d[e]);this.reset()}addProtocol(e,t){this._protocols[e]=t}addEncoding(e,t){this._encodings[e]=t}get activeProtocol(){return this._activeProtocol}get areMouseEventsActive(){return 0!==this._protocols[this._activeProtocol].events}set activeProtocol(e){if(!this._protocols[e])throw new Error(`unknown protocol "${e}"`);this._activeProtocol=e,this._onProtocolChange.fire(this._protocols[e].events)}get activeEncoding(){return this._activeEncoding}set activeEncoding(e){if(!this._encodings[e])throw new Error(`unknown encoding "${e}"`);this._activeEncoding=e}reset(){this.activeProtocol="NONE",this.activeEncoding="DEFAULT",this._lastEvent=null}triggerMouseEvent(e){if(e.col<0||e.col>=this._bufferService.cols||e.row<0||e.row>=this._bufferService.rows)return!1;if(4===e.button&&32===e.action)return!1;if(3===e.button&&32!==e.action)return!1;if(4!==e.button&&(2===e.action||3===e.action))return!1;if(e.col++,e.row++,32===e.action&&this._lastEvent&&this._equalEvents(this._lastEvent,e,"SGR_PIXELS"===this._activeEncoding))return!1;if(!this._protocols[this._activeProtocol].restrict(e))return!1;const t=this._encodings[this._activeEncoding](e);return t&&("DEFAULT"===this._activeEncoding?this._coreService.triggerBinaryEvent(t):this._coreService.triggerDataEvent(t,!0)),this._lastEvent=e,!0}explainEvents(e){return{down:!!(1&e),up:!!(2&e),drag:!!(4&e),move:!!(8&e),wheel:!!(16&e)}}_equalEvents(e,t,i){if(i){if(e.x!==t.x)return!1;if(e.y!==t.y)return!1}else{if(e.col!==t.col)return!1;if(e.row!==t.row)return!1}return e.button===t.button&&e.action===t.action&&e.ctrl===t.ctrl&&e.alt===t.alt&&e.shift===t.shift}};t.CoreMouseService=_=s([r(0,n.IBufferService),r(1,n.ICoreService)],_)},6975:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.CoreService=void 0;const n=i(1439),o=i(8460),a=i(844),h=i(2585),c=Object.freeze({insertMode:!1}),l=Object.freeze({applicationCursorKeys:!1,applicationKeypad:!1,bracketedPasteMode:!1,origin:!1,reverseWraparound:!1,sendFocus:!1,wraparound:!0});let d=t.CoreService=class extends a.Disposable{constructor(e,t,i){super(),this._bufferService=e,this._logService=t,this._optionsService=i,this.isCursorInitialized=!1,this.isCursorHidden=!1,this._onData=this.register(new o.EventEmitter),this.onData=this._onData.event,this._onUserInput=this.register(new o.EventEmitter),this.onUserInput=this._onUserInput.event,this._onBinary=this.register(new o.EventEmitter),this.onBinary=this._onBinary.event,this._onRequestScrollToBottom=this.register(new o.EventEmitter),this.onRequestScrollToBottom=this._onRequestScrollToBottom.event,this.modes=(0,n.clone)(c),this.decPrivateModes=(0,n.clone)(l)}reset(){this.modes=(0,n.clone)(c),this.decPrivateModes=(0,n.clone)(l)}triggerDataEvent(e,t=!1){if(this._optionsService.rawOptions.disableStdin)return;const i=this._bufferService.buffer;t&&this._optionsService.rawOptions.scrollOnUserInput&&i.ybase!==i.ydisp&&this._onRequestScrollToBottom.fire(),t&&this._onUserInput.fire(),this._logService.debug(`sending data "${e}"`,(()=>e.split("").map((e=>e.charCodeAt(0))))),this._onData.fire(e)}triggerBinaryEvent(e){this._optionsService.rawOptions.disableStdin||(this._logService.debug(`sending binary "${e}"`,(()=>e.split("").map((e=>e.charCodeAt(0))))),this._onBinary.fire(e))}};t.CoreService=d=s([r(0,h.IBufferService),r(1,h.ILogService),r(2,h.IOptionsService)],d)},9074:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.DecorationService=void 0;const s=i(8055),r=i(8460),n=i(844),o=i(6106);let a=0,h=0;class c extends n.Disposable{get decorations(){return this._decorations.values()}constructor(){super(),this._decorations=new o.SortedList((e=>null==e?void 0:e.marker.line)),this._onDecorationRegistered=this.register(new r.EventEmitter),this.onDecorationRegistered=this._onDecorationRegistered.event,this._onDecorationRemoved=this.register(new r.EventEmitter),this.onDecorationRemoved=this._onDecorationRemoved.event,this.register((0,n.toDisposable)((()=>this.reset())))}registerDecoration(e){if(e.marker.isDisposed)return;const t=new l(e);if(t){const e=t.marker.onDispose((()=>t.dispose()));t.onDispose((()=>{t&&(this._decorations.delete(t)&&this._onDecorationRemoved.fire(t),e.dispose())})),this._decorations.insert(t),this._onDecorationRegistered.fire(t)}return t}reset(){for(const e of this._decorations.values())e.dispose();this._decorations.clear()}*getDecorationsAtCell(e,t,i){var s,r,n;let o=0,a=0;for(const h of this._decorations.getKeyIterator(t))o=null!==(s=h.options.x)&&void 0!==s?s:0,a=o+(null!==(r=h.options.width)&&void 0!==r?r:1),e>=o&&e<a&&(!i||(null!==(n=h.options.layer)&&void 0!==n?n:"bottom")===i)&&(yield h)}forEachDecorationAtCell(e,t,i,s){this._decorations.forEachByKey(t,(t=>{var r,n,o;a=null!==(r=t.options.x)&&void 0!==r?r:0,h=a+(null!==(n=t.options.width)&&void 0!==n?n:1),e>=a&&e<h&&(!i||(null!==(o=t.options.layer)&&void 0!==o?o:"bottom")===i)&&s(t)}))}}t.DecorationService=c;class l extends n.Disposable{get isDisposed(){return this._isDisposed}get backgroundColorRGB(){return null===this._cachedBg&&(this.options.backgroundColor?this._cachedBg=s.css.toColor(this.options.backgroundColor):this._cachedBg=void 0),this._cachedBg}get foregroundColorRGB(){return null===this._cachedFg&&(this.options.foregroundColor?this._cachedFg=s.css.toColor(this.options.foregroundColor):this._cachedFg=void 0),this._cachedFg}constructor(e){super(),this.options=e,this.onRenderEmitter=this.register(new r.EventEmitter),this.onRender=this.onRenderEmitter.event,this._onDispose=this.register(new r.EventEmitter),this.onDispose=this._onDispose.event,this._cachedBg=null,this._cachedFg=null,this.marker=e.marker,this.options.overviewRulerOptions&&!this.options.overviewRulerOptions.position&&(this.options.overviewRulerOptions.position="full")}dispose(){this._onDispose.fire(),super.dispose()}}},4348:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.InstantiationService=t.ServiceCollection=void 0;const s=i(2585),r=i(8343);class n{constructor(...e){this._entries=new Map;for(const[t,i]of e)this.set(t,i)}set(e,t){const i=this._entries.get(e);return this._entries.set(e,t),i}forEach(e){for(const[t,i]of this._entries.entries())e(t,i)}has(e){return this._entries.has(e)}get(e){return this._entries.get(e)}}t.ServiceCollection=n,t.InstantiationService=class{constructor(){this._services=new n,this._services.set(s.IInstantiationService,this)}setService(e,t){this._services.set(e,t)}getService(e){return this._services.get(e)}createInstance(e,...t){const i=(0,r.getServiceDependencies)(e).sort(((e,t)=>e.index-t.index)),s=[];for(const t of i){const i=this._services.get(t.id);if(!i)throw new Error(`[createInstance] ${e.name} depends on UNKNOWN service ${t.id}.`);s.push(i)}const n=i.length>0?i[0].index:t.length;if(t.length!==n)throw new Error(`[createInstance] First service dependency of ${e.name} at position ${n+1} conflicts with ${t.length} static arguments`);return new e(...[...t,...s])}}},7866:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.traceCall=t.setTraceLogger=t.LogService=void 0;const n=i(844),o=i(2585),a={trace:o.LogLevelEnum.TRACE,debug:o.LogLevelEnum.DEBUG,info:o.LogLevelEnum.INFO,warn:o.LogLevelEnum.WARN,error:o.LogLevelEnum.ERROR,off:o.LogLevelEnum.OFF};let h,c=t.LogService=class extends n.Disposable{get logLevel(){return this._logLevel}constructor(e){super(),this._optionsService=e,this._logLevel=o.LogLevelEnum.OFF,this._updateLogLevel(),this.register(this._optionsService.onSpecificOptionChange("logLevel",(()=>this._updateLogLevel()))),h=this}_updateLogLevel(){this._logLevel=a[this._optionsService.rawOptions.logLevel]}_evalLazyOptionalParams(e){for(let t=0;t<e.length;t++)"function"==typeof e[t]&&(e[t]=e[t]())}_log(e,t,i){this._evalLazyOptionalParams(i),e.call(console,(this._optionsService.options.logger?"":"xterm.js: ")+t,...i)}trace(e,...t){var i,s;this._logLevel<=o.LogLevelEnum.TRACE&&this._log(null!==(s=null===(i=this._optionsService.options.logger)||void 0===i?void 0:i.trace.bind(this._optionsService.options.logger))&&void 0!==s?s:console.log,e,t)}debug(e,...t){var i,s;this._logLevel<=o.LogLevelEnum.DEBUG&&this._log(null!==(s=null===(i=this._optionsService.options.logger)||void 0===i?void 0:i.debug.bind(this._optionsService.options.logger))&&void 0!==s?s:console.log,e,t)}info(e,...t){var i,s;this._logLevel<=o.LogLevelEnum.INFO&&this._log(null!==(s=null===(i=this._optionsService.options.logger)||void 0===i?void 0:i.info.bind(this._optionsService.options.logger))&&void 0!==s?s:console.info,e,t)}warn(e,...t){var i,s;this._logLevel<=o.LogLevelEnum.WARN&&this._log(null!==(s=null===(i=this._optionsService.options.logger)||void 0===i?void 0:i.warn.bind(this._optionsService.options.logger))&&void 0!==s?s:console.warn,e,t)}error(e,...t){var i,s;this._logLevel<=o.LogLevelEnum.ERROR&&this._log(null!==(s=null===(i=this._optionsService.options.logger)||void 0===i?void 0:i.error.bind(this._optionsService.options.logger))&&void 0!==s?s:console.error,e,t)}};t.LogService=c=s([r(0,o.IOptionsService)],c),t.setTraceLogger=function(e){h=e},t.traceCall=function(e,t,i){if("function"!=typeof i.value)throw new Error("not supported");const s=i.value;i.value=function(...e){if(h.logLevel!==o.LogLevelEnum.TRACE)return s.apply(this,e);h.trace(`GlyphRenderer#${s.name}(${e.map((e=>JSON.stringify(e))).join(", ")})`);const t=s.apply(this,e);return h.trace(`GlyphRenderer#${s.name} return`,t),t}}},7302:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.OptionsService=t.DEFAULT_OPTIONS=void 0;const s=i(8460),r=i(844),n=i(6114);t.DEFAULT_OPTIONS={cols:80,rows:24,cursorBlink:!1,cursorStyle:"block",cursorWidth:1,cursorInactiveStyle:"outline",customGlyphs:!0,drawBoldTextInBrightColors:!0,fastScrollModifier:"alt",fastScrollSensitivity:5,fontFamily:"courier-new, courier, monospace",fontSize:15,fontWeight:"normal",fontWeightBold:"bold",ignoreBracketedPasteMode:!1,lineHeight:1,letterSpacing:0,linkHandler:null,logLevel:"info",logger:null,scrollback:1e3,scrollOnUserInput:!0,scrollSensitivity:1,screenReaderMode:!1,smoothScrollDuration:0,macOptionIsMeta:!1,macOptionClickForcesSelection:!1,minimumContrastRatio:1,disableStdin:!1,allowProposedApi:!1,allowTransparency:!1,tabStopWidth:8,theme:{},rightClickSelectsWord:n.isMac,windowOptions:{},windowsMode:!1,windowsPty:{},wordSeparator:" ()[]{}',\"`",altClickMovesCursor:!0,convertEol:!1,termName:"xterm",cancelEvents:!1,overviewRulerWidth:0};const o=["normal","bold","100","200","300","400","500","600","700","800","900"];class a extends r.Disposable{constructor(e){super(),this._onOptionChange=this.register(new s.EventEmitter),this.onOptionChange=this._onOptionChange.event;const i=Object.assign({},t.DEFAULT_OPTIONS);for(const t in e)if(t in i)try{const s=e[t];i[t]=this._sanitizeAndValidateOption(t,s)}catch(e){console.error(e)}this.rawOptions=i,this.options=Object.assign({},i),this._setupOptions()}onSpecificOptionChange(e,t){return this.onOptionChange((i=>{i===e&&t(this.rawOptions[e])}))}onMultipleOptionChange(e,t){return this.onOptionChange((i=>{-1!==e.indexOf(i)&&t()}))}_setupOptions(){const e=e=>{if(!(e in t.DEFAULT_OPTIONS))throw new Error(`No option with key "${e}"`);return this.rawOptions[e]},i=(e,i)=>{if(!(e in t.DEFAULT_OPTIONS))throw new Error(`No option with key "${e}"`);i=this._sanitizeAndValidateOption(e,i),this.rawOptions[e]!==i&&(this.rawOptions[e]=i,this._onOptionChange.fire(e))};for(const t in this.rawOptions){const s={get:e.bind(this,t),set:i.bind(this,t)};Object.defineProperty(this.options,t,s)}}_sanitizeAndValidateOption(e,i){switch(e){case"cursorStyle":if(i||(i=t.DEFAULT_OPTIONS[e]),!function(e){return"block"===e||"underline"===e||"bar"===e}(i))throw new Error(`"${i}" is not a valid value for ${e}`);break;case"wordSeparator":i||(i=t.DEFAULT_OPTIONS[e]);break;case"fontWeight":case"fontWeightBold":if("number"==typeof i&&1<=i&&i<=1e3)break;i=o.includes(i)?i:t.DEFAULT_OPTIONS[e];break;case"cursorWidth":i=Math.floor(i);case"lineHeight":case"tabStopWidth":if(i<1)throw new Error(`${e} cannot be less than 1, value: ${i}`);break;case"minimumContrastRatio":i=Math.max(1,Math.min(21,Math.round(10*i)/10));break;case"scrollback":if((i=Math.min(i,4294967295))<0)throw new Error(`${e} cannot be less than 0, value: ${i}`);break;case"fastScrollSensitivity":case"scrollSensitivity":if(i<=0)throw new Error(`${e} cannot be less than or equal to 0, value: ${i}`);break;case"rows":case"cols":if(!i&&0!==i)throw new Error(`${e} must be numeric, value: ${i}`);break;case"windowsPty":i=null!=i?i:{}}return i}}t.OptionsService=a},2660:function(e,t,i){var s=this&&this.__decorate||function(e,t,i,s){var r,n=arguments.length,o=n<3?t:null===s?s=Object.getOwnPropertyDescriptor(t,i):s;if("object"==typeof Reflect&&"function"==typeof Reflect.decorate)o=Reflect.decorate(e,t,i,s);else for(var a=e.length-1;a>=0;a--)(r=e[a])&&(o=(n<3?r(o):n>3?r(t,i,o):r(t,i))||o);return n>3&&o&&Object.defineProperty(t,i,o),o},r=this&&this.__param||function(e,t){return function(i,s){t(i,s,e)}};Object.defineProperty(t,"__esModule",{value:!0}),t.OscLinkService=void 0;const n=i(2585);let o=t.OscLinkService=class{constructor(e){this._bufferService=e,this._nextId=1,this._entriesWithId=new Map,this._dataByLinkId=new Map}registerLink(e){const t=this._bufferService.buffer;if(void 0===e.id){const i=t.addMarker(t.ybase+t.y),s={data:e,id:this._nextId++,lines:[i]};return i.onDispose((()=>this._removeMarkerFromLink(s,i))),this._dataByLinkId.set(s.id,s),s.id}const i=e,s=this._getEntryIdKey(i),r=this._entriesWithId.get(s);if(r)return this.addLineToLink(r.id,t.ybase+t.y),r.id;const n=t.addMarker(t.ybase+t.y),o={id:this._nextId++,key:this._getEntryIdKey(i),data:i,lines:[n]};return n.onDispose((()=>this._removeMarkerFromLink(o,n))),this._entriesWithId.set(o.key,o),this._dataByLinkId.set(o.id,o),o.id}addLineToLink(e,t){const i=this._dataByLinkId.get(e);if(i&&i.lines.every((e=>e.line!==t))){const e=this._bufferService.buffer.addMarker(t);i.lines.push(e),e.onDispose((()=>this._removeMarkerFromLink(i,e)))}}getLinkData(e){var t;return null===(t=this._dataByLinkId.get(e))||void 0===t?void 0:t.data}_getEntryIdKey(e){return`${e.id};;${e.uri}`}_removeMarkerFromLink(e,t){const i=e.lines.indexOf(t);-1!==i&&(e.lines.splice(i,1),0===e.lines.length&&(void 0!==e.data.id&&this._entriesWithId.delete(e.key),this._dataByLinkId.delete(e.id)))}};t.OscLinkService=o=s([r(0,n.IBufferService)],o)},8343:(e,t)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.createDecorator=t.getServiceDependencies=t.serviceRegistry=void 0;const i="di$target",s="di$dependencies";t.serviceRegistry=new Map,t.getServiceDependencies=function(e){return e[s]||[]},t.createDecorator=function(e){if(t.serviceRegistry.has(e))return t.serviceRegistry.get(e);const r=function(e,t,n){if(3!==arguments.length)throw new Error("@IServiceName-decorator can only be used to decorate a parameter");!function(e,t,r){t[i]===t?t[s].push({id:e,index:r}):(t[s]=[{id:e,index:r}],t[i]=t)}(r,e,n)};return r.toString=()=>e,t.serviceRegistry.set(e,r),r}},2585:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.IDecorationService=t.IUnicodeService=t.IOscLinkService=t.IOptionsService=t.ILogService=t.LogLevelEnum=t.IInstantiationService=t.ICharsetService=t.ICoreService=t.ICoreMouseService=t.IBufferService=void 0;const s=i(8343);var r;t.IBufferService=(0,s.createDecorator)("BufferService"),t.ICoreMouseService=(0,s.createDecorator)("CoreMouseService"),t.ICoreService=(0,s.createDecorator)("CoreService"),t.ICharsetService=(0,s.createDecorator)("CharsetService"),t.IInstantiationService=(0,s.createDecorator)("InstantiationService"),function(e){e[e.TRACE=0]="TRACE",e[e.DEBUG=1]="DEBUG",e[e.INFO=2]="INFO",e[e.WARN=3]="WARN",e[e.ERROR=4]="ERROR",e[e.OFF=5]="OFF"}(r||(t.LogLevelEnum=r={})),t.ILogService=(0,s.createDecorator)("LogService"),t.IOptionsService=(0,s.createDecorator)("OptionsService"),t.IOscLinkService=(0,s.createDecorator)("OscLinkService"),t.IUnicodeService=(0,s.createDecorator)("UnicodeService"),t.IDecorationService=(0,s.createDecorator)("DecorationService")},1480:(e,t,i)=>{Object.defineProperty(t,"__esModule",{value:!0}),t.UnicodeService=void 0;const s=i(8460),r=i(225);t.UnicodeService=class{constructor(){this._providers=Object.create(null),this._active="",this._onChange=new s.EventEmitter,this.onChange=this._onChange.event;const e=new r.UnicodeV6;this.register(e),this._active=e.version,this._activeProvider=e}dispose(){this._onChange.dispose()}get versions(){return Object.keys(this._providers)}get activeVersion(){return this._active}set activeVersion(e){if(!this._providers[e])throw new Error(`unknown Unicode version "${e}"`);this._active=e,this._activeProvider=this._providers[e],this._onChange.fire(e)}register(e){this._providers[e.version]=e}wcwidth(e){return this._activeProvider.wcwidth(e)}getStringCellWidth(e){let t=0;const i=e.length;for(let s=0;s<i;++s){let r=e.charCodeAt(s);if(55296<=r&&r<=56319){if(++s>=i)return t+this.wcwidth(r);const n=e.charCodeAt(s);56320<=n&&n<=57343?r=1024*(r-55296)+n-56320+65536:t+=this.wcwidth(n)}t+=this.wcwidth(r)}return t}}}},t={};function i(s){var r=t[s];if(void 0!==r)return r.exports;var n=t[s]={exports:{}};return e[s].call(n.exports,n,n.exports,i),n.exports}var s={};return(()=>{var e=s;Object.defineProperty(e,"__esModule",{value:!0}),e.Terminal=void 0;const t=i(9042),r=i(3236),n=i(844),o=i(5741),a=i(8285),h=i(7975),c=i(7090),l=["cols","rows"];class d extends n.Disposable{constructor(e){super(),this._core=this.register(new r.Terminal(e)),this._addonManager=this.register(new o.AddonManager),this._publicOptions=Object.assign({},this._core.options);const t=e=>this._core.options[e],i=(e,t)=>{this._checkReadonlyOptions(e),this._core.options[e]=t};for(const e in this._core.options){const s={get:t.bind(this,e),set:i.bind(this,e)};Object.defineProperty(this._publicOptions,e,s)}}_checkReadonlyOptions(e){if(l.includes(e))throw new Error(`Option "${e}" can only be set in the constructor`)}_checkProposedApi(){if(!this._core.optionsService.rawOptions.allowProposedApi)throw new Error("You must set the allowProposedApi option to true to use proposed API")}get onBell(){return this._core.onBell}get onBinary(){return this._core.onBinary}get onCursorMove(){return this._core.onCursorMove}get onData(){return this._core.onData}get onKey(){return this._core.onKey}get onLineFeed(){return this._core.onLineFeed}get onRender(){return this._core.onRender}get onResize(){return this._core.onResize}get onScroll(){return this._core.onScroll}get onSelectionChange(){return this._core.onSelectionChange}get onTitleChange(){return this._core.onTitleChange}get onWriteParsed(){return this._core.onWriteParsed}get element(){return this._core.element}get parser(){return this._parser||(this._parser=new h.ParserApi(this._core)),this._parser}get unicode(){return this._checkProposedApi(),new c.UnicodeApi(this._core)}get textarea(){return this._core.textarea}get rows(){return this._core.rows}get cols(){return this._core.cols}get buffer(){return this._buffer||(this._buffer=this.register(new a.BufferNamespaceApi(this._core))),this._buffer}get markers(){return this._checkProposedApi(),this._core.markers}get modes(){const e=this._core.coreService.decPrivateModes;let t="none";switch(this._core.coreMouseService.activeProtocol){case"X10":t="x10";break;case"VT200":t="vt200";break;case"DRAG":t="drag";break;case"ANY":t="any"}return{applicationCursorKeysMode:e.applicationCursorKeys,applicationKeypadMode:e.applicationKeypad,bracketedPasteMode:e.bracketedPasteMode,insertMode:this._core.coreService.modes.insertMode,mouseTrackingMode:t,originMode:e.origin,reverseWraparoundMode:e.reverseWraparound,sendFocusMode:e.sendFocus,wraparoundMode:e.wraparound}}get options(){return this._publicOptions}set options(e){for(const t in e)this._publicOptions[t]=e[t]}blur(){this._core.blur()}focus(){this._core.focus()}resize(e,t){this._verifyIntegers(e,t),this._core.resize(e,t)}open(e){this._core.open(e)}attachCustomKeyEventHandler(e){this._core.attachCustomKeyEventHandler(e)}registerLinkProvider(e){return this._core.registerLinkProvider(e)}registerCharacterJoiner(e){return this._checkProposedApi(),this._core.registerCharacterJoiner(e)}deregisterCharacterJoiner(e){this._checkProposedApi(),this._core.deregisterCharacterJoiner(e)}registerMarker(e=0){return this._verifyIntegers(e),this._core.registerMarker(e)}registerDecoration(e){var t,i,s;return this._checkProposedApi(),this._verifyPositiveIntegers(null!==(t=e.x)&&void 0!==t?t:0,null!==(i=e.width)&&void 0!==i?i:0,null!==(s=e.height)&&void 0!==s?s:0),this._core.registerDecoration(e)}hasSelection(){return this._core.hasSelection()}select(e,t,i){this._verifyIntegers(e,t,i),this._core.select(e,t,i)}getSelection(){return this._core.getSelection()}getSelectionPosition(){return this._core.getSelectionPosition()}clearSelection(){this._core.clearSelection()}selectAll(){this._core.selectAll()}selectLines(e,t){this._verifyIntegers(e,t),this._core.selectLines(e,t)}dispose(){super.dispose()}scrollLines(e){this._verifyIntegers(e),this._core.scrollLines(e)}scrollPages(e){this._verifyIntegers(e),this._core.scrollPages(e)}scrollToTop(){this._core.scrollToTop()}scrollToBottom(){this._core.scrollToBottom()}scrollToLine(e){this._verifyIntegers(e),this._core.scrollToLine(e)}clear(){this._core.clear()}write(e,t){this._core.write(e,t)}writeln(e,t){this._core.write(e),this._core.write("\r\n",t)}paste(e){this._core.paste(e)}refresh(e,t){this._verifyIntegers(e,t),this._core.refresh(e,t)}reset(){this._core.reset()}clearTextureAtlas(){this._core.clearTextureAtlas()}loadAddon(e){this._addonManager.loadAddon(this,e)}static get strings(){return t}_verifyIntegers(...e){for(const t of e)if(t===1/0||isNaN(t)||t%1!=0)throw new Error("This API only accepts integers")}_verifyPositiveIntegers(...e){for(const t of e)if(t&&(t===1/0||isNaN(t)||t%1!=0||t<0))throw new Error("This API only accepts positive integers")}}e.Terminal=d})(),s})()));
//# sourceMappingURL=xterm.js.map
````

## File: hdmicap/src/main.rs
````rust
// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! hdmicap — warm-stream HDMI capture daemon + thin client CLI.
//!
//! Subcommands:
//!   daemon   run the capture daemon (the controller starts this)
//!   devices  list capture devices
//!   shot     fetch one PNG from a running daemon (--stable, --out)
//!   watch    block until the screen changes, then print the new hash
//!   stop     ask the running daemon to exit
//!   preview  print the URL to open in a browser

mod capture;
mod capture_thread;
mod daemon;
mod frame;
mod server;

use std::io::{Read, Write};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use capture::DeviceSpec;

#[derive(Parser)]
#[command(name = "hdmicap", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the capture daemon (foreground; controller manages the process).
    Daemon {
        /// Device: "auto" (default), an index, or a name substring.
        #[arg(long, default_value = "auto")]
        device: String,
        /// Port to bind on localhost. 0 = OS-assigned.
        #[arg(long, default_value_t = 8723)]
        port: u16,
    },
    /// List available capture devices and exit (no daemon needed).
    Devices,
    /// Fetch one screenshot from the running daemon.
    Shot {
        /// Wait until the signal is Stable before capturing.
        #[arg(long)]
        stable: bool,
        /// Only return once the frame differs from this hex hash.
        #[arg(long)]
        changed_since: Option<String>,
        /// Timeout in ms.
        #[arg(long, default_value_t = 2000)]
        timeout: u64,
        /// Output path; "-" for stdout.
        #[arg(long, default_value = "-")]
        out: String,
    },
    /// Block until the screen changes vs the current frame; print the new hash.
    Watch {
        #[arg(long, default_value_t = 30000)]
        timeout: u64,
    },
    /// Print the preview URL (open in a browser).
    Preview,
    /// Tell the running daemon to shut down.
    Stop,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hdmicap=info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Daemon { device, port } => daemon::run(DeviceSpec::parse(&device), port),
        Cmd::Devices => cmd_devices(),
        Cmd::Shot {
            stable,
            changed_since,
            timeout,
            out,
        } => cmd_shot(stable, changed_since, timeout, &out),
        Cmd::Watch { timeout } => cmd_watch(timeout),
        Cmd::Preview => cmd_preview(),
        Cmd::Stop => cmd_stop(),
    }
}

fn base_url() -> Result<String> {
    let d = daemon::discover()?;
    Ok(format!("http://127.0.0.1:{}", d.port))
}

fn cmd_devices() -> Result<()> {
    for d in capture::enumerate()? {
        println!("{:>3}  {}  [{}]", d.index, d.name, d.misc);
    }
    Ok(())
}

fn cmd_shot(stable: bool, changed_since: Option<String>, timeout: u64, out: &str) -> Result<()> {
    let url = base_url().context("is the daemon running? try `hdmicap daemon`")?;
    let mut snap_url = format!("{url}/snapshot?timeout={timeout}");
    if stable {
        snap_url.push_str("&wait=stable");
    }
    if let Some(ref hash) = changed_since {
        snap_url.push_str(&format!("&changed_since={hash}"));
    }

    let resp = ureq::get(&snap_url)
        .call()
        .context("GET /snapshot failed")?;

    let timed_out = resp.header("x-timeout").map_or(false, |v| v == "1");
    let signal = resp.header("x-signal").unwrap_or("unknown").to_string();
    let hash = resp.header("x-frame-hash").unwrap_or("").to_string();
    eprintln!(
        "signal={}  hash={}{}",
        signal,
        hash,
        if timed_out { "  (timeout)" } else { "" }
    );

    let mut body = Vec::new();
    resp.into_reader().read_to_end(&mut body)?;

    if out == "-" {
        std::io::stdout().write_all(&body)?;
    } else {
        std::fs::write(out, &body).with_context(|| format!("writing {out}"))?;
        eprintln!("wrote {} bytes to {out}", body.len());
    }
    Ok(())
}

fn cmd_watch(timeout: u64) -> Result<()> {
    let url = base_url().context("is the daemon running? try `hdmicap daemon`")?;

    // Read the current hash so we can long-poll for a change.
    let body = ureq::get(&format!("{url}/status"))
        .call()
        .context("GET /status failed")?
        .into_string()
        .context("reading /status body")?;
    let status: serde_json::Value =
        serde_json::from_str(&body).context("parsing /status JSON")?;
    let hash = status["hash"]
        .as_str()
        .unwrap_or("0000000000000000")
        .to_string();

    // Block until the frame changes or we time out.
    let resp =
        ureq::get(&format!("{url}/snapshot?changed_since={hash}&timeout={timeout}"))
            .call()
            .context("GET /snapshot (changed_since) failed")?;

    let new_hash = resp.header("x-frame-hash").unwrap_or("").to_string();
    let timed_out = resp.header("x-timeout").map_or(false, |v| v == "1");
    if timed_out {
        anyhow::bail!("timed out waiting for screen change after {timeout}ms");
    }
    println!("{new_hash}");
    Ok(())
}

fn cmd_preview() -> Result<()> {
    let url = base_url().context("is the daemon running? try `hdmicap daemon`")?;
    println!("Open {url}/preview in a browser to watch the screen.");
    Ok(())
}

fn cmd_stop() -> Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let d = daemon::discover().context("is the daemon running?")?;
    kill(Pid::from_raw(d.pid as i32), Signal::SIGTERM)
        .context("failed to send SIGTERM to daemon")?;
    println!("daemon (pid {}) stopping", d.pid);
    Ok(())
}
````

## File: hdmicap/vendor/nokhwa-bindings-macos/src/lib.rs
````rust
/*
* Copyright 2022 l1npengtul <l1npengtul@protonmail.com> / The Nokhwa Contributors
*
* Licensed under the Apache License, Version 2.0 (the "License");
* you may not use this file except in compliance with the License.
* You may obtain a copy of the License at
*
*     http://www.apache.org/licenses/LICENSE-2.0
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific language governing permissions and
* limitations under the License.
*/

// hello, future peng here
// whatever is written here will induce horrors uncomprehendable.
// save yourselves. write apple code in swift and bind it to rust.

// <some change so we can call this 0.10.4>

#![allow(clippy::not_unsafe_ptr_arg_deref)]

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[macro_use]
extern crate objc;

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod internal {

    #[allow(non_snake_case)]
    pub mod core_media {
        // all of this is stolen from bindgen
        // steal it idc
        use crate::internal::CGFloat;
        use core_media_sys::{
            CMBlockBufferRef, CMFormatDescriptionRef, CMSampleBufferRef, CMTime, CMVideoDimensions,
            FourCharCode,
        };
        use objc::{runtime::Object, Message};
        use std::ops::Deref;

        pub type Id = *mut Object;

        #[repr(transparent)]
        #[derive(Clone)]
        pub struct NSObject(pub Id);
        impl Deref for NSObject {
            type Target = Object;
            fn deref(&self) -> &Self::Target {
                unsafe { &*self.0 }
            }
        }
        unsafe impl Message for NSObject {}
        impl NSObject {
            pub fn alloc() -> Self {
                Self(unsafe { msg_send!(objc::class!(NSObject), alloc) })
            }
        }

        #[repr(transparent)]
        #[derive(Clone)]
        pub struct NSString(pub Id);
        impl Deref for NSString {
            type Target = Object;
            fn deref(&self) -> &Self::Target {
                unsafe { &*self.0 }
            }
        }
        unsafe impl Message for NSString {}
        impl NSString {
            pub fn alloc() -> Self {
                Self(unsafe { msg_send!(objc::class!(NSString), alloc) })
            }
        }

        pub type AVMediaType = NSString;

        #[allow(non_snake_case)]
        #[link(name = "CoreMedia", kind = "framework")]
        extern "C" {
            pub fn CMVideoFormatDescriptionGetDimensions(
                videoDesc: CMFormatDescriptionRef,
            ) -> CMVideoDimensions;

            pub fn CMTimeMake(value: i64, scale: i32) -> CMTime;

            pub fn CMBlockBufferGetDataLength(theBuffer: CMBlockBufferRef) -> std::os::raw::c_int;

            pub fn CMBlockBufferCopyDataBytes(
                theSourceBuffer: CMBlockBufferRef,
                offsetToData: usize,
                dataLength: usize,
                destination: *mut std::os::raw::c_void,
            ) -> std::os::raw::c_int;

            pub fn CMSampleBufferGetDataBuffer(sbuf: CMSampleBufferRef) -> CMBlockBufferRef;

            pub fn CMSampleBufferGetPresentationTimeStamp(sbuf: CMSampleBufferRef) -> CMTime;

            pub fn dispatch_queue_create(
                label: *const std::os::raw::c_char,
                attr: NSObject,
            ) -> NSObject;

            pub fn dispatch_release(object: NSObject);

            pub fn CMSampleBufferGetImageBuffer(sbuf: CMSampleBufferRef) -> CVImageBufferRef;

            pub fn CVPixelBufferLockBaseAddress(
                pixelBuffer: CVPixelBufferRef,
                lockFlags: CVPixelBufferLockFlags,
            ) -> CVReturn;

            pub fn CVPixelBufferUnlockBaseAddress(
                pixelBuffer: CVPixelBufferRef,
                unlockFlags: CVPixelBufferLockFlags,
            ) -> CVReturn;

            pub fn CVPixelBufferGetDataSize(pixelBuffer: CVPixelBufferRef)
                -> std::os::raw::c_ulong;

            pub fn CVPixelBufferGetBaseAddress(
                pixelBuffer: CVPixelBufferRef,
            ) -> *mut std::os::raw::c_void;

            pub fn CVPixelBufferGetPixelFormatType(pixelBuffer: CVPixelBufferRef) -> OSType;
        }

        #[repr(C)]
        #[derive(Clone, Debug, PartialEq, PartialOrd)]
        pub struct CGPoint {
            pub x: CGFloat,
            pub y: CGFloat,
        }

        #[repr(C)]
        #[derive(Debug, Copy, Clone)]
        pub struct __CVBuffer {
            _unused: [u8; 0],
        }

        #[allow(non_snake_case)]
        #[derive(Copy, Clone, Debug, PartialOrd, PartialEq)]
        #[repr(C)]
        pub struct AVCaptureWhiteBalanceGains {
            pub blueGain: f32,
            pub greenGain: f32,
            pub redGain: f32,
        }

        pub type CVBufferRef = *mut __CVBuffer;

        pub type CVImageBufferRef = CVBufferRef;
        pub type CVPixelBufferRef = CVImageBufferRef;
        pub type CVPixelBufferLockFlags = u64;
        pub type CVReturn = i32;

        pub type OSType = FourCharCode;
        pub type AVVideoCodecType = NSString;

        #[link(name = "AVFoundation", kind = "framework")]
        extern "C" {
            pub static AVVideoCodecKey: NSString;
            pub static AVVideoCodecTypeHEVC: AVVideoCodecType;
            pub static AVVideoCodecTypeH264: AVVideoCodecType;
            pub static AVVideoCodecTypeJPEG: AVVideoCodecType;
            pub static AVVideoCodecTypeAppleProRes4444: AVVideoCodecType;
            pub static AVVideoCodecTypeAppleProRes422: AVVideoCodecType;
            pub static AVVideoCodecTypeAppleProRes422HQ: AVVideoCodecType;
            pub static AVVideoCodecTypeAppleProRes422LT: AVVideoCodecType;
            pub static AVVideoCodecTypeAppleProRes422Proxy: AVVideoCodecType;
            pub static AVVideoCodecTypeHEVCWithAlpha: AVVideoCodecType;
            pub static AVVideoCodecHEVC: NSString;
            pub static AVVideoCodecH264: NSString;
            pub static AVVideoCodecJPEG: NSString;
            pub static AVVideoCodecAppleProRes4444: NSString;
            pub static AVVideoCodecAppleProRes422: NSString;
            pub static AVVideoWidthKey: NSString;
            pub static AVVideoHeightKey: NSString;
            pub static AVVideoExpectedSourceFrameRateKey: NSString;

            pub static AVMediaTypeVideo: AVMediaType;
            pub static AVMediaTypeAudio: AVMediaType;
            pub static AVMediaTypeText: AVMediaType;
            pub static AVMediaTypeClosedCaption: AVMediaType;
            pub static AVMediaTypeSubtitle: AVMediaType;
            pub static AVMediaTypeTimecode: AVMediaType;
            pub static AVMediaTypeMetadata: AVMediaType;
            pub static AVMediaTypeMuxed: AVMediaType;
            pub static AVMediaTypeMetadataObject: AVMediaType;
            pub static AVMediaTypeDepthData: AVMediaType;

            pub static AVCaptureLensPositionCurrent: f32;
            pub static AVCaptureExposureTargetBiasCurrent: f32;
            pub static AVCaptureExposureDurationCurrent: CMTime;
            pub static AVCaptureISOCurrent: f32;
        }
    }

    use crate::core_media::{
        dispatch_queue_create, AVCaptureExposureDurationCurrent,
        AVCaptureExposureTargetBiasCurrent, AVCaptureISOCurrent, AVCaptureWhiteBalanceGains,
        AVMediaTypeAudio, AVMediaTypeClosedCaption, AVMediaTypeDepthData, AVMediaTypeMetadata,
        AVMediaTypeMetadataObject, AVMediaTypeMuxed, AVMediaTypeSubtitle, AVMediaTypeText,
        AVMediaTypeTimecode, AVMediaTypeVideo, CGPoint, CMSampleBufferGetImageBuffer,
        CMVideoFormatDescriptionGetDimensions, CVImageBufferRef, CVPixelBufferGetBaseAddress,
        CVPixelBufferGetDataSize, CVPixelBufferLockBaseAddress, CVPixelBufferUnlockBaseAddress,
        NSObject, OSType,
    };

    use block::ConcreteBlock;
    use cocoa_foundation::{
        base::Nil,
        foundation::{NSArray, NSDictionary, NSInteger, NSString, NSUInteger},
    };
    use core_media_sys::{
        kCMPixelFormat_24RGB, kCMPixelFormat_422YpCbCr8_yuvs,
        kCMPixelFormat_8IndexedGray_WhiteIsZero, kCMVideoCodecType_422YpCbCr8,
        kCMVideoCodecType_JPEG, kCMVideoCodecType_JPEG_OpenDML, CMFormatDescriptionGetMediaSubType,
        CMFormatDescriptionRef, CMSampleBufferRef, CMTime, CMVideoDimensions,
    };
    use core_video_sys::{
        kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange,
        kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
        kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
    };
    use flume::{Receiver, Sender};
    use nokhwa_core::{
        error::NokhwaError,
        types::{
            ApiBackend, CameraControl, CameraFormat, CameraIndex, CameraInfo,
            ControlValueDescription, ControlValueSetter, FrameFormat, KnownCameraControl,
            KnownCameraControlFlag, Resolution,
        },
    };
    use objc::runtime::objc_getClass;
    use objc::{
        declare::ClassDecl,
        runtime::{Class, Object, Protocol, Sel, BOOL, NO, YES},
    };
    use once_cell::sync::Lazy;
    use std::ffi::CString;
    use std::{
        borrow::Cow,
        cmp::Ordering,
        collections::BTreeMap,
        convert::TryFrom,
        error::Error,
        ffi::{c_float, c_void, CStr},
        sync::Arc,
        time::Duration,
    };

    const UTF8_ENCODING: usize = 4;
    type CGFloat = c_float;

    extern "C" {
        fn mach_absolute_time() -> u64;
    }

    #[repr(C)]
    struct MachTimebaseInfo {
        numer: u32,
        denom: u32,
    }

    extern "C" {
        fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
    }

    fn mach_absolute_time_nanos() -> u64 {
        static TIMEBASE: once_cell::sync::Lazy<(u32, u32)> = once_cell::sync::Lazy::new(|| {
            let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
            unsafe { mach_timebase_info(&mut info) };
            (info.numer, info.denom)
        });
        let ticks = unsafe { mach_absolute_time() };
        let (numer, denom) = *TIMEBASE;
        ticks.wrapping_mul(numer as u64) / (denom as u64)
    }

    macro_rules! create_boilerplate_impl {
        {
            $( [$class_vis:vis $class_name:ident : $( {$field_vis:vis $field_name:ident : $field_type:ty} ),*] ),+
        } => {
            $(
                $class_vis struct $class_name {
                    inner: *mut Object,
                    $(
                        $field_vis $field_name : $field_type
                    )*
                }

                impl $class_name {
                    pub fn inner(&self) -> *mut Object {
                        self.inner
                    }
                }
            )+
        };

        {
            $( [$class_vis:vis $class_name:ident ] ),+
        } => {
            $(
                $class_vis struct $class_name {
                    inner: *mut Object,
                }

                impl $class_name {
                    pub fn inner(&self) -> *mut Object {
                        self.inner
                    }
                }

                impl From<*mut Object> for $class_name {
                    fn from(obj: *mut Object) -> Self {
                        $class_name {
                            inner: obj,
                        }
                    }
                }
            )+
        };
    }

    fn str_to_nsstr(string: &str) -> *mut Object {
        let cls = class!(NSString);
        let bytes = string.as_ptr() as *const c_void;
        unsafe {
            let obj: *mut Object = msg_send![cls, alloc];
            let obj: *mut Object = msg_send![
                obj,
                initWithBytes:bytes
                length:string.len()
                encoding:UTF8_ENCODING
            ];
            obj
        }
    }

    fn nsstr_to_str<'a>(nsstr: *mut Object) -> Cow<'a, str> {
        let data = unsafe { CStr::from_ptr(nsstr.UTF8String()) };
        data.to_string_lossy()
    }

    fn vec_to_ns_arr<T: Into<*mut Object>>(data: Vec<T>) -> *mut Object {
        let cstr = CString::new("NSMutableArray").unwrap();
        let ns_arr_cls = unsafe { objc_getClass(cstr.as_ptr()) };
        let mutable_array: *mut Object = unsafe { msg_send![ns_arr_cls, array] };
        data.into_iter().for_each(|item| {
            let item_obj: *mut Object = item.into();
            let _: () = unsafe { msg_send![mutable_array, addObject: item_obj] };
        });
        mutable_array
    }

    fn ns_arr_to_vec<T: From<*mut Object>>(data: *mut Object) -> Vec<T> {
        let length = unsafe { NSArray::count(data) };

        let mut out_vec: Vec<T> = Vec::with_capacity(length as usize);
        for index in 0..length {
            let item = unsafe { NSArray::objectAtIndex(data, index) };
            out_vec.push(T::from(item));
        }
        out_vec
    }

    fn try_ns_arr_to_vec<T, TE>(data: *mut Object) -> Result<Vec<T>, TE>
    where
        TE: Error,
        T: TryFrom<*mut Object, Error = TE>,
    {
        let length = unsafe { NSArray::count(data) };

        let mut out_vec: Vec<T> = Vec::with_capacity(length as usize);
        for index in 0..length {
            let item = unsafe { NSArray::objectAtIndex(data, index) };
            out_vec.push(T::try_from(item)?);
        }
        Ok(out_vec)
    }

    fn compare_ns_string(this: *mut Object, other: core_media::NSString) -> bool {
        unsafe {
            let equal: BOOL = msg_send![this, isEqualToString: other];
            equal == YES
        }
    }

    #[allow(non_upper_case_globals)]
    fn raw_fcc_to_frameformat(raw: OSType) -> Option<FrameFormat> {
        match raw {
            kCMVideoCodecType_422YpCbCr8 | kCMPixelFormat_422YpCbCr8_yuvs => {
                Some(FrameFormat::YUYV)
            }
            kCMVideoCodecType_JPEG | kCMVideoCodecType_JPEG_OpenDML => Some(FrameFormat::MJPEG),
            kCMPixelFormat_8IndexedGray_WhiteIsZero => Some(FrameFormat::GRAY),
            kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange
            | kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
            | kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange => Some(FrameFormat::YUYV),
            kCMPixelFormat_24RGB => Some(FrameFormat::RAWRGB),
            _ => None,
        }
    }

    pub type CompressionData<'a> = (Cow<'a, [u8]>, FrameFormat, Option<Duration>);
    pub type DataPipe<'a> = (Sender<CompressionData<'a>>, Receiver<CompressionData<'a>>);

    static CALLBACK_CLASS: Lazy<&'static Class> = Lazy::new(|| {
        {
            let mut decl = ClassDecl::new("MyCaptureCallback", class!(NSObject)).unwrap();

            // frame stack
            // oooh scary provenannce-breaking BULLSHIT AAAAAA I LOVE TYPE ERASURE
            decl.add_ivar::<*const c_void>("_arcmutptr"); // ArkMutex, the not-arknights totally not gacha totally not ripoff new vidya game from l-pleasestop-npengtul

            extern "C" fn my_callback_get_arcmutptr(this: &Object, _: Sel) -> *const c_void {
                unsafe { *this.get_ivar("_arcmutptr") }
            }
            extern "C" fn my_callback_set_arcmutptr(
                this: &mut Object,
                _: Sel,
                new_arcmutptr: *const c_void,
            ) {
                unsafe {
                    this.set_ivar("_arcmutptr", new_arcmutptr);
                }
            }

            // Delegate compliance method
            // SAFETY: This assumes that the buffer byte size is a u8. Any other size will cause unsafety.
            #[allow(non_snake_case)]
            #[allow(non_upper_case_globals)]
            extern "C" fn capture_out_callback(
                this: &mut Object,
                _: Sel,
                _: *mut Object,
                didOutputSampleBuffer: CMSampleBufferRef,
                _: *mut Object,
            ) {
                let image_buffer: CVImageBufferRef =
                    unsafe { CMSampleBufferGetImageBuffer(didOutputSampleBuffer) };
                unsafe {
                    CVPixelBufferLockBaseAddress(image_buffer, 0);
                };

                let buffer_length = unsafe { CVPixelBufferGetDataSize(image_buffer) };
                let buffer_ptr = unsafe { CVPixelBufferGetBaseAddress(image_buffer) };
                let buffer_as_vec = unsafe {
                    std::slice::from_raw_parts_mut(buffer_ptr as *mut u8, buffer_length as usize)
                        .to_vec()
                };

                unsafe { CVPixelBufferUnlockBaseAddress(image_buffer, 0) };

                // CMSampleBufferGetPresentationTimeStamp returns the sensor
                // capture instant on a monotonic clock (mach_absolute_time
                // timebase).  Convert to Unix wallclock:
                //   wall = SystemTime::now() - (mach_now - pts)
                let capture_ts = {
                    let pts = unsafe {
                        core_media::CMSampleBufferGetPresentationTimeStamp(
                            didOutputSampleBuffer,
                        )
                    };
                    if pts.timescale > 0 {
                        let pts_nanos = (pts.value as u128)
                            .saturating_mul(1_000_000_000)
                            / (pts.timescale as u128);
                        let mono_now_nanos = mach_absolute_time_nanos() as u128;
                        let wall_now = std::time::SystemTime::now();

                        let age = Duration::from_nanos(
                            mono_now_nanos.saturating_sub(pts_nanos) as u64,
                        );
                        wall_now
                            .duration_since(std::time::UNIX_EPOCH)
                            .ok()
                            .and_then(|wall_dur| wall_dur.checked_sub(age))
                    } else {
                        None
                    }
                };

                // oooooh scarey unsafe
                // AAAAAAAAAAAAAAAAAAAAAAAAA
                // https://c.tenor.com/0e_zWtFLOzQAAAAC/needy-streamer-overload-needy-girl-overdose.gif
                let bufferlck_cv: *const c_void = unsafe { msg_send![this, bufferPtr] };
                let buffer_sndr = unsafe {
                    let ptr = bufferlck_cv.cast::<Sender<(Vec<u8>, FrameFormat, Option<Duration>)>>();
                    Arc::from_raw(ptr)
                };
                if let Err(_) = buffer_sndr.send((buffer_as_vec, FrameFormat::GRAY, capture_ts)) {
                    // FIXME: dont, what the fuck???
                    return;
                }
                std::mem::forget(buffer_sndr);
            }

            #[allow(non_snake_case)]
            extern "C" fn capture_drop_callback(
                _: &mut Object,
                _: Sel,
                _: *mut Object,
                _: *mut Object,
                _: *mut Object,
            ) {
            }

            unsafe {
                decl.add_method(
                    sel!(bufferPtr),
                    my_callback_get_arcmutptr as extern "C" fn(&Object, Sel) -> *const c_void,
                );
                decl.add_method(
                    sel!(SetBufferPtr:),
                    my_callback_set_arcmutptr as extern "C" fn(&mut Object, Sel, *const c_void),
                );
                decl.add_method(
                    sel!(captureOutput:didOutputSampleBuffer:fromConnection:),
                    capture_out_callback
                        as extern "C" fn(
                            &mut Object,
                            Sel,
                            *mut Object,
                            CMSampleBufferRef,
                            *mut Object,
                        ),
                );
                decl.add_method(
                    sel!(captureOutput:didDropSampleBuffer:fromConnection:),
                    capture_drop_callback
                        as extern "C" fn(&mut Object, Sel, *mut Object, *mut Object, *mut Object),
                );

                decl.add_protocol(
                    Protocol::get("AVCaptureVideoDataOutputSampleBufferDelegate").unwrap(),
                );
            }

            decl.register()
        }
    });

    pub fn request_permission_with_callback(callback: impl Fn(bool) + Send + Sync + 'static) {
        let cls = class!(AVCaptureDevice);

        let wrapper = move |bool: BOOL| {
            callback(bool == YES);
        };

        let objc_fn_block: ConcreteBlock<(BOOL,), (), _> = ConcreteBlock::new(wrapper);
        let objc_fn_pass = objc_fn_block.copy();

        unsafe {
            let _: () = msg_send![cls, requestAccessForMediaType:(AVMediaTypeVideo.clone()) completionHandler:objc_fn_pass];
        }
    }

    pub fn current_authorization_status() -> AVAuthorizationStatus {
        let cls = class!(AVCaptureDevice);
        let status: AVAuthorizationStatus = unsafe {
            msg_send![cls, authorizationStatusForMediaType:AVMediaType::Video.into_ns_str()]
        };
        status
    }

    // fuck it, use deprecated APIs
    pub fn query_avfoundation() -> Result<Vec<CameraInfo>, NokhwaError> {
        Ok(AVCaptureDeviceDiscoverySession::new(vec![
            AVCaptureDeviceType::UltraWide,
            AVCaptureDeviceType::WideAngle,
            AVCaptureDeviceType::Telephoto,
            AVCaptureDeviceType::TrueDepth,
            AVCaptureDeviceType::External,
        ])?
        .devices())
    }

    pub fn get_raw_device_info(index: CameraIndex, device: *mut Object) -> CameraInfo {
        let name = nsstr_to_str(unsafe { msg_send![device, localizedName] });
        let manufacturer = nsstr_to_str(unsafe { msg_send![device, manufacturer] });
        let position: AVCaptureDevicePosition = unsafe { msg_send![device, position] };
        let lens_aperture: f64 = unsafe { msg_send![device, lensAperture] };
        let device_type = nsstr_to_str(unsafe { msg_send![device, deviceType] });
        let model_id = nsstr_to_str(unsafe { msg_send![device, modelID] });
        let description = format!(
            "{}: {} - {}, {:?} f{}",
            manufacturer, model_id, device_type, position, lens_aperture
        );
        let misc = nsstr_to_str(unsafe { msg_send![device, uniqueID] });

        CameraInfo::new(name.as_ref(), &description, misc.as_ref(), index)
    }

    #[derive(Copy, Clone, Debug, Hash, Ord, PartialOrd, Eq, PartialEq)]
    pub enum AVCaptureDeviceType {
        Dual,
        DualWide,
        Triple,
        WideAngle,
        UltraWide,
        Telephoto,
        TrueDepth,
        External,
    }

    impl From<AVCaptureDeviceType> for *mut Object {
        fn from(device_type: AVCaptureDeviceType) -> Self {
            match device_type {
                AVCaptureDeviceType::Dual => str_to_nsstr("AVCaptureDeviceTypeBuiltInDualCamera"),
                AVCaptureDeviceType::DualWide => {
                    str_to_nsstr("AVCaptureDeviceTypeBuiltInDualWideCamera")
                }
                AVCaptureDeviceType::Triple => {
                    str_to_nsstr("AVCaptureDeviceTypeBuiltInTripleCamera")
                }
                AVCaptureDeviceType::WideAngle => {
                    str_to_nsstr("AVCaptureDeviceTypeBuiltInWideAngleCamera")
                }
                AVCaptureDeviceType::UltraWide => {
                    str_to_nsstr("AVCaptureDeviceTypeBuiltInUltraWideCamera")
                }
                AVCaptureDeviceType::Telephoto => {
                    str_to_nsstr("AVCaptureDeviceTypeBuiltInTelephotoCamera")
                }
                AVCaptureDeviceType::TrueDepth => {
                    str_to_nsstr("AVCaptureDeviceTypeBuiltInTrueDepthCamera")
                }
                AVCaptureDeviceType::External => str_to_nsstr("AVCaptureDeviceTypeExternal"),
            }
        }
    }

    impl AVCaptureDeviceType {
        pub fn into_ns_str(self) -> *mut Object {
            <*mut Object>::from(self)
        }
    }

    #[derive(Copy, Clone, Debug, Hash, Ord, PartialOrd, Eq, PartialEq)]
    pub enum AVMediaType {
        Audio,
        ClosedCaption,
        DepthData,
        Metadata,
        MetadataObject,
        Muxed,
        Subtitle,
        Text,
        Timecode,
        Video,
    }

    impl From<AVMediaType> for *mut Object {
        fn from(media_type: AVMediaType) -> Self {
            match media_type {
                AVMediaType::Audio => unsafe { AVMediaTypeAudio.0 },
                AVMediaType::ClosedCaption => unsafe { AVMediaTypeClosedCaption.0 },
                AVMediaType::DepthData => unsafe { AVMediaTypeDepthData.0 },
                AVMediaType::Metadata => unsafe { AVMediaTypeMetadata.0 },
                AVMediaType::MetadataObject => unsafe { AVMediaTypeMetadataObject.0 },
                AVMediaType::Muxed => unsafe { AVMediaTypeMuxed.0 },
                AVMediaType::Subtitle => unsafe { AVMediaTypeSubtitle.0 },
                AVMediaType::Text => unsafe { AVMediaTypeText.0 },
                AVMediaType::Timecode => unsafe { AVMediaTypeTimecode.0 },
                AVMediaType::Video => unsafe { AVMediaTypeVideo.0 },
            }
        }
    }

    impl TryFrom<*mut Object> for AVMediaType {
        type Error = NokhwaError;

        fn try_from(value: *mut Object) -> Result<Self, Self::Error> {
            unsafe {
                if compare_ns_string(value, (AVMediaTypeAudio).clone()) {
                    Ok(AVMediaType::Audio)
                } else if compare_ns_string(value, (AVMediaTypeClosedCaption).clone()) {
                    Ok(AVMediaType::ClosedCaption)
                } else if compare_ns_string(value, (AVMediaTypeDepthData).clone()) {
                    Ok(AVMediaType::DepthData)
                } else if compare_ns_string(value, (AVMediaTypeMetadata).clone()) {
                    Ok(AVMediaType::Metadata)
                } else if compare_ns_string(value, (AVMediaTypeMetadataObject).clone()) {
                    Ok(AVMediaType::MetadataObject)
                } else if compare_ns_string(value, (AVMediaTypeMuxed).clone()) {
                    Ok(AVMediaType::Muxed)
                } else if compare_ns_string(value, (AVMediaTypeSubtitle).clone()) {
                    Ok(AVMediaType::Subtitle)
                } else if compare_ns_string(value, (AVMediaTypeText).clone()) {
                    Ok(AVMediaType::Text)
                } else if compare_ns_string(value, (AVMediaTypeTimecode).clone()) {
                    Ok(AVMediaType::Timecode)
                } else if compare_ns_string(value, (AVMediaTypeVideo).clone()) {
                    Ok(AVMediaType::Video)
                } else {
                    let name = nsstr_to_str(value);
                    Err(NokhwaError::GetPropertyError {
                        property: "AVMediaType".to_string(),
                        error: format!("Invalid AVMediaType {name}"),
                    })
                }
            }
        }
    }

    impl AVMediaType {
        pub fn into_ns_str(self) -> *mut Object {
            <*mut Object>::from(self)
        }
    }

    #[derive(Copy, Clone, Debug, Hash, Ord, PartialOrd, Eq, PartialEq)]
    #[repr(isize)]
    pub enum AVCaptureDevicePosition {
        Unspecified = 0,
        Back = 1,
        Front = 2,
    }

    #[derive(Copy, Clone, Debug, Hash, Ord, PartialOrd, Eq, PartialEq)]
    #[repr(isize)]
    pub enum AVAuthorizationStatus {
        NotDetermined = 0,
        Restricted = 1,
        Denied = 2,
        Authorized = 3,
    }

    pub struct AVCaptureVideoCallback {
        delegate: *mut Object,
        queue: NSObject,
    }

    impl AVCaptureVideoCallback {
        pub fn new(
            device_spec: &CStr,
            buffer: &Arc<Sender<(Vec<u8>, FrameFormat, Option<Duration>)>>,
        ) -> Result<Self, NokhwaError> {
            let cls = &CALLBACK_CLASS as &Class;
            let delegate: *mut Object = unsafe { msg_send![cls, alloc] };
            let delegate: *mut Object = unsafe { msg_send![delegate, init] };
            let buffer_as_ptr = {
                let arc_raw = Arc::as_ptr(buffer);
                arc_raw.cast::<c_void>()
            };
            unsafe {
                let _: () = msg_send![delegate, SetBufferPtr: buffer_as_ptr];
            }

            let queue = unsafe {
                dispatch_queue_create(device_spec.as_ptr(), NSObject(std::ptr::null_mut()))
            };

            Ok(AVCaptureVideoCallback { delegate, queue })
        }

        pub fn data_len(&self) -> usize {
            unsafe { msg_send![self.delegate, dataLength] }
        }

        pub fn inner(&self) -> *mut Object {
            self.delegate
        }

        pub fn queue(&self) -> &NSObject {
            &self.queue
        }
    }

    create_boilerplate_impl! {
        [pub AVFrameRateRange],
        [pub AVCaptureDeviceDiscoverySession],
        [pub AVCaptureDeviceInput],
        [pub AVCaptureSession]
    }

    impl AVFrameRateRange {
        pub fn max(&self) -> f64 {
            unsafe { msg_send![self.inner, maxFrameRate] }
        }

        pub fn min(&self) -> f64 {
            unsafe { msg_send![self.inner, minFrameRate] }
        }
    }

    #[derive(Debug)]
    pub struct AVCaptureDeviceFormat {
        pub(crate) internal: *mut Object,
        pub resolution: CMVideoDimensions,
        pub fps_list: Vec<f64>,
        pub fourcc: FrameFormat,
    }

    impl TryFrom<*mut Object> for AVCaptureDeviceFormat {
        type Error = NokhwaError;

        fn try_from(value: *mut Object) -> Result<Self, Self::Error> {
            let media_type_raw: *mut Object = unsafe { msg_send![value, mediaType] };
            let media_type = AVMediaType::try_from(media_type_raw)?;
            if media_type != AVMediaType::Video {
                return Err(NokhwaError::StructureError {
                    structure: "AVMediaType".to_string(),
                    error: "Not Video".to_string(),
                });
            }
            let mut fps_list = ns_arr_to_vec::<AVFrameRateRange>(unsafe {
                msg_send![value, videoSupportedFrameRateRanges]
            })
            .into_iter()
            .flat_map(|v| {
                if v.min() != 0_f64 && v.min() != 1_f64 {
                    vec![v.min(), v.max()]
                } else {
                    vec![v.max()] // this gets deduped!
                }
            })
            .collect::<Vec<f64>>();
            fps_list.sort_by(|n, m| n.partial_cmp(m).unwrap_or(Ordering::Equal));
            fps_list.dedup();
            let description_obj: *mut Object = unsafe { msg_send![value, formatDescription] };
            let resolution =
                unsafe { CMVideoFormatDescriptionGetDimensions(description_obj as *mut c_void) };
            let fcc_raw =
                unsafe { CMFormatDescriptionGetMediaSubType(description_obj as *mut c_void) };
            #[allow(non_upper_case_globals)]
            let fourcc = match raw_fcc_to_frameformat(fcc_raw) {
                Some(fcc) => fcc,
                None => {
                    return Err(NokhwaError::StructureError {
                        structure: "FourCharCode".to_string(),
                        error: format!("Unknown FourCharCode {fcc_raw:?}"),
                    })
                }
            };

            Ok(AVCaptureDeviceFormat {
                internal: value,
                resolution,
                fps_list,
                fourcc,
            })
        }
    }

    impl AVCaptureDeviceDiscoverySession {
        pub fn new(device_types: Vec<AVCaptureDeviceType>) -> Result<Self, NokhwaError> {
            let device_types = vec_to_ns_arr(device_types);
            let position = 0 as NSInteger;

            let media_type_video = unsafe { AVMediaTypeVideo.clone() }.0;

            let discovery_session_cls = class!(AVCaptureDeviceDiscoverySession);
            let discovery_session: *mut Object = unsafe {
                msg_send![discovery_session_cls, discoverySessionWithDeviceTypes:device_types mediaType:media_type_video position:position]
            };

            Ok(AVCaptureDeviceDiscoverySession {
                inner: discovery_session,
            })
        }

        pub fn default() -> Result<Self, NokhwaError> {
            AVCaptureDeviceDiscoverySession::new(vec![
                AVCaptureDeviceType::UltraWide,
                AVCaptureDeviceType::Telephoto,
                AVCaptureDeviceType::External,
                AVCaptureDeviceType::Dual,
                AVCaptureDeviceType::DualWide,
                AVCaptureDeviceType::Triple,
            ])
        }

        pub fn devices(&self) -> Vec<CameraInfo> {
            let device_ns_array: *mut Object = unsafe { msg_send![self.inner, devices] };
            let objects_len: NSUInteger = unsafe { NSArray::count(device_ns_array) };
            let mut devices = Vec::with_capacity(objects_len as usize);
            for index in 0..objects_len {
                let device = unsafe { device_ns_array.objectAtIndex(index) };
                devices.push(get_raw_device_info(
                    CameraIndex::Index(index as u32),
                    device,
                ));
            }

            devices
        }
    }

    pub struct AVCaptureDevice {
        inner: *mut Object,
        device: CameraInfo,
        locked: bool,
    }

    impl AVCaptureDevice {
        pub fn inner(&self) -> *mut Object {
            self.inner
        }
    }

    impl AVCaptureDevice {
        pub fn new(index: &CameraIndex) -> Result<Self, NokhwaError> {
            match &index {
                CameraIndex::Index(idx) => {
                    let devices = query_avfoundation()?;

                    match devices.get(*idx as usize) {
                        Some(device) => Ok(AVCaptureDevice::from_id(
                            &device.misc(),
                            Some(index.clone()),
                        )?),
                        None => Err(NokhwaError::OpenDeviceError(
                            idx.to_string(),
                            "Not Found".to_string(),
                        )),
                    }
                }
                CameraIndex::String(id) => Ok(AVCaptureDevice::from_id(id, None)?),
            }
        }

        pub fn from_id(id: &str, index_hint: Option<CameraIndex>) -> Result<Self, NokhwaError> {
            let nsstr_id = str_to_nsstr(id);
            let avfoundation_capture_cls = class!(AVCaptureDevice);
            let capture: *mut Object =
                unsafe { msg_send![avfoundation_capture_cls, deviceWithUniqueID: nsstr_id] };
            if capture.is_null() {
                return Err(NokhwaError::OpenDeviceError(
                    id.to_string(),
                    "Device is null".to_string(),
                ));
            }
            let camera_info = get_raw_device_info(
                index_hint.unwrap_or_else(|| CameraIndex::String(id.to_string())),
                capture,
            );

            Ok(AVCaptureDevice {
                inner: capture,
                device: camera_info,
                locked: false,
            })
        }

        pub fn info(&self) -> &CameraInfo {
            &self.device
        }

        pub fn supported_formats_raw(&self) -> Result<Vec<AVCaptureDeviceFormat>, NokhwaError> {
            try_ns_arr_to_vec::<AVCaptureDeviceFormat, NokhwaError>(unsafe {
                msg_send![self.inner, formats]
            })
        }

        pub fn supported_formats(&self) -> Result<Vec<CameraFormat>, NokhwaError> {
            Ok(self
                .supported_formats_raw()?
                .iter()
                .flat_map(|av_fmt| {
                    let resolution = av_fmt.resolution;
                    av_fmt.fps_list.iter().map(move |fps_f64| {
                        let fps = *fps_f64 as u32;

                        let resolution =
                            Resolution::new(resolution.width as u32, resolution.height as u32); // FIXME: what the fuck?
                        CameraFormat::new(resolution, av_fmt.fourcc, fps)
                    })
                })
                .filter(|x| x.frame_rate() != 0)
                .collect())
        }

        pub fn already_in_use(&self) -> bool {
            unsafe {
                let result: BOOL = msg_send![self.inner(), isInUseByAnotherApplication];
                result == YES
            }
        }

        pub fn is_suspended(&self) -> bool {
            unsafe {
                let result: BOOL = msg_send![self.inner, isSuspended];
                result == YES
            }
        }

        pub fn lock(&self) -> Result<(), NokhwaError> {
            if self.locked {
                return Ok(());
            }
            if self.already_in_use() {
                return Err(NokhwaError::InitializeError {
                    backend: ApiBackend::AVFoundation,
                    error: "Already in use".to_string(),
                });
            }
            let err_ptr: *mut c_void = std::ptr::null_mut();
            let accepted: BOOL = unsafe { msg_send![self.inner, lockForConfiguration: err_ptr] };
            if !err_ptr.is_null() {
                return Err(NokhwaError::SetPropertyError {
                    property: "lockForConfiguration".to_string(),
                    value: "Locked".to_string(),
                    error: "Cannot lock for configuration".to_string(),
                });
            }
            // Space these out for debug purposes
            if !accepted == YES {
                return Err(NokhwaError::SetPropertyError {
                    property: "lockForConfiguration".to_string(),
                    value: "Locked".to_string(),
                    error: "Lock Rejected".to_string(),
                });
            }
            Ok(())
        }

        pub fn unlock(&mut self) {
            if self.locked {
                self.locked = false;
                unsafe { msg_send![self.inner, unlockForConfiguration] }
            }
        }

        // thank you ffmpeg
        pub fn set_all(&mut self, descriptor: CameraFormat) -> Result<(), NokhwaError> {
            self.lock()?;
            let format_list = try_ns_arr_to_vec::<AVCaptureDeviceFormat, NokhwaError>(unsafe {
                msg_send![self.inner, formats]
            })?;
            let format_description_sel = sel!(formatDescription);

            let mut selected_format: *mut Object = std::ptr::null_mut();
            let mut selected_range: *mut Object = std::ptr::null_mut();

            for format in format_list {
                let format_desc_ref: CMFormatDescriptionRef =
                    unsafe { msg_send![format.internal, performSelector: format_description_sel] };
                let dimensions = unsafe { CMVideoFormatDescriptionGetDimensions(format_desc_ref) };

                if dimensions.height == descriptor.resolution().height() as i32
                    && dimensions.width == descriptor.resolution().width() as i32
                {
                    selected_format = format.internal;

                    for range in ns_arr_to_vec::<AVFrameRateRange>(unsafe {
                        msg_send![format.internal, videoSupportedFrameRateRanges]
                    }) {
                        let max_fps: f64 = unsafe { msg_send![range.inner, maxFrameRate] };
                        // Older Apple cameras (i.e. iMac 2013) return 29.97000002997 as FPS.
                        if (f64::from(descriptor.frame_rate()) - max_fps).abs() < 0.999 {
                            selected_range = range.inner;
                            break;
                        }
                    }
                }
            }
            if selected_range.is_null() || selected_format.is_null() {
                return Err(NokhwaError::SetPropertyError {
                    property: "CameraFormat".to_string(),
                    value: descriptor.to_string(),
                    error: "Not Found/Rejected/Unsupported".to_string(),
                });
            }

            let activefmtkey = str_to_nsstr("activeFormat");
            let _: () =
                unsafe { msg_send![self.inner, setValue:selected_format forKey:activefmtkey] };
            // Patched: skip activeVideoMin/MaxFrameDuration KVC calls. These throw
            // NSException on HDMI capture cards (e.g. MS2109) that don't support
            // frame-rate control via AVFoundation. The active format is already set
            // above; omitting the frame-duration setting is harmless for capture use.
            self.unlock();
            Ok(())
        }

        // 0 => Focus POI
        // 1 => Focus Manual Setting
        // 2 => Exposure POI
        // 3 => Exposure Face Driven
        // 4 => Exposure Target Bias
        // 5 => Exposure ISO
        // 6 => Exposure Duration
        pub fn get_controls(&self) -> Result<Vec<CameraControl>, NokhwaError> {
            let active_format: *mut Object = unsafe { msg_send![self.inner, activeFormat] };

            let mut controls = vec![];
            // get focus modes

            let focus_current: NSInteger = unsafe { msg_send![self.inner, focusMode] };
            let focus_locked: BOOL =
                unsafe { msg_send![self.inner, isFocusModeSupported:NSInteger::from(0)] };
            let focus_auto: BOOL =
                unsafe { msg_send![self.inner, isFocusModeSupported:NSInteger::from(1)] };
            let focus_continuous: BOOL =
                unsafe { msg_send![self.inner, isFocusModeSupported:NSInteger::from(2)] };

            {
                let mut supported_focus_values = vec![];

                if focus_locked == YES {
                    supported_focus_values.push(0)
                }
                if focus_auto == YES {
                    supported_focus_values.push(1)
                }
                if focus_continuous == YES {
                    supported_focus_values.push(2)
                }

                controls.push(CameraControl::new(
                    KnownCameraControl::Focus,
                    "FocusMode".to_string(),
                    ControlValueDescription::Enum {
                        value: focus_current,
                        possible: supported_focus_values,
                        default: focus_current,
                    },
                    vec![],
                    true,
                ));
            }

            let focus_poi_supported: BOOL =
                unsafe { msg_send![self.inner, isFocusPointOfInterestSupported] };
            let focus_poi: CGPoint = unsafe { msg_send![self.inner, focusPointOfInterest] };

            controls.push(CameraControl::new(
                KnownCameraControl::Other(0),
                "FocusPointOfInterest".to_string(),
                ControlValueDescription::Point {
                    value: (focus_poi.x as f64, focus_poi.y as f64),
                    default: (0.5, 0.5),
                },
                if focus_poi_supported == NO {
                    vec![
                        KnownCameraControlFlag::Disabled,
                        KnownCameraControlFlag::ReadOnly,
                    ]
                } else {
                    vec![]
                },
                focus_auto == YES || focus_continuous == YES,
            ));

            let focus_manual: BOOL =
                unsafe { msg_send![self.inner, isLockingFocusWithCustomLensPositionSupported] };
            let focus_lenspos: f32 = unsafe { msg_send![self.inner, lensPosition] };

            controls.push(CameraControl::new(
                KnownCameraControl::Other(1),
                "FocusManualLensPosition".to_string(),
                ControlValueDescription::FloatRange {
                    min: 0.0,
                    max: 1.0,
                    value: focus_lenspos as f64,
                    step: f64::MIN_POSITIVE,
                    default: 1.0,
                },
                if focus_manual == YES {
                    vec![]
                } else {
                    vec![
                        KnownCameraControlFlag::Disabled,
                        KnownCameraControlFlag::ReadOnly,
                    ]
                },
                focus_manual == YES,
            ));

            // get exposures
            let exposure_current: NSInteger = unsafe { msg_send![self.inner, exposureMode] };
            let exposure_locked: BOOL =
                unsafe { msg_send![self.inner, isExposureModeSupported:NSInteger::from(0)] };
            let exposure_auto: BOOL =
                unsafe { msg_send![self.inner, isExposureModeSupported:NSInteger::from(1)] };
            let exposure_continuous: BOOL =
                unsafe { msg_send![self.inner, isExposureModeSupported:NSInteger::from(2)] };
            let exposure_custom: BOOL =
                unsafe { msg_send![self.inner, isExposureModeSupported:NSInteger::from(3)] };

            {
                let mut supported_exposure_values = vec![];

                if exposure_locked == YES {
                    supported_exposure_values.push(0);
                }
                if exposure_auto == YES {
                    supported_exposure_values.push(1);
                }
                if exposure_continuous == YES {
                    supported_exposure_values.push(2);
                }
                if exposure_custom == YES {
                    supported_exposure_values.push(3);
                }

                controls.push(CameraControl::new(
                    KnownCameraControl::Exposure,
                    "ExposureMode".to_string(),
                    ControlValueDescription::Enum {
                        value: exposure_current,
                        possible: supported_exposure_values,
                        default: exposure_current,
                    },
                    vec![],
                    true,
                ));
            }

            let exposure_poi_supported: BOOL =
                unsafe { msg_send![self.inner, isExposurePointOfInterestSupported] };
            let exposure_poi: CGPoint = unsafe { msg_send![self.inner, exposurePointOfInterest] };

            controls.push(CameraControl::new(
                KnownCameraControl::Other(2),
                "ExposurePointOfInterest".to_string(),
                ControlValueDescription::Point {
                    value: (exposure_poi.x as f64, exposure_poi.y as f64),
                    default: (0.5, 0.5),
                },
                if exposure_poi_supported == NO {
                    vec![
                        KnownCameraControlFlag::Disabled,
                        KnownCameraControlFlag::ReadOnly,
                    ]
                } else {
                    vec![]
                },
                focus_auto == YES || focus_continuous == YES,
            ));

            let expposure_face_driven_supported: BOOL =
                unsafe { msg_send![self.inner, isFaceDrivenAutoExposureEnabled] };
            let exposure_face_driven: BOOL = unsafe {
                msg_send![
                    self.inner,
                    automaticallyAdjustsFaceDrivenAutoExposureEnabled
                ]
            };

            controls.push(CameraControl::new(
                KnownCameraControl::Other(3),
                "ExposureFaceDriven".to_string(),
                ControlValueDescription::Boolean {
                    value: exposure_face_driven == YES,
                    default: false,
                },
                if expposure_face_driven_supported == NO {
                    vec![
                        KnownCameraControlFlag::Disabled,
                        KnownCameraControlFlag::ReadOnly,
                    ]
                } else {
                    vec![]
                },
                exposure_poi_supported == YES,
            ));

            let exposure_bias: f32 = unsafe { msg_send![self.inner, exposureTargetBias] };
            let exposure_bias_min: f32 = unsafe { msg_send![self.inner, minExposureTargetBias] };
            let exposure_bias_max: f32 = unsafe { msg_send![self.inner, maxExposureTargetBias] };

            controls.push(CameraControl::new(
                KnownCameraControl::Other(4),
                "ExposureBiasTarget".to_string(),
                ControlValueDescription::FloatRange {
                    min: exposure_bias_min as f64,
                    max: exposure_bias_max as f64,
                    value: exposure_bias as f64,
                    step: f32::MIN_POSITIVE as f64,
                    default: unsafe { AVCaptureExposureTargetBiasCurrent } as f64,
                },
                vec![],
                true,
            ));

            let exposure_duration: CMTime = unsafe { msg_send![self.inner, exposureDuration] };
            let exposure_duration_min: CMTime =
                unsafe { msg_send![active_format, minExposureDuration] };
            let exposure_duration_max: CMTime =
                unsafe { msg_send![active_format, maxExposureDuration] };

            controls.push(CameraControl::new(
                KnownCameraControl::Gamma,
                "ExposureDuration".to_string(),
                ControlValueDescription::IntegerRange {
                    min: exposure_duration_min.value,
                    max: exposure_duration_max.value,
                    value: exposure_duration.value,
                    step: 1,
                    default: unsafe { AVCaptureExposureDurationCurrent.value },
                },
                if exposure_custom == YES {
                    vec![
                        KnownCameraControlFlag::ReadOnly,
                        KnownCameraControlFlag::Volatile,
                    ]
                } else {
                    vec![KnownCameraControlFlag::Volatile]
                },
                exposure_custom == YES,
            ));

            let exposure_iso: f32 = unsafe { msg_send![self.inner, ISO] };
            let exposure_iso_min: f32 = unsafe { msg_send![active_format, minISO] };
            let exposure_iso_max: f32 = unsafe { msg_send![active_format, maxISO] };

            controls.push(CameraControl::new(
                KnownCameraControl::Brightness,
                "ExposureISO".to_string(),
                ControlValueDescription::FloatRange {
                    min: exposure_iso_min as f64,
                    max: exposure_iso_max as f64,
                    value: exposure_iso as f64,
                    step: f32::MIN_POSITIVE as f64,
                    default: unsafe { AVCaptureISOCurrent } as f64,
                },
                if exposure_custom == YES {
                    vec![
                        KnownCameraControlFlag::ReadOnly,
                        KnownCameraControlFlag::Volatile,
                    ]
                } else {
                    vec![KnownCameraControlFlag::Volatile]
                },
                exposure_custom == YES,
            ));

            let lens_aperture: f32 = unsafe { msg_send![self.inner, lensAperture] };

            controls.push(CameraControl::new(
                KnownCameraControl::Iris,
                "LensAperture".to_string(),
                ControlValueDescription::Float {
                    value: lens_aperture as f64,
                    default: lens_aperture as f64,
                    step: lens_aperture as f64,
                },
                vec![KnownCameraControlFlag::ReadOnly],
                false,
            ));

            // get whiteblaance
            let white_balance_current: NSInteger =
                unsafe { msg_send![self.inner, whiteBalanceMode] };
            let white_balance_manual: BOOL =
                unsafe { msg_send![self.inner, isWhiteBalanceModeSupported:NSInteger::from(0)] };
            let white_balance_auto: BOOL =
                unsafe { msg_send![self.inner, isWhiteBalanceModeSupported:NSInteger::from(1)] };
            let white_balance_continuous: BOOL =
                unsafe { msg_send![self.inner, isWhiteBalanceModeSupported:NSInteger::from(2)] };

            {
                let mut possible = vec![];

                if white_balance_manual == YES {
                    possible.push(0);
                }
                if white_balance_auto == YES {
                    possible.push(1);
                }
                if white_balance_continuous == YES {
                    possible.push(2);
                }

                controls.push(CameraControl::new(
                    KnownCameraControl::WhiteBalance,
                    "WhiteBalanceMode".to_string(),
                    ControlValueDescription::Enum {
                        value: white_balance_current as i64,
                        possible,
                        default: 0,
                    },
                    vec![],
                    true,
                ));
            }

            let white_balance_gains: AVCaptureWhiteBalanceGains =
                unsafe { msg_send![self.inner, deviceWhiteBalanceGains] };
            let white_balance_default: AVCaptureWhiteBalanceGains =
                unsafe { msg_send![self.inner, grayWorldDeviceWhiteBalanceGains] };
            let white_balancne_max: AVCaptureWhiteBalanceGains =
                unsafe { msg_send![self.inner, maxWhiteBalanceGain] };
            let white_balance_gain_supported: BOOL = unsafe {
                msg_send![
                    self.inner,
                    isLockingWhiteBalanceWithCustomDeviceGainsSupported
                ]
            };

            controls.push(CameraControl::new(
                KnownCameraControl::Gain,
                "WhiteBalanceGain".to_string(),
                ControlValueDescription::RGB {
                    value: (
                        white_balance_gains.redGain as f64,
                        white_balance_gains.greenGain as f64,
                        white_balance_gains.blueGain as f64,
                    ),
                    max: (
                        white_balancne_max.redGain as f64,
                        white_balancne_max.greenGain as f64,
                        white_balancne_max.blueGain as f64,
                    ),
                    default: (
                        white_balance_default.redGain as f64,
                        white_balance_default.greenGain as f64,
                        white_balance_default.blueGain as f64,
                    ),
                },
                if white_balance_gain_supported == YES {
                    vec![
                        KnownCameraControlFlag::Disabled,
                        KnownCameraControlFlag::ReadOnly,
                    ]
                } else {
                    vec![]
                },
                white_balance_gain_supported == YES,
            ));

            // get flash
            let has_torch: BOOL = unsafe { msg_send![self.inner, isTorchAvailable] };
            let torch_active: BOOL = unsafe { msg_send![self.inner, isTorchActive] };
            let torch_off: BOOL =
                unsafe { msg_send![self.inner, isTorchModeSupported:NSInteger::from(0)] };
            let torch_on: BOOL =
                unsafe { msg_send![self.inner, isTorchModeSupported:NSInteger::from(1)] };
            let torch_auto: BOOL =
                unsafe { msg_send![self.inner, isTorchModeSupported:NSInteger::from(2)] };

            {
                let mut possible = vec![];

                if torch_off == YES {
                    possible.push(0);
                }
                if torch_on == YES {
                    possible.push(1);
                }
                if torch_auto == YES {
                    possible.push(2);
                }

                controls.push(CameraControl::new(
                    KnownCameraControl::Other(5),
                    "TorchMode".to_string(),
                    ControlValueDescription::Enum {
                        value: (torch_active == YES) as i64,
                        possible,
                        default: 0,
                    },
                    if has_torch == YES {
                        vec![
                            KnownCameraControlFlag::Disabled,
                            KnownCameraControlFlag::ReadOnly,
                        ]
                    } else {
                        vec![]
                    },
                    has_torch == YES,
                ));
            }

            // get low light boost
            let has_llb: BOOL = unsafe { msg_send![self.inner, isLowLightBoostSupported] };
            let llb_enabled: BOOL = unsafe { msg_send![self.inner, isLowLightBoostEnabled] };

            {
                controls.push(CameraControl::new(
                    KnownCameraControl::BacklightComp,
                    "LowLightCompensation".to_string(),
                    ControlValueDescription::Boolean {
                        value: llb_enabled == YES,
                        default: false,
                    },
                    if has_llb == NO {
                        vec![
                            KnownCameraControlFlag::Disabled,
                            KnownCameraControlFlag::ReadOnly,
                        ]
                    } else {
                        vec![]
                    },
                    has_llb == YES,
                ));
            }

            // get zoom factor
            let zoom_current: CGFloat = unsafe { msg_send![self.inner, videoZoomFactor] };
            let zoom_min: CGFloat = unsafe { msg_send![self.inner, minAvailableVideoZoomFactor] };
            let zoom_max: CGFloat = unsafe { msg_send![self.inner, maxAvailableVideoZoomFactor] };

            controls.push(CameraControl::new(
                KnownCameraControl::Zoom,
                "Zoom".to_string(),
                ControlValueDescription::FloatRange {
                    min: zoom_min as f64,
                    max: zoom_max as f64,
                    value: zoom_current as f64,
                    step: f32::MIN_POSITIVE as f64,
                    default: 1.0,
                },
                vec![],
                true,
            ));

            // zoom distortion correction
            let distortion_correction_supported: BOOL =
                unsafe { msg_send![self.inner, isGeometricDistortionCorrectionSupported] };
            let distortion_correction_current_value: BOOL =
                unsafe { msg_send![self.inner, isGeometricDistortionCorrectionEnabled] };

            controls.push(CameraControl::new(
                KnownCameraControl::Other(6),
                "DistortionCorrection".to_string(),
                ControlValueDescription::Boolean {
                    value: distortion_correction_current_value == YES,
                    default: false,
                },
                if distortion_correction_supported == YES {
                    vec![
                        KnownCameraControlFlag::ReadOnly,
                        KnownCameraControlFlag::Disabled,
                    ]
                } else {
                    vec![]
                },
                distortion_correction_supported == YES,
            ));

            Ok(controls)
        }

        pub fn set_control(
            &mut self,
            id: KnownCameraControl,
            value: ControlValueSetter,
        ) -> Result<(), NokhwaError> {
            let rc = self.get_controls()?;
            let controls = rc
                .iter()
                .map(|cc| (cc.control(), cc))
                .collect::<BTreeMap<_, _>>();

            match id {
                KnownCameraControl::Brightness => {
                    let isoctrl = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                        property: id.to_string(),
                        value: value.to_string(),
                        error: "Control does not exist".to_string(),
                    })?;

                    if isoctrl.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error:
                                "Exposure is in improper state to set ISO (Please set to `custom`!)"
                                    .to_string(),
                        });
                    }

                    if isoctrl.flag().contains(&KnownCameraControlFlag::Disabled) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Disabled".to_string(),
                        });
                    }

                    let current_duration = unsafe { AVCaptureExposureDurationCurrent };
                    let new_iso = *value.as_float().ok_or(NokhwaError::SetPropertyError {
                        property: id.to_string(),
                        value: value.to_string(),
                        error: "Expected float".to_string(),
                    })? as f32;

                    if !isoctrl.description().verify_setter(&value) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Failed to verify value".to_string(),
                        });
                    }

                    let _: () = unsafe {
                        msg_send![self.inner, setExposureModeCustomWithDuration:current_duration ISO:new_iso completionHandler:Nil]
                    };

                    Ok(())
                }
                KnownCameraControl::Gamma => {
                    let duration_ctrl = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                        property: id.to_string(),
                        value: value.to_string(),
                        error: "Control does not exist".to_string(),
                    })?;

                    if duration_ctrl
                        .flag()
                        .contains(&KnownCameraControlFlag::ReadOnly)
                    {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Exposure is in improper state to set Duration (Please set to `custom`!)"
                                .to_string(),
                        });
                    }

                    if duration_ctrl
                        .flag()
                        .contains(&KnownCameraControlFlag::Disabled)
                    {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Disabled".to_string(),
                        });
                    }
                    let current_duration: CMTime =
                        unsafe { msg_send![self.inner, exposureDuration] };

                    let current_iso = unsafe { AVCaptureISOCurrent };
                    let new_duration = CMTime {
                        value: *value.as_integer().ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Expected i64".to_string(),
                        })?,
                        timescale: current_duration.timescale,
                        flags: current_duration.flags,
                        epoch: current_duration.epoch,
                    };

                    if !duration_ctrl.description().verify_setter(&value) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Failed to verify value".to_string(),
                        });
                    }

                    let _: () = unsafe {
                        msg_send![self.inner, setExposureModeCustomWithDuration:new_duration ISO:current_iso completionHandler:Nil]
                    };

                    Ok(())
                }
                KnownCameraControl::WhiteBalance => {
                    let wb_enum_value = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                        property: id.to_string(),
                        value: value.to_string(),
                        error: "Control does not exist".to_string(),
                    })?;

                    if wb_enum_value
                        .flag()
                        .contains(&KnownCameraControlFlag::ReadOnly)
                    {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Read Only".to_string(),
                        });
                    }

                    if wb_enum_value
                        .flag()
                        .contains(&KnownCameraControlFlag::Disabled)
                    {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Disabled".to_string(),
                        });
                    }
                    let setter =
                        NSInteger::from(*value.as_enum().ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Expected Enum".to_string(),
                        })? as i32);

                    if !wb_enum_value.description().verify_setter(&value) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Failed to verify value".to_string(),
                        });
                    }

                    let _: () = unsafe { msg_send![self.inner, whiteBalanceMode: setter] };

                    Ok(())
                }
                KnownCameraControl::BacklightComp => {
                    let ctrlvalue = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                        property: id.to_string(),
                        value: value.to_string(),
                        error: "Control does not exist".to_string(),
                    })?;

                    if ctrlvalue.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Read Only".to_string(),
                        });
                    }

                    if ctrlvalue.flag().contains(&KnownCameraControlFlag::Disabled) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Disabled".to_string(),
                        });
                    }

                    let setter =
                        NSInteger::from(*value.as_enum().ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Expected Enum".to_string(),
                        })? as i32);

                    if !ctrlvalue.description().verify_setter(&value) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Failed to verify value".to_string(),
                        });
                    }

                    let _: () = unsafe { msg_send![self.inner, whiteBalanceMode: setter] };

                    Ok(())
                }
                KnownCameraControl::Gain => {
                    let ctrlvalue = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                        property: id.to_string(),
                        value: value.to_string(),
                        error: "Control does not exist".to_string(),
                    })?;

                    if ctrlvalue.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Read Only".to_string(),
                        });
                    }

                    if ctrlvalue.flag().contains(&KnownCameraControlFlag::Disabled) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Disabled".to_string(),
                        });
                    }

                    let setter = NSInteger::from(*value.as_boolean().ok_or(
                        NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Expected Boolean".to_string(),
                        },
                    )? as i32);

                    if !ctrlvalue.description().verify_setter(&value) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Failed to verify value".to_string(),
                        });
                    }

                    let _: () = unsafe { msg_send![self.inner, whiteBalanceMode: setter] };

                    Ok(())
                }
                KnownCameraControl::Zoom => {
                    let ctrlvalue = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                        property: id.to_string(),
                        value: value.to_string(),
                        error: "Control does not exist".to_string(),
                    })?;

                    if ctrlvalue.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Read Only".to_string(),
                        });
                    }

                    if ctrlvalue.flag().contains(&KnownCameraControlFlag::Disabled) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Disabled".to_string(),
                        });
                    }

                    let setter = *value.as_float().ok_or(NokhwaError::SetPropertyError {
                        property: id.to_string(),
                        value: value.to_string(),
                        error: "Expected float".to_string(),
                    })? as c_float;

                    if !ctrlvalue.description().verify_setter(&value) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Failed to verify value".to_string(),
                        });
                    }

                    let _: () = unsafe {
                        msg_send![self.inner, rampToVideoZoomFactor: setter withRate: 1.0_f32]
                    };

                    Ok(())
                }
                KnownCameraControl::Exposure => {
                    let ctrlvalue = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                        property: id.to_string(),
                        value: value.to_string(),
                        error: "Control does not exist".to_string(),
                    })?;

                    if ctrlvalue.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Read Only".to_string(),
                        });
                    }

                    if ctrlvalue.flag().contains(&KnownCameraControlFlag::Disabled) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Disabled".to_string(),
                        });
                    }

                    let setter =
                        NSInteger::from(*value.as_enum().ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Expected Enum".to_string(),
                        })? as i32);

                    if !ctrlvalue.description().verify_setter(&value) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Failed to verify value".to_string(),
                        });
                    }

                    let _: () = unsafe { msg_send![self.inner, exposureMode: setter] };

                    Ok(())
                }
                KnownCameraControl::Iris => Err(NokhwaError::SetPropertyError {
                    property: id.to_string(),
                    value: value.to_string(),
                    error: "Read Only".to_string(),
                }),
                KnownCameraControl::Focus => {
                    let ctrlvalue = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                        property: id.to_string(),
                        value: value.to_string(),
                        error: "Control does not exist".to_string(),
                    })?;

                    if ctrlvalue.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Read Only".to_string(),
                        });
                    }

                    if ctrlvalue.flag().contains(&KnownCameraControlFlag::Disabled) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Disabled".to_string(),
                        });
                    }

                    let setter =
                        NSInteger::from(*value.as_enum().ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Expected Enum".to_string(),
                        })? as i32);

                    if !ctrlvalue.description().verify_setter(&value) {
                        return Err(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Failed to verify value".to_string(),
                        });
                    }

                    let _: () = unsafe { msg_send![self.inner, focusMode: setter] };

                    Ok(())
                }
                KnownCameraControl::Other(i) => match i {
                    0 => {
                        let ctrlvalue = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Control does not exist".to_string(),
                        })?;

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Read Only".to_string(),
                            });
                        }

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::Disabled) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Disabled".to_string(),
                            });
                        }

                        let setter = value
                            .as_point()
                            .ok_or(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Expected Point".to_string(),
                            })
                            .map(|(x, y)| CGPoint {
                                x: *x as f32,
                                y: *y as f32,
                            })?;

                        if !ctrlvalue.description().verify_setter(&value) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Failed to verify value".to_string(),
                            });
                        }

                        let _: () = unsafe { msg_send![self.inner, focusPointOfInterest: setter] };

                        Ok(())
                    }
                    1 => {
                        let ctrlvalue = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Control does not exist".to_string(),
                        })?;

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Read Only".to_string(),
                            });
                        }

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::Disabled) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Disabled".to_string(),
                            });
                        }

                        let setter = *value.as_float().ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Expected float".to_string(),
                        })? as c_float;

                        if !ctrlvalue.description().verify_setter(&value) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Failed to verify value".to_string(),
                            });
                        }

                        let _: () = unsafe {
                            msg_send![self.inner, setFocusModeLockedWithLensPosition: setter handler: Nil]
                        };

                        Ok(())
                    }
                    2 => {
                        let ctrlvalue = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Control does not exist".to_string(),
                        })?;

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Read Only".to_string(),
                            });
                        }

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::Disabled) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Disabled".to_string(),
                            });
                        }

                        let setter = value
                            .as_point()
                            .ok_or(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Expected Point".to_string(),
                            })
                            .map(|(x, y)| CGPoint {
                                x: *x as f32,
                                y: *y as f32,
                            })?;

                        if !ctrlvalue.description().verify_setter(&value) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Failed to verify value".to_string(),
                            });
                        }

                        let _: () =
                            unsafe { msg_send![self.inner, exposurePointOfInterest: setter] };

                        Ok(())
                    }
                    3 => {
                        let ctrlvalue = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Control does not exist".to_string(),
                        })?;

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Read Only".to_string(),
                            });
                        }

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::Disabled) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Disabled".to_string(),
                            });
                        }

                        let setter =
                            if *value.as_boolean().ok_or(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Expected Boolean".to_string(),
                            })? {
                                YES
                            } else {
                                NO
                            };

                        if !ctrlvalue.description().verify_setter(&value) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Failed to verify value".to_string(),
                            });
                        }

                        let _: () = unsafe {
                            msg_send![
                                self.inner,
                                automaticallyAdjustsFaceDrivenAutoExposureEnabled: setter
                            ]
                        };

                        Ok(())
                    }
                    4 => {
                        let ctrlvalue = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Control does not exist".to_string(),
                        })?;

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Read Only".to_string(),
                            });
                        }

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::Disabled) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Disabled".to_string(),
                            });
                        }

                        let setter = *value.as_float().ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Expected Float".to_string(),
                        })? as f32;

                        if !ctrlvalue.description().verify_setter(&value) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Failed to verify value".to_string(),
                            });
                        }

                        let _: () = unsafe {
                            msg_send![self.inner, setExposureTargetBias: setter handler: Nil]
                        };

                        Ok(())
                    }
                    5 => {
                        let ctrlvalue = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Control does not exist".to_string(),
                        })?;

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Read Only".to_string(),
                            });
                        }

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::Disabled) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Disabled".to_string(),
                            });
                        }

                        let setter = NSInteger::from(*value.as_enum().ok_or(
                            NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Expected Enum".to_string(),
                            },
                        )? as i32);

                        if !ctrlvalue.description().verify_setter(&value) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Failed to verify value".to_string(),
                            });
                        }

                        let _: () = unsafe { msg_send![self.inner, torchMode: setter] };

                        Ok(())
                    }
                    6 => {
                        let ctrlvalue = controls.get(&id).ok_or(NokhwaError::SetPropertyError {
                            property: id.to_string(),
                            value: value.to_string(),
                            error: "Control does not exist".to_string(),
                        })?;

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::ReadOnly) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Read Only".to_string(),
                            });
                        }

                        if ctrlvalue.flag().contains(&KnownCameraControlFlag::Disabled) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Disabled".to_string(),
                            });
                        }

                        let setter =
                            if *value.as_boolean().ok_or(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Expected Boolean".to_string(),
                            })? {
                                YES
                            } else {
                                NO
                            };

                        if !ctrlvalue.description().verify_setter(&value) {
                            return Err(NokhwaError::SetPropertyError {
                                property: id.to_string(),
                                value: value.to_string(),
                                error: "Failed to verify value".to_string(),
                            });
                        }

                        let _: () = unsafe {
                            msg_send![self.inner, geometricDistortionCorrectionEnabled: setter]
                        };

                        Ok(())
                    }
                    _ => Err(NokhwaError::SetPropertyError {
                        property: id.to_string(),
                        value: value.to_string(),
                        error: "Unknown Control".to_string(),
                    }),
                },
                _ => Err(NokhwaError::SetPropertyError {
                    property: id.to_string(),
                    value: value.to_string(),
                    error: "Unknown Control".to_string(),
                }),
            }
        }

        pub fn active_format(&self) -> Result<CameraFormat, NokhwaError> {
            let af: *mut Object = unsafe { msg_send![self.inner, activeFormat] };
            let avf_format = AVCaptureDeviceFormat::try_from(af)?;
            let resolution = avf_format.resolution;
            let fourcc = avf_format.fourcc;
            let mut a = avf_format
                .fps_list
                .into_iter()
                .map(move |fps_f64| {
                    let fps = fps_f64 as u32;

                    let resolution =
                        Resolution::new(resolution.width as u32, resolution.height as u32); // FIXME: what the fuck?
                    CameraFormat::new(resolution, fourcc, fps)
                })
                .collect::<Vec<_>>();
            a.sort_by(|a, b| a.frame_rate().cmp(&b.frame_rate()));

            if a.len() != 0 {
                Ok(a[a.len() - 1])
            } else {
                Err(NokhwaError::GetPropertyError {
                    property: "activeFormat".to_string(),
                    error: "None??".to_string(),
                })
            }
        }
    }

    impl AVCaptureDeviceInput {
        pub fn new(capture_device: &AVCaptureDevice) -> Result<Self, NokhwaError> {
            let cls = class!(AVCaptureDeviceInput);
            let err_ptr: *mut c_void = std::ptr::null_mut();
            let capture_input: *mut Object = unsafe {
                let allocated: *mut Object = msg_send![cls, alloc];
                msg_send![allocated, initWithDevice:capture_device.inner() error:err_ptr]
            };
            if !err_ptr.is_null() {
                return Err(NokhwaError::InitializeError {
                    backend: ApiBackend::AVFoundation,
                    error: "Failed to create input".to_string(),
                });
            }

            Ok(AVCaptureDeviceInput {
                inner: capture_input,
            })
        }
    }

    pub struct AVCaptureVideoDataOutput {
        inner: *mut Object,
    }

    impl AVCaptureVideoDataOutput {
        pub fn new() -> Self {
            AVCaptureVideoDataOutput::default()
        }

        pub fn add_delegate(&self, delegate: &AVCaptureVideoCallback) -> Result<(), NokhwaError> {
            unsafe {
                let _: () = msg_send![
                    self.inner,
                    setSampleBufferDelegate: delegate.delegate
                    queue: delegate.queue().0
                ];
            };
            Ok(())
        }

        pub fn set_frame_format(&self, format: FrameFormat) -> Result<(), NokhwaError> {
            let cmpixelfmt = match format {
                FrameFormat::YUYV => kCMPixelFormat_422YpCbCr8_yuvs,
                FrameFormat::MJPEG => kCMVideoCodecType_JPEG,
                FrameFormat::GRAY => kCMPixelFormat_8IndexedGray_WhiteIsZero,
                FrameFormat::NV12 => kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange,
                FrameFormat::RAWRGB => kCMPixelFormat_24RGB,
                FrameFormat::RAWBGR => {
                    return Err(NokhwaError::SetPropertyError {
                        property: "setVideoSettings".to_string(),
                        value: "set frame format".to_string(),
                        error: "Unsupported frame format BGR".to_string(),
                    });
                }
            };
            let obj = CFNumber::from(cmpixelfmt as i32);
            let obj = obj.as_CFTypeRef() as *mut Object;
            let key = unsafe { kCVPixelBufferPixelFormatTypeKey } as *mut Object;
            let dict = unsafe { NSDictionary::dictionaryWithObject_forKey_(nil, obj, key) };
            let _: () = unsafe { msg_send![self.inner, setVideoSettings:dict] };
            Ok(())
        }
    }

    use cocoa_foundation::base::nil;
    use core_foundation::base::TCFType;
    use core_foundation::number::CFNumber;
    use core_video_sys::kCVPixelBufferPixelFormatTypeKey;
    impl Default for AVCaptureVideoDataOutput {
        fn default() -> Self {
            let cls = class!(AVCaptureVideoDataOutput);
            let inner: *mut Object = unsafe { msg_send![cls, new] };

            AVCaptureVideoDataOutput { inner }
        }
    }

    impl AVCaptureSession {
        pub fn new() -> Self {
            AVCaptureSession::default()
        }

        pub fn begin_configuration(&self) {
            unsafe { msg_send![self.inner, beginConfiguration] }
        }

        pub fn commit_configuration(&self) {
            unsafe { msg_send![self.inner, commitConfiguration] }
        }

        pub fn can_add_input(&self, input: &AVCaptureDeviceInput) -> bool {
            let result: BOOL = unsafe { msg_send![self.inner, canAddInput:input.inner] };
            result == YES
        }

        pub fn add_input(&self, input: &AVCaptureDeviceInput) -> Result<(), NokhwaError> {
            if self.can_add_input(input) {
                let _: () = unsafe { msg_send![self.inner, addInput:input.inner] };
                return Ok(());
            }
            Err(NokhwaError::SetPropertyError {
                property: "AVCaptureDeviceInput".to_string(),
                value: "add new input".to_string(),
                error: "Rejected".to_string(),
            })
        }

        pub fn remove_input(&self, input: &AVCaptureDeviceInput) {
            unsafe { msg_send![self.inner, removeInput:input.inner] }
        }

        pub fn can_add_output(&self, output: &AVCaptureVideoDataOutput) -> bool {
            let result: BOOL = unsafe { msg_send![self.inner, canAddOutput:output.inner] };
            result == YES
        }

        pub fn add_output(&self, output: &AVCaptureVideoDataOutput) -> Result<(), NokhwaError> {
            if self.can_add_output(output) {
                let _: () = unsafe { msg_send![self.inner, addOutput:output.inner] };
                return Ok(());
            }
            Err(NokhwaError::SetPropertyError {
                property: "AVCaptureVideoDataOutput".to_string(),
                value: "add new output".to_string(),
                error: "Rejected".to_string(),
            })
        }

        pub fn remove_output(&self, output: &AVCaptureVideoDataOutput) {
            unsafe { msg_send![self.inner, removeOutput:output.inner] }
        }

        pub fn is_running(&self) -> bool {
            let running: BOOL = unsafe { msg_send![self.inner, isRunning] };
            running == YES
        }

        pub fn start(&self) -> Result<(), NokhwaError> {
            let start_stream_fn = || {
                let _: () = unsafe { msg_send![self.inner, startRunning] };
            };

            if std::panic::catch_unwind(start_stream_fn).is_err() {
                return Err(NokhwaError::OpenStreamError(
                    "Cannot run AVCaptureSession".to_string(),
                ));
            }
            Ok(())
        }

        pub fn stop(&self) {
            unsafe { msg_send![self.inner, stopRunning] }
        }

        pub fn is_interrupted(&self) -> bool {
            let interrupted: BOOL = unsafe { msg_send![self.inner, isInterrupted] };
            interrupted == YES
        }
    }

    impl Default for AVCaptureSession {
        fn default() -> Self {
            let cls = class!(AVCaptureSession);
            let session: *mut Object = {
                let alloc: *mut Object = unsafe { msg_send![cls, alloc] };
                unsafe { msg_send![alloc, init] }
            };
            AVCaptureSession { inner: session }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use crate::internal::*;
````

## File: hdmicap/vendor/nokhwa-bindings-macos/.cargo_vcs_info.json
````json
{
  "git": {
    "sha1": "2ea0883136a9d28361a4a72c0afbeb08b6d130c9"
  },
  "path_in_vcs": "nokhwa-bindings-macos"
}
````

## File: hdmicap/vendor/nokhwa-bindings-macos/.cargo-ok
````
{"v":1}
````

## File: hdmicap/vendor/nokhwa-bindings-macos/.gitignore
````
### JetBrains template
# Covers JetBrains IDEs: IntelliJ, RubyMine, PhpStorm, AppCode, PyCharm, CLion, Android Studio, WebStorm and Rider
# Reference: https://intellij-support.jetbrains.com/hc/en-us/articles/206544839

# User-specific stuff
.idea/**/workspace.xml
.idea/**/tasks.xml
.idea/**/usage.statistics.xml
.idea/**/dictionaries
.idea/**/shelf

# Generated files
.idea/**/contentModel.xml

# Sensitive or high-churn files
.idea/**/dataSources/
.idea/**/dataSources.ids
.idea/**/dataSources.local.xml
.idea/**/sqlDataSources.xml
.idea/**/dynamic.xml
.idea/**/uiDesigner.xml
.idea/**/dbnavigator.xml

# Gradle
.idea/**/gradle.xml
.idea/**/libraries

# Gradle and Maven with auto-import
# When using Gradle or Maven with auto-import, you should exclude module files,
# since they will be recreated, and may cause churn.  Uncomment if using
# auto-import.
# .idea/artifacts
# .idea/compiler.xml
# .idea/jarRepositories.xml
# .idea/modules.xml
.idea/*.iml
# .idea/modules
*.iml
# *.ipr

# CMake
cmake-build-*/

# Mongo Explorer plugin
.idea/**/mongoSettings.xml

# File-based project format
*.iws

# IntelliJ
out/

# mpeltonen/sbt-idea plugin
.idea_modules/

# JIRA plugin
atlassian-ide-plugin.xml

nokhwa-bindings-macos.iml
# Cursive Clojure plugin
.idea/replstate.xml

# Crashlytics plugin (for Android Studio and IntelliJ)
com_crashlytics_export_strings.xml
crashlytics.properties
crashlytics-build.properties
fabric.properties

# Editor-based Rest Client
.idea/httpRequests

# Android studio 3.1+ serialized cache file
.idea/caches/build_file_checksums.ser

### Rust template
# Generated by Cargo
# will have compiled files and executables
debug/
target/

# Remove Cargo.lock from gitignore if creating an executable, leave it for libraries
# More information here https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html
Cargo.lock

# These are backup files generated by rustfmt
**/*.rs.bk
````

## File: hdmicap/vendor/nokhwa-bindings-macos/build.rs
````rust
/*
 * Copyright 2022 l1npengtul <l1npengtul@protonmail.com> / The Nokhwa Contributors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    if target_os == "macos" || target_os == "ios" {
        println!("cargo:rustc-link-lib=framework=CoreMedia");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=CoreVideo");
    }
}
````

## File: hdmicap/vendor/nokhwa-bindings-macos/Cargo.toml
````toml
# THIS FILE IS AUTOMATICALLY GENERATED BY CARGO
#
# When uploading crates to the registry Cargo will automatically
# "normalize" Cargo.toml files for maximal compatibility
# with all versions of Cargo and also rewrite `path` dependencies
# to registry (e.g., crates.io) dependencies.
#
# If you are reading this file be aware that the original Cargo.toml
# will likely look very different (and much more reasonable).
# See Cargo.toml.orig for the original contents.

[package]
edition = "2021"
name = "nokhwa-bindings-macos"
version = "0.2.4"
authors = ["l1npengtul"]
build = "build.rs"
autolib = false
autobins = false
autoexamples = false
autotests = false
autobenches = false
description = "The AVFoundation bindings crate for `nokhwa`"
readme = "README.md"
keywords = [
    "avfoundation",
    "macos",
    "capture",
    "webcam",
]
license = "Apache-2.0"
repository = "https://github.com/l1npengtul/nokhwa"

[lib]
name = "nokhwa_bindings_macos"
path = "src/lib.rs"

[dependencies.nokhwa-core]
version = "0.1"

[target.'cfg(any(target_os="macos",target_os="ios"))'.dependencies.block]
version = "0.1"

[target.'cfg(any(target_os="macos",target_os="ios"))'.dependencies.cocoa-foundation]
version = "0.2"

[target.'cfg(any(target_os="macos",target_os="ios"))'.dependencies.core-foundation]
version = "0.10"

[target.'cfg(any(target_os="macos",target_os="ios"))'.dependencies.core-media-sys]
version = "0.1"

[target.'cfg(any(target_os="macos",target_os="ios"))'.dependencies.core-video-sys]
version = "0.1"

[target.'cfg(any(target_os="macos",target_os="ios"))'.dependencies.flume]
version = "0.11"

[target.'cfg(any(target_os="macos",target_os="ios"))'.dependencies.objc]
version = "0.2"
features = ["exception"]

[target.'cfg(any(target_os="macos",target_os="ios"))'.dependencies.once_cell]
version = "1"
````

## File: hdmicap/vendor/nokhwa-bindings-macos/README.md
````markdown
# nokhwa-bindings-macos
This crate is the AVFoundation bindings for the `nokhwa` crate.

It is not meant for general consumption. If you are looking for a MacOS camera capture crate, consider using `nokhwa` with feature `input-native`.

No support or API stability will be given. Subject to change at any time.
````

## File: hdmicap/.gitignore
````
/target/
````

## File: hidrig/control/boot.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""
CONTROL BOARD boot.py

Enables a second USB serial channel (the "data" channel) so the host test
scripts can send commands without colliding with the CircuitPython REPL
console. Runs once at power-on / reset.

After this is in place the control board enumerates TWO serial ports:
  - console port (the REPL)
  - data port    (used by host/example.py)
"""

import usb_cdc

usb_cdc.enable(console=True, data=True)
````

## File: hidrig/control/code.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""
CONTROL BOARD  (KB2040 or USB Trinkey QT2040 plugged into the test computer)

Role:
  - Reads line-based text commands from the host over the USB CDC data channel.
  - Translates them into compact binary packets.
  - Acts as the I2C *controller* on STEMMA QT and writes packets to the target.

Wiring:
  - USB -> test computer (command link)
  - STEMMA QT -> target board (I2C link)

Requirements:
  - CircuitPython 9.x
  - adafruit_hid library bundle copied to /lib (used here only for the
    Keycode name->value table, so we can accept human-readable key names)
  - boot.py from this folder (enables the usb_cdc data channel)

Command protocol (one command per line, terminated by '\n'):
  type <text>            Type a string of text
  key <NAME>             Tap a key (press then release), e.g. "key ENTER"
  combo <NAME> <NAME>... Chord: press all, then release all, e.g. "combo LEFT_CONTROL C"
  down <NAME>            Press and hold a key
  up <NAME>              Release a held key
  releaseall             Release all held keys
  move <dx> <dy>         Relative mouse move in pixels (auto-split into HID steps)
  click <left|right|middle>   Click a mouse button (default left)
  mdown <left|right|middle>   Press and hold a mouse button
  mup <left|right|middle>     Release a mouse button
  scroll <amount>        Scroll wheel (positive = up, negative = down)

<NAME> values are adafruit_hid Keycode names: A-Z, ZERO..NINE, ENTER, TAB,
SPACE, ESCAPE, BACKSPACE, LEFT_CONTROL, LEFT_SHIFT, LEFT_ALT, LEFT_GUI,
UP_ARROW, DOWN_ARROW, F1..F12, etc.

The board replies "OK\n" on success or "ERR <message>\n" on failure.
"""

import board
import time
import usb_cdc
from adafruit_hid.keycode import Keycode

TARGET_ADDRESS = 0x41

serial = usb_cdc.data
i2c = board.STEMMA_I2C()   # fallback if needed: busio.I2C(board.SCL, board.SDA)

# --- Opcodes (MUST match target/code.py) ----------------------------------
OP_KEY_PRESS, OP_KEY_RELEASE, OP_KEY_RELEASE_ALL, OP_TYPE = 0x01, 0x02, 0x03, 0x04
OP_MOUSE_MOVE, OP_MOUSE_PRESS, OP_MOUSE_RELEASE, OP_MOUSE_SCROLL = 0x10, 0x11, 0x12, 0x13

BUTTONS = {"left": 1, "right": 2, "middle": 4}

MAX_TYPE_CHUNK = 30


def send(packet):
    while not i2c.try_lock():
        pass
    try:
        i2c.writeto(TARGET_ADDRESS, bytes(packet))
    finally:
        i2c.unlock()
    # Handshake: poll a 1-byte read from the target until it returns 0x01,
    # meaning handle() has finished and it is ready for the next command.
    buf = bytearray(1)
    while True:
        while not i2c.try_lock():
            pass
        try:
            i2c.readfrom_into(TARGET_ADDRESS, buf)
            if buf[0] == 0x01:
                break
        except OSError:
            time.sleep(0.001)
        finally:
            i2c.unlock()


def keycode_for(name):
    return getattr(Keycode, name.upper())


def clamp(v, lo, hi):
    return max(lo, min(hi, v))


def u8(v):
    """Two's-complement encode a signed value into an unsigned byte."""
    return v & 0xFF


def do_move(dx, dy):
    # HID relative movement is int8 per report; split larger moves into steps.
    while dx or dy:
        sx, sy = clamp(dx, -127, 127), clamp(dy, -127, 127)
        send([OP_MOUSE_MOVE, u8(sx), u8(sy)])
        dx -= sx
        dy -= sy


def handle_line(line):
    parts = line.strip().split(" ")
    cmd = parts[0].lower()
    if not cmd:
        return

    if cmd == "type":
        data = (line.split(" ", 1)[1] if " " in line else "").encode("utf-8")
        for i in range(0, len(data), MAX_TYPE_CHUNK):
            send([OP_TYPE] + list(data[i:i + MAX_TYPE_CHUNK]))
    elif cmd == "key":
        kc = keycode_for(parts[1])
        send([OP_KEY_PRESS, kc])
        send([OP_KEY_RELEASE, kc])
    elif cmd == "combo":
        kcs = [keycode_for(p) for p in parts[1:]]
        send([OP_KEY_PRESS] + kcs)
        send([OP_KEY_RELEASE] + kcs)
    elif cmd == "down":
        send([OP_KEY_PRESS, keycode_for(parts[1])])
    elif cmd == "up":
        send([OP_KEY_RELEASE, keycode_for(parts[1])])
    elif cmd == "releaseall":
        send([OP_KEY_RELEASE_ALL])
    elif cmd == "move":
        do_move(int(parts[1]), int(parts[2]))
    elif cmd == "click":
        b = BUTTONS[parts[1].lower()] if len(parts) > 1 else 1
        send([OP_MOUSE_PRESS, b])
        send([OP_MOUSE_RELEASE, b])
    elif cmd == "mdown":
        send([OP_MOUSE_PRESS, BUTTONS[parts[1].lower()]])
    elif cmd == "mup":
        send([OP_MOUSE_RELEASE, BUTTONS[parts[1].lower()]])
    elif cmd == "scroll":
        send([OP_MOUSE_SCROLL, u8(clamp(int(parts[1]), -127, 127))])
    else:
        raise ValueError("unknown command: " + cmd)


buf = b""
while True:
    if serial.in_waiting:
        buf += serial.read(serial.in_waiting)
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            try:
                handle_line(line.decode("utf-8"))
                serial.write(b"OK\n")
            except Exception as e:  # report back instead of dropping the line
                serial.write(b"ERR " + str(e).encode("utf-8") + b"\n")
````

## File: hidrig/host/hid_seize_reports.c
````c
// hid_seize_reports.c
//
// Exclusively seize a single HID device on macOS and receive its RAW input
// reports — suitable for forwarding to a simulated HID device.
//
// Build:
//   clang -framework IOKit -framework CoreFoundation -o hid_seize_reports hid_seize_reports.c
//
// Run (Input Monitoring permission required):
//   ./hid_seize_reports
//
// Edit kVendorID / kProductID below to target your device.

#include <IOKit/hid/IOHIDManager.h>
#include <CoreFoundation/CoreFoundation.h>
#include <stdio.h>
#include <stdlib.h>
#include <signal.h>

// KB2040 running CircuitPython HID target firmware
static const long kVendorID  = 0x239A;
static const long kProductID = 0x8106;

static IOHIDManagerRef gManager = NULL;

static CFMutableDictionaryRef CreateMatchingDict(long vid, long pid) {
    CFMutableDictionaryRef d = CFDictionaryCreateMutable(
        kCFAllocatorDefault, 0,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks);
    if (!d) return NULL;
    if (vid) {
        CFNumberRef n = CFNumberCreate(kCFAllocatorDefault, kCFNumberLongType, &vid);
        CFDictionarySetValue(d, CFSTR(kIOHIDVendorIDKey), n);
        CFRelease(n);
    }
    if (pid) {
        CFNumberRef n = CFNumberCreate(kCFAllocatorDefault, kCFNumberLongType, &pid);
        CFDictionarySetValue(d, CFSTR(kIOHIDProductIDKey), n);
        CFRelease(n);
    }
    return d;
}

// Query an integer property from the device, with a fallback default.
static long GetDeviceLongProperty(IOHIDDeviceRef device, CFStringRef key, long fallback) {
    CFTypeRef prop = IOHIDDeviceGetProperty(device, key);
    if (prop && CFGetTypeID(prop) == CFNumberGetTypeID()) {
        long v = fallback;
        CFNumberGetValue((CFNumberRef)prop, kCFNumberLongType, &v);
        return v;
    }
    return fallback;
}

// RAW input report callback. `report` is the report payload; `reportID` is the
// numbered-report ID (0 if the device doesn't use numbered reports). NOTE: the
// reportID byte is NOT included in `report` — prepend it yourself if your
// simulated device uses numbered reports.
static void InputReportCallback(void *context, IOReturn result, void *sender,
                                IOHIDReportType type, uint32_t reportID,
                                uint8_t *report, CFIndex reportLength) {
    (void)context; (void)result; (void)sender; (void)type;

    // ---- Hook your forwarding here. For now just dump hex. ----
    printf("report id=%u len=%ld:", reportID, (long)reportLength);
    for (CFIndex i = 0; i < reportLength; i++) printf(" %02X", report[i]);
    printf("\n");
    fflush(stdout);

    // forward_to_simulated_device(reportID, report, reportLength);
}

static void DeviceMatchedCallback(void *context, IOReturn result,
                                  void *sender, IOHIDDeviceRef device) {
    (void)context; (void)result; (void)sender;

    IOReturn r = IOHIDDeviceOpen(device, kIOHIDOptionsTypeSeizeDevice);
    if (r != kIOReturnSuccess) {
        fprintf(stderr, "IOHIDDeviceOpen(seize) failed: 0x%08X\n", r);
        return;
    }

    // Optional but recommended: grab the report descriptor so your simulated
    // device can present an identical one. Then forwarded reports are valid
    // byte-for-byte.
    CFTypeRef desc = IOHIDDeviceGetProperty(device, CFSTR(kIOHIDReportDescriptorKey));
    if (desc && CFGetTypeID(desc) == CFDataGetTypeID()) {
        CFDataRef data = (CFDataRef)desc;
        CFIndex len = CFDataGetLength(data);
        printf("report descriptor (%ld bytes):", (long)len);
        const uint8_t *bytes = CFDataGetBytePtr(data);
        for (CFIndex i = 0; i < len; i++) printf(" %02X", bytes[i]);
        printf("\n");
    } else {
        printf("(report descriptor not available via property)\n");
    }

    // Size the receive buffer from the device's max input report size.
    long maxLen = GetDeviceLongProperty(device, CFSTR(kIOHIDMaxInputReportSizeKey), 64);
    if (maxLen <= 0) maxLen = 64;

    uint8_t *buf = (uint8_t *)malloc((size_t)maxLen);
    if (!buf) { fprintf(stderr, "malloc failed\n"); return; }

    // The buffer must stay alive for the life of the registration. Leaking it
    // here is fine for a single-device test tool; track it if you support many.
    IOHIDDeviceRegisterInputReportCallback(device, buf, maxLen,
                                           InputReportCallback, NULL);

    printf("Device seized; raw reports routed here (buf=%ld bytes).\n", maxLen);
    fflush(stdout);
}

static void DeviceRemovedCallback(void *context, IOReturn result,
                                  void *sender, IOHIDDeviceRef device) {
    (void)context; (void)result; (void)sender;
    printf("Device removed.\n");
    fflush(stdout);
}

static void HandleSignal(int sig) {
    (void)sig;
    if (gManager) IOHIDManagerClose(gManager, kIOHIDOptionsTypeNone);
    printf("\nReleased device. Bye.\n");
    exit(0);
}

int main(void) {
    signal(SIGINT, HandleSignal);

    gManager = IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone);
    if (!gManager) { fprintf(stderr, "IOHIDManagerCreate failed\n"); return 1; }

    CFMutableDictionaryRef match = CreateMatchingDict(kVendorID, kProductID);
    IOHIDManagerSetDeviceMatching(gManager, match);
    if (match) CFRelease(match);

    IOHIDManagerRegisterDeviceMatchingCallback(gManager, DeviceMatchedCallback, NULL);
    IOHIDManagerRegisterDeviceRemovalCallback(gManager, DeviceRemovedCallback, NULL);
    IOHIDManagerScheduleWithRunLoop(gManager, CFRunLoopGetCurrent(),
                                    kCFRunLoopDefaultMode);

    IOReturn r = IOHIDManagerOpen(gManager, kIOHIDOptionsTypeNone);
    if (r != kIOReturnSuccess) {
        fprintf(stderr, "IOHIDManagerOpen failed: 0x%08X\n", r);
        return 1;
    }

    printf("Waiting for device VID=0x%04lX PID=0x%04lX ... (Ctrl-C to quit)\n",
           kVendorID, kProductID);
    fflush(stdout);

    CFRunLoopRun();
    return 0;
}
````

## File: hidrig/host/Makefile
````
hid_seize_reports: hid_seize_reports.c
	clang -framework IOKit -framework CoreFoundation -o $@ $<
	codesign --sign - --force $@

clean:
	rm -f hid_seize_reports

.PHONY: clean
````

## File: hidrig/HANDOFF.md
````markdown
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
````

## File: hidrig/README.md
````markdown
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
````

## File: hidrig/SETUP.md
````markdown
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
````

## File: ocr/.gitignore
````
/visionocr
````

## File: ocr/linuxocr
````
#!/usr/bin/env python3
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Linux OCR helper using Tesseract. Mirrors the visionocr interface.

Reads a PNG from stdin (pass '-') or a file path, applies 2x upscale and
black padding (same preprocessing as visionocr for small console fonts),
then pipes to tesseract and prints the recognized text.

Requires: tesseract-ocr  (sudo apt-get install tesseract-ocr)
Optional: Pillow         (pip install Pillow) — used for preprocessing;
          without it, the raw PNG is passed directly to tesseract.
"""

import argparse
import io
import os
import subprocess
import sys
import tempfile


def _preprocess(png: bytes) -> bytes:
    """2x upscale + black pad. Falls back to identity if Pillow is absent."""
    try:
        from PIL import Image  # type: ignore

        img = Image.open(io.BytesIO(png)).convert("RGB")
        w, h = img.size
        img = img.resize((w * 2, h * 2), Image.LANCZOS)
        padded = Image.new("RGB", (w * 2 + 20, h * 2 + 20), (0, 0, 0))
        padded.paste(img, (10, 10))
        buf = io.BytesIO()
        padded.save(buf, "PNG")
        return buf.getvalue()
    except ImportError:
        return png


def main() -> None:
    ap = argparse.ArgumentParser(
        description="OCR a PNG image using Tesseract (Linux visionocr replacement)."
    )
    ap.add_argument("input", nargs="?", default="-", help="PNG file path, or - for stdin")
    ap.add_argument("--json", action="store_true", help="Ignored (reserved for future use)")
    args = ap.parse_args()

    if args.input == "-":
        png = sys.stdin.buffer.read()
    else:
        with open(args.input, "rb") as f:
            png = f.read()

    png = _preprocess(png)

    with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as tf:
        tf.write(png)
        tmp = tf.name

    try:
        result = subprocess.run(
            ["tesseract", tmp, "stdout", "--psm", "6", "--oem", "1"],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            sys.stderr.write(result.stderr)
            sys.exit(1)
        sys.stdout.write(result.stdout)
    finally:
        os.unlink(tmp)


if __name__ == "__main__":
    main()
````

## File: ocr/visionocr.swift
````swift
// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// visionocr — read text from an image using Apple's Vision framework.
// On-device, no network, no model download. Reads an image (path arg or PNG on
// stdin) and prints recognized text in reading order, one observation per line.
//
//   visionocr [--fast] [--json] [PATH | -]
//
//   --fast   use the fast recognition level (lower latency, less accurate)
//   --json   emit [{text, confidence, x, y, w, h}] with normalized bboxes
//            (origin top-left) instead of plain text lines

import CoreGraphics
import Foundation
import ImageIO
import Vision

func die(_ msg: String) -> Never {
    FileHandle.standardError.write(("visionocr: " + msg + "\n").data(using: .utf8)!)
    exit(1)
}

// Upscale and black-pad an image. Small thin console text recognizes far better
// when enlarged, and padding stops glyphs flush to the frame edge from being
// clipped (which drops the first/last character of a line).
func upscaleAndPad(_ img: CGImage, scale: CGFloat, pad: Int) -> CGImage? {
    let w = Int((CGFloat(img.width) * scale).rounded())
    let h = Int((CGFloat(img.height) * scale).rounded())
    let outW = w + pad * 2
    let outH = h + pad * 2
    guard
        let ctx = CGContext(
            data: nil, width: outW, height: outH, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { return nil }
    ctx.setFillColor(CGColor(red: 0, green: 0, blue: 0, alpha: 1))
    ctx.fill(CGRect(x: 0, y: 0, width: outW, height: outH))
    ctx.interpolationQuality = .high
    ctx.draw(img, in: CGRect(x: pad, y: pad, width: w, height: h))
    return ctx.makeImage()
}

var accurate = false
var json = false
var path: String? = nil
for arg in CommandLine.arguments.dropFirst() {
    switch arg {
    case "--accurate": accurate = true
    case "--fast": accurate = false  // default; accepted for compatibility
    case "--json": json = true
    case "-": path = nil
    default: path = arg
    }
}

let data: Data
if let p = path {
    guard let d = FileManager.default.contents(atPath: p) else { die("cannot read \(p)") }
    data = d
} else {
    data = FileHandle.standardInput.readDataToEndOfFile()
}
if data.isEmpty { die("no image data") }

guard let src = CGImageSourceCreateWithData(data as CFData, nil),
    let decoded = CGImageSourceCreateImageAtIndex(src, 0, nil)
else { die("could not decode image") }

let image = upscaleAndPad(decoded, scale: 2.0, pad: 16) ?? decoded

let request = VNRecognizeTextRequest()
// Counterintuitively, .fast detects small thin console fonts that .accurate
// (tuned for natural document text) misses entirely. Default to .fast; let
// callers opt into .accurate for large, clean text.
request.recognitionLevel = accurate ? .accurate : .fast
// Console/boot/code text is not natural language; correction hurts more than
// it helps (it "fixes" identifiers, hex, paths).
request.usesLanguageCorrection = false
// Vision's default minimumTextHeight (1/32 of image height) skips small console
// fonts. It's a fraction of height; 0.0 means "default", so use a small
// positive floor to catch tiny text.
request.minimumTextHeight = 0.005

let handler = VNImageRequestHandler(cgImage: image, options: [:])
do {
    try handler.perform([request])
} catch {
    die("\(error)")
}

let observations = request.results ?? []

// Vision returns observations unordered. Sort into reading order. boundingBox
// origin is bottom-left, so a larger y is higher on screen.
let sorted = observations.sorted { a, b in
    let dy = a.boundingBox.origin.y - b.boundingBox.origin.y
    if abs(dy) > 0.01 { return dy > 0 }
    return a.boundingBox.origin.x < b.boundingBox.origin.x
}

if json {
    var items: [[String: Any]] = []
    for obs in sorted {
        guard let top = obs.topCandidates(1).first else { continue }
        let b = obs.boundingBox
        items.append([
            "text": top.string,
            "confidence": top.confidence,
            "x": b.origin.x,
            // Convert to top-left origin for consumers that expect it.
            "y": 1.0 - b.origin.y - b.size.height,
            "w": b.size.width,
            "h": b.size.height,
        ])
    }
    let out = try JSONSerialization.data(withJSONObject: items, options: [.prettyPrinted])
    FileHandle.standardOutput.write(out)
    FileHandle.standardOutput.write("\n".data(using: .utf8)!)
} else {
    for obs in sorted {
        if let top = obs.topCandidates(1).first {
            print(top.string)
        }
    }
}
````

## File: serialcap/src/server.rs
````rust
// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Localhost HTTP API. The daemon can own several named serial interfaces; every
//! per-interface endpoint takes `?interface=NAME` and falls back to the default
//! (first-configured) interface when it's omitted, so single-interface clients
//! (and the existing dashboard) keep working unchanged.
//!
//! `/stream` is a bidirectional WebSocket: the daemon sends serial output (binary
//! frames) and accepts client keystrokes (binary or text) to write back to the
//! port. The hdmicap preview page connects here cross-port, so responses carry a
//! permissive CORS header.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tracing::debug;

use crate::serial_io::{NamedSerial, SerialHandle, Serials};

#[derive(Clone)]
pub struct AppState {
    pub serials: Serials,
}

#[derive(Deserialize)]
pub struct IfaceParam {
    interface: Option<String>,
}

#[derive(Deserialize)]
pub struct ButtonParam {
    interface: Option<String>,
    ms: u64,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/stream", get(stream))
        .route("/status", get(status))
        .route("/interfaces", get(interfaces))
        .route("/devices", get(devices))
        .route("/button", post(button))
        .with_state(state)
}

const CORS: (header::HeaderName, &str) = (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");

/// Resolve the requested interface, or the default (first) when none is named.
fn resolve<'a>(serials: &'a Serials, name: &Option<String>) -> Option<&'a SerialHandle> {
    match name {
        Some(n) => serials.get(n),
        None => serials.default().map(|ns| &ns.handle),
    }
}

fn status_json(ns: &NamedSerial) -> serde_json::Value {
    let st = ns.handle.status();
    serde_json::json!({
        "name": ns.name,
        "device": st.device,
        "baud": st.baud,
        "connected": st.connected,
        "power_on": st.power_on,   // null when no sense signal is configured
    })
}

/// Status of one interface (`?interface=NAME`) or, by default, all of them.
async fn status(State(s): State<AppState>, Query(q): Query<IfaceParam>) -> Response {
    match &q.interface {
        Some(name) => match s.serials.all().iter().find(|ns| &ns.name == name) {
            Some(ns) => ([CORS], Json(status_json(ns))).into_response(),
            None => (
                StatusCode::NOT_FOUND,
                [CORS],
                format!("no interface '{name}'"),
            )
                .into_response(),
        },
        None => {
            let all: Vec<_> = s.serials.all().iter().map(status_json).collect();
            ([CORS], Json(all)).into_response()
        }
    }
}

/// All interfaces this daemon owns (name, device, baud, connected).
async fn interfaces(State(s): State<AppState>) -> Response {
    let all: Vec<_> = s.serials.all().iter().map(status_json).collect();
    ([CORS], Json(all)).into_response()
}

async fn devices() -> Response {
    match crate::serial_io::list_ports() {
        Ok(list) => (
            [CORS],
            Json(
                list.into_iter()
                    .map(|(path, desc)| serde_json::json!({"path": path, "misc": desc}))
                    .collect::<Vec<_>>(),
            ),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("{e:#}"),
        )
            .into_response(),
    }
}

/// Press the J2 power button on the attached target for `ms` milliseconds.
///
/// `POST /button?ms=200[&interface=NAME]`
///
/// Short presses (≤500 ms) deliver a power-button event to the OS (graceful
/// reboot/halt, target-OS-defined).  Long presses (≥3000 ms) trigger a PMIC
/// hard power-off.  The call blocks until the press completes.
/// Returns 200 on success, 503 if the supervisor is not running.
async fn button(State(s): State<AppState>, Query(q): Query<ButtonParam>) -> Response {
    let handle = match resolve(&s.serials, &q.interface) {
        Some(h) => h.clone(),
        None => {
            let what = q.interface.as_deref().unwrap_or("(default)");
            return (
                StatusCode::NOT_FOUND,
                [CORS],
                format!("no interface '{what}'"),
            )
                .into_response();
        }
    };
    match handle.dtr_press(q.ms).await {
        Ok(()) => ([CORS], format!("button pressed for {} ms\n", q.ms)).into_response(),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, [CORS], format!("{e:#}\n")).into_response(),
    }
}

async fn stream(
    ws: WebSocketUpgrade,
    State(s): State<AppState>,
    Query(q): Query<IfaceParam>,
) -> Response {
    let handle = match resolve(&s.serials, &q.interface) {
        Some(h) => h.clone(),
        None => {
            let what = q.interface.as_deref().unwrap_or("(default)");
            return (
                StatusCode::NOT_FOUND,
                [CORS],
                format!("no interface '{what}'"),
            )
                .into_response();
        }
    };
    ws.on_upgrade(move |socket| handle_ws(socket, handle))
}

async fn handle_ws(socket: WebSocket, serial: SerialHandle) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = serial.subscribe();

    // Send recent scrollback so a mid-stream connection isn't blank.
    let snapshot = serial.scrollback();
    if !snapshot.is_empty() && sender.send(Message::Binary(snapshot)).await.is_err() {
        return;
    }

    // serial -> client
    let mut send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(bytes) => {
                    if sender.send(Message::Binary(bytes.to_vec())).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    debug!("ws client lagged, dropped {n} messages");
                }
                Err(RecvError::Closed) => break,
            }
        }
    });

    // client -> serial
    let write_tx = serial.write_tx.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Binary(b) => {
                    let _ = write_tx.try_send(Bytes::from(b));
                }
                Message::Text(t) => {
                    let _ = write_tx.try_send(Bytes::from(t.into_bytes()));
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}
````

## File: serialcap/.gitignore
````
/target/
````

## File: serialcap/Cargo.toml
````toml
[package]
name = "serialcap"
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"
description = "Serial console daemon — owns a serial port and fans it out over a localhost WebSocket"

[[bin]]
name = "serialcap"
path = "src/main.rs"

[dependencies]
# --- Async runtime + HTTP ------------------------------------------------
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "signal", "time", "net", "io-util"] }
axum = { version = "0.7", features = ["ws"] }
futures-util = "0.3"

# --- Serial --------------------------------------------------------------
# tokio-serial wraps the cross-platform serialport crate with async I/O.
tokio-serial = "5"
# Direct dep so we can name the SerialPort trait for DTR control.
serialport = "4"

# --- CLI -----------------------------------------------------------------
clap = { version = "4", features = ["derive"] }

# --- Daemon plumbing -----------------------------------------------------
fs2 = "0.4"
directories = "5"
nix = { version = "0.29", features = ["signal", "process"] }

# --- Errors / logging / serde -------------------------------------------
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
bytes = "1"

# --- Client subcommands --------------------------------------------------
ureq = "2"

[profile.release]
opt-level = 3
lto = "thin"
````

## File: src/paniolo/__init__.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
````

## File: src/paniolo/_power.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Power control helpers: FTDI DTR line via the serialcap daemon or pyserial fallback."""

from __future__ import annotations

import time
import urllib.error
import urllib.request
from typing import Optional


def dtr_button_press(daemon_url: str, interface_name: str, duration_ms: int) -> None:
    """Assert DTR (J2 power button) for duration_ms milliseconds via the serialcap daemon.

    The daemon owns the serial port exclusively and drives the DTR line on its
    supervisor task.  This call blocks until the press completes.

    duration_ms guidance (Raspberry Pi 5 / DA9091 PMIC):
      ≤500 ms  — soft reset signal; OS handles it (graceful reboot or halt)
      ≥3000 ms — hard power-off; follow with another call to power the board on

    Raises RuntimeError on HTTP error, OSError on network failure.
    """
    url = f"{daemon_url}/button?interface={interface_name}&ms={duration_ms}"
    req = urllib.request.Request(url, method="POST", data=b"")
    try:
        with urllib.request.urlopen(req, timeout=max(15, duration_ms // 1000 + 5)) as resp:
            resp.read()
    except urllib.error.HTTPError as exc:
        raise RuntimeError(f"serialcap /button returned {exc.code}: {exc.reason}") from exc


def dtr_direct_button_press(device: str, duration_ms: int) -> None:
    """Assert DTR (J2 power button) for duration_ms milliseconds directly via pyserial.

    Fallback for when the serialcap daemon is not running.  Opens the serial
    port, asserts DTR for the requested duration, then releases and closes.

    Raises RuntimeError if pyserial is not installed or on serial errors.
    """
    try:
        import serial as _serial
    except ImportError as exc:
        raise RuntimeError(
            "pyserial is required for direct DTR control. "
            "Install it with: uv add pyserial"
        ) from exc

    port = _serial.Serial()
    port.port = device
    port.baudrate = 115200
    port.open()
    try:
        port.dtr = False
        time.sleep(0.05)   # brief settle after open
        port.dtr = True
        time.sleep(duration_ms / 1000.0)
        port.dtr = False
    finally:
        port.close()
````

## File: tests/test_serial.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Host-side tests for serial helpers — no hardware, no serialcap binary."""

from __future__ import annotations

from paniolo import _serial
from paniolo._config import SerialInterface


def test_log_cmd_defaults_to_bare_subcommand():
    assert _serial.log_cmd("serialcap") == ["serialcap", "log"]


def test_log_cmd_forwards_only_set_flags():
    cmd = _serial.log_cmd("serialcap", tail=50)
    assert cmd == ["serialcap", "log", "--tail", "50"]


def test_log_cmd_interface():
    assert _serial.log_cmd("serialcap", interface="bmc", tail=10) == [
        "serialcap", "log", "--interface", "bmc", "--tail", "10",
    ]


def test_interface_arg():
    assert _serial.interface_arg("console", "/dev/ttyUSB0", 115200) == "console=/dev/ttyUSB0@115200"


def test_daemon_cmd_one_per_interface():
    ifaces = [
        SerialInterface("console", "/dev/ttyUSB0", 115200),
        SerialInterface("bmc", "/dev/ttyUSB1", 9600),
    ]
    assert _serial.daemon_cmd("serialcap", ifaces, port=8724) == [
        "serialcap", "daemon", "--port", "8724",
        "--interface", "console=/dev/ttyUSB0@115200",
        "--interface", "bmc=/dev/ttyUSB1@9600",
    ]


def test_daemon_cmd_buffer_lines():
    ifaces = [SerialInterface("console", "/dev/ttyUSB0", 115200)]
    assert _serial.daemon_cmd("serialcap", ifaces, port=9, buffer_lines=1000) == [
        "serialcap", "daemon", "--port", "9", "--buffer-lines", "1000",
        "--interface", "console=/dev/ttyUSB0@115200",
    ]


def test_log_cmd_range_and_since():
    assert _serial.log_cmd("serialcap", from_seq=10, to_seq=20) == [
        "serialcap", "log", "--from", "10", "--to", "20",
    ]
    assert _serial.log_cmd("serialcap", since=7) == ["serialcap", "log", "--since", "7"]


def test_log_cmd_boolean_flags():
    cmd = _serial.log_cmd("serialcap", raw=True, as_json=True, no_pending=True)
    assert cmd == ["serialcap", "log", "--raw", "--json", "--no-pending"]
    # Defaults stay off.
    assert "--raw" not in _serial.log_cmd("serialcap", tail=1)
````

## File: .gitignore
````
.venv/
__pycache__/
*.py[cod]
*.egg-info/
dist/
build/
.ruff_cache/
.mypy_cache/
.pytest_cache/
hidrig/host/hid_seize_reports
````

## File: LICENSE
````
Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
````

## File: pyproject.toml
````toml
[project]
name = "paniolo"
version = "0.1.0"
description = "Agent-controlled target machine wrangler for low-level software development"
requires-python = ">=3.11"
license = { text = "Apache-2.0" }
dependencies = [
    "typer>=0.12",
]

[project.scripts]
paniolo = "paniolo._cli:app"

[project.optional-dependencies]
hid = [
    "pyserial>=3.5",
]

[tool.uv]
package = true

[build-system]
requires = ["setuptools>=69"]
build-backend = "setuptools.build_meta"

[dependency-groups]
dev = [
    "pytest>=9.0.3",
]

[tool.setuptools.packages.find]
where = ["src"]
````

## File: hdmicap/assets/index.html
````html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>paniolo · video + serial</title>
  <link rel="stylesheet" href="/xterm.css">
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    html, body { height: 100%; }
    body { background: #000; display: flex; flex-direction: column;
           font: 12px ui-monospace, Menlo, monospace; color: #ddd; }
    #video { flex: 1 1 auto; min-height: 0; position: relative;
             display: flex; align-items: center; justify-content: center; }
    #video img { max-width: 100%; max-height: 100%; object-fit: contain; }
    #vstatus { position: absolute; top: 8px; right: 12px; color: #0f0;
               background: rgba(0,0,0,.6); padding: 4px 8px; border-radius: 4px; }
    #sidebar { flex: 0 0 auto; display: flex; flex-direction: column; min-height: 0; }
    #bar { flex: 0 0 24px; background: #111; display: flex; align-items: center;
           gap: 14px; padding: 0 12px; border-top: 1px solid #333; }
    #bar .label { color: #888; }
    #sstatus { color: #fa0; flex: 1 1 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    #layoutbtn { margin-left: auto; cursor: pointer; background: none; border: 1px solid #444;
                 border-radius: 3px; color: #888; font: 11px ui-monospace, monospace;
                 padding: 1px 6px; }
    #layoutbtn:hover { color: #cdf; border-color: #46c; }
    #serial { flex: 0 0 40vh; background: #000; min-height: 0; display: flex; }
    .serial-pane { flex: 1 1 0; min-width: 0; display: flex; flex-direction: column; }
    .serial-pane + .serial-pane { border-left: 1px solid #333; }
    .pane-bar { flex: 0 0 20px; background: #111; border-bottom: 1px solid #222;
                display: flex; align-items: center; padding: 0 8px; gap: 8px; font-size: 11px; }
    .pane-name { color: #888; }
    .pane-status { color: #fa0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .pane-term { flex: 1 1 0; min-height: 0; padding: 4px 6px; }
    #videobtns { position: absolute; top: 8px; left: 12px; z-index: 3;
                 display: flex; gap: 6px; }
    #ocrbtn, #pwrbtn { cursor: pointer; font: 12px ui-monospace, monospace;
                       padding: 4px 10px; border-radius: 4px; }
    #ocrbtn { color: #cdf; background: rgba(0,0,0,.6); border: 1px solid #46c; }
    #ocrbtn:hover { background: rgba(40,80,160,.7); }
    #pwrbtn { color: #fca; background: rgba(0,0,0,.6); border: 1px solid #840;
              display: none; }
    #pwrbtn:hover { background: rgba(80,30,0,.7); }
    #pwrmodal { position: absolute; inset: 0; z-index: 10; display: none;
                align-items: center; justify-content: center;
                background: rgba(0,0,0,.7); }
    #pwrmodal.show { display: flex; }
    #pwrbox { background: #111; border: 1px solid #840; border-radius: 8px;
              padding: 20px 24px; display: flex; flex-direction: column; gap: 14px;
              color: #ddd; font: 13px ui-monospace, monospace; max-width: 300px; }
    #pwrbox p { color: #fca; }
    #pwrbtns { display: flex; gap: 10px; justify-content: flex-end; }
    #pwrcancel { cursor: pointer; background: none; border: 1px solid #555;
                 border-radius: 4px; color: #aaa; padding: 4px 14px; font: inherit; }
    #pwrcancel:hover { border-color: #888; color: #ddd; }
    #pwrconfirm { cursor: pointer; background: rgba(80,30,0,.8); border: 1px solid #a60;
                  border-radius: 4px; color: #fca; padding: 4px 14px; font: inherit; }
    #pwrconfirm:hover { background: rgba(120,50,0,.9); }
    #ocrpanel { position: absolute; inset: 8px 8px auto 8px; max-height: 70%; z-index: 4;
                display: none; flex-direction: column; background: rgba(0,0,0,.92);
                border: 1px solid #46c; border-radius: 6px; }
    #ocrpanel.show { display: flex; }
    #ocrhead { display: flex; justify-content: space-between; align-items: center;
               padding: 6px 10px; border-bottom: 1px solid #333; color: #cdf; }
    #ocrclose { cursor: pointer; color: #f88; padding: 0 6px; font-size: 16px; }
    #ocrtext { margin: 0; padding: 10px; overflow: auto; white-space: pre-wrap;
               color: #dfd; font: 12px ui-monospace, monospace; }

    /* right-panel layout */
    body.layout-right { flex-direction: row; }
    body.layout-right #video { flex: 1 1 0; min-width: 0; min-height: 0; }
    body.layout-right #sidebar { flex: 0 0 380px; min-width: 380px; min-height: 0;
                                  border-top: none; border-left: 1px solid #333; }
    body.layout-right #bar { border-top: none; border-bottom: 1px solid #333; }
    body.layout-right #serial { flex: 1 1 0; min-height: 0; flex-direction: column; }
    body.layout-right .serial-pane + .serial-pane { border-left: none; border-top: 1px solid #333; }
  </style>
</head>
<body>
  <div id="video">
    <img id="feed" src="/preview" alt="HDMI capture feed">
    <div id="videobtns">
      <button id="ocrbtn">⌕ OCR</button>
      <button id="pwrbtn">⏻ Power Cycle</button>
    </div>
    <div id="vstatus">connecting…</div>
    <div id="ocrpanel">
      <div id="ocrhead"><span id="ocrtitle">OCR</span><span id="ocrclose">×</span></div>
      <pre id="ocrtext"></pre>
    </div>
    <div id="pwrmodal">
      <div id="pwrbox">
        <p>Power cycle the target?</p>
        <div id="pwrbtns">
          <button id="pwrcancel">Cancel</button>
          <button id="pwrconfirm">Power Cycle</button>
        </div>
      </div>
    </div>
  </div>
  <div id="sidebar">
    <div id="bar">
      <span class="label">serial</span>
      <span id="sstatus">disconnected</span>
      <button id="layoutbtn" title="Toggle serial panel position"></button>
    </div>
    <div id="serial"></div>
  </div>

  <script src="/xterm.js"></script>
  <script src="/xterm-addon-fit.js"></script>
  <script>
    // ── layout toggle ──
    const layoutBtn = document.getElementById('layoutbtn');
    const LAYOUT_KEY = 'paniolo-serial-layout';
    const allFits = [];
    function fitAll() { allFits.forEach(f => { try { f.fit(); } catch (e) {} }); }
    function setLayout(layout) {
      const right = layout === 'right';
      document.body.classList.toggle('layout-right', right);
      layoutBtn.textContent = right ? 'serial ↓ bottom' : 'serial → right';
      localStorage.setItem(LAYOUT_KEY, layout);
      setTimeout(fitAll, 50);
    }
    layoutBtn.onclick = () =>
      setLayout(document.body.classList.contains('layout-right') ? 'bottom' : 'right');
    setLayout(localStorage.getItem(LAYOUT_KEY) || 'bottom');

    window.addEventListener('resize', fitAll);

    // ── video: poll signal/resolution from this (hdmicap) daemon ──
    const vimg = document.getElementById('feed');
    const vst  = document.getElementById('vstatus');
    function pollVideo() {
      fetch('/status').then(r => r.json()).then(d => {
        vst.textContent = d.signal + ' · ' + d.width + '×' + d.height;
        vst.style.color = d.signal === 'stable' ? '#0f0' : '#fa0';
      }).catch(() => { vst.textContent = 'daemon unreachable'; vst.style.color = '#f44'; });
    }
    pollVideo();
    setInterval(pollVideo, 2000);
    vimg.onerror = () => { vst.textContent = 'stream error — reload'; vst.style.color = '#f44'; };

    // ── OCR: run Apple Vision on the current frame via the /ocr endpoint ──
    const ocrPanel = document.getElementById('ocrpanel');
    const ocrText = document.getElementById('ocrtext');
    const ocrTitle = document.getElementById('ocrtitle');
    document.getElementById('ocrclose').onclick = () => ocrPanel.classList.remove('show');
    document.getElementById('ocrbtn').onclick = () => {
      ocrPanel.classList.add('show');
      ocrTitle.textContent = 'OCR — reading…';
      ocrText.textContent = '';
      fetch('/ocr')
        .then(r => r.text().then(t => ({ ok: r.ok, t })))
        .then(({ ok, t }) => {
          ocrTitle.textContent = ok ? 'OCR' : 'OCR — error';
          ocrText.textContent = ok ? (t.trim() || '(no text detected)') : t;
        })
        .catch(e => { ocrTitle.textContent = 'OCR — error'; ocrText.textContent = String(e); });
    };

    // ── Power Cycle button — only shown when /power-cycle is available ──
    const pwrBtn = document.getElementById('pwrbtn');
    const pwrModal = document.getElementById('pwrmodal');
    fetch('/power-cycle', { method: 'POST' })
      .then(r => { if (r.status !== 501) pwrBtn.style.display = ''; })
      .catch(() => {});
    pwrBtn.onclick = () => pwrModal.classList.add('show');
    document.getElementById('pwrcancel').onclick = () => pwrModal.classList.remove('show');
    document.getElementById('pwrconfirm').onclick = () => {
      pwrModal.classList.remove('show');
      pwrBtn.textContent = '⏻ cycling…';
      pwrBtn.disabled = true;
      fetch('/power-cycle', { method: 'POST' })
        .then(r => r.text().then(t => {
          pwrBtn.textContent = r.ok ? '⏻ Power Cycle' : '⏻ Error';
          if (!r.ok) console.error('power-cycle:', t);
        }))
        .catch(e => { pwrBtn.textContent = '⏻ Error'; console.error(e); })
        .finally(() => { pwrBtn.disabled = false; });
    };

    // ── serial: xterm.js panes fed by serialcap WebSocket (cross-port) ──
    // ?serial=PORT or ?serialws=URL override the default serialcap location.
    // ?interface=NAME locks to a single interface (single-pane mode).
    // Without ?interface, all daemon interfaces get their own pane side-by-side.
    const params = new URLSearchParams(location.search);
    const serialPort = params.get('serial') || '8724';
    const baseWsUrl = params.get('serialws') ||
                      ('ws://' + location.hostname + ':' + serialPort + '/stream');
    const httpBase = baseWsUrl.replace(/^ws/, 'http').replace(/\/stream.*$/, '');
    const singleInterface = params.get('interface') || null;

    const sst = document.getElementById('sstatus');
    const serialEl = document.getElementById('serial');

    function wsUrlFor(iface) {
      if (!iface) return baseWsUrl;
      const sep = baseWsUrl.includes('?') ? '&' : '?';
      return baseWsUrl + sep + 'interface=' + encodeURIComponent(iface);
    }

    function openTerminal(ifaceName, termDiv, statusEl) {
      const term = new Terminal({
        fontSize: 13, cursorBlink: true, scrollback: 5000,
        theme: { background: '#000000' },
      });
      const fit = new FitAddon.FitAddon();
      term.loadAddon(fit);
      term.open(termDiv);
      allFits.push(fit);

      const wsUrl = wsUrlFor(ifaceName);
      let ws, gen = 0;
      function connect() {
        const myGen = ++gen;
        statusEl.textContent = 'connecting…';
        statusEl.style.color = '#fa0';
        ws = new WebSocket(wsUrl);
        ws.binaryType = 'arraybuffer';
        ws.onopen = () => { statusEl.textContent = 'connected'; statusEl.style.color = '#0f0'; };
        ws.onmessage = ev => {
          if (ev.data instanceof ArrayBuffer) term.write(new Uint8Array(ev.data));
          else term.write(ev.data);
        };
        ws.onclose = () => {
          if (myGen !== gen) return;
          statusEl.textContent = 'unreachable — retrying';
          statusEl.style.color = '#f44';
          setTimeout(() => { if (myGen === gen) connect(); }, 3000);
        };
        ws.onerror = () => { try { ws.close(); } catch (e) {} };
      }
      term.onData(d => { if (ws && ws.readyState === WebSocket.OPEN) ws.send(d); });
      connect();
    }

    function buildPanes(ifaceNames) {
      serialEl.innerHTML = '';
      allFits.length = 0;

      if (ifaceNames.length <= 1) {
        // Single pane: global #sstatus, no per-pane label
        const termDiv = document.createElement('div');
        termDiv.className = 'pane-term';
        serialEl.appendChild(termDiv);
        openTerminal(ifaceNames[0] || null, termDiv, sst);
      } else {
        // Multi-pane: hide global status; each pane gets a label + status bar
        sst.style.display = 'none';
        for (const name of ifaceNames) {
          const pane = document.createElement('div');
          pane.className = 'serial-pane';

          const bar = document.createElement('div');
          bar.className = 'pane-bar';
          const nameEl = document.createElement('span');
          nameEl.className = 'pane-name';
          nameEl.textContent = name;
          const statusEl = document.createElement('span');
          statusEl.className = 'pane-status';
          bar.append(nameEl, statusEl);

          const termDiv = document.createElement('div');
          termDiv.className = 'pane-term';

          pane.append(bar, termDiv);
          serialEl.appendChild(pane);
          openTerminal(name, termDiv, statusEl);
        }
      }
      setTimeout(fitAll, 50);
    }

    if (singleInterface) {
      buildPanes([singleInterface]);
    } else {
      fetch(httpBase + '/interfaces')
        .then(r => r.json())
        .then(list => buildPanes(Array.isArray(list) ? list.map(i => i.name) : [null]))
        .catch(() => buildPanes([null]));
    }
  </script>
</body>
</html>
````

## File: hdmicap/src/capture_thread.rs
````rust
// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The capture thread. Owns the device, runs the warm decode loop, classifies
//! each frame, and publishes the latest FrameState into a `watch` channel.
//!
//! This is a plain std::thread, NOT a tokio task: nokhwa's grab is blocking and
//! must not sit on the async runtime. `watch::Sender::send` is sync, so the
//! thread publishes freely.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tracing::{info, warn};

use crate::capture::{open_backend, DeviceSpec};
use crate::frame::{ahash, is_no_signal, FrameState, Signal, STABLE_FRAMES};

pub type FrameRx = watch::Receiver<Arc<FrameState>>;

/// Spawn the capture thread. Returns the receiver end and the JoinHandle.
pub fn spawn(spec: DeviceSpec) -> (FrameRx, thread::JoinHandle<()>) {
    let (tx, rx) = watch::channel(Arc::new(FrameState::no_device()));

    let handle = thread::Builder::new()
        .name("capture".into())
        .spawn(move || capture_loop(spec, tx))
        .expect("failed to spawn capture thread");

    (rx, handle)
}

fn capture_loop(spec: DeviceSpec, tx: watch::Sender<Arc<FrameState>>) {
    // Reconnect loop: if the device is absent or vanishes mid-run, publish
    // NoDevice and keep retrying so hot-plug just works.
    loop {
        let mut backend = match open_backend(&spec) {
            Ok(b) => {
                info!("capture device opened");
                b
            }
            Err(e) => {
                warn!("open failed: {e:#}");
                let _ = tx.send(Arc::new(FrameState::no_device()));
                if all_receivers_gone(&tx) {
                    return;
                }
                // Brief pause before retry. The MS2109 firmware needs time to
                // reset its isochronous endpoint state after a stream stop;
                // immediately reopening can catch it mid-reset and cause stalls.
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        // Watchdog: fallback for any stall the v4l timeout doesn't catch.
        // The cancel flag is set when we exit the inner loop normally so the
        // watchdog doesn't fire across reconnect iterations.
        let frame_count = Arc::new(AtomicU64::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let frame_count = frame_count.clone();
            let cancelled = cancelled.clone();
            thread::Builder::new()
                .name("stall-watchdog".into())
                .spawn(move || {
                    const GRACE: Duration = Duration::from_secs(12);
                    const POLL: Duration = Duration::from_secs(4);
                    thread::sleep(GRACE);
                    if cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    let mut prev = frame_count.load(Ordering::Relaxed);
                    if prev == 0 {
                        warn!("no frames in {GRACE:?} after device open — exiting for restart");
                        std::process::exit(1);
                    }
                    loop {
                        thread::sleep(POLL);
                        if cancelled.load(Ordering::Relaxed) {
                            return;
                        }
                        let cur = frame_count.load(Ordering::Relaxed);
                        if cur == prev {
                            warn!("capture stalled ({POLL:?} with no new frames) — exiting for restart");
                            std::process::exit(1);
                        }
                        prev = cur;
                    }
                })
                .ok();
        }

        let mut last_dims = (0u32, 0u32);
        let mut epoch = 0u64;
        let mut stable_count = 0u32;
        let mut last_hash = 0u64;
        let mut frame_start = Instant::now();
        // Consecutive decode errors. Transient errors (bad buffer after open,
        // UVC flush frames) are tolerated; only a sustained run triggers reconnect.
        let mut consecutive_errors = 0u32;
        const MAX_CONSECUTIVE_ERRORS: u32 = 8;

        loop {
            if all_receivers_gone(&tx) {
                info!("no receivers left; capture thread exiting");
                return;
            }

            let captured = match backend.frame() {
                Ok(f) => {
                    consecutive_errors = 0;
                    f
                }
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        warn!("frame error ({consecutive_errors} consecutive): {e:#}");
                        cancelled.store(true, Ordering::Relaxed);
                        let _ = tx.send(Arc::new(FrameState::no_device()));
                        break;
                    }
                    // Transient: skip this frame and try again.
                    tracing::debug!("transient frame error (#{consecutive_errors}): {e:#}");
                    continue;
                }
            };
            let jpeg = captured.jpeg;
            let img = captured.rgb;

            let (w, h) = (img.width(), img.height());

            if (w, h) != last_dims {
                epoch += 1;
                stable_count = 0;
                last_dims = (w, h);
                last_hash = 0;
                info!("resolution -> {w}x{h} (epoch {epoch})");
            }

            let hash = ahash(&img);

            // Skip the expensive per-pixel is_no_signal scan when the frame
            // hash is unchanged and we're already Stable — static screens cost
            // almost nothing after the first pass.
            let signal = if hash == last_hash && stable_count >= STABLE_FRAMES {
                Signal::Stable
            } else if is_no_signal(&img) {
                stable_count = 0;
                Signal::NoSignal
            } else if stable_count < STABLE_FRAMES {
                stable_count += 1;
                Signal::ModeSwitching
            } else {
                Signal::Stable
            };

            last_hash = hash;
            frame_count.fetch_add(1, Ordering::Relaxed);

            // When raw JPEG bytes are available (Linux MJPEG path), store them
            // for zero-cost preview serving. The RGB is kept for snapshot/OCR
            // but only when JPEG is absent (YUYV or macOS paths).
            let (jpeg_arc, rgb_arc) = if let Some(j) = jpeg {
                (Some(j), Arc::from([] as [u8; 0]) as Arc<[u8]>)
            } else {
                let expected = w as usize * h as usize * 3;
                let mut raw = img.into_raw();
                raw.truncate(expected);
                (None, Arc::from(raw.into_boxed_slice()) as Arc<[u8]>)
            };

            let _ = tx.send(Arc::new(FrameState {
                jpeg: jpeg_arc,
                rgb: rgb_arc,
                width: w,
                height: h,
                hash,
                signal,
                resolution_epoch: epoch,
                captured_at: Instant::now(),
            }));

            // Cap to TARGET_FPS. MJPEG decode is ~50ms/frame in software, so
            // 10fps is a reasonable ceiling until the hot path uses lazy decode.
            const TARGET_INTERVAL: Duration = Duration::from_millis(1000 / 10);
            let elapsed = frame_start.elapsed();
            if elapsed < TARGET_INTERVAL {
                thread::sleep(TARGET_INTERVAL - elapsed);
            }
            frame_start = Instant::now();
        }
    }
}

fn all_receivers_gone(tx: &watch::Sender<Arc<FrameState>>) -> bool {
    tx.receiver_count() == 0
}
````

## File: hdmicap/src/daemon.rs
````rust
// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Daemon lifecycle: advisory lock, discovery file, runtime wiring, shutdown.

use std::fs::{self, File};
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::capture::DeviceSpec;
use crate::capture_thread;
use crate::server::{self, AppState};

#[derive(Serialize, Deserialize)]
pub struct Discovery {
    pub pid: u32,
    pub port: u16,
}

fn runtime_dir() -> Result<PathBuf> {
    // XDG_RUNTIME_DIR on Linux; a per-user tmp path on macOS via `directories`.
    let dirs = directories::BaseDirs::new().context("no base dirs")?;
    let base = dirs
        .runtime_dir()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir());
    let dir = base.join("hdmicap");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn lock_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("daemon.lock"))
}

fn discovery_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("daemon.json"))
}

/// Read the discovery file so the CLI knows which port to hit.
pub fn discover() -> Result<Discovery> {
    let p = discovery_path()?;
    let s = fs::read_to_string(&p).with_context(|| format!("daemon not running? {p:?}"))?;
    Ok(serde_json::from_str(&s)?)
}

/// Blocking entry point for `hdmicap daemon`. Builds the tokio runtime itself
/// so the capture thread can stay a plain std::thread alongside it.
pub fn run(device: DeviceSpec, port: u16) -> Result<()> {
    // 1. Acquire the advisory lock. Held for the lifetime of the process.
    let lock_file = File::create(lock_path()?)?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| anyhow!("another hdmicap daemon is already running"))?;

    // 2. Spawn the capture thread BEFORE the runtime. It owns the device and
    //    publishes into the watch channel.
    let (frames, _capture_handle) = capture_thread::spawn(device);

    // 3. Build a multi-thread runtime for axum and run the server.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let bound = listener.local_addr()?;

        // 4. Publish discovery info now that we have the real port.
        let disc = Discovery {
            pid: std::process::id(),
            port: bound.port(),
        };
        let mut f = File::create(discovery_path()?)?;
        f.write_all(serde_json::to_string(&disc)?.as_bytes())?;
        info!("hdmicap daemon listening on http://{bound}");

        let app = server::router(AppState { frames });

        // 5. Serve until SIGTERM/SIGINT. The /preview MJPEG stream is an
        //    infinite response, so a plain graceful shutdown would block on it
        //    forever. Remove the discovery file, give short in-flight requests a
        //    brief grace period, then hard-exit (the OS releases the device).
        let disc = discovery_path()?;
        let lock = lock_path()?;
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_signal().await;
                let _ = fs::remove_file(&disc);
                let _ = fs::remove_file(&lock);
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                info!("daemon shut down");
                std::process::exit(0);
            })
            .await?;

        Ok::<(), anyhow::Error>(())
    })?;

    drop(lock_file);
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.ok();
    };

    #[cfg(unix)]
    let term = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    info!("shutdown signal received");
}
````

## File: hdmicap/src/frame.rs
````rust
// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The single value that flows from the capture thread to every HTTP handler.
//!
//! Design rule: the capture thread does the cheap classification (dims, hash,
//! signal) inline, but NEVER encodes PNG/JPEG here. Encoding is lazy, done in
//! the handler that actually needs bytes, so the hot loop cost is bounded.

use std::sync::Arc;
use std::time::Instant;

use image::RgbImage;
use serde::Serialize;

/// How many consecutive same-resolution, non-black frames we require before
/// trusting the signal as `Stable`. A booting machine renegotiates HDMI at
/// firmware -> bootloader -> OS handoffs; the dongle emits black/torn frames
/// across each switch. This debounce stops an agent reading a black rectangle.
pub const STABLE_FRAMES: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    /// No capture device present / handle lost.
    NoDevice,
    /// Device is streaming but the frame is (near-)black: HDMI source off,
    /// unplugged, or mid-blank. Distinct from a stale-but-valid frame.
    NoSignal,
    /// Resolution just changed or we haven't seen enough stable frames yet.
    ModeSwitching,
    /// Frame is trustworthy for OCR / agent reading.
    Stable,
}

/// Immutable snapshot of "what's on screen right now", shared via `watch`.
#[derive(Clone)]
pub struct FrameState {
    /// Raw JPEG/MJPEG bytes as delivered by the capture device. Present when
    /// the device natively delivers MJPEG (always on Linux). The preview
    /// endpoint serves these directly — no server-side decode/re-encode.
    pub jpeg: Option<Arc<[u8]>>,
    /// Decoded RGB8, populated when jpeg is None (YUYV sources) or on demand
    /// for snapshot/OCR. Empty slice when not available.
    pub rgb: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    /// Perceptual hash of an 8x8 grayscale downscale (aHash). Powers
    /// change-detection and a secondary torn-frame check. Cheap every frame.
    pub hash: u64,
    pub signal: Signal,
    /// Bumps every time the capture resolution changes. Lets a consumer notice
    /// "the machine switched video modes" even if pixel hashes happen to match.
    pub resolution_epoch: u64,
    pub captured_at: Instant,
}

impl FrameState {
    pub fn no_device() -> Self {
        FrameState {
            jpeg: None,
            rgb: Arc::from(Vec::new().into_boxed_slice()),
            width: 0,
            height: 0,
            hash: 0,
            signal: Signal::NoDevice,
            resolution_epoch: 0,
            captured_at: Instant::now(),
        }
    }
}

/// JSON shape returned by `GET /status`. Cheap for the agent to poll.
#[derive(Serialize)]
pub struct StatusDto {
    pub signal: Signal,
    pub width: u32,
    pub height: u32,
    pub hash: String, // hex, so it round-trips cleanly into ?changed_since=
    pub resolution_epoch: u64,
    pub captured_at_ms_ago: u128,
}

impl From<&FrameState> for StatusDto {
    fn from(f: &FrameState) -> Self {
        StatusDto {
            signal: f.signal,
            width: f.width,
            height: f.height,
            hash: format!("{:016x}", f.hash),
            resolution_epoch: f.resolution_epoch,
            captured_at_ms_ago: f.captured_at.elapsed().as_millis(),
        }
    }
}

/// aHash over an 8x8 grayscale downscale (64 bits, one per pixel). Robust to
/// capture noise and cheap enough to run on every frame. Uses a bilinear
/// (Triangle) filter, which is sufficient for mode-switch discrimination.
pub fn ahash(img: &RgbImage) -> u64 {
    use image::imageops::{grayscale, resize, FilterType};
    let small = resize(&grayscale(img), 8, 8, FilterType::Triangle);
    let pixels: Vec<u8> = small.pixels().map(|p| p.0[0]).collect();
    let avg = (pixels.iter().map(|&p| p as u32).sum::<u32>() / pixels.len() as u32) as u8;
    let mut bits = 0u64;
    for (i, &p) in pixels.iter().enumerate() {
        if p >= avg {
            bits |= 1 << i;
        }
    }
    bits
}

/// (Near-)black detection: low mean luma + low variance => NoSignal.
///
/// Thresholds are intentionally conservative (mean < 10, var < 64). Tune
/// against the real MS2109 if dark-grey blanking frames cause false negatives.
pub fn is_no_signal(img: &RgbImage) -> bool {
    let mut sum = 0u64;
    let mut sum_sq = 0u64;
    let n = (img.width() * img.height()) as u64;
    if n == 0 {
        return true;
    }
    for p in img.pixels() {
        // Rec.601-ish luma, integer.
        let y = (p.0[0] as u64 * 77 + p.0[1] as u64 * 150 + p.0[2] as u64 * 29) >> 8;
        sum += y;
        sum_sq += y * y;
    }
    let mean = sum / n;
    let var = (sum_sq / n).saturating_sub(mean * mean);
    mean < 10 && var < 64
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    fn solid_image(r: u8, g: u8, b: u8, w: u32, h: u32) -> RgbImage {
        let pixels: Vec<u8> = (0..w * h)
            .flat_map(|_| [r, g, b])
            .collect();
        RgbImage::from_raw(w, h, pixels).unwrap()
    }

    #[test]
    fn black_is_no_signal() {
        assert!(is_no_signal(&solid_image(0, 0, 0, 320, 240)));
    }

    #[test]
    fn near_black_is_no_signal() {
        // Dark grey (luma ~7) should still register as no-signal.
        assert!(is_no_signal(&solid_image(8, 8, 8, 320, 240)));
    }

    #[test]
    fn content_frame_is_not_no_signal() {
        // Mid-grey has enough luma.
        assert!(!is_no_signal(&solid_image(128, 128, 128, 320, 240)));
    }

    #[test]
    fn ahash_same_image_stable() {
        let img = solid_image(100, 150, 200, 320, 240);
        assert_eq!(ahash(&img), ahash(&img));
    }

    #[test]
    fn ahash_different_images_differ() {
        // aHash measures structure (above/below mean), not absolute brightness.
        // Solid images of any value all produce the same hash (all bits set),
        // so we need images with opposing gradients: left-dark/right-bright vs
        // left-bright/right-dark.
        let w = 320u32;
        let h = 240u32;
        let gradient = |left_dark: bool| {
            let pixels: Vec<u8> = (0..w * h)
                .flat_map(|i| {
                    let x = i % w;
                    let v: u8 = if (x < w / 2) == left_dark { 50 } else { 200 };
                    [v, v, v]
                })
                .collect();
            RgbImage::from_raw(w, h, pixels).unwrap()
        };
        assert_ne!(ahash(&gradient(true)), ahash(&gradient(false)));
    }
}
````

## File: hdmicap/Cargo.toml
````toml
[package]
name = "hdmicap"
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"
description = "Warm-stream HDMI capture daemon optimized for agent screenshotting + human preview"

[[bin]]
name = "hdmicap"
path = "src/main.rs"

[dependencies]
# --- Capture -------------------------------------------------------------
# nokhwa unifies V4L2 (Linux) and AVFoundation (macOS) behind one API.
# `input-native` pulls the right per-OS backend in recent 0.10.x.
nokhwa = { version = "0.10", features = ["input-native"] }

# --- Async runtime + HTTP ------------------------------------------------
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "signal", "time", "net", "process", "io-util"] }
axum = "0.7"

# --- Imaging -------------------------------------------------------------
image = { version = "0.25", default-features = false, features = ["png", "jpeg"] }

# --- CLI -----------------------------------------------------------------
clap = { version = "4", features = ["derive"] }

# --- Daemon plumbing -----------------------------------------------------
fs2 = "0.4"
directories = "5"

# --- Errors / logging / serde -------------------------------------------
anyhow = "1"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# --- Client subcommands --------------------------------------------------
ureq = "2"
nix = { version = "0.29", features = ["signal", "process"] }

# --- Preview stream ------------------------------------------------------
async-stream = "0.3"
bytes = "1"

# On Linux we bypass nokhwa for the actual frame loop and use v4l directly
# so we can call stream.set_timeout() and avoid an indefinite VIDIOC_DQBUF block.
# turbojpeg provides fast MJPEG decode (~5ms vs ~50ms pure-Rust) for signal detection.
[target.'cfg(target_os = "linux")'.dependencies]
v4l = "0.14"
turbojpeg = { version = "1.4", features = ["image"] }

[patch.crates-io]
# Local patch: skip activeVideoMin/MaxFrameDuration KVC calls that throw NSException
# on HDMI capture cards (MS2109) under AVFoundation. See vendor/nokhwa-bindings-macos.
nokhwa-bindings-macos = { path = "vendor/nokhwa-bindings-macos" }

[profile.release]
opt-level = 3
lto = "thin"
````

## File: serialcap/src/capture.rs
````rust
// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Timestamped, line-oriented capture of serial output.
//!
//! The daemon owns the port and is the only process that sees every byte, so it
//! assembles incoming bytes into lines, stamps each with a UTC timestamp and a
//! monotonic sequence number, and appends them to a rolling on-disk log under the
//! runtime dir. History survives daemon restarts (the sequence counter resumes
//! from the last line on disk) and grows well past the live-view window; old
//! lines age out by segment rotation.
//!
//! The `serialcap log` client reads these files directly — no daemon round-trip,
//! and it still works after the daemon has stopped. The current unterminated line
//! (e.g. a `login:` prompt that hasn't emitted a newline yet) lives only in the
//! daemon's memory, so it is mirrored to a small sidecar file the reader folds in
//! as the most recent (partial) line.
//!
//! Lines are stored *raw* (ANSI escapes and control bytes preserved); the reader
//! cleans them for display unless `--raw` is given, so no information is lost.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::error;

/// Default number of lines retained across all rotated segments.
pub const DEFAULT_BUFFER_LINES: u64 = 50_000;
/// Rotated segments kept (active + this many `.N` files).
const MAX_SEGMENTS: usize = 5;
/// Hard cap on a single unterminated line; bytes past this are force-flushed so a
/// newline-less stream can't grow the pending buffer without bound.
const MAX_PENDING: usize = 64 * 1024;

const ACTIVE: &str = "serial.jsonl";
const PENDING: &str = "pending.json";

/// One line of captured serial output.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Line {
    pub seq: u64,
    /// Wall-clock capture time, milliseconds since the Unix epoch (UTC).
    pub ts_ms: u64,
    /// Raw line text: ANSI escapes / control bytes preserved, trailing CR removed.
    pub text: String,
    /// True only for the in-flight line that has not seen its newline yet.
    #[serde(default, skip_serializing_if = "is_false")]
    pub partial: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Base directory holding per-interface capture sub-directories. Shares the
/// daemon's runtime dir so the writer (daemon) and reader (`serialcap log`)
/// always agree on the path.
pub fn capture_dir() -> Result<PathBuf> {
    let dir = crate::daemon::runtime_dir()?.join("capture");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The capture sub-directory for one named interface.
pub fn interface_dir(base: &Path, name: &str) -> PathBuf {
    base.join(sanitize(name))
}

/// Make an interface name safe as a single path component (interface names are
/// user-chosen). Keeps alphanumerics, `-`, `_`, `.`; collapses everything else
/// to `_`. An empty/degenerate result falls back to `_`.
fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() || s == "." || s == ".." {
        "_".to_string()
    } else {
        s
    }
}

/// Names of interfaces that have a capture sub-directory under `base`.
fn list_interface_dirs(base: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(base) {
        for e in entries.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(n) = e.file_name().to_str() {
                    names.push(n.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

// ── writer (owned by the capture thread) ────────────────────────────────────

/// Append-only writer that turns a byte stream into timestamped lines on disk.
/// Lives on a dedicated thread; never shared, so it needs no locking.
pub struct LineLog {
    dir: PathBuf,
    active_path: PathBuf,
    writer: Option<File>,
    next_seq: u64,
    seg_lines: u64,
    active_lines: u64,
    pending: Vec<u8>,
    pending_ts: Option<u64>,
}

impl LineLog {
    /// Open (creating it if needed) the capture log in `dir`, resuming the
    /// sequence counter after the highest line already on disk. A stale pending
    /// sidecar from a previous run is discarded (its line was never completed).
    pub fn open(dir: PathBuf, buffer_lines: u64) -> Self {
        let _ = fs::create_dir_all(&dir);
        let active_path = dir.join(ACTIVE);
        let seg_lines = (buffer_lines / MAX_SEGMENTS as u64).max(1);
        let next_seq = recover_next_seq(&dir);
        let active_lines = count_lines(&active_path);
        let _ = fs::remove_file(dir.join(PENDING));
        let writer = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&active_path)
            .ok();
        LineLog {
            dir,
            active_path,
            writer,
            next_seq,
            seg_lines,
            active_lines,
            pending: Vec::new(),
            pending_ts: None,
        }
    }

    /// Feed a chunk of received bytes. Completed lines are appended to the log;
    /// any trailing partial line is mirrored to the sidecar.
    pub fn ingest(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if b == b'\n' {
                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                let text = String::from_utf8_lossy(&self.pending).into_owned();
                let ts = self.pending_ts.take().unwrap_or_else(now_ms);
                self.commit(ts, text);
                self.pending.clear();
            } else {
                if self.pending.is_empty() {
                    self.pending_ts = Some(now_ms());
                }
                self.pending.push(b);
                if self.pending.len() >= MAX_PENDING {
                    let text = String::from_utf8_lossy(&self.pending).into_owned();
                    let ts = self.pending_ts.take().unwrap_or_else(now_ms);
                    self.commit(ts, text);
                    self.pending.clear();
                }
            }
        }
        self.write_pending_sidecar();
    }

    fn commit(&mut self, ts_ms: u64, text: String) {
        let line = Line {
            seq: self.next_seq,
            ts_ms,
            text,
            partial: false,
        };
        self.next_seq += 1;
        match self.writer.as_mut() {
            None => error!("capture writer is None; line seq={} lost", line.seq),
            Some(w) => match serde_json::to_string(&line) {
                Err(e) => error!("capture serialize seq={} failed: {e}", line.seq),
                Ok(mut s) => {
                    s.push('\n');
                    match w.write_all(s.as_bytes()) {
                        Ok(()) => self.active_lines += 1,
                        Err(e) => error!("capture write seq={} failed: {e}", line.seq),
                    }
                }
            },
        }
        if self.active_lines >= self.seg_lines {
            self.rotate();
        }
    }

    /// Shift `serial.jsonl(.k)` → `.k+1`, dropping the oldest, and start a fresh
    /// active segment. Keeps at most `MAX_SEGMENTS` files.
    fn rotate(&mut self) {
        self.writer = None; // close before renaming
        let _ = fs::remove_file(self.dir.join(format!("{ACTIVE}.{}", MAX_SEGMENTS - 1)));
        for k in (1..MAX_SEGMENTS - 1).rev() {
            let from = self.dir.join(format!("{ACTIVE}.{k}"));
            let to = self.dir.join(format!("{ACTIVE}.{}", k + 1));
            let _ = fs::rename(&from, &to);
        }
        let _ = fs::rename(&self.active_path, self.dir.join(format!("{ACTIVE}.1")));
        self.writer = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.active_path)
            .ok();
        self.active_lines = 0;
    }

    /// Mirror the current unterminated line to the sidecar (or remove it when the
    /// pending buffer is empty). Written via a temp file + rename so a concurrent
    /// reader never sees a half-written sidecar.
    fn write_pending_sidecar(&self) {
        let path = self.dir.join(PENDING);
        if self.pending.is_empty() {
            let _ = fs::remove_file(&path);
            return;
        }
        let line = Line {
            seq: self.next_seq,
            ts_ms: self.pending_ts.unwrap_or_else(now_ms),
            text: String::from_utf8_lossy(&self.pending).into_owned(),
            partial: true,
        };
        if let Ok(s) = serde_json::to_string(&line) {
            let tmp = self.dir.join("pending.tmp");
            if fs::write(&tmp, s.as_bytes()).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
    }
}

// ── reader (used by `serialcap log`) ─────────────────────────────────────────

/// A line selection for [`read_lines`]. An unset field imposes no constraint.
#[derive(Default)]
pub struct Query {
    /// Keep only the most recent N lines (applied after the seq filters).
    pub tail: Option<u64>,
    /// Lowest sequence number to include (inclusive).
    pub from: Option<u64>,
    /// Highest sequence number to include (inclusive).
    pub to: Option<u64>,
    /// Only lines with `seq` strictly greater than this.
    pub since: Option<u64>,
    /// Fold in the current unterminated line as the last (partial) entry.
    pub include_pending: bool,
}

/// Read captured lines from `dir`, oldest first, applying `q`.
pub fn read_lines(dir: &Path, q: &Query) -> Vec<Line> {
    let mut all: Vec<Line> = Vec::new();
    for k in (1..MAX_SEGMENTS).rev() {
        for_each_line(&dir.join(format!("{ACTIVE}.{k}")), |l| all.push(l));
    }
    for_each_line(&dir.join(ACTIVE), |l| all.push(l));

    if q.include_pending {
        if let Some(p) = read_pending(dir) {
            all.push(p);
        }
    }

    let mut out: Vec<Line> = all
        .into_iter()
        .filter(|l| {
            if let Some(s) = q.since {
                if l.seq <= s {
                    return false;
                }
            }
            if let Some(f) = q.from {
                if l.seq < f {
                    return false;
                }
            }
            if let Some(t) = q.to {
                if l.seq > t {
                    return false;
                }
            }
            true
        })
        .collect();

    if let Some(n) = q.tail {
        let n = n as usize;
        if out.len() > n {
            out.drain(0..out.len() - n);
        }
    }
    out
}

fn read_pending(dir: &Path) -> Option<Line> {
    let s = fs::read_to_string(dir.join(PENDING)).ok()?;
    serde_json::from_str(&s).ok()
}

/// Parse each JSON-lines record in `path`, invoking `f` per valid line. Missing
/// files and unparseable lines (e.g. a torn final append) are skipped silently.
fn for_each_line(path: &Path, mut f: impl FnMut(Line)) {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if let Ok(parsed) = serde_json::from_str::<Line>(&line) {
            f(parsed);
        }
    }
}

fn recover_next_seq(dir: &Path) -> u64 {
    let mut max_seq: Option<u64> = None;
    for k in (1..MAX_SEGMENTS).rev() {
        for_each_line(&dir.join(format!("{ACTIVE}.{k}")), |l| {
            max_seq = Some(max_seq.map_or(l.seq, |m| m.max(l.seq)));
        });
    }
    for_each_line(&dir.join(ACTIVE), |l| {
        max_seq = Some(max_seq.map_or(l.seq, |m| m.max(l.seq)));
    });
    max_seq.map_or(0, |m| m + 1)
}

fn count_lines(path: &Path) -> u64 {
    let mut n = 0;
    for_each_line(path, |_| n += 1);
    n
}

// ── the `log` subcommand ─────────────────────────────────────────────────────

/// Options for [`cmd_log`], mirroring the `serialcap log` CLI flags.
pub struct LogArgs {
    pub interface: Option<String>,
    pub tail: Option<u64>,
    pub from: Option<u64>,
    pub to: Option<u64>,
    pub since: Option<u64>,
    pub raw: bool,
    pub json: bool,
    pub no_pending: bool,
}

/// Print captured lines to stdout per `args`.
pub fn cmd_log(args: LogArgs) -> Result<()> {
    let base = capture_dir().context("locating capture dir")?;
    let dir = match &args.interface {
        Some(name) => interface_dir(&base, name),
        None => {
            let names = list_interface_dirs(&base);
            match names.len() {
                // Exactly one interface (or none yet): no need to disambiguate.
                0 | 1 => names.first().map_or(base.clone(), |n| base.join(n)),
                // Expected user-choice condition, not a fault: a clean message,
                // no error chain / backtrace.
                _ => {
                    eprintln!(
                        "serialcap: multiple interfaces captured ({}); pass --interface NAME",
                        names.join(", ")
                    );
                    std::process::exit(2);
                }
            }
        }
    };

    // With no selector, default to a recent window rather than the whole history.
    let no_selector =
        args.tail.is_none() && args.from.is_none() && args.to.is_none() && args.since.is_none();
    let q = Query {
        tail: if no_selector { Some(200) } else { args.tail },
        from: args.from,
        to: args.to,
        since: args.since,
        include_pending: !args.no_pending,
    };

    let lines = read_lines(&dir, &q);
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for l in lines {
        if args.json {
            if let Ok(s) = serde_json::to_string(&l) {
                let _ = writeln!(out, "{s}");
            }
        } else {
            let text = if args.raw {
                l.text
            } else {
                strip_ansi(&l.text)
            };
            let seq = if l.partial {
                format!("{}*", l.seq)
            } else {
                l.seq.to_string()
            };
            let _ = writeln!(out, "[{}] #{:<7} {}", format_utc(l.ts_ms), seq, text);
        }
    }
    Ok(())
}

// ── formatting helpers ───────────────────────────────────────────────────────

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Format epoch milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC). Done by hand so
/// the crate needs no calendar dependency.
fn format_utc(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let millis = ms % 1000;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60,
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 → (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (y + if m <= 2 { 1 } else { 0 }, m as u32, d)
}

/// Strip ANSI escape sequences and control noise for readable, agent-friendly
/// text. Removes CSI/OSC and other escape sequences, applies bare-`\r` carriage
/// returns as overwrites (keeps text after the last `\r`), and drops remaining
/// control characters except tab. Operates on raw bytes so it is robust to
/// partial UTF-8.
pub fn strip_ansi(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'[' => {
                    // CSI: parameters/intermediates until a final byte 0x40..=0x7e.
                    i += 1;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    if i < bytes.len() {
                        i += 1;
                    }
                }
                b']' => {
                    // OSC: until BEL or ST (ESC \).
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => i += 1, // two-byte escape: drop the following byte too
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }

    let cleaned = String::from_utf8_lossy(&out);
    // Bare carriage returns redraw the line; keep only the final overwrite.
    let cleaned = match cleaned.rfind('\r') {
        Some(idx) => &cleaned[idx + 1..],
        None => &cleaned,
    };
    cleaned
        .chars()
        .filter(|&c| c == '\t' || !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "serialcap-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn splits_lines_and_strips_cr() {
        let dir = tmp();
        let mut log = LineLog::open(dir.clone(), DEFAULT_BUFFER_LINES);
        log.ingest(b"hello\r\nworld\n");
        let lines = read_lines(&dir, &Query::default());
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "hello");
        assert_eq!(lines[0].seq, 0);
        assert_eq!(lines[1].text, "world");
        assert_eq!(lines[1].seq, 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unterminated_line_is_pending() {
        let dir = tmp();
        let mut log = LineLog::open(dir.clone(), DEFAULT_BUFFER_LINES);
        log.ingest(b"done\nlogin: ");
        let with = read_lines(
            &dir,
            &Query {
                include_pending: true,
                ..Default::default()
            },
        );
        assert_eq!(with.len(), 2);
        assert!(with[1].partial);
        assert_eq!(with[1].text, "login: ");
        assert_eq!(with[1].seq, 1); // seq it will take once completed

        let without = read_lines(&dir, &Query::default());
        assert_eq!(without.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tail_and_range_and_since() {
        let dir = tmp();
        let mut log = LineLog::open(dir.clone(), DEFAULT_BUFFER_LINES);
        for i in 0..10 {
            log.ingest(format!("line{i}\n").as_bytes());
        }
        let tail = read_lines(
            &dir,
            &Query {
                tail: Some(3),
                ..Default::default()
            },
        );
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0].text, "line7");
        assert_eq!(tail[2].text, "line9");

        let range = read_lines(
            &dir,
            &Query {
                from: Some(2),
                to: Some(4),
                ..Default::default()
            },
        );
        assert_eq!(
            range.iter().map(|l| l.seq).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );

        let since = read_lines(
            &dir,
            &Query {
                since: Some(7),
                ..Default::default()
            },
        );
        assert_eq!(since.iter().map(|l| l.seq).collect::<Vec<_>>(), vec![8, 9]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn seq_resumes_after_reopen() {
        let dir = tmp();
        {
            let mut log = LineLog::open(dir.clone(), DEFAULT_BUFFER_LINES);
            log.ingest(b"a\nb\n");
        }
        let mut log = LineLog::open(dir.clone(), DEFAULT_BUFFER_LINES);
        log.ingest(b"c\n");
        let lines = read_lines(&dir, &Query::default());
        assert_eq!(
            lines.iter().map(|l| l.seq).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(lines[2].text, "c");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rotation_drops_oldest_keeps_recent() {
        let dir = tmp();
        // buffer_lines / MAX_SEGMENTS = seg_lines; 50 / 5 = 10 lines per segment.
        let mut log = LineLog::open(dir.clone(), 50);
        for i in 0..120 {
            log.ingest(format!("line{i}\n").as_bytes());
        }
        let lines = read_lines(&dir, &Query::default());
        // At most MAX_SEGMENTS * seg_lines retained, oldest aged out.
        assert!(lines.len() <= 50, "retained {} lines", lines.len());
        assert_eq!(lines.last().unwrap().text, "line119");
        assert_eq!(lines.last().unwrap().seq, 119);
        // Sequence numbers stay monotonic and contiguous over what survives.
        for w in lines.windows(2) {
            assert_eq!(w[1].seq, w[0].seq + 1);
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn strip_ansi_removes_color_and_handles_cr() {
        assert_eq!(strip_ansi("\x1b[1;32mgreen\x1b[0m text"), "green text");
        assert_eq!(strip_ansi("progress 10%\rprogress 90%"), "progress 90%");
        assert_eq!(strip_ansi("keep\ttab"), "keep\ttab");
        assert_eq!(strip_ansi("bell\x07gone"), "bellgone");
        // OSC title sequence terminated by BEL.
        assert_eq!(strip_ansi("\x1b]0;my title\x07shell"), "shell");
    }

    #[test]
    fn format_utc_known_values() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00.000Z");
        // 2021-01-01T00:00:00Z = 1609459200 s.
        assert_eq!(format_utc(1_609_459_200_000), "2021-01-01T00:00:00.000Z");
        assert_eq!(format_utc(1_609_459_200_123), "2021-01-01T00:00:00.123Z");
    }
}
````

## File: serialcap/src/daemon.rs
````rust
// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Daemon lifecycle: advisory lock, discovery file, runtime wiring, shutdown.
//! Mirrors hdmicap's daemon so the two read the same way.

use std::fs::{self, File};
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::serial_io::{InterfaceSpec, Serials};
use crate::server::{self, AppState};

#[derive(Serialize, Deserialize)]
pub struct DiscoveryInterface {
    pub name: String,
    pub device: String,
    pub baud: u32,
}

#[derive(Serialize, Deserialize)]
pub struct Discovery {
    pub pid: u32,
    pub port: u16,
    pub interfaces: Vec<DiscoveryInterface>,
}

pub fn runtime_dir() -> Result<PathBuf> {
    let dirs = directories::BaseDirs::new().context("no base dirs")?;
    let base = dirs
        .runtime_dir()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("serialcap");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn lock_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("daemon.lock"))
}

fn discovery_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("daemon.json"))
}

/// Read the discovery file so the CLI knows which port to hit.
pub fn discover() -> Result<Discovery> {
    let p = discovery_path()?;
    let s = fs::read_to_string(&p).with_context(|| format!("daemon not running? {p:?}"))?;
    Ok(serde_json::from_str(&s)?)
}

/// Blocking entry point for `serialcap daemon`.
pub fn run(interfaces: Vec<InterfaceSpec>, port: u16, buffer_lines: u64) -> Result<()> {
    if interfaces.is_empty() {
        return Err(anyhow!("no serial interfaces specified"));
    }

    let lock_file = File::create(lock_path()?)?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| anyhow!("another serialcap daemon is already running"))?;

    let capture_dir = crate::capture::capture_dir()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move {
        // The serial supervisors use tokio::spawn, so start them inside the runtime.
        let serials = Serials::spawn_all(&interfaces, &capture_dir, buffer_lines);

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let bound = listener.local_addr()?;

        let disc = Discovery {
            pid: std::process::id(),
            port: bound.port(),
            interfaces: interfaces
                .iter()
                .map(|i| DiscoveryInterface {
                    name: i.name.clone(),
                    device: i.device.clone(),
                    baud: i.baud,
                })
                .collect(),
        };
        let mut f = File::create(discovery_path()?)?;
        f.write_all(serde_json::to_string(&disc)?.as_bytes())?;
        info!(
            "serialcap daemon listening on http://{bound} ({} interface(s))",
            interfaces.len()
        );

        let app = server::router(AppState { serials });

        // The /stream WebSocket is long-lived, so a plain graceful shutdown
        // would block on it forever. Remove the discovery file, give short
        // in-flight requests a brief grace period, then hard-exit (the OS
        // releases the serial port).
        let disc = discovery_path()?;
        let lock = lock_path()?;
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_signal().await;
                let _ = fs::remove_file(&disc);
                let _ = fs::remove_file(&lock);
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                info!("daemon shut down");
                std::process::exit(0);
            })
            .await?;

        Ok::<(), anyhow::Error>(())
    })?;

    drop(lock_file);
    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.ok();
    };

    #[cfg(unix)]
    let term = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    info!("shutdown signal received");
}
````

## File: serialcap/src/main.rs
````rust
// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! serialcap — serial console daemon + thin client CLI.
//!
//! Subcommands:
//!   daemon   own a serial port and serve it over a localhost WebSocket
//!   log      print captured serial output (timestamped, by line range)
//!   devices  list serial devices
//!   stop     ask the running daemon to exit (SIGTERM)

mod capture;
mod daemon;
mod serial_io;
mod server;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::serial_io::InterfaceSpec;

#[derive(Parser)]
#[command(name = "serialcap", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Parse a `NAME=DEVICE[@BAUD][:SENSE]` interface spec.
///
/// - `BAUD` defaults to 115200 when omitted.
/// - `SENSE` is optional; valid values are `cts`, `dsr`, `dcd`, `ri`.
///   It denotes which FTDI modem-control input is wired to the target's 3.3 V
///   rail for power-state sensing.
///
/// Example: `console=/dev/ttyUSB0@115200:cts`
fn parse_interface(s: &str) -> Result<InterfaceSpec, String> {
    let (name, rest) = s
        .split_once('=')
        .ok_or("expected NAME=DEVICE[@BAUD][:SENSE], e.g. console=/dev/ttyUSB0@115200:cts")?;
    if name.is_empty() {
        return Err("interface name is empty".into());
    }

    // Peel off optional :SENSE suffix. Only recognised signal names count;
    // anything else (including colons embedded in /dev/serial/by-path/… paths)
    // is treated as part of the device path.
    let (dev_baud, power_sense_signal) = if let Some((prefix, maybe_sense)) = rest.rsplit_once(':')
    {
        match maybe_sense {
            "cts" | "dsr" | "dcd" | "ri" => (prefix, Some(maybe_sense.to_string())),
            _ => (rest, None),
        }
    } else {
        (rest, None)
    };

    let (device, baud) = match dev_baud.rsplit_once('@') {
        Some((dev, b)) => (
            dev,
            b.parse::<u32>()
                .map_err(|_| format!("invalid baud '{b}'"))?,
        ),
        None => (dev_baud, 115_200_u32),
    };
    if device.is_empty() {
        return Err("device path is empty".into());
    }
    Ok(InterfaceSpec {
        name: name.to_string(),
        device: device.to_string(),
        baud,
        power_sense_signal,
    })
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the serial console daemon (foreground; controller manages it).
    ///
    /// Owns one or more named interfaces; repeat --interface for each.
    Daemon {
        /// A serial interface as NAME=DEVICE[@BAUD] (repeatable), e.g.
        /// console=/dev/ttyUSB0@115200.
        #[arg(long = "interface", value_name = "NAME=DEVICE[@BAUD]", value_parser = parse_interface, required = true)]
        interfaces: Vec<InterfaceSpec>,
        /// Port to bind on localhost. 0 = OS-assigned.
        #[arg(long, default_value_t = 8724)]
        port: u16,
        /// Approximate number of recent lines retained on disk, per interface.
        #[arg(long, default_value_t = capture::DEFAULT_BUFFER_LINES)]
        buffer_lines: u64,
    },
    /// Print captured serial output. Reads the daemon's on-disk log directly, so
    /// it works whether or not the daemon is currently running.
    Log {
        /// Interface name. Optional when only one interface has been captured.
        #[arg(long, short = 'i')]
        interface: Option<String>,
        /// Show only the most recent N lines.
        #[arg(long, short = 'n')]
        tail: Option<u64>,
        /// Lowest line sequence number to include (inclusive).
        #[arg(long)]
        from: Option<u64>,
        /// Highest line sequence number to include (inclusive).
        #[arg(long)]
        to: Option<u64>,
        /// Show only lines newer than this sequence number (for polling).
        #[arg(long)]
        since: Option<u64>,
        /// Keep raw bytes (ANSI escapes, control chars) instead of cleaning them.
        #[arg(long)]
        raw: bool,
        /// Emit JSON Lines (seq, ts_ms, text) instead of formatted text.
        #[arg(long)]
        json: bool,
        /// Exclude the current unterminated line.
        #[arg(long)]
        no_pending: bool,
    },
    /// List available serial devices and exit (no daemon needed).
    Devices,
    /// Tell the running daemon to shut down.
    Stop,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "serialcap=info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Daemon {
            interfaces,
            port,
            buffer_lines,
        } => daemon::run(interfaces, port, buffer_lines),
        Cmd::Log {
            interface,
            tail,
            from,
            to,
            since,
            raw,
            json,
            no_pending,
        } => capture::cmd_log(capture::LogArgs {
            interface,
            tail,
            from,
            to,
            since,
            raw,
            json,
            no_pending,
        }),
        Cmd::Devices => cmd_devices(),
        Cmd::Stop => cmd_stop(),
    }
}

fn cmd_devices() -> Result<()> {
    for (path, misc) in serial_io::list_ports()? {
        println!("{path}  [{misc}]");
    }
    Ok(())
}

fn cmd_stop() -> Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let d = daemon::discover().context("is the daemon running?")?;
    kill(Pid::from_raw(d.pid as i32), Signal::SIGTERM)
        .context("failed to send SIGTERM to daemon")?;
    println!("daemon (pid {}) stopping", d.pid);
    Ok(())
}
````

## File: skills/paniolo/SKILL.md
````markdown
---
name: paniolo
description: Control a physical target machine (SBC, e.g. a Raspberry Pi) with the paniolo CLI during low-level bring-up — netboot it over a direct USB-Ethernet link, watch and OCR its HDMI screen, drive its serial console, and power-cycle it. Use when you need to boot, observe, type into, screenshot, read the screen of, or power a target/board through paniolo.
---

# Paniolo — controlling a target machine

Paniolo drives a physical target machine (a single-board computer such as a
Raspberry Pi) during firmware/OS bring-up. It runs on the control host that is
physically wired to the target (USB-Ethernet for netboot, an HDMI capture
dongle for video, a USB-serial adapter for the console, a smart plug for power).

Almost every command operates on a named **target**. If exactly one target is
configured, the name can be omitted.

## First-time setup

```
cd ~/src/paniolo
uv tool install .          # installs the `paniolo` CLI into ~/.local/bin
paniolo setup              # installs dnsmasq, tftp-now, hdmicap, serialcap, visionocr
```

`uv tool install .` is required first — without it the `paniolo` command doesn't
exist yet. Run both steps once per machine. Make sure `~/.local/bin` (uv tools)
and `~/.cargo/bin` (Rust daemons) are on your `PATH`.

To pick up Python code changes after pulling or editing:

```
cd ~/src/paniolo && uv tool install --reinstall .
```

## Configure a target

```
paniolo target set <name> --interface <iface> \
    [--tftp-root <dir>] \
    [--ha-power-entity <switch.entity>]
```

- `--interface` auto-detects a USB-Ethernet adapter if omitted.
- Serial consoles are configured separately with `paniolo serial setup` (a target
  can have several named interfaces); they're preserved across `target set` runs.
- Inspect or remove: `paniolo target show` / `paniolo target clear <name>`.

## Netboot (DHCP + TFTP)

Boot a board over the direct USB-Ethernet link:

```
paniolo netboot start [target]        # serve DHCP + TFTP on the interface
paniolo netboot tftp-root [target]    # print where to drop boot files
paniolo netboot status [target]
paniolo netboot logs -f [target]      # follow the combined log
paniolo netboot stop [target]
```

Put boot files in the target's TFTP root (for a Raspberry Pi 5, the kernel goes
in as `kernel_2712.img`). Needs passwordless `sudo` for `ifconfig` (it assigns
the interface's static IP).

## Video — capture, preview, OCR

```
paniolo video setup                   # detect + save the capture device
paniolo video watch                   # start the capture daemon (background)
paniolo video preview                 # open the dashboard in a browser
paniolo video shot [--stable] [--out frame.png]   # one lossless PNG
paniolo video read [--stable]         # OCR the current screen, print text
paniolo video show                    # device + daemon status
paniolo video stop
```

- The **dashboard** (default `http://127.0.0.1:8723/`) shows live video on top, a
  serial terminal below, and an **OCR button** that reads the current screen.
- `--stable` waits for a steady frame before capturing (useful right after a mode
  switch or reboot).
- **OCR** (`video read` and the dashboard button) is on-device (Apple Vision). It
  reads large boot-screen / BIOS text well; very small console fonts can produce
  a few character confusions (e.g. `1`/`l`, `2`/`Z`).

## Serial console

A target can have **several named serial interfaces** (e.g. a main `console` and
a `bmc`). Each port is **exclusive** — only one consumer can hold it at a time —
but a single `watch` daemon owns *all* of them at once.

```
paniolo serial setup [target] --name console   # add/update a named interface
                                               #   (--device auto-detected if omitted)
paniolo serial setup [target] --name bmc --device /dev/ttyUSB1 --baud 9600
paniolo serial remove <name> [-t target]       # drop a named interface
paniolo serial connect [target] [-i name]      # interactive terminal (tio) in your shell
paniolo serial watch [target]                  # run the daemon for ALL interfaces;
                                               #   they appear in the dashboard pane
paniolo serial log [-i name] [options]         # print captured output (timestamped)
paniolo serial show [target]                   # list interfaces + daemon status
paniolo serial stop                            # release the ports
paniolo serial devices                         # list serial devices on the host
paniolo serial dtr [target] [-i name] [--ms N] # pulse DTR line (J2 power button header)
paniolo serial reset [target] [-i name]        # soft reset via brief DTR pulse
```

`--name` defaults to `console`, so a single-interface setup needs no flags. With
one interface, `-i`/`--interface` can be omitted everywhere. Don't run `connect`
and `watch` (or an external `screen`/`tio`) on the **same device** at once —
start one, or `stop`/close the other first.

### Reading captured output

While `watch` is running, the daemon keeps a **rolling, timestamped capture log**
per interface (persisted on disk, so it survives a daemon restart and you can read
it even after `stop`). The live view stays in the dashboard; use `serial log` when
you want to *read back* what scrolled past:

```
paniolo serial log                    # most recent ~200 lines (sole interface)
paniolo serial log -i bmc --tail 50   # most recent 50 lines of the 'bmc' interface
paniolo serial log --since 1840       # only lines after sequence #1840 (polling)
paniolo serial log --from 1800 --to 1860   # a specific line range
paniolo serial log --json             # JSON Lines (seq, ts_ms, text) for parsing
paniolo serial log --raw              # keep ANSI colors / control bytes
```

With more than one interface configured, pass `-i <name>` to choose one (omitting
it errors and lists the names). Each line is shown as `[<UTC timestamp>] #<seq>
<text>`. The `seq` is a stable, monotonic line number — note it, then come back
later with `--since <seq>` to get only what's new, or `--from/--to` to re-read an
exact span. Output is ANSI-stripped by default; a `*` after the sequence number
marks the current unterminated line (e.g. a `login:` prompt with no newline yet).

## Power control

```
paniolo power-cycle [target]           # run the target's power_cycle_cmd script
paniolo power-state [target]           # show power state (requires sense signal + daemon)
paniolo serial dtr [target] [--ms N]   # pulse DTR line on J2 header (soft/hard press)
paniolo serial reset [target]          # soft reset via brief DTR pulse
```

`power-cycle` runs the shell script set with
`paniolo target set <name> --power-cycle-cmd <script>`.
The script is responsible for the full off→on sequence (HA API, PDU relay, GPIO, etc.).

DTR commands drive the target's physical power button via an FTDI serial
adapter wired to the Pi J2 header. A ≤500 ms pulse is a soft button event; ≥3000 ms
is a hard PMIC power-off. Set the default interface with
`paniolo target set <name> --power-serial console`.

## Driving it remotely over SSH

The control host runs paniolo; you can operate it from anywhere:

```
ssh control "paniolo netboot start fortune"
TFTP=$(ssh control "paniolo netboot tftp-root fortune")
scp kernel.img control:"$TFTP/kernel_2712.img"
ssh control "paniolo netboot logs -f fortune"
ssh control "paniolo power-cycle fortune"
ssh control "paniolo netboot stop fortune"
```

## Quick reference — gotchas

- Serial port is exclusive: one of `connect` / `watch` / external `tio`/`screen`.
- `~/.local/bin` (uv tool) and `~/.cargo/bin` (Rust daemons) must be on `PATH`.
- `paniolo console` auto-starts both daemons if they aren't running.
- Netboot requires passwordless `sudo` (`ip` on Linux, `ifconfig` on macOS).
- OCR is strongest on large text; tiny console fonts may misread some characters.

---

Licensed under the Apache License, Version 2.0.
````

## File: src/paniolo/_config.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

from __future__ import annotations

import dataclasses
import logging
import re
import tomllib
from pathlib import Path
from typing import Optional

log = logging.getLogger(__name__)

_CTRL_RE = re.compile(r"[\x00-\x1f\x7f]")
_CTRL_ESCAPE = {"\n": "\\n", "\r": "\\r", "\t": "\\t", "\x08": "\\b", "\x0c": "\\f"}

CONFIG_DIR = Path.home() / ".config" / "paniolo"
TARGETS_DIR = CONFIG_DIR / "targets"

DEFAULT_SERIAL_NAME = "console"


VALID_SENSE_SIGNALS = ("cts", "dsr", "dcd", "ri")


@dataclasses.dataclass
class SerialInterface:
    """A named serial console attached to a target (e.g. 'console', 'bmc')."""

    name: str
    device: str
    baud: int = 115200
    power_sense_signal: Optional[str] = None  # "cts" | "dsr" | "dcd" | "ri" | None


@dataclasses.dataclass
class TargetConfig:
    name: str
    interface: str
    host_ip: str = "192.168.99.1"
    tftp_root: Optional[str] = None
    power_cycle_cmd: Optional[str] = None
    power_serial_interface: Optional[str] = None
    serial_interfaces: list[SerialInterface] = dataclasses.field(default_factory=list)

    def serial_interface(self, name: Optional[str] = None) -> SerialInterface:
        """Resolve a serial interface by name, defaulting to the sole one.

        Raises ValueError if none are configured, the name is unknown, or no name
        was given but several exist (ambiguous)."""
        if not self.serial_interfaces:
            raise ValueError(f"no serial interfaces configured for '{self.name}'")
        if name is None:
            if len(self.serial_interfaces) == 1:
                return self.serial_interfaces[0]
            have = ", ".join(i.name for i in self.serial_interfaces)
            raise ValueError(f"multiple serial interfaces ({have}); specify one with --interface")
        for iface in self.serial_interfaces:
            if iface.name == name:
                return iface
        have = ", ".join(i.name for i in self.serial_interfaces)
        raise ValueError(f"no serial interface '{name}' (have: {have})")

    def upsert_serial_interface(self, iface: SerialInterface) -> None:
        """Add the interface, or replace an existing one with the same name."""
        for idx, existing in enumerate(self.serial_interfaces):
            if existing.name == iface.name:
                self.serial_interfaces[idx] = iface
                return
        self.serial_interfaces.append(iface)

    def remove_serial_interface(self, name: str) -> bool:
        """Drop the named interface; return True if one was removed."""
        kept = [i for i in self.serial_interfaces if i.name != name]
        removed = len(kept) != len(self.serial_interfaces)
        self.serial_interfaces = kept
        return removed


def target_path(name: str) -> Path:
    return TARGETS_DIR / f"{name}.toml"


def save_target(cfg: TargetConfig) -> None:
    TARGETS_DIR.mkdir(parents=True, exist_ok=True)
    target_path(cfg.name).write_text(_to_toml(cfg))


def load_target(name: str) -> TargetConfig:
    path = target_path(name)
    if not path.exists():
        raise FileNotFoundError(name)
    with open(path, "rb") as f:
        data = tomllib.load(f)
    return _from_dict(data)


def list_targets() -> list[str]:
    if not TARGETS_DIR.exists():
        return []
    return sorted(p.stem for p in TARGETS_DIR.glob("*.toml"))


def _from_dict(data: dict) -> TargetConfig:
    """Build a TargetConfig from parsed TOML, migrating the legacy single-serial
    fields (`serial_device`/`serial_baud`) into a named interface."""
    data = dict(data)
    serial = data.pop("serial", None)
    legacy_device = data.pop("serial_device", None)
    legacy_baud = data.pop("serial_baud", None)
    data.pop("ha_power_entity", None)  # removed field — ignore if present in old configs

    interfaces: list[SerialInterface] = []
    if serial:
        for entry in serial:
            interfaces.append(
                SerialInterface(
                    name=entry["name"],
                    device=entry["device"],
                    baud=int(entry.get("baud", 115200)),
                    power_sense_signal=entry.get("power_sense_signal"),
                )
            )
    elif legacy_device:
        interfaces.append(
            SerialInterface(
                name=DEFAULT_SERIAL_NAME,
                device=legacy_device,
                baud=int(legacy_baud or 115200),
            )
        )

    _known = {f.name for f in dataclasses.fields(TargetConfig)} - {"serial_interfaces"}
    unknown = set(data) - _known
    if unknown:
        log.warning("ignoring unknown config keys: %s", ", ".join(sorted(unknown)))
    data = {k: v for k, v in data.items() if k in _known}
    return TargetConfig(serial_interfaces=interfaces, **data)


def _escape_toml_string(s: str) -> str:
    """Escape a string for use in a TOML basic string (double-quoted)."""
    s = s.replace("\\", "\\\\").replace('"', '\\"')
    return _CTRL_RE.sub(lambda m: _CTRL_ESCAPE.get(m.group(), f"\\u{ord(m.group()):04x}"), s)


def _toml_kv(key: str, value) -> str:
    if isinstance(value, bool):
        return f'{key} = {"true" if value else "false"}'
    if isinstance(value, str):
        return f'{key} = "{_escape_toml_string(value)}"'
    return f"{key} = {value}"


def _to_toml(cfg: TargetConfig) -> str:
    scalars = {
        "name": cfg.name,
        "interface": cfg.interface,
        "host_ip": cfg.host_ip,
        "tftp_root": cfg.tftp_root,
        "power_cycle_cmd": cfg.power_cycle_cmd,
        "power_serial_interface": cfg.power_serial_interface,
    }
    lines = [_toml_kv(k, v) for k, v in scalars.items() if v is not None]
    out = "\n".join(lines) + "\n"
    for iface in cfg.serial_interfaces:
        out += "\n[[serial]]\n"
        out += _toml_kv("name", iface.name) + "\n"
        out += _toml_kv("device", iface.device) + "\n"
        out += _toml_kv("baud", iface.baud) + "\n"
        if iface.power_sense_signal is not None:
            out += _toml_kv("power_sense_signal", iface.power_sense_signal) + "\n"
    return out
````

## File: src/paniolo/_hid.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Host control client for the KB2040 HID rig (see hidrig/).

Sends line-based text commands to the control board over its USB CDC *data*
port; the control board parses them and relays HID keyboard/mouse events to the
target board, which injects them into the Pi over USB. The board owns the wire
protocol — `hidrig/control/code.py` and `hidrig/README.md` are the source of
truth. This module is a thin text-command client plus host-side sequencing.
"""

from __future__ import annotations

import dataclasses
import glob
import sys
import time
import tomllib
from pathlib import Path
from typing import Callable, Optional

from . import _config
from ._config import _toml_kv

HID_CONFIG_PATH = _config.CONFIG_DIR / "hid.toml"

DEFAULT_BAUD = 115200  # irrelevant over USB CDC, but pyserial requires a value

# Absolute-mouse logical range the OS spreads across the screen (HID convention).
ABS_MAX = 32767


@dataclasses.dataclass
class HidConfig:
    """Saved configuration for the HID control board."""

    port: str


def _to_toml(data: dict) -> str:
    lines = [_toml_kv(k, v) for k, v in data.items() if v is not None]
    return "\n".join(lines) + "\n"


def save_hid_config(cfg: HidConfig) -> None:
    _config.CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    HID_CONFIG_PATH.write_text(_to_toml(dataclasses.asdict(cfg)))


def load_hid_config() -> Optional[HidConfig]:
    if not HID_CONFIG_PATH.exists():
        return None
    with open(HID_CONFIG_PATH, "rb") as f:
        data = tomllib.load(f)
    return HidConfig(port=data["port"])


def list_serial_ports() -> list[str]:
    """Candidate USB CDC ports for the control board."""
    if sys.platform == "darwin":
        return sorted(glob.glob("/dev/cu.usbmodem*"))
    return sorted(glob.glob("/dev/ttyACM*"))


def guess_data_port() -> Optional[str]:
    """Best guess at the control board's *data* CDC port.

    The board exposes two CDC ports (console + data); the data port is
    conventionally the higher-numbered node. Returns None if no candidates.
    """
    ports = list_serial_ports()
    return ports[-1] if ports else None


def scale_to_logical(px: int, screen_px: int) -> int:
    """Map a pixel coordinate to the 0..32767 absolute-mouse logical range.

    The host OS maps that range across the full screen dimension, so callers
    scale each pixel axis against the screen's size in that axis. Clamped.
    """
    if screen_px <= 1:
        return 0
    v = round(px * ABS_MAX / (screen_px - 1))
    return max(0, min(ABS_MAX, v))


class HidRig:
    """Text-command client for the control board over USB serial.

    Pass `transport` (any object with `write(bytes)`, `readline() -> bytes`,
    `close()`) to drive it without real hardware (used by tests). Otherwise a
    `pyserial` Serial port is opened lazily on the given `port`.
    """

    def __init__(
        self,
        port: Optional[str] = None,
        baud: int = DEFAULT_BAUD,
        timeout: float = 1.0,
        transport=None,
    ):
        if transport is not None:
            self._transport = transport
            return
        try:
            import serial  # lazy: only the live path needs pyserial
        except ImportError as exc:
            raise RuntimeError(
                "pyserial not installed — install the hid extra: "
                "uv sync --extra hid  (or: pip install 'paniolo[hid]')"
            ) from exc
        if not port:
            raise ValueError("no serial port given")
        self._transport = serial.Serial(port, baud, timeout=timeout)
        time.sleep(0.2)
        self._transport.reset_input_buffer()

    def cmd(self, text: str) -> str:
        """Send one command line; return the board's reply, raise on ERR."""
        if "\n" in text or "\r" in text:
            raise ValueError(f"command contains newline: {text!r}")
        self._transport.write((text + "\n").encode("utf-8"))
        reply = self._transport.readline().decode("utf-8", "replace").strip()
        if reply.startswith("ERR"):
            raise RuntimeError(f"control board rejected '{text}': {reply}")
        return reply

    # Command wrappers — mirror hidrig/control/code.py's text protocol.
    def type(self, text: str) -> str:
        return self.cmd(f"type {text}")

    def key(self, name: str) -> str:
        return self.cmd(f"key {name}")

    def combo(self, *names: str) -> str:
        return self.cmd("combo " + " ".join(names))

    def down(self, name: str) -> str:
        return self.cmd(f"down {name}")

    def up(self, name: str) -> str:
        return self.cmd(f"up {name}")

    def releaseall(self) -> str:
        return self.cmd("releaseall")

    def move(self, dx: int, dy: int) -> str:
        return self.cmd(f"move {dx} {dy}")

    def click(self, button: str = "left") -> str:
        return self.cmd(f"click {button}")

    def mdown(self, button: str = "left") -> str:
        return self.cmd(f"mdown {button}")

    def mup(self, button: str = "left") -> str:
        return self.cmd(f"mup {button}")

    def scroll(self, amount: int) -> str:
        return self.cmd(f"scroll {amount}")

    def close(self) -> None:
        self._transport.close()


# --- Host-side sequencing / timing (the board firmware stays dumb) ----------

def parse_sequence(text: str) -> list[tuple[str, object]]:
    """Parse a command file into steps.

    Each non-blank, non-`#`-comment line is either a command or a timing
    directive: `delay <ms>` or `sleep <seconds>`. Returns a list of
    `("cmd", line)` / `("delay", seconds)` tuples.
    """
    steps: list[tuple[str, object]] = []
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        head, _, rest = line.partition(" ")
        low = head.lower()
        if low == "delay":
            try:
                steps.append(("delay", float(rest) / 1000.0))
            except ValueError:
                raise ValueError(f"invalid delay value: {rest!r}")
        elif low == "sleep":
            try:
                steps.append(("delay", float(rest)))
            except ValueError:
                raise ValueError(f"invalid sleep value: {rest!r}")
        else:
            steps.append(("cmd", line))
    return steps


def run_sequence(
    rig: HidRig,
    steps: list[tuple[str, object]],
    default_delay: float = 0.0,
    sleep: Callable[[float], None] = time.sleep,
) -> None:
    """Execute parsed steps against `rig`. `sleep` is injectable for tests."""
    for kind, value in steps:
        if kind == "delay":
            sleep(float(value))
        else:
            rig.cmd(str(value))
            if default_delay:
                sleep(default_delay)


def repeat_key(
    rig: HidRig,
    name: str,
    count: int,
    delay: float = 0.0,
    sleep: Callable[[float], None] = time.sleep,
) -> None:
    """Tap a key `count` times with an inter-tap delay (auto-repeat)."""
    for i in range(count):
        rig.key(name)
        if delay and i < count - 1:
            sleep(delay)
````

## File: src/paniolo/_ocr.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""OCR helpers — wraps platform OCR tools.

- macOS: `visionocr` (Apple Vision framework, compiled from ocr/visionocr.swift)
- Linux: `linuxocr` (Tesseract-backed, from ocr/linuxocr)

Both tools share the same interface: read PNG on stdin, print text on stdout.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path
from typing import Optional


def visionocr_binary() -> Optional[str]:
    """Return the installed visionocr path: PATH, then ~/.cargo/bin. None if absent."""
    found = shutil.which("visionocr")
    if found:
        return found
    cargo_bin = Path.home() / ".cargo" / "bin" / "visionocr"
    return str(cargo_bin) if cargo_bin.exists() else None


def visionocr_source() -> Path:
    """Path to the visionocr Swift source in the repo (for `paniolo setup`)."""
    return Path(__file__).parent.parent.parent / "ocr" / "visionocr.swift"


def build_visionocr(dest: Path) -> None:
    """Compile visionocr.swift to `dest` (used by `paniolo setup`). Raises on error."""
    source = visionocr_source()
    if not source.exists():
        raise FileNotFoundError(f"visionocr source not found: {source}")
    if not shutil.which("swiftc"):
        raise FileNotFoundError("swiftc not found (install Xcode command line tools)")
    dest.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["swiftc", "-O", "-o", str(dest), str(source)], check=True)


def linuxocr_binary() -> Optional[str]:
    """Return the installed linuxocr path: PATH, then ~/.cargo/bin. None if absent."""
    found = shutil.which("linuxocr")
    if found:
        return found
    cargo_bin = Path.home() / ".cargo" / "bin" / "linuxocr"
    return str(cargo_bin) if cargo_bin.exists() else None


def linuxocr_source() -> Path:
    """Path to the linuxocr Python script in the repo (for `paniolo setup`)."""
    return Path(__file__).parent.parent.parent / "ocr" / "linuxocr"


def install_linuxocr(dest: Path) -> None:
    """Copy ocr/linuxocr to `dest` and make it executable (used by `paniolo setup`)."""
    import shutil as _shutil
    source = linuxocr_source()
    if not source.exists():
        raise FileNotFoundError(f"linuxocr source not found: {source}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    _shutil.copy2(source, dest)
    dest.chmod(0o755)


def ocr_binary() -> Optional[str]:
    """Return the platform OCR binary: visionocr on macOS, linuxocr on Linux."""
    if sys.platform == "darwin":
        return visionocr_binary()
    return linuxocr_binary()


def read_text(png: bytes, fast: bool = False, as_json: bool = False) -> str:
    """OCR PNG bytes and return recognized text (or JSON with bboxes).

    `fast` is only meaningful on macOS (visionocr --fast); ignored on Linux.
    `as_json` requests bounding-box JSON output; not yet supported on Linux.
    """
    binary = ocr_binary()
    if not binary:
        platform = "macOS" if sys.platform == "darwin" else "Linux"
        tool = "visionocr" if sys.platform == "darwin" else "linuxocr"
        raise FileNotFoundError(f"{tool} not installed on {platform} — run: paniolo setup")
    cmd = [binary]
    if sys.platform == "darwin":
        if fast:
            cmd.append("--fast")
        if as_json:
            cmd.append("--json")
    cmd.append("-")
    result = subprocess.run(cmd, input=png, capture_output=True)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.decode(errors="replace").strip() or f"{binary} failed")
    return result.stdout.decode(errors="replace")
````

## File: tests/test_config.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Tests for target config (de)serialization and serial-interface helpers."""

from __future__ import annotations

import tomllib

import pytest

from paniolo import _config
from paniolo._config import SerialInterface, TargetConfig


def roundtrip(cfg: TargetConfig) -> TargetConfig:
    return _config._from_dict(tomllib.loads(_config._to_toml(cfg)))


def test_roundtrip_multiple_interfaces():
    cfg = TargetConfig(
        name="fortune",
        interface="en3",
        tftp_root="/pxe",
        serial_interfaces=[
            SerialInterface("console", "/dev/ttyUSB0", 115200),
            SerialInterface("bmc", "/dev/ttyUSB1", 9600),
        ],
    )
    got = roundtrip(cfg)
    assert (got.name, got.interface, got.tftp_root) == ("fortune", "en3", "/pxe")
    assert [(i.name, i.device, i.baud) for i in got.serial_interfaces] == [
        ("console", "/dev/ttyUSB0", 115200),
        ("bmc", "/dev/ttyUSB1", 9600),
    ]


def test_roundtrip_no_interfaces():
    got = roundtrip(TargetConfig(name="x", interface="en0"))
    assert got.serial_interfaces == []


def test_legacy_single_serial_migrates():
    data = tomllib.loads(
        'name = "x"\ninterface = "en0"\nserial_device = "/dev/ttyUSB0"\nserial_baud = 57600\n'
    )
    cfg = _config._from_dict(data)
    assert len(cfg.serial_interfaces) == 1
    iface = cfg.serial_interfaces[0]
    assert (iface.name, iface.device, iface.baud) == (_config.DEFAULT_SERIAL_NAME, "/dev/ttyUSB0", 57600)


def test_legacy_default_baud():
    data = tomllib.loads('name = "x"\ninterface = "en0"\nserial_device = "/dev/ttyUSB0"\n')
    assert _config._from_dict(data).serial_interfaces[0].baud == 115200


def test_serial_interface_resolution():
    cfg = TargetConfig(
        name="x",
        interface="en0",
        serial_interfaces=[SerialInterface("console", "/dev/a"), SerialInterface("bmc", "/dev/b")],
    )
    assert cfg.serial_interface("bmc").device == "/dev/b"
    with pytest.raises(ValueError):
        cfg.serial_interface()  # ambiguous
    with pytest.raises(ValueError):
        cfg.serial_interface("nope")  # unknown


def test_serial_interface_single_is_default():
    cfg = TargetConfig(name="x", interface="en0", serial_interfaces=[SerialInterface("console", "/dev/a")])
    assert cfg.serial_interface().name == "console"


def test_serial_interface_none_configured():
    with pytest.raises(ValueError):
        TargetConfig(name="x", interface="en0").serial_interface()


def test_upsert_replaces_same_name():
    cfg = TargetConfig(name="x", interface="en0")
    cfg.upsert_serial_interface(SerialInterface("console", "/dev/a", 115200))
    cfg.upsert_serial_interface(SerialInterface("console", "/dev/a2", 9600))
    assert len(cfg.serial_interfaces) == 1
    assert (cfg.serial_interfaces[0].device, cfg.serial_interfaces[0].baud) == ("/dev/a2", 9600)


def test_remove_interface():
    cfg = TargetConfig(
        name="x",
        interface="en0",
        serial_interfaces=[SerialInterface("console", "/dev/a"), SerialInterface("bmc", "/dev/b")],
    )
    assert cfg.remove_serial_interface("console") is True
    assert cfg.remove_serial_interface("console") is False
    assert [i.name for i in cfg.serial_interfaces] == ["bmc"]


# --- S2: TOML control-character escaping -----------------------------------

def test_toml_roundtrip_newline_in_power_cycle_cmd():
    """Newlines in string values must survive a TOML round-trip without breaking the file."""
    cfg = TargetConfig(name="x", interface="en0", power_cycle_cmd="cmd1\ncmd2")
    got = roundtrip(cfg)
    assert got.power_cycle_cmd == "cmd1\ncmd2"


def test_toml_roundtrip_tab_and_cr():
    cfg = TargetConfig(name="x", interface="en0", power_cycle_cmd="a\tb\rc")
    got = roundtrip(cfg)
    assert got.power_cycle_cmd == "a\tb\rc"


def test_toml_kv_escapes_backslash():
    line = _config._toml_kv("k", "a\\b")
    assert line == 'k = "a\\\\b"'
    parsed = tomllib.loads(line)
    assert parsed["k"] == "a\\b"


# --- C2: unknown TOML keys are ignored gracefully --------------------------

def test_unknown_keys_in_toml_are_silently_dropped():
    data = tomllib.loads('name = "x"\ninterface = "en0"\nunknown_future_key = "foo"\n')
    cfg = _config._from_dict(data)
    assert cfg.name == "x"
    assert not hasattr(cfg, "unknown_future_key")
````

## File: tests/test_hid.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Host-side tests for the HID rig client — no hardware, no pyserial."""

from __future__ import annotations

import pytest

from paniolo import _hid


class FakeTransport:
    """Stand-in for a pyserial port: records writes, replies from a queue."""

    def __init__(self, replies=None):
        self.writes: list[bytes] = []
        self._replies = list(replies) if replies else []
        self.closed = False

    def write(self, data: bytes) -> None:
        self.writes.append(data)

    def readline(self) -> bytes:
        return self._replies.pop(0) if self._replies else b"OK\n"

    def close(self) -> None:
        self.closed = True


def make_rig(replies=None):
    t = FakeTransport(replies)
    return _hid.HidRig(transport=t), t


# --- command construction ---------------------------------------------------

@pytest.mark.parametrize(
    "call, expected",
    [
        (lambda r: r.type("hello world"), b"type hello world\n"),
        (lambda r: r.key("ENTER"), b"key ENTER\n"),
        (lambda r: r.combo("LEFT_CONTROL", "C"), b"combo LEFT_CONTROL C\n"),
        (lambda r: r.down("LEFT_SHIFT"), b"down LEFT_SHIFT\n"),
        (lambda r: r.up("LEFT_SHIFT"), b"up LEFT_SHIFT\n"),
        (lambda r: r.releaseall(), b"releaseall\n"),
        (lambda r: r.move(300, -50), b"move 300 -50\n"),
        (lambda r: r.click(), b"click left\n"),
        (lambda r: r.click("right"), b"click right\n"),
        (lambda r: r.mdown("middle"), b"mdown middle\n"),
        (lambda r: r.mup("middle"), b"mup middle\n"),
        (lambda r: r.scroll(-3), b"scroll -3\n"),
    ],
)
def test_command_construction(call, expected):
    rig, t = make_rig()
    call(rig)
    assert t.writes == [expected]


def test_cmd_returns_reply():
    rig, _ = make_rig(replies=[b"OK\n"])
    assert rig.cmd("releaseall") == "OK"


def test_cmd_raises_on_err():
    rig, _ = make_rig(replies=[b"ERR unknown command: frob\n"])
    with pytest.raises(RuntimeError, match="control board rejected"):
        rig.cmd("frob")


def test_close_delegates():
    rig, t = make_rig()
    rig.close()
    assert t.closed


# --- absolute-mouse scaling -------------------------------------------------

@pytest.mark.parametrize(
    "px, screen, expected",
    [
        (0, 1920, 0),
        (1919, 1920, 32767),
        (960, 1920, 16392),
        (-100, 1920, 0),        # clamp low
        (99999, 1920, 32767),   # clamp high
        (5, 1, 0),              # degenerate screen size
    ],
)
def test_scale_to_logical(px, screen, expected):
    assert _hid.scale_to_logical(px, screen) == expected


# --- sequence parsing -------------------------------------------------------

def test_parse_sequence_skips_blanks_and_comments():
    text = "\n# a comment\n  \nkey ENTER\n# another\ntype hi\n"
    assert _hid.parse_sequence(text) == [("cmd", "key ENTER"), ("cmd", "type hi")]


def test_parse_sequence_timing_directives():
    text = "delay 250\nkey A\nsleep 2\n"
    assert _hid.parse_sequence(text) == [
        ("delay", 0.25),
        ("cmd", "key A"),
        ("delay", 2.0),
    ]


def test_run_sequence_executes_in_order_with_delays():
    rig, t = make_rig(replies=[b"OK\n", b"OK\n"])
    slept: list[float] = []
    steps = [("cmd", "key A"), ("delay", 0.5), ("cmd", "type hi")]
    _hid.run_sequence(rig, steps, default_delay=0.0, sleep=slept.append)
    assert t.writes == [b"key A\n", b"type hi\n"]
    assert slept == [0.5]


def test_run_sequence_default_delay_between_commands():
    rig, _ = make_rig(replies=[b"OK\n", b"OK\n"])
    slept: list[float] = []
    steps = [("cmd", "key A"), ("cmd", "key B")]
    _hid.run_sequence(rig, steps, default_delay=0.1, sleep=slept.append)
    assert slept == [0.1, 0.1]


def test_repeat_key():
    rig, t = make_rig(replies=[b"OK\n"] * 3)
    slept: list[float] = []
    _hid.repeat_key(rig, "TAB", 3, delay=0.2, sleep=slept.append)
    assert t.writes == [b"key TAB\n"] * 3
    assert slept == [0.2, 0.2]  # no trailing delay after the last tap


# --- S3: newline injection guard -------------------------------------------

def test_cmd_rejects_newline():
    rig, _ = make_rig()
    with pytest.raises(ValueError, match="newline"):
        rig.cmd("type hello\nkey ENTER")


def test_cmd_rejects_carriage_return():
    rig, _ = make_rig()
    with pytest.raises(ValueError, match="newline"):
        rig.cmd("type hello\rworld")


def test_type_rejects_embedded_newline():
    rig, _ = make_rig()
    with pytest.raises(ValueError, match="newline"):
        rig.type("line1\nline2")


# --- C3: parse_sequence error messages -------------------------------------

def test_parse_sequence_bad_delay_raises_friendly_error():
    with pytest.raises(ValueError, match="invalid delay value"):
        _hid.parse_sequence("delay abc\nkey A\n")


def test_parse_sequence_bad_sleep_raises_friendly_error():
    with pytest.raises(ValueError, match="invalid sleep value"):
        _hid.parse_sequence("sleep xyz\n")
````

## File: README.md
````markdown
# paniolo

Agent-controlled target machine wrangler for low-level software development.

"Paniolo" is the Hawaiian word for cowboy. The idea: an AI agent sits at the
reins while you're writing bootloaders, firmware, or OS bring-up code — paniolo
gives it the controls to netboot the target, watch its output, send it input,
and power-cycle it without human intervention at each iteration.

---

## Capabilities

| Subsystem | Commands | What it does |
|---|---|---|
| [Netboot](docs/netboot.md) | `paniolo netboot` | DHCP + TFTP netboot over a direct USB-Ethernet link |
| [Video](docs/video.md) | `paniolo video` | HDMI capture via warm-stream daemon; on-device OCR |
| [Serial](docs/serial.md) | `paniolo serial` | Serial console — interactive (tio) or daemon-backed with timestamped rolling log |
| [Power control](docs/power.md) | `paniolo serial dtr/reset`, `paniolo power-cycle`, `paniolo power-state` | DTR-based hardware power button (J2 header) and script-based power cycling |
| [HID injection](docs/hid.md) | `paniolo hid` | USB keyboard/mouse injection via a two-board KB2040 rig |
| [Dashboard](docs/dashboard.md) | `paniolo console` | Combined video + serial web UI; auto-starts daemons; `-i <name>` preselects a serial interface |

---

## Requirements

- macOS 10.14 (Mojave) or later, or Linux (x86-64 / arm64)
- Python 3.11+
- [uv](https://docs.astral.sh/uv/) (`brew install uv` on macOS, or the [uv installer](https://docs.astral.sh/uv/getting-started/installation/) on Linux)
- [Homebrew](https://brew.sh) (macOS only — Linux uses the system package manager)
- Rust toolchain (for hdmicap, serialcap — `brew install rustup` on macOS, or `rustup.rs` on Linux)

---

## Installation

```bash
git clone https://github.com/curtisgalloway/paniolo ~/src/paniolo
uv tool install ~/src/paniolo
paniolo setup          # installs dnsmasq, tftp-now, hdmicap, serialcap, visionocr
```

`paniolo setup` compiles and installs the Rust daemons (`hdmicap`, `serialcap`)
and the Swift OCR helper (`visionocr`) into `~/.cargo/bin`, and installs the
TFTP and DHCP servers via Homebrew.

To pick up code changes after pulling or editing:

```bash
uv tool install --reinstall ~/src/paniolo
cargo install --path ~/src/paniolo/hdmicap    # if hdmicap changed
cargo install --path ~/src/paniolo/serialcap  # if serialcap changed
```

For the USB HID commands, install the optional `pyserial` extra:

```bash
uv tool install --with pyserial ~/src/paniolo
```

---

## Remote control pattern

The intended use is an AI agent or script on a dev machine SSHing into the
control Mac to drive the target:

```bash
# Configure target once
ssh control-mac "paniolo target set target-machine \
    --interface en3 \
    --tftp-root ~/pxe \
    --power-cycle-cmd /path/to/power-cycle.sh"

# Deploy a new kernel and boot
TFTP_ROOT=$(ssh control-mac "paniolo netboot tftp-root target-machine")
scp out/kernel.img control-mac:"${TFTP_ROOT}/kernel_2712.img"
ssh control-mac "paniolo netboot start target-machine"
ssh control-mac "paniolo netboot logs -f target-machine"

# Interact with the console
ssh control-mac "paniolo serial log -i console --since --tail 50 target-machine"

# Power cycle and repeat
ssh control-mac "paniolo power-cycle target-machine"
```

---

## Concepts

### Target

A *target* is a named machine you want to control. Its configuration lives in
`~/.config/paniolo/targets/<name>.toml`. One config file per target; no daemon
required. If exactly one target is configured it is the default and can be
omitted from every command.

See [`paniolo target set --help`](docs/netboot.md#target-configuration) for all fields.

### Runtime paths

| Purpose | Path |
|---|---|
| Target configs | `~/.config/paniolo/targets/<name>.toml` |
| Video config | `~/.config/paniolo/video.toml` |
| HID config | `~/.config/paniolo/hid.toml` |
| Netboot daemon state | `~/.local/share/paniolo/<name>/netboot.json` |
| hdmicap discovery | `$XDG_RUNTIME_DIR/hdmicap/daemon.json` (Linux) / `$TMPDIR/hdmicap/daemon.json` (macOS) |
| serialcap discovery | `$XDG_RUNTIME_DIR/serialcap/daemon.json` (Linux) / `$TMPDIR/serialcap/daemon.json` (macOS) |
| Serial capture logs | `$XDG_RUNTIME_DIR/serialcap/capture/<name>/serial.jsonl` (Linux) |

---

## License

Apache 2.0 — see [LICENSE](LICENSE).
````

## File: .github/workflows/ci.yml
````yaml
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

name: CI

on:
  push:
    branches: [main]
  pull_request:

# Cancel superseded runs on the same ref (e.g. rapid pushes to a PR).
concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

permissions:
  contents: read

jobs:
  python:
    name: python (pytest)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install uv
        uses: astral-sh/setup-uv@v5
        with:
          enable-cache: true
          python-version: "3.11"
      - name: Sync dependencies
        run: uv sync
      - name: Run tests
        run: uv run pytest -q

  serialcap:
    name: serialcap (fmt, clippy, test)
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: serialcap
    steps:
      - uses: actions/checkout@v4
      - name: Install system dependencies
        run: sudo apt-get update && sudo apt-get install -y pkg-config libudev-dev
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: serialcap
      - name: Format check
        run: cargo fmt --check
      - name: Clippy
        run: cargo clippy --all-targets -- -D warnings
      - name: Test
        run: cargo test

  hdmicap-linux:
    name: hdmicap (Linux, V4L2 build)
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: hdmicap
    steps:
      - uses: actions/checkout@v4
      - name: Install system dependencies
        run: sudo apt-get update && sudo apt-get install -y build-essential pkg-config libclang-dev clang cmake nasm libturbojpeg0-dev
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: hdmicap
      - name: Build hdmicap (V4L2 path)
        run: cargo build

  macos:
    # Exercises the macOS-only stack the Linux jobs can't: hdmicap's AVFoundation
    # capture path and the Apple Vision OCR helper. fmt/clippy aren't gated here —
    # hdmicap predates this workflow and isn't yet rustfmt-clean / warning-free, so
    # those run only against serialcap on Linux above.
    name: macos (hdmicap, visionocr)
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: hdmicap
      - name: Build + test hdmicap (AVFoundation capture path)
        working-directory: hdmicap
        run: cargo test
      - name: Compile visionocr (Apple Vision OCR helper)
        run: swiftc -O -o "$RUNNER_TEMP/visionocr" ocr/visionocr.swift
````

## File: hdmicap/src/capture.rs
````rust
// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Capture backend abstraction.
//!
//! On Linux we bypass nokhwa and use the `v4l` crate directly so we can:
//!   - Call `stream.set_timeout()` to avoid an indefinite VIDIOC_DQBUF block
//!   - Keep raw MJPEG bytes for zero-cost preview serving
//!   - Use turbojpeg (libjpeg-turbo) for fast RGB decode when signal detection
//!     or OCR needs pixel data
//!
//! On macOS the nokhwa + AVFoundation path is kept as-is.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use image::RgbImage;

use nokhwa::utils::{ApiBackend, CameraIndex};
use nokhwa::query;

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub index: u32,
    pub name: String,
    pub misc: String,
}

/// How the user asked us to pick a device.
#[derive(Clone, Debug)]
pub enum DeviceSpec {
    Auto,
    Index(u32),
    Name(String),
}

impl DeviceSpec {
    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("auto") {
            DeviceSpec::Auto
        } else if let Ok(i) = s.parse::<u32>() {
            DeviceSpec::Index(i)
        } else if let Some(idx) = s
            .strip_prefix("/dev/video")
            .and_then(|n| n.parse::<u32>().ok())
        {
            DeviceSpec::Index(idx)
        } else {
            DeviceSpec::Name(s.to_string())
        }
    }
}

const BUILTIN_HINTS: &[&str] = &["facetime", "built-in", "integrated", "isight"];

/// One captured frame. `jpeg` carries raw MJPEG bytes when available (Linux
/// MJPEG path); the preview endpoint serves these directly with zero server
/// decode/re-encode. `rgb` is always populated for signal detection (ahash,
/// is_no_signal); on the Linux path it comes from turbojpeg (fast).
pub struct CapturedFrame {
    /// Raw MJPEG bytes from the device. Present on the Linux v4l path when the
    /// device is in MJPEG mode; None on macOS or YUYV sources.
    pub jpeg: Option<Arc<[u8]>>,
    /// Decoded RGB8 pixels for signal detection. Always populated.
    pub rgb: RgbImage,
}

pub trait CaptureBackend {
    fn frame(&mut self) -> Result<CapturedFrame>;
    fn dims(&self) -> (u32, u32);
}

pub fn enumerate() -> Result<Vec<DeviceInfo>> {
    let cams = query(ApiBackend::Auto).context("nokhwa device query failed")?;
    Ok(cams
        .into_iter()
        .map(|c| DeviceInfo {
            index: match c.index() {
                CameraIndex::Index(i) => *i,
                CameraIndex::String(_) => u32::MAX,
            },
            name: c.human_name(),
            misc: c.description().to_string(),
        })
        .collect())
}

pub fn resolve(spec: &DeviceSpec) -> Result<u32> {
    match spec {
        DeviceSpec::Index(i) => Ok(*i),
        _ => {
            let devices = enumerate()?;
            if devices.is_empty() {
                return Err(anyhow!("no capture devices found"));
            }
            match spec {
                DeviceSpec::Index(i) => Ok(*i),
                DeviceSpec::Name(sub) => {
                    let sub = sub.to_lowercase();
                    devices
                        .iter()
                        .find(|d| d.name.to_lowercase().contains(&sub))
                        .map(|d| d.index)
                        .ok_or_else(|| anyhow!("no device matching name {:?}", sub))
                }
                DeviceSpec::Auto => {
                    let external = devices.iter().find(|d| {
                        let n = d.name.to_lowercase();
                        !BUILTIN_HINTS.iter().any(|h| n.contains(h))
                    });
                    Ok(external.unwrap_or(&devices[0]).index)
                }
            }
        }
    }
}

pub fn open_backend(spec: &DeviceSpec) -> Result<Box<dyn CaptureBackend>> {
    #[cfg(target_os = "linux")]
    {
        linux::LinuxV4LBackend::open(spec).map(|b| Box::new(b) as Box<dyn CaptureBackend>)
    }
    #[cfg(not(target_os = "linux"))]
    {
        macos::NokhwaBackend::open(spec).map(|b| Box::new(b) as Box<dyn CaptureBackend>)
    }
}

// ── Linux backend ─────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod linux {
    use std::io;
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::{anyhow, Context, Result};
    use image::RgbImage;
    use v4l::buffer::Type;
    use v4l::device::Device;
    use v4l::format::{Format, FourCC};
    use v4l::io::mmap::Stream;
    use v4l::io::traits::CaptureStream;
    use v4l::video::Capture;

    use super::{CaptureBackend, CapturedFrame, DeviceSpec, resolve};

    const FORMATS: &[(u32, u32, &[u8; 4])] = &[
        (1280, 720,  b"MJPG"),
        (1920, 1080, b"MJPG"),
        (1280, 720,  b"YUYV"),
        (640,  480,  b"YUYV"),
    ];

    const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

    pub struct LinuxV4LBackend {
        stream: Stream<'static>,
        dev: Box<Device>,
        dims: (u32, u32),
        is_mjpeg: bool,
    }

    impl LinuxV4LBackend {
        pub fn open(spec: &DeviceSpec) -> Result<Self> {
            let idx = resolve(spec)?;
            let dev = Box::new(
                Device::new(idx as usize).map_err(|e| anyhow!("open /dev/video{idx}: {e}"))?,
            );

            let mut last_err = anyhow!("no formats succeeded");
            for &(w, h, fourcc) in FORMATS {
                let fmt = Format::new(w, h, FourCC::new(fourcc));
                if dev.set_format(&fmt).is_err() {
                    continue;
                }
                let dev_ref: &'static Device = unsafe { &*(dev.as_ref() as *const Device) };
                match Stream::with_buffers(dev_ref, Type::VideoCapture, 4) {
                    Ok(mut stream) => {
                        stream.set_timeout(FRAME_TIMEOUT);
                        let actual_fmt = dev.format().unwrap_or(fmt);
                        let is_mjpeg = actual_fmt.fourcc == FourCC::new(b"MJPG");
                        tracing::info!(
                            "capture opened {}x{} {:?}",
                            actual_fmt.width, actual_fmt.height,
                            if is_mjpeg { "MJPEG" } else { "YUYV" }
                        );
                        return Ok(LinuxV4LBackend {
                            stream,
                            dev,
                            dims: (actual_fmt.width, actual_fmt.height),
                            is_mjpeg,
                        });
                    }
                    Err(e) => {
                        last_err = anyhow!("stream init {w}x{h}: {e}");
                    }
                }
            }
            Err(last_err)
        }
    }

    impl CaptureBackend for LinuxV4LBackend {
        fn frame(&mut self) -> Result<CapturedFrame> {
            let (buf, _meta) = self.stream.next().map_err(|e| {
                if e.kind() == io::ErrorKind::TimedOut {
                    anyhow!("frame timeout (device stalled)")
                } else {
                    anyhow!("VIDIOC_DQBUF: {e}")
                }
            })?;

            if self.is_mjpeg {
                // Keep a copy of the raw JPEG bytes for zero-cost preview serving.
                let jpeg_bytes: Arc<[u8]> = Arc::from(buf.to_vec().into_boxed_slice());

                // Decode with turbojpeg (libjpeg-turbo) for signal detection.
                // ~5ms at 720p vs ~50ms with the pure-Rust image crate.
                let rgb = turbojpeg::decompress_image::<image::Rgb<u8>>(buf)
                    .context("turbojpeg MJPEG decode failed")?;
                let (w, h) = (rgb.width(), rgb.height());
                self.dims = (w, h);

                Ok(CapturedFrame { jpeg: Some(jpeg_bytes), rgb })
            } else {
                // YUYV: no raw JPEG, decode to RGB for signal detection and storage.
                let fmt = self.dev.format().ok();
                let (w, h) = fmt
                    .as_ref()
                    .map(|f| (f.width, f.height))
                    .unwrap_or(self.dims);
                self.dims = (w, h);
                Ok(CapturedFrame { jpeg: None, rgb: yuyv_to_rgb(buf, w, h) })
            }
        }

        fn dims(&self) -> (u32, u32) {
            self.dims
        }
    }

    fn yuyv_to_rgb(buf: &[u8], w: u32, h: u32) -> RgbImage {
        let mut rgb = RgbImage::new(w, h);
        let pairs = (w * h / 2) as usize;
        for i in 0..pairs {
            let base = i * 4;
            if base + 3 >= buf.len() { break; }
            let y0 = buf[base] as f32;
            let cb = buf[base + 1] as f32 - 128.0;
            let y1 = buf[base + 2] as f32;
            let cr = buf[base + 3] as f32 - 128.0;
            let to_u8 = |v: f32| v.clamp(0.0, 255.0) as u8;
            let r = |y: f32| to_u8(y + 1.402 * cr);
            let g = |y: f32| to_u8(y - 0.344 * cb - 0.714 * cr);
            let b = |y: f32| to_u8(y + 1.772 * cb);
            let x0 = ((i * 2) % w as usize) as u32;
            let y_row = ((i * 2) / w as usize) as u32;
            if x0 < w && y_row < h {
                rgb.put_pixel(x0, y_row, image::Rgb([r(y0), g(y0), b(y0)]));
            }
            if x0 + 1 < w && y_row < h {
                rgb.put_pixel(x0 + 1, y_row, image::Rgb([r(y1), g(y1), b(y1)]));
            }
        }
        rgb
    }
}

// ── macOS backend ─────────────────────────────────────────────────────────────

#[cfg(not(target_os = "linux"))]
mod macos {
    use std::sync::Arc;

    use anyhow::{anyhow, Context, Result};
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{
        CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
    };
    use nokhwa::Camera;

    use super::{CaptureBackend, CapturedFrame, DeviceSpec, resolve};

    pub struct NokhwaBackend {
        cam: Camera,
        dims: (u32, u32),
    }

    impl NokhwaBackend {
        pub fn open(spec: &DeviceSpec) -> Result<Self> {
            let idx = resolve(spec)?;
            let format_types: &[RequestedFormatType] = &[
                RequestedFormatType::Closest(CameraFormat::new(
                    Resolution::new(1280, 720),
                    FrameFormat::MJPEG,
                    30,
                )),
                RequestedFormatType::Closest(CameraFormat::new(
                    Resolution::new(1920, 1080),
                    FrameFormat::MJPEG,
                    30,
                )),
                RequestedFormatType::AbsoluteHighestResolution,
            ];

            let mut last_err: anyhow::Error = anyhow!("no formats to try");
            for &fmt_type in format_types {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    try_open(idx, fmt_type)
                }));
                match result {
                    Ok(Ok(backend)) => return Ok(backend),
                    Ok(Err(e)) => { last_err = e; }
                    Err(_) => { last_err = anyhow!("format {:?} not supported", fmt_type); }
                }
            }
            Err(last_err)
        }
    }

    fn try_open(idx: u32, fmt_type: RequestedFormatType) -> Result<NokhwaBackend> {
        let requested = RequestedFormat::new::<RgbFormat>(fmt_type);
        let mut cam = Camera::new(CameraIndex::Index(idx), requested)
            .map_err(|e| anyhow!("failed to open capture device {idx}: {e}"))?;
        cam.open_stream()
            .map_err(|e| anyhow!("failed to open capture device {idx}: {e}"))?;
        let res = cam.resolution();
        Ok(NokhwaBackend { cam, dims: (res.width(), res.height()) })
    }

    impl CaptureBackend for NokhwaBackend {
        fn frame(&mut self) -> Result<CapturedFrame> {
            let buf = self.cam.frame().context("frame grab failed")?;
            let decoded = buf.decode_image::<RgbFormat>().context("decode failed")?;
            self.dims = (decoded.width(), decoded.height());
            Ok(CapturedFrame {
                jpeg: None,  // nokhwa gives decoded pixels, not raw JPEG
                rgb: decoded,
            })
        }

        fn dims(&self) -> (u32, u32) {
            self.dims
        }
    }
}
````

## File: hdmicap/src/server.rs
````rust
// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Localhost HTTP API. Handlers never touch the device — they only read the
//! latest FrameState from their `watch::Receiver`. PNG encoding is lazy, here.

use std::io::Cursor;
use std::process::Stdio;
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use image::codecs::jpeg::JpegEncoder;
use image::{ImageBuffer, ImageEncoder, Rgb};
#[cfg(target_os = "linux")]
use turbojpeg;
use serde::Deserialize;
use tokio::sync::watch;

use crate::capture_thread::FrameRx;
use crate::frame::{FrameState, Signal, StatusDto};

#[derive(Clone)]
pub struct AppState {
    pub frames: FrameRx,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/status", get(status))
        .route("/snapshot", get(snapshot))
        .route("/preview", get(preview))
        .route("/ocr", get(ocr))
        .route("/power-cycle", post(power_cycle))
        .route("/devices", get(devices))
        // Vendored xterm.js assets for the serial terminal pane.
        .route("/xterm.js", get(xterm_js))
        .route("/xterm.css", get(xterm_css))
        .route("/xterm-addon-fit.js", get(xterm_fit_js))
        .with_state(state)
}

async fn index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../assets/index.html"),
    )
}

async fn xterm_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        include_str!("../assets/xterm.js"),
    )
}

async fn xterm_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../assets/xterm.css"),
    )
}

async fn xterm_fit_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        include_str!("../assets/xterm-addon-fit.js"),
    )
}

async fn status(State(s): State<AppState>) -> Json<StatusDto> {
    let f = s.frames.borrow().clone();
    Json(StatusDto::from(f.as_ref()))
}

#[derive(Deserialize)]
struct SnapReq {
    /// "stable" -> wait until signal == Stable.
    wait: Option<String>,
    /// Hex hash from a prior /status; wait until the published hash differs.
    changed_since: Option<String>,
    /// Milliseconds; default applied below.
    timeout: Option<u64>,
}

const DEFAULT_TIMEOUT_MS: u64 = 2000;

async fn snapshot(State(s): State<AppState>, Query(q): Query<SnapReq>) -> Response {
    let mut rx = s.frames.clone();
    let timeout_ms = q.timeout.unwrap_or(DEFAULT_TIMEOUT_MS);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms).min(Duration::from_secs(60));
    let want_stable = q.wait.as_deref() == Some("stable");
    let changed_since = q
        .changed_since
        .as_ref()
        .and_then(|h| u64::from_str_radix(h, 16).ok());

    loop {
        let ready = {
            let f = rx.borrow_and_update();
            match (want_stable, changed_since) {
                (true, _) => f.signal == Signal::Stable,
                (_, Some(h)) => f.hash != h,
                _ => true,
            }
        };

        if ready {
            let f = rx.borrow().clone();
            return png_response(&f, false);
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let f = rx.borrow().clone();
            return png_response(&f, true);
        }
        if tokio::time::timeout(remaining, rx.changed()).await.is_err() {
            let f = rx.borrow().clone();
            return png_response(&f, true);
        }
    }
}

/// Decode the frame to an RGB image. On the Linux MJPEG path `rgb` is empty
/// and we decode `jpeg` with turbojpeg. On other paths `rgb` is pre-decoded.
fn decode_rgb(f: &FrameState) -> Option<ImageBuffer<Rgb<u8>, Vec<u8>>> {
    if !f.rgb.is_empty() {
        return ImageBuffer::from_raw(f.width, f.height, f.rgb.to_vec());
    }
    #[cfg(target_os = "linux")]
    if let Some(ref jpeg) = f.jpeg {
        return turbojpeg::decompress_image::<Rgb<u8>>(jpeg).ok();
    }
    None
}

/// Encode the frame to PNG bytes. Shared by /snapshot and /ocr.
fn encode_png(f: &FrameState) -> Option<Vec<u8>> {
    let img = decode_rgb(f)?;
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .ok()?;
    Some(bytes)
}

/// Lazily encode the current RGB buffer to PNG. PNG for agent snapshots: text
/// edges matter for OCR and the dongle already adds MJPEG artifacts.
fn png_response(f: &FrameState, timed_out: bool) -> Response {
    if f.signal == Signal::NoDevice || f.width == 0 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::HeaderName::from_static("x-signal"), "no_device")],
            "no capture device",
        )
            .into_response();
    }

    let bytes = match encode_png(f) {
        Some(b) => b,
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "frame buffer size mismatch")
                .into_response()
        }
    };

    let signal_str = signal_name(f.signal);

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png".to_string()),
            (header::HeaderName::from_static("x-signal"), signal_str.to_string()),
            (
                header::HeaderName::from_static("x-resolution-epoch"),
                f.resolution_epoch.to_string(),
            ),
            (
                header::HeaderName::from_static("x-frame-hash"),
                format!("{:016x}", f.hash),
            ),
            (
                header::HeaderName::from_static("x-timeout"),
                (timed_out as u8).to_string(),
            ),
        ],
        bytes,
    )
        .into_response()
}

/// multipart/x-mixed-replace MJPEG stream for the human browser preview.
/// Reads the same warm buffer as /snapshot — zero device contention.
/// When raw JPEG bytes are available (Linux MJPEG path), they are served
/// directly with zero server-side decode or re-encode. Otherwise we re-encode
/// from the decoded RGB buffer at quality 80.
async fn preview(State(s): State<AppState>) -> Response {
    let mut frames = s.frames.clone();

    let stream = async_stream::stream! {
        let mut interval = tokio::time::interval(Duration::from_millis(67));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            let f = frames.borrow_and_update().clone();

            if f.signal == Signal::NoDevice || f.width == 0 {
                continue;
            }

            // Fast path: raw JPEG bytes from the device — no decode/re-encode.
            let jpeg_bytes: Vec<u8> = if let Some(ref raw) = f.jpeg {
                raw.to_vec()
            } else {
                // Fallback: re-encode from decoded RGB (macOS / YUYV path).
                let img: ImageBuffer<Rgb<u8>, _> =
                    match ImageBuffer::from_raw(f.width, f.height, f.rgb.to_vec()) {
                        Some(i) => i,
                        None => continue,
                    };
                let mut buf = Vec::new();
                let encoder = JpegEncoder::new_with_quality(Cursor::new(&mut buf), 80);
                if encoder
                    .write_image(
                        img.as_raw(),
                        img.width(),
                        img.height(),
                        image::ExtendedColorType::Rgb8,
                    )
                    .is_err()
                {
                    continue;
                }
                buf
            };

            let part_header = format!(
                "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                jpeg_bytes.len()
            );
            let mut chunk = Vec::with_capacity(part_header.len() + jpeg_bytes.len() + 2);
            chunk.extend_from_slice(part_header.as_bytes());
            chunk.extend_from_slice(&jpeg_bytes);
            chunk.extend_from_slice(b"\r\n");

            yield Ok::<Bytes, std::io::Error>(Bytes::from(chunk));
        }
    };

    Response::builder()
        .header(
            header::CONTENT_TYPE,
            "multipart/x-mixed-replace;boundary=frame",
        )
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// OCR the current warm frame by shelling out to the `visionocr` tool (Apple
/// Vision). The daemon doesn't link Vision itself — it pipes a PNG to whatever
/// `PANIOLO_VISIONOCR` points at (paniolo sets this), falling back to PATH.
async fn ocr(State(s): State<AppState>) -> Response {
    let f = s.frames.borrow().clone();
    if f.signal == Signal::NoDevice || f.width == 0 {
        return (StatusCode::SERVICE_UNAVAILABLE, "no capture device").into_response();
    }
    let png = match encode_png(&f) {
        Some(p) => p,
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "png encode failed").into_response()
        }
    };

    let bin = std::env::var("PANIOLO_VISIONOCR").unwrap_or_else(|_| "visionocr".to_string());
    let mut child = match tokio::process::Command::new(&bin)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::NOT_IMPLEMENTED,
                format!("visionocr unavailable ({bin}): {e}"),
            )
                .into_response()
        }
    };

    // Write the PNG to stdin on a task while we collect stdout, so a large
    // frame can't deadlock the pipe.
    if let Some(mut stdin) = child.stdin.take() {
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(&png).await;
            // stdin dropped here -> EOF, so visionocr stops reading.
        });
    }

    match child.wait_with_output().await {
        Ok(out) if out.status.success() => (
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            out.stdout,
        )
            .into_response(),
        Ok(out) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("visionocr failed: {}", String::from_utf8_lossy(&out.stderr)),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("visionocr wait: {e}")).into_response(),
    }
}

/// Trigger a power cycle by calling `paniolo power-cycle <target>`.
/// Requires PANIOLO_TARGET to be set in the daemon's environment (done by
/// `paniolo video watch <target>`). Returns 501 if not configured.
async fn power_cycle() -> Response {
    let target = match std::env::var("PANIOLO_TARGET") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            return (
                StatusCode::NOT_IMPLEMENTED,
                "PANIOLO_TARGET not set — start the daemon with: paniolo video watch <target>",
            )
                .into_response()
        }
    };
    let paniolo = std::env::var("PANIOLO_BIN").unwrap_or_else(|_| "paniolo".to_string());
    match tokio::process::Command::new(&paniolo)
        .args(["power-cycle", &target])
        .status()
        .await
    {
        Ok(s) if s.success() => (StatusCode::OK, "power cycle triggered").into_response(),
        Ok(s) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("paniolo power-cycle exited with {s}"),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to run {paniolo}: {e}"),
        )
            .into_response(),
    }
}

async fn devices() -> Response {
    match crate::capture::enumerate() {
        Ok(list) => Json(
            list.into_iter()
                .map(|d| {
                    serde_json::json!({"index": d.index, "name": d.name, "misc": d.misc})
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    }
}

fn signal_name(s: Signal) -> &'static str {
    match s {
        Signal::Stable => "stable",
        Signal::ModeSwitching => "mode_switching",
        Signal::NoSignal => "no_signal",
        Signal::NoDevice => "no_device",
    }
}

#[allow(unused_imports)]
use watch as _watch;
````

## File: serialcap/src/serial_io.rs
````rust
// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Owns the serial port. A supervisor task opens the port, reconnects on
//! loss/hot-unplug, fans received bytes out to all WebSocket clients via a
//! `broadcast` channel, and drains a single `mpsc` of client input back to the
//! port. A small ring buffer keeps recent output so a client that connects
//! mid-stream sees scrollback (e.g. boot log already in progress).
//!
//! Every received chunk is also tee'd to a dedicated OS thread that owns the
//! [`capture::LineLog`], which assembles timestamped lines and persists them to
//! disk. Keeping that work (and its file I/O) off the supervisor's select loop
//! means a slow disk can never stall the live WebSocket fan-out.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc as stdmpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use serialport::SerialPort;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_serial::SerialPortBuilderExt;
use tracing::{info, warn};

use crate::capture::LineLog;

const RING_BYTES: usize = 64 * 1024;
const BROADCAST_CAP: usize = 1024;
const WRITE_CAP: usize = 256;
const REOPEN_DELAY: Duration = Duration::from_millis(750);

#[derive(Clone)]
pub struct Status {
    pub device: String,
    pub baud: u32,
    pub connected: bool,
    /// Current power state as read from the configured modem-control sense line.
    /// `None` when no sense signal is configured for this interface.
    pub power_on: Option<bool>,
}

/// One serial interface the daemon owns: a stable `name` (used by the CLI, the
/// dashboard selector, and the capture sub-directory) bound to a `device`/`baud`.
#[derive(Clone)]
pub struct InterfaceSpec {
    pub name: String,
    pub device: String,
    pub baud: u32,
    /// Optional modem-control input pin wired to the target's 3.3 V rail so the
    /// host can detect whether the target is powered on.  Values: "cts", "dsr",
    /// "dcd", "ri".  None when not wired up.
    pub power_sense_signal: Option<String>,
}

/// One running interface: its name paired with the live handle.
#[derive(Clone)]
pub struct NamedSerial {
    pub name: String,
    pub handle: SerialHandle,
}

/// All interfaces the daemon is serving, in the order they were configured. The
/// first is the default for endpoints/commands that don't name one. Cheap to
/// clone (everything inside is channels or `Arc`s).
#[derive(Clone)]
pub struct Serials {
    inner: Arc<Vec<NamedSerial>>,
}

impl Serials {
    /// Spawn a supervisor + capture thread per interface and collect the handles.
    /// Each interface captures into `<capture_base>/<name>/`.
    pub fn spawn_all(specs: &[InterfaceSpec], capture_base: &Path, buffer_lines: u64) -> Self {
        let inner = specs
            .iter()
            .map(|spec| {
                let dir = crate::capture::interface_dir(capture_base, &spec.name);
                let handle = spawn_interface(spec.clone(), dir, buffer_lines);
                NamedSerial {
                    name: spec.name.clone(),
                    handle,
                }
            })
            .collect();
        Serials {
            inner: Arc::new(inner),
        }
    }

    /// Look up an interface by name.
    pub fn get(&self, name: &str) -> Option<&SerialHandle> {
        self.inner
            .iter()
            .find(|ns| ns.name == name)
            .map(|ns| &ns.handle)
    }

    /// The default interface (the first configured), if any.
    pub fn default(&self) -> Option<&NamedSerial> {
        self.inner.first()
    }

    /// All interfaces, in configuration order.
    pub fn all(&self) -> &[NamedSerial] {
        &self.inner
    }
}

/// Handle shared with the HTTP layer. Cheap to clone (all fields are channels
/// or `Arc`s).
#[derive(Clone)]
pub struct SerialHandle {
    /// Serial bytes flowing out to every connected client.
    to_clients: broadcast::Sender<Bytes>,
    /// Client keystrokes flowing back to the port.
    pub write_tx: mpsc::Sender<Bytes>,
    ring: Arc<Mutex<VecDeque<u8>>>,
    status: Arc<Mutex<Status>>,
    /// Button-press requests: caller sends (duration_ms, responder); the
    /// supervisor asserts DTR for that many milliseconds then replies.
    dtr_tx: mpsc::Sender<(u64, oneshot::Sender<()>)>,
}

impl SerialHandle {
    pub fn subscribe(&self) -> broadcast::Receiver<Bytes> {
        self.to_clients.subscribe()
    }

    /// Snapshot of recent output for a newly-connected client.
    pub fn scrollback(&self) -> Vec<u8> {
        self.ring.lock().unwrap().iter().copied().collect()
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// Assert DTR for `duration_ms` milliseconds then release.
    ///
    /// Models pressing the J2 power button on a Raspberry Pi (or equivalent).
    /// The caller decides what the press means for the target hardware:
    /// - short press (≤500 ms): OS receives power-button event → graceful reboot/halt
    /// - long press (≥3 s): PMIC hard power-off
    ///
    /// Blocks until the press completes. Concurrent calls queue and execute serially.
    pub async fn dtr_press(&self, duration_ms: u64) -> anyhow::Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.dtr_tx
            .send((duration_ms, resp_tx))
            .await
            .map_err(|_| anyhow::anyhow!("supervisor not running"))?;
        resp_rx
            .await
            .map_err(|_| anyhow::anyhow!("supervisor dropped response"))
    }
}

/// Spawn the supervisor for one interface on the current tokio runtime and return
/// its handle. `capture_dir` / `buffer_lines` configure that interface's on-disk
/// line log; its capture thread is started here and fed via a non-blocking channel.
pub fn spawn_interface(
    spec: InterfaceSpec,
    capture_dir: PathBuf,
    buffer_lines: u64,
) -> SerialHandle {
    let (to_clients, _) = broadcast::channel(BROADCAST_CAP);
    let (write_tx, write_rx) = mpsc::channel(WRITE_CAP);
    let (dtr_tx, dtr_rx) = mpsc::channel::<(u64, oneshot::Sender<()>)>(1);
    let ring = Arc::new(Mutex::new(VecDeque::with_capacity(RING_BYTES)));
    let status = Arc::new(Mutex::new(Status {
        device: spec.device.clone(),
        baud: spec.baud,
        connected: false,
        power_on: None,
    }));

    let line_tx = spawn_capture(capture_dir, buffer_lines);

    tokio::spawn(supervisor(
        spec,
        to_clients.clone(),
        write_rx,
        dtr_rx,
        ring.clone(),
        status.clone(),
        line_tx,
    ));

    SerialHandle {
        to_clients,
        write_tx,
        ring,
        status,
        dtr_tx,
    }
}

/// Start the OS thread that owns the line log and return a non-blocking sender
/// for raw byte chunks. An unbounded channel means the supervisor never blocks
/// on capture; serial throughput is tiny, so the queue can't grow meaningfully.
fn spawn_capture(capture_dir: PathBuf, buffer_lines: u64) -> stdmpsc::Sender<Bytes> {
    let (tx, rx) = stdmpsc::channel::<Bytes>();
    std::thread::Builder::new()
        .name("serialcap-capture".into())
        .spawn(move || {
            let mut log = LineLog::open(capture_dir, buffer_lines);
            // Iteration ends when every sender has dropped (daemon shutting down).
            for chunk in rx {
                log.ingest(&chunk);
            }
        })
        .expect("spawn capture thread");
    tx
}

/// Read one modem-control sense signal and translate it to `power_on`.
///
/// The target's 3.3 V rail is wired (with a pull-down) to the chosen FTDI input
/// pin.  The pin is HIGH when the rail is up (power on) and LOW when off.  FTDI
/// signal sense is active-low in RS-232 convention, so `read_*()` returns
/// `true` when the pin is LOW — meaning powered off.  We invert to get a
/// natural `power_on = true` when the board is running.
fn read_power_sense(port: &mut impl SerialPort, signal: &str) -> Option<bool> {
    match signal {
        "cts" => port.read_clear_to_send().ok().map(|v| !v),
        "dsr" => port.read_data_set_ready().ok().map(|v| !v),
        "dcd" => port.read_carrier_detect().ok().map(|v| !v),
        "ri" => port.read_ring_indicator().ok().map(|v| !v),
        _ => None,
    }
}

async fn supervisor(
    spec: InterfaceSpec,
    to_clients: broadcast::Sender<Bytes>,
    mut write_rx: mpsc::Receiver<Bytes>,
    mut dtr_rx: mpsc::Receiver<(u64, oneshot::Sender<()>)>,
    ring: Arc<Mutex<VecDeque<u8>>>,
    status: Arc<Mutex<Status>>,
    line_tx: stdmpsc::Sender<Bytes>,
) {
    let InterfaceSpec {
        device,
        baud,
        power_sense_signal,
        ..
    } = spec;

    // Track whether we've ever connected so the first open shows "connected"
    // and later opens show "reconnected".
    let mut ever_connected = false;

    loop {
        let port = match tokio_serial::new(&device, baud).open_native_async() {
            Ok(mut p) => {
                info!("serial port opened: {device} @ {baud}");
                {
                    let mut st = status.lock().unwrap();
                    st.connected = true;
                    if let Some(sig) = &power_sense_signal {
                        st.power_on = read_power_sense(&mut p, sig);
                    }
                }
                if ever_connected {
                    emit_marker(&ring, &to_clients, &line_tx, "reconnected", 32);
                // green
                } else {
                    emit_marker(&ring, &to_clients, &line_tx, "connected", 36); // cyan
                    ever_connected = true;
                }
                p
            }
            Err(e) => {
                warn!("open {device} failed: {e}");
                status.lock().unwrap().connected = false;
                tokio::time::sleep(REOPEN_DELAY).await;
                continue;
            }
        };

        let (mut rd, mut wr) = tokio::io::split(port);
        let mut buf = [0u8; 65536];

        enum InnerExit {
            Disconnect,
            DtrPress {
                duration_ms: u64,
                resp_tx: oneshot::Sender<()>,
            },
        }

        let exit = loop {
            tokio::select! {
                read = rd.read(&mut buf) => match read {
                    Ok(0) => {
                        // Serial ports don't have EOF; Ok(0) means the async
                        // read resolved without data. Yield to avoid a spin loop.
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    Ok(n) => {
                        let chunk = Bytes::copy_from_slice(&buf[..n]);
                        push_ring(&ring, &chunk);
                        if line_tx.send(chunk.clone()).is_err() {
                            warn!("capture thread dead — bytes lost");
                        }
                        // Err just means no subscribers; that's fine.
                        let _ = to_clients.send(chunk);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    Err(e) => { warn!("serial read error: {e}"); break InnerExit::Disconnect; }
                },
                Some(data) = write_rx.recv() => {
                    if let Err(e) = wr.write_all(&data).await {
                        warn!("serial write error: {e}");
                        break InnerExit::Disconnect;
                    }
                },
                Some((duration_ms, resp_tx)) = dtr_rx.recv() => {
                    break InnerExit::DtrPress { duration_ms, resp_tx };
                }
            }
        };

        match exit {
            InnerExit::DtrPress {
                duration_ms,
                resp_tx,
            } => {
                // Rejoin the split halves to regain the SerialPort trait methods.
                let mut port = rd.unsplit(wr);
                emit_marker(&ring, &to_clients, &line_tx, "button press", 35); // magenta
                port.write_data_terminal_ready(true).ok();
                tokio::time::sleep(Duration::from_millis(duration_ms)).await;
                port.write_data_terminal_ready(false).ok();
                // Read power state immediately after releasing the button — the
                // 3.3 V rail may have dropped (long press → power-off).
                if let Some(sig) = &power_sense_signal {
                    status.lock().unwrap().power_on = read_power_sense(&mut port, sig);
                }
                resp_tx.send(()).ok();
                // Drop the port and re-enter the outer reconnect loop immediately.
                drop(port);
                continue;
            }
            InnerExit::Disconnect => {
                // We only reach here after a successful open, so this is a real
                // disconnect (link dropped / device unplugged), not a failed open.
                status.lock().unwrap().connected = false;
                emit_marker(&ring, &to_clients, &line_tx, "disconnected", 31); // red
                tokio::time::sleep(REOPEN_DELAY).await;
            }
        }
    }
}

/// Inject a styled, timestamped status line into the stream and scrollback so the
/// web terminal shows exactly when the serial link dropped or came back. ANSI
/// color `code` (31 red / 32 green / 36 cyan); only the WS terminal renders it —
/// `tio` uses a different path.
fn emit_marker(
    ring: &Arc<Mutex<VecDeque<u8>>>,
    to_clients: &broadcast::Sender<Bytes>,
    line_tx: &stdmpsc::Sender<Bytes>,
    label: &str,
    code: u8,
) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let sod = secs % 86_400;
    let line = format!(
        "\r\n\x1b[1;{code}m── serial {label} [{:02}:{:02}:{:02} UTC] ──\x1b[0m\r\n",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60,
    );
    let bytes = Bytes::from(line.into_bytes());
    push_ring(ring, &bytes);
    let _ = line_tx.send(bytes.clone());
    let _ = to_clients.send(bytes);
}

fn push_ring(ring: &Arc<Mutex<VecDeque<u8>>>, chunk: &[u8]) {
    // try_lock: if scrollback() holds the lock, skip rather than blocking
    // the supervisor OS thread (which stalls the read loop and risks FIFO overflow).
    if let Ok(mut r) = ring.try_lock() {
        r.extend(chunk.iter().copied());
        let overflow = r.len().saturating_sub(RING_BYTES);
        if overflow > 0 {
            r.drain(0..overflow);
        }
    }
}

/// Enumerate serial ports on this host.
pub fn list_ports() -> anyhow::Result<Vec<(String, String)>> {
    let ports = tokio_serial::available_ports()?;
    Ok(ports
        .into_iter()
        .map(|p| (p.port_name, describe(&p.port_type)))
        .collect())
}

fn describe(t: &tokio_serial::SerialPortType) -> String {
    use tokio_serial::SerialPortType;
    match t {
        SerialPortType::UsbPort(info) => {
            let product = info.product.as_deref().unwrap_or("");
            let manuf = info.manufacturer.as_deref().unwrap_or("");
            format!("USB {:04x}:{:04x} {manuf} {product}", info.vid, info.pid)
                .trim()
                .to_string()
        }
        SerialPortType::PciPort => "PCI".into(),
        SerialPortType::BluetoothPort => "Bluetooth".into(),
        SerialPortType::Unknown => "unknown".into(),
    }
}
````

## File: src/paniolo/_dhcp.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Minimal DHCP server for paniolo netboot.

Sends broadcast DHCP responses (no BPF required, no root required on macOS 14+).
Handles DISCOVER→OFFER and REQUEST→ACK for a single netboot client.

Usage (as subprocess):
    python -m paniolo._dhcp <host_ip> [--boot-file <filename>]

host_ip is both the interface address and the TFTP/siaddr advertised to clients.
"""

from __future__ import annotations

import argparse
import logging
import socket
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path

_BOOTREQUEST = 1
_BOOTREPLY = 2
_HTYPE_ETHERNET = 1
_MAGIC = b"\x63\x82\x53\x63"

_OPT_SUBNET = 1
_OPT_ROUTER = 3
_OPT_LEASE = 51
_OPT_MSG_TYPE = 53
_OPT_SERVER_ID = 54
_OPT_TFTP_SERVER = 66
_OPT_BOOTFILE = 67
_OPT_END = 255

_DHCP_DISCOVER = 1
_DHCP_OFFER = 2
_DHCP_REQUEST = 3
_DHCP_ACK = 5
_DHCP_NAK = 6

_LEASE_SECONDS = 12 * 3600
_ASSIGNED_IP = "192.168.99.100"

# Shared file written by this DHCP server, read by the co-process TFTP server.
# The TFTP server needs the client's real MAC to build BPF raw frames (the Pi
# bootloader sends TFTP from a different ephemeral MAC than the one it used for
# DHCP, which causes macOS to install the wrong ARP entry — see _tftp.py).
# Placed in the user state dir (not /tmp) to prevent symlink and spoofing attacks.
_CLIENT_MAC_FILE = Path.home() / ".local" / "share" / "paniolo" / "client-mac"

log = logging.getLogger(__name__)


def _parse_options(options: bytes) -> dict[int, bytes]:
    result: dict[int, bytes] = {}
    if options[:4] != _MAGIC:
        return result
    i = 4
    while i < len(options):
        tag = options[i]
        if tag == _OPT_END:
            break
        if tag == 0:
            i += 1
            continue
        if i + 1 >= len(options):
            break
        length = options[i + 1]
        result[tag] = options[i + 2 : i + 2 + length]
        i += 2 + length
    return result


def _encode_option(tag: int, value: bytes) -> bytes:
    return bytes([tag, len(value)]) + value


def _build_reply(
    xid: bytes,
    chaddr: bytes,
    msg_type: int,
    server_ip: str,
    assigned_ip: str,
    boot_file: str,
) -> bytes:
    server_b = socket.inet_aton(server_ip)
    client_b = socket.inet_aton(assigned_ip)

    opts = _MAGIC
    opts += _encode_option(_OPT_MSG_TYPE, bytes([msg_type]))
    opts += _encode_option(_OPT_SERVER_ID, server_b)
    opts += _encode_option(_OPT_LEASE, struct.pack("!I", _LEASE_SECONDS))
    opts += _encode_option(_OPT_SUBNET, socket.inet_aton("255.255.255.0"))
    opts += _encode_option(_OPT_ROUTER, server_b)
    opts += _encode_option(_OPT_TFTP_SERVER, server_ip.encode())
    opts += _encode_option(_OPT_BOOTFILE, boot_file.encode())
    opts += bytes([_OPT_END])

    pkt = struct.pack("!BBBB", _BOOTREPLY, _HTYPE_ETHERNET, 6, 0)
    pkt += xid
    pkt += struct.pack("!HH", 0, 0x8000)
    pkt += b"\x00" * 4  # ciaddr
    pkt += client_b  # yiaddr
    pkt += server_b  # siaddr (next-server = TFTP)
    pkt += b"\x00" * 4  # giaddr
    pkt += chaddr[:16]  # chaddr (padded to 16)
    pkt += b"\x00" * 64  # sname
    file_bytes = boot_file.encode()[:127]
    pkt += file_bytes + b"\x00" * (128 - len(file_bytes))  # file (null-padded)
    pkt += opts
    return pkt


def _set_arp(ip: str, mac: str, interface: str | None = None) -> None:
    """Pin a static ARP entry mapping the client IP to the MAC we just saw in a
    DHCP packet.

    The Pi's netboot firmware sends us DHCP/TFTP but does NOT answer ARP
    requests. We already know the MAC from the DHCP frame, so install it
    directly. Calling this on each DHCP exchange tracks the active MAC (the Pi
    cycles through several boot phases). Needs root.

    macOS: uses `arp -s` (net-tools syntax).
    Linux: uses `ip neigh replace` (iproute2, requires interface name).
    """
    if sys.platform == "darwin":
        r = subprocess.run(
            ["sudo", "arp", "-s", ip, mac], capture_output=True, text=True
        )
        if r.returncode != 0:
            log.warning("arp -s %s %s failed: %s", ip, mac, r.stderr.strip() or r.stdout.strip())
    else:
        cmd = ["sudo", "ip", "neigh", "replace", ip, "lladdr", mac, "nud", "permanent"]
        if interface:
            cmd += ["dev", interface]
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0:
            log.warning(
                "ip neigh replace %s lladdr %s failed: %s", ip, mac, r.stderr.strip()
            )
    # Share with the co-process TFTP server so it can build BPF raw frames
    # (macOS) or just for diagnostics (Linux).
    try:
        _CLIENT_MAC_FILE.parent.mkdir(parents=True, exist_ok=True)
        _CLIENT_MAC_FILE.write_text(mac)
    except OSError as exc:
        log.debug("could not write client MAC file: %s", exc)


def _has_interface_ip(interface: str, host_ip: str) -> bool:
    """Return True if `host_ip` is currently assigned to `interface`."""
    if sys.platform == "darwin":
        try:
            out = subprocess.check_output(
                ["ifconfig", interface], text=True, stderr=subprocess.DEVNULL
            )
            return f"inet {host_ip} " in out
        except (subprocess.CalledProcessError, FileNotFoundError):
            return False
    else:
        # Check sysfs; /sys/class/net/<iface>/address holds MAC but not IP.
        # Use `ip addr show` instead.
        try:
            out = subprocess.check_output(
                ["ip", "addr", "show", "dev", interface],
                text=True,
                stderr=subprocess.DEVNULL,
            )
            return f"inet {host_ip}/" in out or f"inet {host_ip} " in out
        except (subprocess.CalledProcessError, FileNotFoundError):
            return False


def _is_link_up(interface: str) -> bool:
    """Return True if the interface link is currently up."""
    if sys.platform == "darwin":
        try:
            out = subprocess.check_output(
                ["ifconfig", interface], text=True, stderr=subprocess.DEVNULL
            )
            return "status: active" in out
        except (subprocess.CalledProcessError, FileNotFoundError):
            return False
    else:
        try:
            carrier = Path(f"/sys/class/net/{interface}/carrier").read_text().strip()
            return carrier == "1"
        except OSError:
            return False


def _monitor_interface(interface: str, host_ip: str) -> None:
    """Continuously enforce the static IP on the interface.

    The netboot client flaps the link on every power-cycle and at several
    points during its own boot. macOS drops a manually-set IPv4 on link flap;
    Linux is more stable but NetworkManager may reset the address. We poll fast
    and re-apply immediately so the client's next retry always succeeds.
    """
    had_ip = True
    while True:
        time.sleep(1.0)
        has_ip = _has_interface_ip(interface, host_ip)
        is_active = _is_link_up(interface)

        if not has_ip and is_active:
            if sys.platform == "darwin":
                subprocess.run(
                    ["sudo", "ifconfig", interface, host_ip, "netmask", "255.255.255.0", "up"],
                    check=False,
                )
            else:
                subprocess.run(
                    ["sudo", "ip", "addr", "add", f"{host_ip}/24", "dev", interface],
                    check=False,
                    capture_output=True,
                )
            if had_ip:
                log.warning("interface %s lost IP %s — restoring", interface, host_ip)
        elif has_ip and not had_ip:
            log.info("interface %s restored with IP %s", interface, host_ip)

        had_ip = has_ip


def serve(
    host_ip: str, boot_file: str = "kernel_2712.img", interface: str | None = None
) -> None:
    prefix = host_ip.rsplit(".", 1)[0]
    bcast = f"{prefix}.255"

    if interface:
        t = threading.Thread(
            target=_monitor_interface, args=(interface, host_ip), daemon=True
        )
        t.start()

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    try:
        sock.bind(("", 67))
    except PermissionError:
        log.error(
            "Cannot bind to port 67 (DHCP). On Linux, run paniolo as root or "
            "grant CAP_NET_BIND_SERVICE: sudo setcap cap_net_bind_service=+ep "
            "$(which python3)"
        )
        raise
    log.info(
        "DHCP listening on 0.0.0.0:67  host_ip=%s  bcast=%s  boot_file=%s",
        host_ip,
        bcast,
        boot_file,
    )

    while True:
        try:
            data, _addr = sock.recvfrom(4096)
        except OSError as exc:
            log.error("recvfrom: %s", exc)
            continue

        if len(data) < 240:
            continue
        op = data[0]
        if op != _BOOTREQUEST:
            continue

        xid = data[4:8]
        chaddr = data[28:44]
        mac = data[28:34].hex(":")
        options = _parse_options(data[236:])

        msg_type = options.get(_OPT_MSG_TYPE, b"")
        if not msg_type:
            continue
        msg_type_val = msg_type[0]

        if msg_type_val == _DHCP_DISCOVER:
            log.info("DHCPDISCOVER from %s", mac)
            _set_arp(_ASSIGNED_IP, mac, interface)
            reply = _build_reply(xid, chaddr, _DHCP_OFFER, host_ip, _ASSIGNED_IP, boot_file)
            sock.sendto(reply, (bcast, 68))
            log.info(
                "DHCPOFFER → %s  ip=%s  tftp=%s  file=%s",
                mac,
                _ASSIGNED_IP,
                host_ip,
                boot_file,
            )

        elif msg_type_val == _DHCP_REQUEST:
            log.info("DHCPREQUEST from %s", mac)
            _set_arp(_ASSIGNED_IP, mac, interface)
            reply = _build_reply(xid, chaddr, _DHCP_ACK, host_ip, _ASSIGNED_IP, boot_file)
            sock.sendto(reply, (bcast, 68))
            log.info("DHCPACK → %s  ip=%s", mac, _ASSIGNED_IP)


def main() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
        stream=sys.stderr,
    )
    parser = argparse.ArgumentParser(description="Paniolo minimal DHCP server")
    parser.add_argument("host_ip", help="Interface IP (also advertised as TFTP server)")
    parser.add_argument("--boot-file", default="kernel_2712.img")
    parser.add_argument("--interface", help="Interface device name (e.g. en11) for IP monitoring")
    args = parser.parse_args()
    serve(args.host_ip, args.boot_file, args.interface)


if __name__ == "__main__":
    main()
````

## File: src/paniolo/_serial.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Serial console helpers for paniolo targets.

Two paths share this module:
- `tio` for an interactive terminal in the current shell (`paniolo serial connect`)
- the `serialcap` daemon, which owns the port and fans it out over a localhost
  WebSocket for the combined video+serial dashboard (`paniolo serial watch`)
"""

from __future__ import annotations

import glob
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path
from typing import TYPE_CHECKING, Optional, Sequence

if TYPE_CHECKING:
    from ._config import SerialInterface


def list_serial_devices() -> list[str]:
    """Return available serial device paths on this platform.

    On Linux, returns /dev/serial/by-path/ symlinks when available so the paths
    are stable across USB re-enumeration. Falls back to raw /dev/ttyUSB* paths.
    """
    if sys.platform == "darwin":
        paths = glob.glob("/dev/tty.usbserial-*") + glob.glob("/dev/tty.usbmodem*")
        return sorted(paths)
    by_path = sorted(glob.glob("/dev/serial/by-path/*"))
    if by_path:
        return by_path
    return sorted(glob.glob("/dev/ttyUSB*") + glob.glob("/dev/ttyACM*"))


def canonical_device_path(device: str) -> str:
    """Return the stable /dev/serial/by-path symlink for a raw device path.

    If device already contains '/dev/serial/', it is returned unchanged.
    On macOS, returns device unchanged. Returns device unchanged if no
    matching by-path symlink is found.
    """
    if sys.platform == "darwin" or "/dev/serial/" in device:
        return device
    try:
        target = Path(device).resolve()
        for link in sorted(Path("/dev/serial/by-path").glob("*")):
            if link.resolve() == target:
                return str(link)
    except OSError:
        pass
    return device


def tio_binary() -> str | None:
    """Return the path to tio, or None if not found."""
    return shutil.which("tio")


def connect_cmd(device: str, baud: int = 115200) -> list[str]:
    """Build the tio command to open an interactive serial terminal."""
    binary = tio_binary()
    if not binary:
        raise FileNotFoundError("tio not found in PATH")
    return [binary, "--baudrate", str(baud), device]


def log_cmd(
    binary: str,
    *,
    interface: Optional[str] = None,
    tail: Optional[int] = None,
    from_seq: Optional[int] = None,
    to_seq: Optional[int] = None,
    since: Optional[int] = None,
    raw: bool = False,
    as_json: bool = False,
    no_pending: bool = False,
) -> list[str]:
    """Build the `serialcap log` argv for the captured-output reader.

    serialcap reads its own on-disk capture log, so this works whether or not the
    daemon is running. `interface` selects which interface's log to read (optional
    when only one was captured). Only set flags are forwarded; the binary applies
    its own defaults (most recent lines, ANSI-stripped, pending line included)."""
    cmd = [binary, "log"]
    if interface is not None:
        cmd += ["--interface", interface]
    if tail is not None:
        cmd += ["--tail", str(tail)]
    if from_seq is not None:
        cmd += ["--from", str(from_seq)]
    if to_seq is not None:
        cmd += ["--to", str(to_seq)]
    if since is not None:
        cmd += ["--since", str(since)]
    if raw:
        cmd.append("--raw")
    if as_json:
        cmd.append("--json")
    if no_pending:
        cmd.append("--no-pending")
    return cmd


def serialcap_binary() -> Optional[str]:
    """Return the installed serialcap path: PATH, then ~/.cargo/bin. None if absent.

    Installed by `paniolo setup` (cargo install). Never resolved from the in-repo
    build tree, so a running daemon can't point at an ephemeral build artifact.
    """
    found = shutil.which("serialcap")
    if found:
        return found
    cargo_bin = Path.home() / ".cargo" / "bin" / "serialcap"
    return str(cargo_bin) if cargo_bin.exists() else None


def _discovery_path() -> Path:
    """Path where serialcap writes its daemon.json discovery file.

    Mirrors serialcap/src/daemon.rs::runtime_dir(): prefer $XDG_RUNTIME_DIR
    (set by systemd on Linux), fall back to tempfile.gettempdir().
    """
    base = os.environ.get("XDG_RUNTIME_DIR") or tempfile.gettempdir()
    return Path(base) / "serialcap" / "daemon.json"


def read_discovery() -> Optional[dict]:
    """Read serialcap's discovery file, returning {pid, port, device, baud} or None."""
    path = _discovery_path()
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return None


def daemon_url() -> Optional[str]:
    """Return the base URL of the running serialcap daemon, or None if stopped."""
    disc = read_discovery()
    if disc is None:
        return None
    try:
        os.kill(int(disc["pid"]), 0)
    except (ProcessLookupError, PermissionError, KeyError):
        return None
    return f"http://127.0.0.1:{disc['port']}"


def interface_arg(name: str, device: str, baud: int, power_sense_signal: Optional[str] = None) -> str:
    """Format one interface for the daemon's repeatable --interface flag.

    Format: NAME=DEVICE[@BAUD][:SENSE]
    SENSE is one of cts, dsr, dcd, ri — the FTDI modem-control input wired to
    the target's 3.3 V rail for power-state sensing.
    """
    arg = f"{name}={device}@{baud}"
    if power_sense_signal:
        arg += f":{power_sense_signal}"
    return arg


def daemon_cmd(
    binary: str,
    interfaces: "Sequence[SerialInterface]",
    port: int = 8724,
    buffer_lines: Optional[int] = None,
) -> list[str]:
    """Build the `serialcap daemon` argv owning every given interface."""
    cmd = [binary, "daemon", "--port", str(port)]
    if buffer_lines is not None:
        cmd += ["--buffer-lines", str(buffer_lines)]
    for iface in interfaces:
        cmd += [
            "--interface",
            interface_arg(iface.name, iface.device, iface.baud, iface.power_sense_signal),
        ]
    return cmd


def wait_power_off(daemon_url: str, interface_name: str, timeout_s: float = 10.0) -> bool:
    """Poll GET /status until power_on == False or timeout.

    Returns True if the power-off was confirmed by the sense signal before the
    timeout.  Returns False if the sense signal is not configured for this
    interface (power_on is null in the response) or if the timeout expires.
    """
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            url = f"{daemon_url}/status?interface={interface_name}"
            req = urllib.request.Request(url)
            with urllib.request.urlopen(req, timeout=2) as resp:
                data = json.loads(resp.read())
                if data.get("power_on") is False:
                    return True
        except Exception:
            pass
        time.sleep(0.5)
    return False


def read_power_state(daemon_url: str, interface_name: str) -> Optional[bool]:
    """Return the current power state from the daemon status, or None if unknown."""
    try:
        url = f"{daemon_url}/status?interface={interface_name}"
        req = urllib.request.Request(url)
        with urllib.request.urlopen(req, timeout=2) as resp:
            data = json.loads(resp.read())
            return data.get("power_on")
    except Exception:
        return None


def start_daemon(
    interfaces: "Sequence[SerialInterface]",
    port: int = 8724,
    buffer_lines: Optional[int] = None,
) -> subprocess.Popen:
    """Start the serialcap daemon (owning all interfaces) detached; caller should
    poll daemon_url()."""
    binary = serialcap_binary()
    if not binary:
        raise FileNotFoundError("serialcap not found in PATH or project build dir")
    return subprocess.Popen(
        daemon_cmd(binary, interfaces, port, buffer_lines),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
````

## File: src/paniolo/_state.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

from __future__ import annotations

import dataclasses
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Optional

STATE_DIR = Path.home() / ".local" / "share" / "paniolo"


@dataclasses.dataclass
class NetbootState:
    target: str
    dhcp_pid: int
    tftp_pid: int
    started_at: float
    interface: str
    tftp_root: str


def _target_dir(target: str) -> Path:
    return STATE_DIR / target


def netboot_state_path(target: str) -> Path:
    return _target_dir(target) / "netboot.json"


def netboot_log_path(target: str) -> Path:
    return _target_dir(target) / "netboot.log"


def ensure_target_dir(target: str) -> Path:
    d = _target_dir(target)
    d.mkdir(parents=True, exist_ok=True)
    return d


def save_netboot_state(state: NetbootState) -> None:
    ensure_target_dir(state.target)
    netboot_state_path(state.target).write_text(
        json.dumps(dataclasses.asdict(state), indent=2)
    )


def load_netboot_state(target: str) -> Optional[NetbootState]:
    path = netboot_state_path(target)
    if not path.exists():
        return None
    try:
        data = json.loads(path.read_text())
        return NetbootState(**data)
    except (json.JSONDecodeError, TypeError, KeyError):
        return None


def is_pid_alive(pid: int) -> bool:
    """Return True if any process with this PID exists (signal-0 probe)."""
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        # PID exists but we cannot signal it -- still alive.
        return True


def _pid_cmdline(pid: int) -> str:
    """Return the full command-line string for pid, or empty string on failure."""
    if sys.platform != "darwin":
        try:
            return (
                Path(f"/proc/{pid}/cmdline")
                .read_bytes()
                .replace(b"\x00", b" ")
                .decode(errors="replace")
                .strip()
            )
        except OSError:
            return ""
    try:
        result = subprocess.run(
            ["ps", "-p", str(pid), "-o", "args="],
            capture_output=True,
            text=True,
        )
        return result.stdout.strip()
    except Exception:  # pylint: disable=broad-except
        return ""


def is_paniolo_child_alive(pid: int, module: str) -> bool:
    """Return True only if pid is alive AND its command line contains module.

    Guards against stale PIDs reused by unrelated processes after a paniolo
    child crashes.  module is the Python module name passed to -m, e.g.
    'paniolo._tftp'.
    """
    if not is_pid_alive(pid):
        return False
    return module in _pid_cmdline(pid)


def is_netboot_running(target: str) -> bool:
    """Return True only if both child processes are alive and are our processes."""
    state = load_netboot_state(target)
    if state is None:
        return False
    return (
        is_paniolo_child_alive(state.dhcp_pid, "paniolo._dhcp")
        and is_paniolo_child_alive(state.tftp_pid, "paniolo._tftp")
    )
````

## File: src/paniolo/_tftp.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Minimal read-only TFTP server for paniolo netboot.

Read-only (RRQ) TFTP per RFC 1350, with the blksize (RFC 2348) and tsize
(RFC 2349) options the Raspberry Pi bootloader negotiates.

Why a custom server instead of an off-the-shelf one (e.g. tftp-now): on macOS
a non-root process can bind a privileged port (69) only on the wildcard
address 0.0.0.0, NOT on a specific interface IP. But the host we serve sits on
a *secondary* USB-Ethernet interface, and a reply socket left on 0.0.0.0 lets
macOS pick the wrong egress (the primary interface) -> sendto() fails with
EHOSTUNREACH ("no route to host"). The first fix is to listen on the wildcard
while binding each reply socket to the specific interface IP on an ephemeral
port, pinning egress to the right NIC.

However, on macOS 15+ (Sequoia / "macOS 26") the kernel refuses to unicast to
the Pi bootloader even with a permanent static ARP entry, because the bootloader
sends TFTP packets from a different ephemeral source MAC than the one it used for
DHCP. The kernel learns that ephemeral MAC as the host route for the client IP,
then won't deliver frames to it (the bootloader only receives on its real MAC).
The second fix is a BPF raw-frame sender: when sendto returns EHOSTUNREACH we
write a complete Ethernet/IPv4/UDP frame directly to /dev/bpf, using the
bootloader's real DHCP MAC (shared via /tmp/paniolo-client-mac by _dhcp.py) as
the destination, bypassing the kernel's ARP table entirely.

Usage (as subprocess):
    python -m paniolo._tftp <host_ip> <root> [--port 69] [--interface <iface>]
"""

from __future__ import annotations

import argparse
import errno
import fcntl
import logging
import os
import re
import socket
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path

_OP_RRQ = 1
_OP_WRQ = 2
_OP_DATA = 3
_OP_ACK = 4
_OP_ERROR = 5
_OP_OACK = 6

_ERR_NOT_FOUND = 1
_ERR_ACCESS = 2
_ERR_ILLEGAL = 4

_DEFAULT_BLKSIZE = 512
_ACK_TIMEOUT = 1.0
_MAX_RETRIES = 6
_ARP_RESOLVE_TIMEOUT = 4.0

# Shared with _dhcp.py: the DHCP server writes the client MAC here so we can
# use it as the BPF frame destination, bypassing the kernel's ARP table.
# Placed in the user state dir (not /tmp) to prevent symlink and spoofing attacks.
_CLIENT_MAC_FILE = Path.home() / ".local" / "share" / "paniolo" / "client-mac"

# macOS BPF ioctl constants (64-bit).  Used by BpfSender below.
_BIOCSETIF = 0x8020426C  # bind BPF fd to an interface (struct ifreq, 32 B)
_BIOCSHDRCMPLT = 0x80044275  # tell kernel we write complete L2 headers

log = logging.getLogger(__name__)


# ── BPF raw-frame sender ──────────────────────────────────────────────────────


def _inet_checksum(data: bytes) -> int:
    if len(data) % 2:
        data += b"\x00"
    total = sum(struct.unpack("!%dH" % (len(data) // 2), data))
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return ~total & 0xFFFF


def _build_udp_frame(
    src_mac: bytes,
    dst_mac: bytes,
    src_ip: str,
    dst_ip: str,
    src_port: int,
    dst_port: int,
    payload: bytes,
) -> bytes:
    """Construct a raw Ethernet/IPv4/UDP frame."""
    src_a = socket.inet_aton(src_ip)
    dst_a = socket.inet_aton(dst_ip)
    udp_len = 8 + len(payload)
    ip_len = 20 + udp_len

    ip_hdr = struct.pack(
        "!BBHHHBBH4s4s",
        0x45,
        0,
        ip_len,
        0,
        0x4000,
        64,
        17,
        0,
        src_a,
        dst_a,
    )
    ip_ck = _inet_checksum(ip_hdr)
    ip_hdr = ip_hdr[:10] + struct.pack("!H", ip_ck) + ip_hdr[12:]

    udp_hdr_no_ck = struct.pack("!HHH", src_port, dst_port, udp_len)
    pseudo = src_a + dst_a + b"\x00\x11" + struct.pack("!H", udp_len)
    udp_ck = _inet_checksum(pseudo + udp_hdr_no_ck + b"\x00\x00" + payload)
    udp_hdr = udp_hdr_no_ck + struct.pack("!H", udp_ck)

    return dst_mac + src_mac + b"\x08\x00" + ip_hdr + udp_hdr + payload


def _get_if_mac(iface: str) -> bytes:
    if sys.platform != "darwin":
        # Linux: read directly from sysfs (no ifconfig needed).
        addr = Path(f"/sys/class/net/{iface}/address").read_text().strip()
        return bytes(int(b, 16) for b in addr.split(":"))
    out = subprocess.check_output(
        ["ifconfig", iface], text=True, stderr=subprocess.DEVNULL
    )
    m = re.search(r"\bether\s+((?:[0-9a-f]{2}:){5}[0-9a-f]{2})\b", out)
    if not m:
        raise ValueError(f"No ether address for {iface}")
    return bytes(int(b, 16) for b in m.group(1).split(":"))


def _open_bpf_fd(iface: str) -> int | None:
    """Open a writable BPF device bound to iface. macOS only; returns None elsewhere."""
    if sys.platform != "darwin":
        return None
    for n in range(10):
        try:
            fd = os.open(f"/dev/bpf{n}", os.O_RDWR)
        except OSError:
            continue
        try:
            ifreq = bytearray(32)
            ifreq[: len(iface)] = iface.encode()
            fcntl.ioctl(fd, _BIOCSETIF, ifreq)
            fcntl.ioctl(fd, _BIOCSHDRCMPLT, struct.pack("I", 1))
            return fd
        except OSError as exc:
            os.close(fd)
            log.debug("BPF /dev/bpf%d bind %s: %s", n, iface, exc)
    return None


class BpfSender:
    """Sends UDP packets as raw Ethernet frames via /dev/bpf, bypassing the
    kernel ARP table.  Used on macOS when sendto returns EHOSTUNREACH because
    the kernel has installed the wrong destination MAC for the Pi bootloader.
    On Linux, BPF is not available; `available` is always False."""

    def __init__(self, iface: str, host_ip: str) -> None:
        self._host_ip = host_ip
        self._fd: int | None = None
        self._src_mac: bytes | None = None
        self._lock = threading.Lock()
        if sys.platform != "darwin":
            return
        try:
            self._src_mac = _get_if_mac(iface)
            self._fd = _open_bpf_fd(iface)
            if self._fd is not None:
                log.info(
                    "BPF sender ready on %s (src %s)",
                    iface,
                    self._src_mac.hex(":"),
                )
            else:
                log.warning(
                    "BPF unavailable on %s — check /dev/bpf* permissions or "
                    "add user to 'access_bpf' group",
                    iface,
                )
        except Exception as exc:
            log.warning("BPF init failed: %s", exc)

    @property
    def available(self) -> bool:
        return self._fd is not None and self._src_mac is not None

    def _read_client_mac(self) -> bytes | None:
        try:
            mac_str = _CLIENT_MAC_FILE.read_text().strip()
            return bytes(int(b, 16) for b in mac_str.split(":"))
        except Exception:
            return None

    def send(self, sock: socket.socket, packet: bytes, peer: tuple) -> bool:
        """Send packet as a raw frame. sock supplies the ephemeral src port."""
        if not self.available:
            return False
        dst_mac = self._read_client_mac()
        if dst_mac is None:
            log.warning("BPF: no client MAC in %s", _CLIENT_MAC_FILE)
            return False
        src_port = sock.getsockname()[1]
        dst_ip, dst_port = peer
        try:
            frame = _build_udp_frame(
                self._src_mac,
                dst_mac,  # type: ignore[arg-type]
                self._host_ip,
                dst_ip,
                src_port,
                dst_port,
                packet,
            )
            with self._lock:
                os.write(self._fd, frame)  # type: ignore[arg-type]
            log.debug(
                "BPF sent %d B to %s:%d (dst MAC %s)",
                len(frame),
                dst_ip,
                dst_port,
                dst_mac.hex(":"),
            )
            return True
        except OSError as exc:
            log.warning("BPF write failed: %s", exc)
            return False

    def close(self) -> None:
        with self._lock:
            if self._fd is not None:
                os.close(self._fd)
                self._fd = None


def _sendto(
    sock: socket.socket, packet: bytes, peer, bpf: "BpfSender | None" = None
) -> bool:
    """sendto() with BPF raw-frame fallback for EHOSTUNREACH.

    macOS 15+ refuses to deliver unicast UDP to the Pi bootloader even with a
    permanent static ARP entry, because the bootloader's TFTP packets arrive
    from a random source MAC (different from its DHCP MAC), and the kernel
    installs that ephemeral MAC as the host route.  When sendto hits
    EHOSTUNREACH we fall back to a /dev/bpf raw frame addressed to the real
    DHCP MAC (written by _dhcp.py to _CLIENT_MAC_FILE).  If BPF is not
    available, retry for _ARP_RESOLVE_TIMEOUT seconds in case the ARP entry
    heals on its own (covers older macOS and the brief post-link-flap window).
    """
    if bpf is not None and bpf.available:
        # With arp_llreach_base=0 (NUD disabled), sendto() to the Pi "succeeds"
        # even when the kernel's ARP table has the wrong ephemeral source MAC the
        # bootloader used for TFTP (not its DHCP/receive MAC).  The packet is sent
        # but the Pi never receives it.  Always use BPF when available so we bypass
        # the ARP table entirely and address frames to the real DHCP MAC directly.
        if bpf.send(sock, packet, peer):
            return True
        log.debug("BPF failed, falling back to kernel sendto %s:%d", peer[0], peer[1])

    deadline = time.monotonic() + _ARP_RESOLVE_TIMEOUT
    while True:
        try:
            sock.sendto(packet, peer)
            return True
        except OSError as exc:
            if exc.errno == errno.EHOSTUNREACH and time.monotonic() < deadline:
                time.sleep(0.1)
                continue
            log.warning("sendto %s:%d failed: %s", peer[0], peer[1], exc)
            return False


def _parse_rrq(data: bytes) -> tuple[str, str, dict[str, str]] | None:
    """Return (filename, mode, options) from an RRQ payload, or None if malformed."""
    parts = data[2:].split(b"\x00")
    if len(parts) < 2:
        return None
    filename = parts[0].decode("latin-1")
    mode = parts[1].decode("latin-1").lower()
    options: dict[str, str] = {}
    rest = parts[2:]
    for i in range(0, len(rest) - 1, 2):
        key = rest[i].decode("latin-1").lower()
        if key:
            options[key] = rest[i + 1].decode("latin-1")
    return filename, mode, options


def _error_packet(code: int, msg: str) -> bytes:
    return struct.pack("!HH", _OP_ERROR, code) + msg.encode("latin-1") + b"\x00"


def _resolve(root: Path, filename: str) -> Path | None:
    """Resolve a requested filename inside root, rejecting traversal outside it."""
    candidate = (root / filename.lstrip("/")).resolve()
    try:
        candidate.relative_to(root.resolve())
    except ValueError:
        return None
    return candidate


def _send_and_wait_ack(
    sock: socket.socket,
    packet: bytes,
    peer,
    expect_block: int,
    bpf: "BpfSender | None" = None,
) -> bool:
    """Send a packet and wait for ACK of expect_block, retransmitting on timeout."""
    for attempt in range(_MAX_RETRIES):
        if not _sendto(sock, packet, peer, bpf):
            log.warning(
                "sendto %s:%d failed (attempt %d/%d), retrying",
                peer[0],
                peer[1],
                attempt + 1,
                _MAX_RETRIES,
            )
            time.sleep(0.05)
            continue
        sock.settimeout(_ACK_TIMEOUT)
        try:
            while True:
                resp, raddr = sock.recvfrom(4)
                if raddr != peer:
                    continue
                if len(resp) < 4:
                    continue
                opcode, block = struct.unpack("!HH", resp[:4])
                if opcode == _OP_ACK and block == expect_block:
                    return True
                if opcode == _OP_ERROR:
                    log.warning(
                        "ERROR from %s:%d (code=%d) waiting for ACK of block %d",
                        peer[0],
                        peer[1],
                        block,
                        expect_block,
                    )
                    return False
        except socket.timeout:
            continue
    return False


def _handle_rrq(
    host_ip: str,
    root: Path,
    data: bytes,
    peer,
    bpf: "BpfSender | None" = None,
) -> None:
    try:
        _do_rrq(host_ip, root, data, peer, bpf)
    except Exception:  # noqa: BLE001 - never let a transfer crash the server
        log.exception("RRQ handler from %s:%d crashed", peer[0], peer[1])


def _bind_reply_socket(host_ip: str) -> socket.socket | None:
    """Create a reply socket bound to host_ip:ephemeral. Retries briefly because
    the interface IP may be momentarily absent while a link flap is being
    repaired (bind would otherwise fail with EADDRNOTAVAIL)."""
    deadline = time.monotonic() + _ARP_RESOLVE_TIMEOUT
    while True:
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            sock.bind((host_ip, 0))
            return sock
        except OSError as exc:
            sock.close()
            if exc.errno == errno.EADDRNOTAVAIL and time.monotonic() < deadline:
                time.sleep(0.1)
                continue
            log.warning("cannot bind reply socket to %s: %s", host_ip, exc)
            return None


def _do_rrq(
    host_ip: str,
    root: Path,
    data: bytes,
    peer,
    bpf: "BpfSender | None" = None,
) -> None:
    parsed = _parse_rrq(data)
    # Bind the reply socket to the specific interface IP (ephemeral port) so
    # macOS routes the transfer out the correct (secondary) interface.
    xfer = _bind_reply_socket(host_ip)
    if xfer is None:
        return
    try:
        if parsed is None:
            _sendto(xfer, _error_packet(_ERR_ILLEGAL, "malformed request"), peer, bpf)
            return
        filename, mode, options = parsed
        if mode != "octet":
            _sendto(
                xfer, _error_packet(_ERR_ILLEGAL, f"unsupported mode {mode}"), peer, bpf
            )
            return

        path = _resolve(root, filename)
        if path is None or not path.is_file():
            log.info("RRQ %s from %s:%d -> NOT FOUND", filename, peer[0], peer[1])
            _sendto(xfer, _error_packet(_ERR_NOT_FOUND, "file not found"), peer, bpf)
            return

        size = path.stat().st_size
        blksize = _DEFAULT_BLKSIZE
        oack_opts: dict[str, str] = {}
        if "blksize" in options:
            try:
                req = int(options["blksize"])
                blksize = max(8, min(req, 65464))
                oack_opts["blksize"] = str(blksize)
            except ValueError:
                pass
        if "tsize" in options:
            oack_opts["tsize"] = str(size)

        log.info(
            "RRQ %s from %s:%d -> serving %d bytes (blksize=%d)",
            filename,
            peer[0],
            peer[1],
            size,
            blksize,
        )

        if oack_opts:
            payload = struct.pack("!H", _OP_OACK)
            for k, v in oack_opts.items():
                payload += k.encode("latin-1") + b"\x00" + v.encode("latin-1") + b"\x00"
            if not _send_and_wait_ack(xfer, payload, peer, 0, bpf):
                log.warning("no ACK for OACK from %s:%d", peer[0], peer[1])
                return

        with path.open("rb") as f:
            block = 1
            while True:
                chunk = f.read(blksize)
                packet = struct.pack("!HH", _OP_DATA, block & 0xFFFF) + chunk
                if not _send_and_wait_ack(xfer, packet, peer, block & 0xFFFF, bpf):
                    log.warning(
                        "transfer of %s to %s:%d failed at block %d",
                        filename,
                        peer[0],
                        peer[1],
                        block,
                    )
                    return
                block += 1
                if len(chunk) < blksize:
                    break
        log.info("completed %s to %s:%d", filename, peer[0], peer[1])
    finally:
        xfer.close()


def serve(
    host_ip: str, root: str, port: int = 69, interface: str | None = None
) -> None:
    root_path = Path(root).resolve()

    bpf: BpfSender | None = None
    if interface is not None:
        bpf = BpfSender(interface, host_ip)

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
    # Wildcard bind on 0.0.0.0: rootless on macOS 14+ for privileged ports.
    # On Linux, port 69 requires root or CAP_NET_BIND_SERVICE.
    try:
        sock.bind(("", port))
    except PermissionError:
        log.error(
            "Cannot bind to port %d (TFTP). On Linux, run paniolo as root or "
            "grant CAP_NET_BIND_SERVICE.", port
        )
        raise
    log.info(
        "TFTP listening on 0.0.0.0:%d  reply_src=%s  root=%s  bpf=%s",
        port,
        host_ip,
        root_path,
        "yes" if (bpf and bpf.available) else "no",
    )

    while True:
        try:
            data, peer = sock.recvfrom(4096)
        except OSError as exc:
            log.error("recvfrom: %s", exc)
            continue
        if len(data) < 2:
            continue
        opcode = struct.unpack("!H", data[:2])[0]
        if opcode == _OP_RRQ:
            t = threading.Thread(
                target=_handle_rrq,
                args=(host_ip, root_path, data, peer, bpf),
                daemon=True,
            )
            t.start()
        elif opcode == _OP_WRQ:
            err_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            try:
                err_sock.bind((host_ip, 0))
                _sendto(err_sock, _error_packet(_ERR_ACCESS, "read-only server"), peer, bpf)
            except OSError as exc:
                log.debug("WRQ error reply failed: %s", exc)
            finally:
                err_sock.close()


def main() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
        stream=sys.stderr,
    )
    parser = argparse.ArgumentParser(
        description="Paniolo minimal read-only TFTP server"
    )
    parser.add_argument("host_ip", help="Interface IP to bind reply sockets to")
    parser.add_argument("root", help="TFTP root directory")
    parser.add_argument("--port", type=int, default=69)
    parser.add_argument(
        "--interface", help="Interface name for BPF raw-frame fallback (e.g. en14)"
    )
    args = parser.parse_args()
    serve(args.host_ip, args.root, args.port, args.interface)


if __name__ == "__main__":
    main()
````

## File: src/paniolo/_netboot.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

from __future__ import annotations

import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

from ._config import TargetConfig
from ._state import (
    NetbootState,
    ensure_target_dir,
    is_netboot_running,
    is_paniolo_child_alive,
    is_pid_alive,
    load_netboot_state,
    netboot_log_path,
    netboot_state_path,
    save_netboot_state,
)

_BREW_PATHS = [
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
    "/usr/local/bin",
    "/usr/local/sbin",
]

# Linux: dnsmasq and other netboot tools commonly live in /usr/sbin or /sbin.
_LINUX_SBIN_PATHS = ["/usr/sbin", "/sbin"]

_EXCLUDE_PORT_PREFIXES = (
    "Wi-Fi",
    "Thunderbolt",
    "Bluetooth",
    "FireWire",
    "iPhone",
    "iPad",
)
_EXCLUDE_DEVICES = {"bridge0", "lo0"}

# Linux interfaces to skip when listing candidates for netboot.
_LINUX_SKIP_PREFIXES = ("lo", "docker", "veth", "br", "virbr", "vlan", "bond", "dummy")


def _find_bin(name: str) -> str:
    found = shutil.which(name)
    if found:
        return found
    extra = _LINUX_SBIN_PATHS if sys.platform != "darwin" else _BREW_PATHS
    for d in extra:
        p = Path(d) / name
        if p.exists():
            return str(p)
    return name


def check_deps() -> list[str]:
    # DHCP and TFTP are both pure-Python (see _dhcp.py, _tftp.py); no external
    # binaries required.
    return []


def _is_interface_active(device: str) -> bool:
    if sys.platform == "darwin":
        try:
            out = subprocess.check_output(
                ["ifconfig", device], text=True, stderr=subprocess.DEVNULL
            )
            return "status: active" in out
        except (subprocess.CalledProcessError, FileNotFoundError):
            return False
    else:
        try:
            carrier = Path(f"/sys/class/net/{device}/carrier").read_text().strip()
            return carrier == "1"
        except OSError:
            return False


def _list_linux_ethernet_interfaces() -> list[dict]:
    """Return Ethernet interfaces on Linux using sysfs.

    Each entry: {"port": str, "device": str, "active": bool}
    Skips loopback, virtual bridges, Docker, and other non-physical interfaces.
    """
    net_dir = Path("/sys/class/net")
    candidates: list[dict] = []
    try:
        entries = sorted(net_dir.iterdir())
    except OSError:
        return []
    for iface_path in entries:
        name = iface_path.name
        if any(name.startswith(p) for p in _LINUX_SKIP_PREFIXES):
            continue
        # Type 1 = Ethernet (ARPHRD_ETHER).
        try:
            if (iface_path / "type").read_text().strip() != "1":
                continue
        except OSError:
            continue
        active = _is_interface_active(name)
        candidates.append({"port": name, "device": name, "active": active})
    return sorted(candidates, key=lambda x: (not x["active"], x["device"]))


def list_usb_ethernet_interfaces() -> list[dict]:
    """Return external (non-built-in) Ethernet interfaces, active ones first.

    Each entry: {"port": str, "device": str, "active": bool}
    On macOS: queries networksetup and excludes Wi-Fi, Thunderbolt, Bluetooth, etc.
    On Linux: reads sysfs and excludes loopback and virtual interfaces.
    """
    if sys.platform != "darwin":
        return _list_linux_ethernet_interfaces()

    try:
        out = subprocess.check_output(
            ["networksetup", "-listallhardwareports"],
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return []

    candidates: list[dict] = []
    port: str | None = None
    for line in out.splitlines():
        if line.startswith("Hardware Port:"):
            port = line.split(":", 1)[1].strip()
        elif line.startswith("Device:") and port is not None:
            device = line.split(":", 1)[1].strip()
            if device not in _EXCLUDE_DEVICES and not any(
                port.startswith(p) for p in _EXCLUDE_PORT_PREFIXES
            ):
                candidates.append(
                    {
                        "port": port,
                        "device": device,
                        "active": _is_interface_active(device),
                    }
                )
            port = None

    return sorted(candidates, key=lambda x: (not x["active"], x["device"]))




def _spawn(cmd: list[str], log_path: Path, append: bool = False) -> subprocess.Popen:
    if not append:
        log_path.unlink(missing_ok=True)
    log_file = open(log_path, "a")
    env = {**os.environ, "PYTHONUNBUFFERED": "1"}
    try:
        proc = subprocess.Popen(
            cmd,
            stdout=log_file,
            stderr=log_file,
            stdin=subprocess.DEVNULL,
            start_new_session=True,
            env=env,
        )
    finally:
        log_file.close()
    return proc


def _sudo_prefix() -> list[str]:
    """Return a sudo prefix for privileged subprocesses on Linux.

    On macOS, DHCP/TFTP bind to ports 67/69 without root; no prefix needed.
    On Linux they require root (or CAP_NET_BIND_SERVICE). If we're already
    running as root, no prefix needed either.

    Uses 'sudo env PYTHONUNBUFFERED=1' so the env var reaches Python through
    sudo's environment reset without requiring the SETENV sudoers option.
    Each exec in the chain (sudo → env → python) keeps the same PID, so the
    saved PID in the state file still refers to the Python process.
    """
    if sys.platform == "darwin" or os.getuid() == 0:
        return []
    return ["sudo", "env", "PYTHONUNBUFFERED=1"]


def _find_network_service(interface: str) -> str | None:
    """Return the networksetup service name for a given device (e.g. 'en11' → 'USB 10/100/1000 LAN').
    macOS only; returns None on Linux."""
    if sys.platform != "darwin":
        return None
    try:
        out = subprocess.check_output(
            ["networksetup", "-listallhardwareports"],
            text=True,
            stderr=subprocess.DEVNULL,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None
    service: str | None = None
    for line in out.splitlines():
        if line.startswith("Hardware Port:"):
            service = line.split(":", 1)[1].strip()
        elif line.startswith("Device:"):
            if line.split(":", 1)[1].strip() == interface:
                return service
    return None


def _configure_interface(interface: str, host_ip: str) -> None:
    if sys.platform == "darwin":
        service = _find_network_service(interface)
        if service:
            subprocess.run(
                ["sudo", "networksetup", "-setmanual", service, host_ip, "255.255.255.0"],
                check=False,
            )
        result = subprocess.run(
            ["sudo", "ifconfig", interface, host_ip, "netmask", "255.255.255.0", "up"],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"ifconfig {interface} failed: {result.stderr.strip()}\n"
                "Ensure passwordless sudo is configured (NOPASSWD) for the control machine."
            )
    else:
        # Remove any existing addresses on this interface, then assign ours.
        subprocess.run(
            ["sudo", "ip", "addr", "flush", "dev", interface],
            capture_output=True,
            text=True,
            check=False,
        )
        result = subprocess.run(
            ["sudo", "ip", "addr", "add", f"{host_ip}/24", "dev", interface],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0 and "already assigned" not in result.stderr:
            raise RuntimeError(
                f"ip addr add {host_ip}/24 dev {interface} failed: {result.stderr.strip()}\n"
                "Ensure passwordless sudo is configured (NOPASSWD) for the control machine."
            )
        subprocess.run(
            ["sudo", "ip", "link", "set", interface, "up"],
            check=False,
        )


def _restore_interface(interface: str) -> None:
    """Release the static IP and return the interface to OS-managed networking."""
    if sys.platform == "darwin":
        service = _find_network_service(interface)
        if service:
            subprocess.run(
                ["sudo", "networksetup", "-setdhcp", service],
                check=False,
            )
    else:
        # Flush our static address; leave link up. A DHCP client (NetworkManager,
        # systemd-networkd, dhclient) will re-acquire an address if configured.
        subprocess.run(
            ["sudo", "ip", "addr", "flush", "dev", interface],
            check=False,
        )


def _tune_arp_for_silent_client() -> None:
    """Tweak OS neighbor-unreachability detection (NUD) for the netboot link.

    The Pi's bootloader sends us DHCP/TFTP but never answers ARP probes. Without
    tuning, the OS may mark the neighbor unreachable and refuse to send packets.

    macOS (26.x+): zeros arp_llreach_base and host_down_time so NUD never fires.
    Linux: no tuning needed — ARP entries installed via _dhcp._set_arp persist
    across link flaps and Linux's NUD does not block sends to permanent entries.
    """
    if sys.platform != "darwin":
        return
    for key, val in (
        ("net.link.ether.inet.arp_llreach_base", "0"),
        ("net.link.ether.inet.host_down_time", "0"),
    ):
        subprocess.run(["sudo", "sysctl", "-w", f"{key}={val}"], capture_output=True, text=True)


def _cleanup_stale(target: str) -> None:
    """Kill any lingering pids from a previous crashed netboot session."""
    state = load_netboot_state(target)
    if state is None:
        return
    for pid, module in (
        (state.dhcp_pid, "paniolo._dhcp"),
        (state.tftp_pid, "paniolo._tftp"),
    ):
        if is_paniolo_child_alive(pid, module):
            try:
                os.kill(pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
    netboot_state_path(target).unlink(missing_ok=True)


def start(cfg: TargetConfig) -> None:
    if is_netboot_running(cfg.name):
        raise RuntimeError(f"netboot already running for '{cfg.name}'")

    missing = check_deps()
    if missing:
        raise RuntimeError(
            f"Missing required tools: {', '.join(missing)}\n"
            "Run: paniolo setup"
        )

    if not cfg.tftp_root:
        raise RuntimeError("No tftp_root configured. Run: paniolo target set <name> --tftp-root <path>")
    tftp_root = Path(cfg.tftp_root)
    if not tftp_root.exists():
        raise RuntimeError(f"TFTP root does not exist: {tftp_root}")

    _cleanup_stale(cfg.name)
    _configure_interface(cfg.interface, cfg.host_ip)
    _tune_arp_for_silent_client()

    ensure_target_dir(cfg.name)
    log_path = netboot_log_path(cfg.name)
    sudo = _sudo_prefix()

    dhcp = _spawn(
        sudo + [sys.executable, "-m", "paniolo._dhcp", cfg.host_ip, "--interface", cfg.interface],
        log_path,
    )
    # Pure-Python TFTP server. Binds the listen socket on the wildcard so a
    # non-root process can use port 69 on macOS; on Linux we prepend sudo
    # (see _sudo_prefix). Each reply socket is bound to cfg.host_ip so the
    # OS routes transfers out the correct secondary interface (see _tftp.py).
    tftp = _spawn(
        sudo + [sys.executable, "-m", "paniolo._tftp", cfg.host_ip, str(tftp_root),
                "--interface", cfg.interface],
        log_path,
        append=True,
    )

    save_netboot_state(NetbootState(
        target=cfg.name,
        dhcp_pid=dhcp.pid,
        tftp_pid=tftp.pid,
        started_at=time.time(),
        interface=cfg.interface,
        tftp_root=str(tftp_root),
    ))


def stop(target: str) -> None:
    state = load_netboot_state(target)
    if state is None:
        raise RuntimeError(f"No netboot state for '{target}'")

    for pid in (state.dhcp_pid, state.tftp_pid):
        if is_pid_alive(pid):
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            except PermissionError:
                subprocess.run(["sudo", "kill", "-TERM", str(pid)], check=False)

    deadline = time.time() + 3.0
    while time.time() < deadline:
        if not is_pid_alive(state.dhcp_pid) and not is_pid_alive(state.tftp_pid):
            break
        time.sleep(0.1)

    netboot_state_path(target).unlink(missing_ok=True)
    _restore_interface(state.interface)


def get_status(target: str) -> dict:
    state = load_netboot_state(target)
    if state is None:
        return {"running": False, "target": target}

    dhcp_alive = is_pid_alive(state.dhcp_pid)
    tftp_alive = is_pid_alive(state.tftp_pid)

    return {
        "running": dhcp_alive and tftp_alive,
        "target": target,
        "dhcp_pid": state.dhcp_pid,
        "dhcp_alive": dhcp_alive,
        "tftp_pid": state.tftp_pid,
        "tftp_alive": tftp_alive,
        "interface": state.interface,
        "tftp_root": state.tftp_root,
        "started_at": state.started_at,
        "uptime_seconds": time.time() - state.started_at if (dhcp_alive and tftp_alive) else None,
    }
````

## File: AGENTS.md
````markdown
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

# Paniolo — Agent Instructions

## Skill

A Repomix-generated reference skill lives at `.claude/skills/paniolo-reference/`.
Load it at the start of any session to get a full structural map of the codebase.

## Before opening a PR

Run through this checklist before calling `gh pr create`:

1. **Update docs that the PR affects.** For each changed subsystem, check:
   - `docs/<subsystem>.md` — commands, config fields, workflows
   - `README.md` — capabilities table, installation steps
   - `AGENTS.md` — module layout, command descriptions, architecture notes
   Include doc updates in the same PR, not a follow-up.

2. **Regenerate the reference skill.** Use the Repomix CLI (`brew install repomix`,
   or `npx -y repomix@latest`) from the repo root:

   ```
   repomix --skill-generate paniolo-reference \
       --skill-output .claude/skills/paniolo-reference --force \
       -i "captures/**,.git/**,.claude/skills/**"
   ```

   `target/` dirs and `ocr/visionocr` are gitignored, so Repomix skips them.
   Excluding `.claude/skills/**` prevents the old skill from being packed into
   the new one. Stage the updated skill files in the same commit.

3. **Open the PR; do not merge it.** Push the branch and create the PR with
   `gh pr create`, then stop. The merge decision belongs to the user.

## Purpose

Paniolo is a CLI tool that lets an AI agent fully control a target machine
during low-level software development (bootloader, firmware, OS bring-up).
"Paniolo" is the Hawaiian word for cowboy — the agent wrangles the target.

Current capabilities:
- DHCP + TFTP netboot over a direct USB-Ethernet link (`paniolo netboot`)
- HDMI/USB capture via hdmicap warm-stream daemon (`paniolo video`)
- Serial console — interactive (tio) or daemon-backed for the web dashboard (`paniolo serial`);
  one daemon owns several named interfaces, each with a timestamped rolling capture
  log queryable by line range (`paniolo serial log -i <name>`)
- Combined video+serial web dashboard (hdmicap's `GET /`: video on top, xterm.js terminal below)
- On-device OCR of the captured screen (`paniolo video read`, dashboard OCR button): Apple Vision on macOS, Tesseract on Linux
- USB HID input (keyboard/mouse injection) via the KB2040 rig (`paniolo hid`)
- Power cycling via DTR (J2 wiring) or a configurable shell script (`paniolo serial dtr`, `paniolo power-cycle`)

## Architecture

**Option A (current):** one daemon per subsystem, controlled via SSH. No
long-running parent process; state lives in JSON + PID files under
`~/.local/share/paniolo/<target>/`. The `paniolo` binary is the only process
that needs to persist in PATH; each subsystem daemon is a backgrounded
subprocess.

**Option B (future):** single long-running server with socket-based RPC,
enabling inter-subsystem coordination (e.g., "stream serial output whenever
a netboot attempt fires"). Will be implemented in Rust when the complexity
of option A is no longer sufficient.

## Module layout

```
src/paniolo/
  _cli.py       typer CLI — subcommand groups: target, netboot, video, serial, hid
                            top-level commands: console, power-cycle, power-state, setup
  _config.py    TargetConfig CRUD (+ named SerialInterface list) (~/.config/paniolo/targets/<name>.toml)
  _state.py     daemon state files (~/.local/share/paniolo/<target>/)
  _netboot.py   dnsmasq + tftp-now subprocess management
  _video.py     VideoConfig, hdmicap device discovery, daemon start/stop/URL helpers
  _serial.py    serial helpers: tio (interactive) + serialcap daemon start/stop/URL
  _ocr.py       OCR tool discovery + read_text(): visionocr (macOS) or linuxocr (Linux)
  _hid.py       HID rig client: text commands over serial, scaling + sequencing
  _power.py     DTR button-press helpers: dtr_button_press() (via serialcap daemon), dtr_direct_button_press() (pyserial fallback)

tests/           pytest suite (host-side; no hardware) — currently test_hid.py

hdmicap/         Rust crate: warm-stream HDMI capture daemon
  src/
    main.rs      CLI subcommands: daemon, devices, shot, watch, preview, stop
    capture.rs   nokhwa-backed capture backend (avfoundation/v4l2)
    capture_thread.rs  std::thread owning device, publishes into watch channel
    frame.rs     FrameState, Signal enum, aHash, is_no_signal
    server.rs    axum HTTP API: GET / (dashboard), /status, /snapshot, /preview,
                 /ocr, /devices, POST /power-cycle, and /xterm.* static assets
    daemon.rs    advisory lock, discovery file, tokio runtime, graceful shutdown
  assets/        index.html (combined dashboard) + vendored xterm.js/css/fit addon
  vendor/
    nokhwa-bindings-macos/  patched: removes frame-duration KVC calls that throw
                            NSException on HDMI capture cards (e.g. MS2109)

serialcap/       Rust crate: serial console daemon (parallels hdmicap)
  src/
    main.rs      CLI subcommands: daemon (--interface NAME=DEV[@BAUD], repeatable),
                 log (-i NAME), devices, stop
    serial_io.rs one supervisor per interface: tokio-serial port owner; reconnect
                 loop; broadcast fan-out to WS clients; mpsc client->port; 64KB
                 scrollback ring; tees every chunk to that interface's capture
                 thread (off the live fan-out path). `Serials` holds the named set
    capture.rs   line assembler: splits bytes into timestamped, sequence-numbered
                 lines; appends them to a rotating on-disk JSONL log under
                 capture/<name>/ (survives restarts; resumes the seq counter);
                 mirrors the current unterminated line to a pending sidecar. Also
                 the `log` reader (interface select; tail / range / since,
                 ANSI-stripped by default) + UTC formatting
    server.rs    axum: GET /stream (bidirectional WebSocket), /status, /interfaces,
                 /devices. Per-interface endpoints take ?interface=NAME, defaulting
                 to the first configured interface
    daemon.rs    advisory lock, discovery file, tokio runtime, graceful shutdown;
                 spawns one supervisor per interface

ocr/             OCR helpers (compiled/installed binaries are gitignored):
                   visionocr.swift  Apple Vision OCR (macOS); built by paniolo setup via swiftc
                   linuxocr         Tesseract OCR wrapper (Linux); copied by paniolo setup

hidrig/          CircuitPython firmware for the USB HID injection rig
  target/code.py   I2C target -> USB HID (KB2040 plugged into the Pi)
  control/code.py  USB serial -> I2C controller (text command parser; owns the
                   wire protocol — source of truth, kept in sync with target)
  control/boot.py  enables the usb_cdc data channel
  host/hid_seize_reports.c  macOS IOKit tool: seizes the HID device exclusively
                   and prints raw input reports — for pipeline testing without
                   keystrokes reaching the focused app. Build with host/Makefile.
  README.md        wiring, command + wire protocol; HANDOFF.md: remaining firmware work
```

## Combined dashboard (video + serial)

hdmicap's `GET /` serves a two-pane page: the MJPEG video on top, an xterm.js
terminal below. The terminal opens a WebSocket to **serialcap** (a separate
daemon/port), so the two subsystems stay decoupled — hdmicap only references
serialcap by URL. Defaults to `ws://<host>:8724/stream`; override with
`?serial=<port>` or `?serialws=<url>`. serialcap sends serial bytes as binary
frames and accepts keystrokes back over the same socket. xterm.js is vendored
(not CDN) so the dashboard works on an isolated lab network. This is the first
concrete instance of the "Option B" inter-subsystem coordination described above.

**Multi-pane serial:** the page fetches `GET /interfaces` from serialcap on
load and calls `buildPanes(names)`. With one interface a single terminal fills
the panel and connection status appears in the top bar. With multiple interfaces
each gets its own `.serial-pane` div (label + status bar + xterm.js terminal),
laid out side by side in bottom mode or stacked in right-panel mode. All fits
are tracked in `allFits[]` so resize and layout-toggle events re-fit every
terminal. `?interface=<name>` bypasses the fetch and opens single-pane mode
pinned to that interface.

**Layout toggle:** a button in the status bar switches the serial panel between
bottom (default, 40 vh) and right-panel (380 px fixed, video fills remaining
width) layouts. The choice is persisted in `localStorage` under the key
`paniolo-serial-layout`.

**Power-cycle button:** an amber "⏻ Power Cycle" button appears in the video
overlay when hdmicap's `POST /power-cycle` endpoint returns non-501. The endpoint
delegates to `paniolo power-cycle <target>` using the `PANIOLO_TARGET` env var set
when the daemon is started with `paniolo video watch <target>`. Clicking the button
shows a confirmation modal before firing. The button is hidden if no target was
passed at daemon start, so it is safe to use on shared dashboards.

## OCR

Two entry points, both feeding the same warm frame:
- **`paniolo video read`** — fetches a snapshot (via `hdmicap shot`) and OCRs it.
- **Dashboard button + hdmicap `GET /ocr`** — the daemon PNG-encodes the current
  frame and pipes it to the OCR tool (`tokio::process`), returning the text. The
  daemon finds the tool via `PANIOLO_VISIONOCR` (the installed path, set by
  `paniolo video watch`), falling back to PATH; if absent, `/ocr` returns 501 and
  the button shows an error.

`_ocr.ocr_binary()` returns the platform-appropriate tool; `paniolo setup`
installs it. `PANIOLO_VISIONOCR` is set to the resolved path when the daemon
starts, so the daemon always uses the installed binary (never a stale PATH hit).

**macOS — `ocr/visionocr.swift`** (`VNRecognizeTextRequest`, Apple Vision):
on-device, no network, no model download. `paniolo setup` compiles it (`swiftc`)
into `~/.cargo/bin`.

Tuning that matters for small console text:
- `recognitionLevel = .fast` is the default, not `.accurate`. `.accurate` is
  tuned for natural document text and returns *nothing* on thin console fonts.
- The tool 2×-upscales and black-pads the frame before recognition (fixes colon
  misreads and first-character clipping at the frame edge).
- `minimumTextHeight` is lowered (it's a fraction of image height; the default
  1/32 skips ~16px console text).

**Linux — `ocr/linuxocr`** (Tesseract via `tesseract-ocr` system package):
`paniolo setup` copies the script to `~/.cargo/bin/linuxocr`. Requires
`sudo apt-get install tesseract-ocr`; Pillow (`pip install Pillow`) is optional
but enables the same 2×-upscale + black-pad preprocessing as visionocr.

**Do not change the target's console font** to try to improve OCR accuracy —
the font is relied upon by other agents (e.g. the Fuchsia bring-up agent that
reads kernel/bootloader output). Character confusions on thin console fonts
(`1`↔`l`↔`I`, IPv6 colons, etc.) are better addressed by increasing capture
resolution or adjusting Tesseract's `--psm` mode.

## _config.py

`TargetConfig` is a `@dataclass` with fields: `name`, `interface`,
`host_ip` (default `192.168.99.1`), `tftp_root` (optional),
`power_cycle_cmd` (optional shell command/script for `paniolo power-cycle`),
`power_serial_interface` (optional — default interface name for DTR commands),
and `serial_interfaces` — a list of `SerialInterface(name, device, baud,
power_sense_signal)`. A target can have several named serial consoles (e.g.
`console`, `bmc`); helpers `serial_interface(name=None)` (resolves by name, or
the sole one when omitted — raising on ambiguity), `upsert_serial_interface()`,
and `remove_serial_interface()` manage them.

Serialized as TOML using a hand-rolled writer (`_to_toml()` + `_toml_kv()`):
scalar fields first, then one `[[serial]]` array-of-tables block per interface
(Python 3.11 `tomllib` reads TOML but does not write it; avoids adding `tomli-w`).
`_from_dict()` reads it back and **migrates** the legacy single-serial fields
(`serial_device`/`serial_baud`) into one interface named `console`, and silently
drops any `ha_power_entity` field from old configs, so older target files keep loading.

Config files live at `~/.config/paniolo/targets/<name>.toml`.

## _state.py

Runtime state for each subsystem daemon. Currently only netboot.

`NetbootState` is a `@dataclass`: `target`, `dnsmasq_pid`, `tftp_pid`,
`started_at` (float epoch), `interface`, `tftp_root`. Stored as JSON at
`~/.local/share/paniolo/<name>/netboot.json`.

`is_pid_alive(pid)` uses `os.kill(pid, 0)`: returns `True` if the process
exists; catches `ProcessLookupError` (dead) and `PermissionError` (exists but
owned by another user — treat as alive).

## _netboot.py

Manages dnsmasq and tftp-now as backgrounded subprocesses.

**`_find_bin(name)`** searches `PATH` via `shutil.which`, then falls back to
`_BREW_PATHS = ["/opt/homebrew/bin", "/usr/local/bin"]`. This is needed
because SSH non-interactive shells often lack Homebrew in PATH even when the
user's interactive shell has it.

**`_dnsmasq_conf(cfg, tftp_root)`** generates the dnsmasq config string.
Key choices:
- `bind-interfaces` is intentionally absent — dnsmasq binds `0.0.0.0:67`,
  which works without root on macOS 10.14+.
- `dhcp-boot=kernel_2712.img,,{host_ip}` sets `siaddr` (BOOTP next-server).
- `dhcp-option=66,{host_ip}` sets DHCP option 66 (TFTP server name). The
  RPi 5 EEPROM reads option 66 preferentially over `siaddr` — set both.
- `log-facility=-` redirects dnsmasq syslog output to stderr → log file.
- `port=0` disables DNS.

**`start(cfg)`** flow: guard → check deps → validate tftp_root →
configure interface (sudo ifconfig) → write dnsmasq config → spawn dnsmasq
→ spawn tftp-now → save state.

**`stop(target)`** sends SIGTERM to both PIDs, waits up to 3 s, removes state.

## _video.py

`VideoConfig` dataclass: `device` only. Saved to `~/.config/paniolo/video.toml`.

`hdmicap_binary()` resolves the *installed* binary — PATH, then `~/.cargo/bin`.
It never points at the in-repo `target/` build tree, so a running daemon can't
reference an ephemeral build artifact that a checkout/cleanup deletes. `paniolo
setup` installs it (`cargo install`).

`list_devices()` runs `hdmicap devices` and parses its text output
(`  <index>  <name>  [<misc>]`) into `[{index, name, misc}]` dicts.

`guess_capture_device(devices)` returns the single non-built-in device (filters
out FaceTime, iSight, iPhone, iPad), or None if ambiguous.

`daemon_url()` reads hdmicap's discovery file (`$TMPDIR/hdmicap/daemon.json`),
verifies the PID is alive, and returns `http://127.0.0.1:<port>` or None.

`start_daemon(cfg, port)` spawns `hdmicap daemon --device <name> --port <port>`
detached (`start_new_session=True`). Caller polls `daemon_url()` to confirm
startup.

## _serial.py

Two paths share this module:

- **Interactive (`paniolo serial connect`):** `tio_binary()` + `connect_cmd()`
  build a `tio` invocation; `_cli.py` `os.execvp`s into it for a foreground
  terminal. Unchanged, dependency-light path.
- **Daemon (`paniolo serial watch`):** `serialcap_binary()` resolves the
  installed binary (PATH then `~/.cargo/bin`, never the build tree, same as
  `hdmicap_binary`), `start_daemon(interfaces, port, buffer_lines=None)` spawns
  one daemon owning *all* the target's interfaces (`daemon_cmd()` builds the argv,
  one repeated `--interface NAME=DEVICE@BAUD` per interface via `interface_arg()`),
  `daemon_url()` reads the discovery file (see Runtime paths) and verifies the PID,
  mirroring `_video.py`. Interfaces come from the target's
  `TargetConfig.serial_interfaces`.

`list_serial_devices()` returns `/dev/serial/by-path/` symlinks on Linux when
available (stable across USB re-enumeration), falling back to raw `/dev/ttyUSB*`
/ `/dev/ttyACM*` paths. On macOS it globs `/dev/tty.usb*`. serialcap itself
enumerates via the cross-platform `serialport` crate (`serialcap devices`), which
gives richer USB VID/PID info.

`canonical_device_path(device)` upgrades a raw `/dev/ttyUSBX` path to its
corresponding `/dev/serial/by-path/` symlink when one exists. `serial setup`
calls this automatically before saving, so the stored config is always stable.

**Captured output (`paniolo serial log`):** `log_cmd()` builds the `serialcap
log` argv; `_cli.py` resolves the binary and execs it as a passthrough. All the
buffering, line assembly, timestamping, and range logic live in Rust
(`serialcap/src/capture.rs`) — the daemon owns the port and is the only thing
that sees every byte, so it persists timestamped lines to an on-disk JSONL log.
`serialcap log` reads that log *directly* (no daemon round-trip), so it works
whether or not the daemon is running. Flags: `--interface/-i NAME` (which
interface; optional when only one was captured), `--tail N`, `--from/--to` (seq
range), `--since` (poll for new lines), `--raw` (keep ANSI), `--json`,
`--no-pending`. Lines carry a monotonic `seq` (stable across eviction, so a
range/`--since` query stays valid) and a UTC `ts_ms`; output is ANSI-stripped by
default. Each interface captures into its own `capture/<name>/` dir, so logs
never conflate. The live WebSocket dashboard view is unchanged — capture is
purely additive and runs on a separate thread so disk I/O can't stall the fan-out.

## _hid.py

Host client for the `hidrig/` USB HID injection rig. `paniolo hid` is a **thin
text-command client**: it sends line commands (`type ...`, `key ENTER`, `move
dx dy`, ...) to the control board's USB CDC *data* port; the board parses them
and relays HID events. The board owns the wire protocol — `_hid.py` does not
re-encode packets host-side.

- `HidConfig(port)` saved to `~/.config/paniolo/hid.toml`; `list_serial_ports()`
  / `guess_data_port()` find the control board's data CDC node (the
  higher-numbered of the two it exposes).
- `HidRig` opens the port (lazy `pyserial` import) and `cmd()`s lines, raising on
  the board's `ERR` reply. Pass `transport=` to drive it without hardware (tests).
- Host-side sequencing (the board stays dumb): `parse_sequence()` (command files
  with `# comments` and `delay <ms>` / `sleep <s>` directives), `run_sequence()`,
  `repeat_key()`, and `scale_to_logical()` (pixel -> 0..32767 for future abs mouse).

`pyserial` is an **optional extra** (`pip install 'paniolo[hid]'` / `uv sync
--extra hid`), imported lazily — the core install stays typer-only and the
test suite needs neither pyserial nor hardware.

## hidrig firmware

The `hidrig/` directory contains CircuitPython 9.x firmware for two RP2040
boards that together form a USB HID injection rig.

### Architecture

```
[test computer]
  |-- USB serial (data CDC) --> [control board: Trinkey QT2040 or KB2040]
                                     |-- STEMMA QT (I2C, 100 kHz) -->
                                                         [target board: KB2040]
                                                              |-- USB HID -->
                                                                     [Pi / DUT]
```

The control board (`control/code.py`) reads line-delimited text commands from
the USB CDC **data** port, encodes them as compact binary I2C packets, and
writes them to the target at address `0x41`. The target board (`target/code.py`)
receives packets over I2C and replays them as USB HID keyboard/mouse events.

### Wire protocol

Each I2C write is `[opcode][payload...]`. Opcodes 0x01–0x04 are keyboard;
0x10–0x13 are mouse. The `TYPE` opcode (0x04) carries UTF-8 text; the control
board chunks it at 30 bytes so the packet never exceeds 31 bytes total.

### I2C FIFO and drain loop

The RP2040 I2C hardware RX FIFO is **16 bytes deep**. For packets larger than
16 bytes, `req.read()` in CircuitPython returns only the bytes currently
buffered — it does not wait for the STOP condition — so calling it once after a
fixed sleep truncates large TYPE packets.

The target uses a drain loop instead of a fixed sleep:

```python
data = bytearray()
while True:
    chunk = req.read(64)
    if not chunk:
        break
    data.extend(chunk)
    time.sleep(0.001)  # let next FIFO batch arrive
handle(bytes(data))
```

The 1 ms inter-read sleep allows the next bytes to clock in (at 100 kHz, 11
bytes arrive per ms). The loop terminates when `req.read()` returns empty bytes
after the STOP condition is received. This approach is correct for any packet
size and any I2C clock rate.

### Handshake

After each I2C write the control board polls a 1-byte I2C read from the target
until the target responds `0x01`. The target sends `0x01` only after `handle()`
returns (i.e., after the HID event has been submitted to the host). This
back-pressure prevents the control board from sending the next packet while the
target is still processing, which previously caused ENTER key flooding (release
packet dropped → key held → auto-repeat) and `[Errno 5]` I/O errors on the I2C
bus.

### Host testing tool (`hidrig/host/`)

`hid_seize_reports.c` is a macOS IOKit utility that opens the target board's
HID interface with `kIOHIDOptionsTypeSeizeDevice`, preventing any keystroke from
reaching the focused application. It registers an input report callback and
prints hex dumps of every keyboard and mouse report. Use it to verify the full
pipeline end-to-end without the Pi:

```bash
cd hidrig/host && make
sudo ./hid_seize_reports   # grant Input Monitoring in System Settings first
```

Run `paniolo hid type/key/move/click/scroll` in a second terminal and read the
reports. The tool prints the 156-byte report descriptor on first device match,
so you can verify the HID descriptor matches expectations.

VID/PID are 0x239A/0x8106 (KB2040 running CircuitPython). The built binary is
gitignored; re-run `make` after cloning.

### Negative number arguments (`move`, `scroll`)

Click's tokenizer treats any token starting with `-` as a potential option flag.
`paniolo hid move` and `paniolo hid scroll` use
`context_settings={"ignore_unknown_options": True}` and accept `dx`/`dy`/`amount`
as `str` arguments (cast to `int` internally) so that `paniolo hid move 50 -30`
and `paniolo hid scroll -3` work without the `--` separator.

## _power.py

Two functions; no new dependencies beyond stdlib `urllib.request` and an optional
lazy `pyserial` import:

`dtr_button_press(daemon_url, interface_name, duration_ms)` — POSTs to the
serialcap daemon's `/button?interface=<name>&ms=<N>` endpoint. Blocks until the
press completes. Raises `RuntimeError` on HTTP error, `OSError` on network
failure.

`dtr_direct_button_press(device, duration_ms)` — pyserial fallback for when the
daemon is not running. Opens the port, asserts DTR for the given duration, then
releases. Raises `RuntimeError` if pyserial is not installed.

## _cli.py

Built with [Typer](https://typer.tiangolo.com/) (rich output included).

**`_resolve(name)`** applies the default-target rule: if `name` is None and
exactly one target is configured, use it; otherwise require an explicit name.

Subcommand groups:
- `target_app` (`paniolo target`) — `set`, `show`, `clear`
- `netboot_app` (`paniolo netboot`) — `start`, `stop`, `status`, `tftp-root`,
  `logs` (Rich viewer; `--boot` for current session, `--dhcp`/`--tftp`/`--errors`
  to filter, `--tail N`, `-f` to follow), `link-up`, `link-down`, `link-status`
- `video_app` (`paniolo video`) — `setup`, `watch [TARGET]` (optional target enables
  the dashboard power-cycle button via `PANIOLO_TARGET`), `preview`, `shot`,
  `read` (OCR), `devices`, `show`, `stop`
- `serial_app` (`paniolo serial`) — `setup` (`--name`), `remove`, `connect` (tio, `-i`),
  `watch`/`stop` (serialcap daemon, all interfaces), `log` (captured output, `-i`),
  `devices`, `show`, `dtr` (`--ms`, `-i` — pulse DTR on any interface), `reset` (`--ms`, `-i`)
- `hid_app` (`paniolo hid`) — `setup`, `type`, `key`, `releaseall`, `combo`, `down`, `up`, `click`, `mdown`, `mup`, `move`, `scroll`, `run <file>`, `show`

Top-level commands:
- `paniolo console [TARGET] [-i INTERFACE]` — open the combined video+serial dashboard;
  starts daemons if needed (using TARGET for power-cycle wiring), opens the hdmicap URL
- `paniolo power-cycle [TARGET]` — runs `cfg.power_cycle_cmd` via `subprocess.run(..., shell=True)`
- `paniolo power-state [TARGET]` — reads power state from the serialcap daemon `/status` endpoint (requires sense signal wired)
- `paniolo setup` — installs tftp-now (Homebrew) and builds/installs paniolo's
  own binaries: hdmicap + serialcap (`cargo install`), visionocr (`swiftc`,
  macOS only), and linuxocr (copied script, Linux only) — all into `~/.cargo/bin`

## Runtime paths

| Purpose | Path |
|---|---|
| Target configs | `~/.config/paniolo/targets/<name>.toml` |
| Video config | `~/.config/paniolo/video.toml` |
| Netboot daemon state | `~/.local/share/paniolo/<name>/netboot.json` |
| Generated dnsmasq config | `~/.local/share/paniolo/<name>/dnsmasq.conf` |
| Combined netboot log | `~/.local/share/paniolo/<name>/netboot.log` |
| hdmicap discovery file | `$XDG_RUNTIME_DIR/hdmicap/daemon.json` (`{pid, port}`) — falls back to `$TMPDIR` |
| hdmicap advisory lock | `$XDG_RUNTIME_DIR/hdmicap/daemon.lock` |
| serialcap discovery file | `$XDG_RUNTIME_DIR/serialcap/daemon.json` (`{pid, port, interfaces:[{name, device, baud}]}`) — falls back to `$TMPDIR` |
| serialcap advisory lock | `$XDG_RUNTIME_DIR/serialcap/daemon.lock` |
| serialcap capture log | `$XDG_RUNTIME_DIR/serialcap/capture/<name>/serial.jsonl(.1..)` (rotated JSONL, per interface) |
| serialcap pending line | `$XDG_RUNTIME_DIR/serialcap/capture/<name>/pending.json` (current unterminated line) |

## Source code constraints

- **No hardcoded network addresses, URLs, or hostnames.** All site-specific
  values go in config files under `~/.config/paniolo/` and are populated via
  setup commands. Error messages must be generic.
- **No new dependencies without discussion.** Core dep: `typer` only; stdlib for
  everything else (`urllib.request`, `tomllib`, `subprocess`). `pyserial` is an
  optional extra (`[hid]`), lazy-imported, used only by `paniolo hid`. Dev: `pytest`.

## Remote control pattern

```bash
ssh control-mac "paniolo target set target-machine --interface en3 --tftp-root ~/pxe \
  --power-cycle-cmd /Users/you/.config/paniolo/scripts/power-cycle-target-machine.sh"
ssh control-mac "paniolo netboot start target-machine"
TFTP_ROOT=$(ssh control-mac "paniolo netboot tftp-root target-machine")
scp kernel.img control-mac:"${TFTP_ROOT}/kernel_2712.img"
ssh control-mac "paniolo netboot logs -f target-machine"
op run --env-file .env -- ssh control-mac "paniolo power-cycle target-machine"
ssh control-mac "paniolo netboot stop target-machine"
```

## Adding a new subsystem

1. Create `src/paniolo/_<subsystem>.py`.
2. Add state dataclass + path helpers to `_state.py` if the subsystem is a
   daemon with a PID.
3. Add a `<subsystem>_app = typer.Typer(...)` group in `_cli.py`.
4. Add optional config fields to `TargetConfig` in `_config.py`.
5. Regenerate the skill and update this file.

## Linux support

Paniolo runs on Linux as well as macOS. Platform differences:

- **OCR backend is platform-specific.** macOS uses Apple Vision (`visionocr.swift`,
  compiled by `paniolo setup`). Linux uses Tesseract (`ocr/linuxocr`, copied by
  `paniolo setup`; requires `tesseract-ocr` system package). Both expose the same
  stdin-PNG → stdout-text interface via `PANIOLO_VISIONOCR`.
- **Netboot uses `sudo` internally on Linux.** DHCP (port 67) and TFTP (port 69)
  require root on Linux; macOS 14+ allows them rootless. `paniolo netboot start`
  auto-prepends `sudo env PYTHONUNBUFFERED=1 <python>` when spawning the two
  server subprocesses on Linux. With passwordless sudoers this is transparent;
  otherwise sudo prompts for a password. Interface config (`ip addr add`) also
  uses sudo, same as macOS uses it for `ifconfig`.
- **Interface management uses `ip` on Linux.** `_configure_interface()` runs
  `ip addr add`/`ip link set up` (iproute2) instead of `networksetup`+`ifconfig`.
  `_restore_interface()` flushes with `ip addr flush dev <iface>`.
- **ARP pinning uses `ip neigh replace` on Linux.** `_dhcp._set_arp()` calls
  `arp -s` on macOS and `ip neigh replace ... nud permanent` on Linux.
- **BPF raw-frame sender is macOS-only.** `BpfSender` in `_tftp.py` uses
  `/dev/bpf*` ioctls that don't exist on Linux. On Linux `available` is always
  `False` and the server falls back to normal `sendto()` with retry.
- **hdmicap build deps on Linux.** Building hdmicap requires system packages:
  `build-essential pkg-config libclang-dev clang` (for V4L2 bindgen via
  `v4l2-sys-mit`). `paniolo setup` prints a reminder.
- **Interface listing uses sysfs on Linux.** `list_usb_ethernet_interfaces()`
  reads `/sys/class/net/` (type, carrier) instead of `networksetup`.
- **Serial device paths use by-path symlinks on Linux.** `list_serial_devices()`
  returns `/dev/serial/by-path/` entries when available; `canonical_device_path()`
  upgrades a raw `ttyUSBX` path to its stable symlink. Store by-path paths in
  target configs so serial interfaces survive USB adapter re-enumeration. The
  serialcap `--interface` parser accepts by-path paths (colons in the path are
  not confused with the optional `:SENSE` suffix because only known signal names
  `cts`, `dsr`, `dcd`, `ri` are treated as the sense suffix).

## Known limitations / gotchas

- **Interface configuration requires root.** `_configure_interface()` needs
  NOPASSWD sudo (`ifconfig`/`networksetup` on macOS, `ip` on Linux).
- **SSH PATH.** Non-interactive SSH shells often lack `/opt/homebrew/bin`.
  `_find_bin()` probes `_BREW_PATHS` on macOS and `/usr/sbin`+`/sbin` on Linux.
- **hdmicap device auto-detection.** With two non-built-in cameras (e.g. MS2109
  + Razer Kiyo), `guess_capture_device` returns None and the user is prompted.
  Pass `--device "USB Video"` (or whatever substring matches) to skip the prompt.
- **nokhwa MS2109 compatibility.** The MS2109 HDMI capture card doesn't expose
  standard MJPEG/YUYV formats through nokhwa's filtered list and throws
  NSException from AVFoundation frame-duration KVC calls. The vendor patch in
  `hdmicap/vendor/nokhwa-bindings-macos/` fixes this.
- **Daemon shutdown hard-exits.** Both hdmicap (`/preview` MJPEG) and serialcap
  (`/stream` WebSocket) serve infinite responses, so a plain axum graceful
  shutdown would block on them forever. On SIGTERM each daemon removes its
  discovery file, gives a 300 ms grace, then `std::process::exit(0)`. The OS
  releases the capture device / serial port on exit.
- **Serial ports are exclusive.** Only one of `tio`/`screen`/serialcap can hold
  a port at a time. `paniolo serial watch` and `paniolo serial connect` conflict
  on the same device — use one or the other.
- **macOS serialport can't open PTYs.** The `serialport` crate sets baud via the
  `IOSSIOSPEED` ioctl, which returns ENOTTY ("Not a typewriter") on pseudo-
  terminals. serialcap byte-flow can only be tested against a real serial device,
  not a `pty.openpty()` pair.
- **OCR character confusions on small console fonts.** Both visionocr and linuxocr
  2×-upscale and black-pad before recognition, but thin terminal fonts still
  produce confusions (`1`↔`l`↔`I`, `2`↔`Z`, colon spacing in IPv6). Accuracy
  improves markedly on larger boot-screen text. Do not change the target's console
  font to work around this — the font is relied upon by other agents (see OCR section).
  On macOS, `VNRecognizeTextRequest` `.accurate` returns nothing on thin console
  fonts; visionocr uses `.fast`.
````

## File: src/paniolo/_video.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Video capture helpers — delegates to the hdmicap daemon."""

from __future__ import annotations

import dataclasses
import json
import os
import re
import shutil
import subprocess
import tempfile
import tomllib
from pathlib import Path
from typing import Optional

from . import _config
from ._config import _toml_kv

VIDEO_CONFIG_PATH = _config.CONFIG_DIR / "video.toml"

_BUILTIN_NAMES = ("FaceTime", "Capture screen", "iSight", "iPhone", "iPad")


@dataclasses.dataclass
class VideoConfig:
    """Saved configuration for the HDMI/USB capture device."""

    device: str


def _to_toml(data: dict) -> str:
    lines = [_toml_kv(k, v) for k, v in data.items() if v is not None]
    return "\n".join(lines) + "\n"


def save_video_config(cfg: VideoConfig) -> None:
    _config.CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    VIDEO_CONFIG_PATH.write_text(_to_toml(dataclasses.asdict(cfg)))


def load_video_config() -> Optional[VideoConfig]:
    if not VIDEO_CONFIG_PATH.exists():
        return None
    with open(VIDEO_CONFIG_PATH, "rb") as f:
        data = tomllib.load(f)
    return VideoConfig(device=data["device"])


def hdmicap_binary() -> Optional[str]:
    """Return the installed hdmicap path: PATH, then ~/.cargo/bin. None if absent.

    Installed by `paniolo setup` (cargo install). Never resolved from the in-repo
    build tree, so a running daemon can't point at an ephemeral build artifact.
    """
    found = shutil.which("hdmicap")
    if found:
        return found
    cargo_bin = Path.home() / ".cargo" / "bin" / "hdmicap"
    return str(cargo_bin) if cargo_bin.exists() else None


_DEVICE_RE = re.compile(r"^\s*(\d+)\s+(.+?)\s+\[([^\]]*)\]")


def list_devices() -> list[dict]:
    """Return [{index, name, misc}, ...] via `hdmicap devices`."""
    binary = hdmicap_binary()
    if not binary:
        return []
    try:
        result = subprocess.run(
            [binary, "devices"],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            return []
        devices = []
        for line in result.stdout.splitlines():
            m = _DEVICE_RE.match(line)
            if m:
                devices.append({"index": int(m.group(1)), "name": m.group(2), "misc": m.group(3)})
        return devices
    except FileNotFoundError:
        return []


def guess_capture_device(devices: list[dict]) -> Optional[dict]:
    """Return the one non-built-in device, or None if ambiguous."""
    candidates = [d for d in devices if not any(s in d["name"] for s in _BUILTIN_NAMES)]
    return candidates[0] if len(candidates) == 1 else None


def _discovery_path() -> Path:
    """Path where hdmicap writes its daemon.json discovery file.

    Mirrors hdmicap/src/daemon.rs::runtime_dir(): prefer $XDG_RUNTIME_DIR
    (set by systemd on Linux), fall back to tempfile.gettempdir().
    """
    base = os.environ.get("XDG_RUNTIME_DIR") or tempfile.gettempdir()
    return Path(base) / "hdmicap" / "daemon.json"


def read_discovery() -> Optional[dict]:
    """Read hdmicap's discovery file, returning {pid, port} or None."""
    path = _discovery_path()
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return None


def daemon_url() -> Optional[str]:
    """Return the base URL of the running daemon, or None if not running."""
    disc = read_discovery()
    if disc is None:
        return None
    try:
        os.kill(int(disc["pid"]), 0)
    except (ProcessLookupError, PermissionError, KeyError):
        return None
    return f"http://127.0.0.1:{disc['port']}"


def start_daemon(
    cfg: VideoConfig,
    port: int = 8723,
    ocr_bin: Optional[str] = None,
    target_name: Optional[str] = None,
) -> subprocess.Popen:
    """Start hdmicap daemon in the background; caller should poll daemon_url().

    ocr_bin is exported as PANIOLO_VISIONOCR for the /ocr endpoint.
    target_name is exported as PANIOLO_TARGET so the /power-cycle endpoint
    can call `paniolo power-cycle <target>`.
    """
    binary = hdmicap_binary()
    if not binary:
        raise FileNotFoundError("hdmicap not found in PATH or project build dir")
    env = dict(os.environ)
    if ocr_bin:
        env["PANIOLO_VISIONOCR"] = ocr_bin
    if target_name:
        env["PANIOLO_TARGET"] = target_name
    return subprocess.Popen(
        [binary, "daemon", "--device", cfg.device, "--port", str(port)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
        env=env,
    )


def stop_daemon() -> bool:
    """Ask the running hdmicap daemon to stop. Returns True if it was running."""
    binary = hdmicap_binary()
    if not binary:
        return False
    result = subprocess.run([binary, "stop"], check=False,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return result.returncode == 0
````

## File: src/paniolo/_cli.py
````python
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

from __future__ import annotations

import grp
import os
import pwd
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Annotated, Optional

import typer
from rich.console import Console
from rich.table import Table

from . import _config, _hid, _netboot, _ocr, _power, _serial, _state, _video

app = typer.Typer(help="Paniolo — agent-controlled target machine wrangler.", no_args_is_help=True)
target_app = typer.Typer(help="Manage target configurations.", no_args_is_help=True)
netboot_app = typer.Typer(help="Control DHCP+TFTP netboot for a target.", no_args_is_help=True)
video_app = typer.Typer(help="Capture screen frames via HDMI/USB capture device.", no_args_is_help=True)
serial_app = typer.Typer(help="Manage serial console connection to a target.", no_args_is_help=True)
hid_app = typer.Typer(help="Inject USB keyboard/mouse input via the HID rig.", no_args_is_help=True)
app.add_typer(target_app, name="target")
app.add_typer(netboot_app, name="netboot")
app.add_typer(video_app, name="video")
app.add_typer(serial_app, name="serial")
app.add_typer(hid_app, name="hid")

console = Console()
err = Console(stderr=True)


def _resolve(name: Optional[str]) -> _config.TargetConfig:
    if name is None:
        targets = _config.list_targets()
        if len(targets) == 1:
            name = targets[0]
        elif not targets:
            err.print("[red]No targets configured.[/red] Run: paniolo target set <name> --interface <iface>")
            raise typer.Exit(1)
        else:
            err.print(f"[red]Multiple targets ({', '.join(targets)}) — specify one.[/red]")
            raise typer.Exit(1)
    try:
        return _config.load_target(name)
    except FileNotFoundError:
        err.print(f"[red]Target '{name}' not found.[/red]")
        raise typer.Exit(1)


# ── target ────────────────────────────────────────────────────────────────────


@target_app.command("set")
def target_set(
    name: Annotated[str, typer.Argument(help="Target name (e.g. fortune)")],
    interface: Annotated[
        Optional[str],
        typer.Option("--interface", "-i", help="USB-Ethernet interface (e.g. en3); auto-detected if omitted"),
    ] = None,
    tftp_root: Annotated[Optional[str], typer.Option("--tftp-root", "-r", help="Path to TFTP files directory")] = None,
    host_ip: Annotated[str, typer.Option("--host-ip", help="Static IP to assign to the interface")] = "192.168.99.1",
    power_cycle_cmd: Annotated[
        Optional[str],
        typer.Option("--power-cycle-cmd", help="Shell command or script path to power-cycle the target"),
    ] = None,
    power_serial: Annotated[
        Optional[str],
        typer.Option(
            "--power-serial",
            help="Serial interface name used for DTR power cycling via J2 (e.g. console)",
        ),
    ] = None,
) -> None:
    """Create or update a target configuration.

    Serial consoles are managed separately with `paniolo serial setup` (a target
    can have several named interfaces), so they're preserved across updates here."""
    if interface is None:
        candidates = _netboot.list_usb_ethernet_interfaces()
        if not candidates:
            err.print("[red]No USB-Ethernet interfaces found.[/red] Specify one with --interface.")
            raise typer.Exit(1)
        active = [c for c in candidates if c["active"]]
        if len(active) == 1:
            interface = active[0]["device"]
            console.print(f"[dim]Auto-detected interface:[/dim] {interface} ({active[0]['port']})")
        elif len(candidates) == 1:
            interface = candidates[0]["device"]
            console.print(
                f"[dim]Auto-detected interface:[/dim] {interface} ({candidates[0]['port']}) "
                "[dim](no cable detected)[/dim]"
            )
        else:
            console.print("[yellow]Multiple USB-Ethernet interfaces found — use --interface to choose:[/yellow]")
            for c in candidates:
                status = "[green]active[/green]" if c["active"] else "[dim]inactive[/dim]"
                console.print(f"  {c['device']:6s}  {c['port']}  {status}")
            raise typer.Exit(1)

    try:
        existing = _config.load_target(name)
    except FileNotFoundError:
        existing = None

    cfg = _config.TargetConfig(
        name=name,
        interface=interface,
        host_ip=host_ip,
        tftp_root=tftp_root,
        power_cycle_cmd=(
            power_cycle_cmd if power_cycle_cmd is not None
            else (existing.power_cycle_cmd if existing else None)
        ),
        power_serial_interface=(
            power_serial if power_serial is not None
            else (existing.power_serial_interface if existing else None)
        ),
        serial_interfaces=existing.serial_interfaces if existing else [],
    )
    _config.save_target(cfg)
    console.print(f"[green]Target '[bold]{name}[/bold]' saved.[/green]")
    console.print(f"  interface   : {interface}")
    console.print(f"  host_ip     : {host_ip}")
    if tftp_root:
        console.print(f"  tftp_root   : {tftp_root}")
    if cfg.power_cycle_cmd:
        console.print(f"  power_cycle : {cfg.power_cycle_cmd}")
    if cfg.power_serial_interface:
        console.print(f"  power_serial: {cfg.power_serial_interface}")
    for iface in cfg.serial_interfaces:
        console.print(f"  serial      : {iface.name}: {iface.device} @ {iface.baud}")


@target_app.command("show")
def target_show(
    name: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Show target configuration(s)."""
    names = [name] if name else _config.list_targets()
    if not names:
        console.print("No targets configured.")
        return
    for tname in names:
        try:
            cfg = _config.load_target(tname)
        except FileNotFoundError:
            err.print(f"[red]Target '{tname}' not found.[/red]")
            continue
        t = Table(title=f"Target: {cfg.name}", show_header=False, box=None, padding=(0, 2))
        t.add_row("interface", cfg.interface)
        t.add_row("host_ip", cfg.host_ip)
        t.add_row("tftp_root", cfg.tftp_root or "[dim]not set[/dim]")
        t.add_row("power_cycle_cmd", cfg.power_cycle_cmd or "[dim]not set[/dim]")
        t.add_row("power_serial", cfg.power_serial_interface or "[dim]not set[/dim]")
        if cfg.serial_interfaces:
            for idx, iface in enumerate(cfg.serial_interfaces):
                t.add_row("serial" if idx == 0 else "", f"{iface.name}: {iface.device} @ {iface.baud}")
        else:
            t.add_row("serial", "[dim]not set[/dim]")
        console.print(t)


@target_app.command("clear")
def target_clear(
    name: Annotated[str, typer.Argument()],
) -> None:
    """Remove a target configuration."""
    path = _config.target_path(name)
    if not path.exists():
        err.print(f"[red]Target '{name}' not found.[/red]")
        raise typer.Exit(1)
    path.unlink()
    console.print(f"Target '[bold]{name}[/bold]' cleared.")


# ── netboot ───────────────────────────────────────────────────────────────────


@netboot_app.command("start")
def netboot_start(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Start DHCP + TFTP netboot for a target."""
    cfg = _resolve(target)
    try:
        _netboot.start(cfg)
    except RuntimeError as exc:
        err.print(f"[red]Error:[/red] {exc}")
        raise typer.Exit(1)

    time.sleep(0.5)
    s = _netboot.get_status(cfg.name)
    if s["running"]:
        console.print(f"[green]Netboot started[/green] for [bold]{cfg.name}[/bold]")
        console.print(f"  DHCP+TFTP  {cfg.interface}  ({cfg.host_ip}/24)")
        console.print(f"  tftp_root  {s['tftp_root']}")
        console.print(f"  log        {_state.netboot_log_path(cfg.name)}")
    else:
        err.print("[red]Failed to start — check log:[/red]")
        err.print(f"  {_state.netboot_log_path(cfg.name)}")
        raise typer.Exit(1)


@netboot_app.command("stop")
def netboot_stop(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Stop netboot for a target."""
    cfg = _resolve(target)
    if not _state.is_netboot_running(cfg.name):
        console.print(f"Netboot is not running for '{cfg.name}'.")
        return
    try:
        _netboot.stop(cfg.name)
        console.print("[green]Stopped.[/green]")
    except RuntimeError as exc:
        err.print(f"[red]Error:[/red] {exc}")
        raise typer.Exit(1)


@netboot_app.command("status")
def netboot_status(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Show netboot status for a target."""
    cfg = _resolve(target)
    s = _netboot.get_status(cfg.name)

    if not s["running"]:
        console.print(f"Netboot: [red]stopped[/red]  (target: {cfg.name})")
        return

    uptime = int(s.get("uptime_seconds") or 0)
    h, rem = divmod(uptime, 3600)
    m, sec = divmod(rem, 60)

    t = Table(show_header=False, box=None, padding=(0, 2))
    t.add_row("target", cfg.name)
    t.add_row("status", "[green]running[/green]")
    t.add_row("interface", s["interface"])
    t.add_row("dhcp", f"pid {s['dhcp_pid']}  {'[green]alive[/green]' if s['dhcp_alive'] else '[red]dead[/red]'}")
    t.add_row("tftp-now", f"pid {s['tftp_pid']}  {'[green]alive[/green]' if s['tftp_alive'] else '[red]dead[/red]'}")
    t.add_row("tftp_root", s["tftp_root"])
    t.add_row("uptime", f"{h:02d}:{m:02d}:{sec:02d}")
    console.print(t)


@netboot_app.command("tftp-root")
def netboot_tftp_root(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Print the TFTP root path (bare, for shell command substitution via SSH)."""
    cfg = _resolve(target)
    state = _state.load_netboot_state(cfg.name)
    if state:
        print(state.tftp_root)
    elif cfg.tftp_root:
        print(cfg.tftp_root)
    else:
        err.print("[red]No tftp_root configured for this target.[/red]")
        raise typer.Exit(1)


def _netboot_format_line(line: str) -> Optional[str]:
    """Return a Rich markup string for one log line, or None to skip blank lines."""
    line = line.rstrip()
    if not line:
        return None
    parts = line.split(" ", 3)
    if len(parts) < 4:
        return line
    ts = f"[dim]{parts[0]} {parts[1]}[/dim]"
    level, msg = parts[2], parts[3]
    if level == "WARNING":
        return f"{ts} [yellow]{msg}[/yellow]"
    if level == "ERROR" or level == "CRITICAL":
        return f"{ts} [red bold]{msg}[/red bold]"
    if any(k in msg for k in ("DHCP", "dhcp")):
        return f"{ts} [cyan]{msg}[/cyan]"
    if msg.startswith("completed "):
        return f"{ts} [green]{msg}[/green]"
    if "NOT FOUND" in msg:
        return f"{ts} [dim yellow]{msg}[/dim yellow]"
    if msg.startswith("RRQ ") or msg.startswith("TFTP "):
        return f"{ts} [blue]{msg}[/blue]"
    return f"{ts} {msg}"


def _netboot_line_passes(line: str, dhcp: bool, tftp: bool, errors: bool) -> bool:
    if not (dhcp or tftp or errors):
        return True
    parts = line.split(" ", 3)
    level = parts[2] if len(parts) >= 3 else ""
    msg = parts[3] if len(parts) >= 4 else line
    if errors and level in ("WARNING", "ERROR", "CRITICAL"):
        return True
    if dhcp and any(k in msg for k in ("DHCP", "dhcp")):
        return True
    if tftp and any(k in msg for k in ("RRQ", "completed", "TFTP", "NOT FOUND", "OACK")):
        return True
    return False


@netboot_app.command("logs")
def netboot_logs(
    target: Annotated[Optional[str], typer.Argument()] = None,
    follow: Annotated[bool, typer.Option("--follow", "-f", help="Stream new lines as they arrive")] = False,
    tail: Annotated[int, typer.Option("--tail", "-n", help="Number of recent lines to show")] = 100,
    boot: Annotated[bool, typer.Option("--boot", help="Show only the current boot session")] = False,
    dhcp: Annotated[bool, typer.Option("--dhcp", help="Show only DHCP events")] = False,
    tftp: Annotated[bool, typer.Option("--tftp", help="Show only TFTP events")] = False,
    errors: Annotated[bool, typer.Option("--errors", "-e", help="Show only warnings and errors")] = False,
) -> None:
    """Show DHCP/TFTP netboot logs with color-coded DHCP and TFTP events.

    Use --dhcp / --tftp / --errors to filter. --boot shows only the current
    session (from the last 'netboot start'). --follow streams live output."""
    cfg = _resolve(target)
    log_path = _state.netboot_log_path(cfg.name)
    if not log_path.exists():
        console.print("No log file yet.")
        return

    lines = log_path.read_text(errors="replace").splitlines()

    if boot:
        # Find the last session start (last "DHCP listening" line).
        start = 0
        for i, ln in enumerate(lines):
            if "DHCP listening on" in ln:
                start = i
        lines = lines[start:]
    else:
        lines = lines[-tail:]

    for ln in lines:
        if _netboot_line_passes(ln, dhcp, tftp, errors):
            formatted = _netboot_format_line(ln)
            if formatted:
                console.print(formatted, highlight=False)

    if not follow:
        return

    with log_path.open(errors="replace") as f:
        f.seek(0, 2)  # seek to end
        while True:
            ln = f.readline()
            if ln:
                if _netboot_line_passes(ln, dhcp, tftp, errors):
                    formatted = _netboot_format_line(ln)
                    if formatted:
                        console.print(formatted, highlight=False)
            else:
                try:
                    time.sleep(0.2)
                except KeyboardInterrupt:
                    break


def _link_state(interface: str) -> dict:
    """Read raw link state for an interface from sysfs (Linux) or ifconfig (macOS)."""
    if sys.platform == "darwin":
        result = subprocess.run(
            ["ifconfig", interface], capture_output=True, text=True
        )
        output = result.stdout
        up = "status: active" in output
        addrs = [
            line.strip().split()[1]
            for line in output.splitlines()
            if line.strip().startswith("inet ")
        ]
        return {"up": up, "carrier": up, "addrs": addrs}
    sysfs = Path(f"/sys/class/net/{interface}")
    operstate = (sysfs / "operstate").read_text().strip() if (sysfs / "operstate").exists() else "unknown"
    try:
        carrier = int((sysfs / "carrier").read_text().strip()) == 1
    except (OSError, ValueError):
        carrier = False
    result = subprocess.run(
        ["ip", "-brief", "addr", "show", "dev", interface],
        capture_output=True, text=True,
    )
    addrs = result.stdout.split()[2:] if result.returncode == 0 else []
    return {"up": operstate in ("up", "unknown"), "carrier": carrier, "addrs": addrs, "operstate": operstate}


@netboot_app.command("link-up")
def netboot_link_up(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Bring the target's USB-Ethernet link up and assign the host IP."""
    cfg = _resolve(target)
    try:
        _netboot._configure_interface(cfg.interface, cfg.host_ip)
    except RuntimeError as exc:
        err.print(f"[red]{exc}[/red]")
        raise typer.Exit(1)
    state = _link_state(cfg.interface)
    status = "[green]up[/green]" if state["up"] else "[yellow]not yet up[/yellow]"
    console.print(f"Link {status}  {cfg.interface}  {cfg.host_ip}")


@netboot_app.command("link-down")
def netboot_link_down(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Take the target's USB-Ethernet link down and release the host IP."""
    cfg = _resolve(target)
    _netboot._restore_interface(cfg.interface)
    console.print(f"Link down  {cfg.interface}")


@netboot_app.command("link-status")
def netboot_link_status(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Show the current state of the target's USB-Ethernet link."""
    cfg = _resolve(target)
    state = _link_state(cfg.interface)
    t = Table(show_header=False, box=None, padding=(0, 2))
    t.add_row("interface", cfg.interface)
    if sys.platform != "darwin":
        t.add_row("operstate", state.get("operstate", "unknown"))
    carrier_str = "[green]yes[/green]" if state["carrier"] else "[red]no[/red]"
    t.add_row("carrier", carrier_str)
    t.add_row("link", "[green]up[/green]" if state["up"] else "[red]down[/red]")
    t.add_row("addresses", " ".join(state["addrs"]) if state["addrs"] else "(none)")
    console.print(t)


# ── video ─────────────────────────────────────────────────────────────────────


@video_app.command("setup")
def video_setup(
    device: Annotated[Optional[str], typer.Option("--device", help="Device name or substring (auto-detected if omitted)")] = None,
) -> None:
    """Discover and save the HDMI/USB capture device configuration."""
    if device is None:
        devices = _video.list_devices()
        if not devices:
            err.print("[red]No capture devices found.[/red] Is hdmicap installed?")
            raise typer.Exit(1)

        auto = _video.guess_capture_device(devices)
        if auto:
            device = auto["name"]
            console.print(f"[dim]Auto-detected capture device:[/dim] {device}")
        else:
            console.print("Available video devices:")
            for d in devices:
                console.print(f"  [{d['index']}] {d['name']}")
            choice = typer.prompt("Enter device name or index")
            if choice.isdigit():
                matches = [d for d in devices if d["index"] == int(choice)]
                if not matches:
                    err.print(f"[red]No device with index {choice}.[/red]")
                    raise typer.Exit(1)
                device = matches[0]["name"]
            else:
                device = choice

    cfg = _video.VideoConfig(device=device)
    _video.save_video_config(cfg)
    console.print(f"[green]Video device configured:[/green] {device}")


def _start_video_daemon(
    cfg: "_video.VideoConfig", port: int, target_name: Optional[str] = None
) -> str:
    """Start the hdmicap daemon and wait for it to come up. Returns the URL.

    target_name is passed as PANIOLO_TARGET so the /power-cycle endpoint can
    call `paniolo power-cycle <target>`. Raises typer.Exit on failure."""
    binary = _video.hdmicap_binary()
    if not binary:
        err.print("[red]hdmicap not found.[/red] Run: paniolo setup")
        raise typer.Exit(1)
    ocr_bin = _ocr.ocr_binary()
    _video.start_daemon(cfg, port, ocr_bin=ocr_bin, target_name=target_name)
    for _ in range(50):
        time.sleep(0.1)
        url = _video.daemon_url()
        if url:
            return url
    err.print("[red]Video daemon did not start within 5 s.[/red]")
    raise typer.Exit(1)


@video_app.command("watch")
def video_watch(
    target: Annotated[Optional[str], typer.Argument()] = None,
    port: Annotated[int, typer.Option("--port")] = 8723,
    restart: Annotated[bool, typer.Option("--restart")] = False,
) -> None:
    """Start the hdmicap daemon in the background.

    Pass a target name to enable the dashboard power-cycle button.
    Use --restart to force-restart a running (but possibly stalled) daemon."""
    cfg = _video.load_video_config()
    if not cfg:
        err.print("[red]No video device configured.[/red] Run: paniolo video setup")
        raise typer.Exit(1)

    target_name = _resolve(target).name if target else None

    url = _video.daemon_url()
    if url and not restart:
        console.print(f"[dim]Daemon already running at[/dim] {url}")
        return
    if url and restart:
        _video.stop_daemon()
        time.sleep(1)

    console.print("[dim]Starting video daemon…[/dim]")
    url = _start_video_daemon(cfg, port, target_name=target_name)
    console.print(f"[green]Daemon started.[/green] Preview at {url}")


@video_app.command("preview")
def video_preview() -> None:
    """Open the live preview page in the default browser."""
    url = _video.daemon_url()
    if not url:
        err.print("[red]No daemon running.[/red] Start one with: paniolo video watch")
        raise typer.Exit(1)

    import webbrowser

    webbrowser.open(url)
    console.print(f"Opened {url}")


@video_app.command("shot")
def video_shot(
    stable: Annotated[bool, typer.Option("--stable")] = False,
    changed_since: Annotated[Optional[str], typer.Option("--changed-since")] = None,
    timeout: Annotated[int, typer.Option("--timeout")] = 2000,
    out: Annotated[str, typer.Option("--out", "-o")] = "-",
) -> None:
    """Fetch one PNG screenshot from the running daemon."""
    binary = _video.hdmicap_binary()
    if not binary:
        err.print("[red]hdmicap not found.[/red]")
        raise typer.Exit(1)

    cmd = [binary, "shot", "--timeout", str(timeout), "--out", out]
    if stable:
        cmd.append("--stable")
    if changed_since:
        cmd.extend(["--changed-since", changed_since])

    result = subprocess.run(cmd, check=False)
    raise typer.Exit(result.returncode)


@video_app.command("read")
def video_read(
    stable: Annotated[bool, typer.Option("--stable")] = False,
    fast: Annotated[bool, typer.Option("--fast", help="Lower-latency, less accurate recognition")] = False,
    as_json: Annotated[bool, typer.Option("--json", help="Emit text with bounding boxes")] = False,
    timeout: Annotated[int, typer.Option("--timeout")] = 2000,
) -> None:
    """OCR the current captured frame (Apple Vision) and print the text."""
    binary = _video.hdmicap_binary()
    if not binary:
        err.print("[red]hdmicap not found.[/red]")
        raise typer.Exit(1)
    if not _video.daemon_url():
        err.print("[red]No daemon running.[/red] Start one with: paniolo video watch")
        raise typer.Exit(1)

    shot_cmd = [binary, "shot", "--out", "-", "--timeout", str(timeout)]
    if stable:
        shot_cmd.append("--stable")
    shot = subprocess.run(shot_cmd, capture_output=True)
    if shot.returncode != 0:
        err.print(shot.stderr.decode(errors="replace").strip() or "snapshot failed")
        raise typer.Exit(1)

    try:
        text = _ocr.read_text(shot.stdout, fast=fast, as_json=as_json)
    except (FileNotFoundError, RuntimeError) as exc:
        err.print(f"[red]OCR failed:[/red] {exc}")
        raise typer.Exit(1)

    # Print raw — boot logs are full of [brackets] that rich would parse as markup.
    typer.echo(text, nl=False)


@video_app.command("devices")
def video_devices() -> None:
    """List available capture devices."""
    devices = _video.list_devices()
    if not devices:
        console.print("No capture devices found (or hdmicap not available).")
        return
    for d in devices:
        console.print(f"  [{d['index']}] {d['name']}  [{d.get('misc', '')}]")


@video_app.command("show")
def video_show() -> None:
    """Show the video capture configuration and daemon status."""
    cfg = _video.load_video_config()
    if not cfg:
        console.print("No video device configured. Run: paniolo video setup")
        return

    url = _video.daemon_url()
    t = Table(show_header=False, box=None, padding=(0, 2))
    t.add_row("device", cfg.device)
    t.add_row("daemon", f"[green]running[/green] at {url}" if url else "[dim]stopped[/dim]")
    console.print(t)


@video_app.command("stop")
def video_stop() -> None:
    """Stop the running hdmicap daemon."""
    binary = _video.hdmicap_binary()
    if not binary:
        err.print("[red]hdmicap not found.[/red]")
        raise typer.Exit(1)

    result = subprocess.run([binary, "stop"], check=False)
    if result.returncode == 0:
        console.print("[green]Daemon stopped.[/green]")
    else:
        raise typer.Exit(result.returncode)


# ── console ───────────────────────────────────────────────────────────────────


@app.command("console")
def open_dashboard(
    target: Annotated[Optional[str], typer.Argument()] = None,
    interface: Annotated[
        Optional[str],
        typer.Option("--interface", "-i", help="Serial interface name to preselect"),
    ] = None,
    video_port: Annotated[int, typer.Option("--video-port")] = 8723,
    serial_port: Annotated[int, typer.Option("--serial-port")] = 8724,
) -> None:
    """Open the combined video+serial dashboard, starting daemons if needed."""
    # ── video daemon ──────────────────────────────────────────────────────────
    video_url = _video.daemon_url()
    if not video_url:
        cfg_v = _video.load_video_config()
        if not cfg_v:
            err.print("[red]No video device configured.[/red] Run: paniolo video setup")
            raise typer.Exit(1)
        binary_v = _video.hdmicap_binary()
        if not binary_v:
            err.print("[red]hdmicap not found.[/red] Run: paniolo setup")
            raise typer.Exit(1)
        ocr_bin = _ocr.visionocr_binary()
        _video.start_daemon(cfg_v, video_port, ocr_bin=ocr_bin)
        console.print("[dim]Starting video daemon…[/dim]")
        for _ in range(50):
            time.sleep(0.1)
            video_url = _video.daemon_url()
            if video_url:
                break
        if not video_url:
            err.print("[red]Video daemon did not start within 5 s.[/red]")
            raise typer.Exit(1)
        console.print(f"[green]Video daemon started.[/green]")

    # ── serial daemon ─────────────────────────────────────────────────────────
    if not _serial.daemon_url():
        cfg_s = _resolve(target)
        if not cfg_s.serial_interfaces:
            err.print(
                f"[red]No serial interfaces configured for '{cfg_s.name}'.[/red] "
                "Run: paniolo serial setup"
            )
            raise typer.Exit(1)
        if not _serial.serialcap_binary():
            err.print("[red]serialcap not found.[/red] Run: paniolo setup")
            raise typer.Exit(1)
        _serial.start_daemon(cfg_s.serial_interfaces, serial_port)
        names = ", ".join(i.name for i in cfg_s.serial_interfaces)
        console.print(f"[dim]Starting serial daemon ({names})…[/dim]")
        serial_url = None
        for _ in range(50):
            time.sleep(0.1)
            serial_url = _serial.daemon_url()
            if serial_url:
                break
        if not serial_url:
            err.print("[red]Serial daemon did not start within 5 s.[/red]")
            raise typer.Exit(1)
        console.print(f"[green]Serial daemon started.[/green]")

    url = video_url if not interface else f"{video_url}?interface={interface}"

    import webbrowser

    webbrowser.open(url)
    console.print(f"Opened {url}")


# ── power-cycle ───────────────────────────────────────────────────────────────


def _dtr_button(
    cfg: "_config.TargetConfig", interface_name: Optional[str], duration_ms: int, label: str
) -> None:
    """Assert DTR for duration_ms ms via daemon or direct fallback. Exits on error."""
    try:
        iface = cfg.serial_interface(interface_name)
    except ValueError as exc:
        err.print(f"[red]{exc}.[/red] Run: paniolo serial setup")
        raise typer.Exit(1)

    daemon_url = _serial.daemon_url()
    if daemon_url:
        console.print(f"[dim]{label} ({duration_ms} ms via serialcap daemon)[/dim]")
        try:
            _power.dtr_button_press(daemon_url, iface.name, duration_ms)
        except OSError as exc:
            err.print(f"[red]Could not reach serialcap daemon:[/red] {exc}")
            raise typer.Exit(1)
        except RuntimeError as exc:
            err.print(f"[red]{label} failed:[/red] {exc}")
            raise typer.Exit(1)
    else:
        console.print(f"[dim]{label} ({duration_ms} ms via {iface.device} directly)[/dim]")
        try:
            _power.dtr_direct_button_press(iface.device, duration_ms)
        except (OSError, RuntimeError) as exc:
            err.print(f"[red]{label} failed:[/red] {exc}")
            raise typer.Exit(1)


@app.command("power-cycle")
def power_cycle(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Run the power-cycle script configured for this target.

    Requires power_cycle_cmd to be set. Configure with:
      paniolo target set <name> --power-cycle-cmd /path/to/script"""
    cfg = _resolve(target)
    if not cfg.power_cycle_cmd:
        err.print(
            f"[red]No power_cycle_cmd configured for '{cfg.name}'.[/red] "
            "Set one with: paniolo target set <name> --power-cycle-cmd /path/to/script"
        )
        raise typer.Exit(1)

    console.print(
        f"[dim]Power cycling[/dim] [bold]{cfg.name}[/bold] "
        f"[dim]via {cfg.power_cycle_cmd}[/dim]"
    )
    result = subprocess.run(cfg.power_cycle_cmd, shell=True, check=False)
    if result.returncode == 0:
        console.print("[green]Power cycle complete.[/green]")
    else:
        err.print(f"[red]Power cycle script exited with code {result.returncode}.[/red]")
        raise typer.Exit(result.returncode)


@app.command("power-state")
def power_state(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Show whether the target is powered on (requires sense signal wired and daemon running)."""
    cfg = _resolve(target)
    if not cfg.power_serial_interface:
        err.print(
            f"[red]No power serial interface configured for '{cfg.name}'.[/red] "
            "Set one with: paniolo target set <name> --power-serial <interface-name>"
        )
        raise typer.Exit(1)

    daemon_url = _serial.daemon_url()
    if not daemon_url:
        err.print("[red]serialcap daemon is not running.[/red] Start it with: paniolo serial watch")
        raise typer.Exit(1)

    state = _serial.read_power_state(daemon_url, cfg.power_serial_interface)
    if state is None:
        err.print(
            "[yellow]Power state unknown[/yellow] — sense signal may not be configured "
            "on this interface. Run: paniolo serial setup --power-sense <cts|dsr|dcd|ri>"
        )
        raise typer.Exit(1)
    if state:
        console.print(f"[green]Power ON[/green]  ({cfg.name})")
    else:
        console.print(f"[red]Power OFF[/red]  ({cfg.name})")


# ── serial ────────────────────────────────────────────────────────────────────


@serial_app.command("setup")
def serial_setup(
    target: Annotated[Optional[str], typer.Argument()] = None,
    name: Annotated[str, typer.Option("--name", help="Interface name (e.g. console, bmc)")] = _config.DEFAULT_SERIAL_NAME,
    device: Annotated[Optional[str], typer.Option("--device", help="Serial device path; auto-detected if omitted")] = None,
    baud: Annotated[int, typer.Option("--baud", help="Baud rate")] = 115200,
    power_sense: Annotated[
        Optional[str],
        typer.Option(
            "--power-sense",
            help=(
                "FTDI modem-control input wired to the target 3.3 V rail "
                "(cts | dsr | dcd | ri | none). "
                "Enables power-state sensing in GET /status and smart power-cycle waits."
            ),
        ),
    ] = None,
) -> None:
    """Add or update a named serial interface for a target.

    A target may have several (run setup once per interface, e.g. --name console,
    --name bmc). Re-running with an existing name updates that interface.

    Use --power-sense to specify which FTDI input pin is wired to the target's
    3.3 V rail for power-state detection (see hardware notes in AGENTS.md)."""
    cfg = _resolve(target)

    if device is None:
        devices = _serial.list_serial_devices()
        if not devices:
            err.print("[red]No serial devices found.[/red] Specify one with --device.")
            raise typer.Exit(1)
        if len(devices) == 1:
            device = devices[0]
            console.print(f"[dim]Auto-detected serial device:[/dim] {device}")
        else:
            console.print("Available serial devices:")
            for d in devices:
                console.print(f"  {d}")
            err.print("[red]Multiple devices found — specify one with --device.[/red]")
            raise typer.Exit(1)

    device = _serial.canonical_device_path(device)

    if power_sense is not None and power_sense.lower() == "none":
        power_sense = None
    elif power_sense is not None and power_sense.lower() not in _config.VALID_SENSE_SIGNALS:
        err.print(
            f"[red]Unknown sense signal '{power_sense}'.[/red] "
            f"Valid values: {', '.join(_config.VALID_SENSE_SIGNALS)}, none"
        )
        raise typer.Exit(1)
    elif power_sense is not None:
        power_sense = power_sense.lower()

    # Preserve existing sense signal when --power-sense is not given.
    if power_sense is None:
        existing_iface = next((i for i in cfg.serial_interfaces if i.name == name), None)
        sense = existing_iface.power_sense_signal if existing_iface else None
    else:
        sense = power_sense

    cfg.upsert_serial_interface(
        _config.SerialInterface(name=name, device=device, baud=baud, power_sense_signal=sense)
    )
    _config.save_target(cfg)
    sense_label = f"  power_sense : {sense}" if sense else ""
    console.print(
        f"[green]Serial interface '[bold]{name}[/bold]' saved for "
        f"'[bold]{cfg.name}[/bold]':[/green] {device} @ {baud}"
    )
    if sense_label:
        console.print(sense_label)


@serial_app.command("remove")
def serial_remove(
    name: Annotated[str, typer.Argument(help="Interface name to remove")],
    target: Annotated[Optional[str], typer.Option("--target", "-t")] = None,
) -> None:
    """Remove a named serial interface from a target."""
    cfg = _resolve(target)
    if cfg.remove_serial_interface(name):
        _config.save_target(cfg)
        console.print(f"[green]Removed serial interface '[bold]{name}[/bold]'.[/green]")
    else:
        have = ", ".join(i.name for i in cfg.serial_interfaces) or "none"
        err.print(f"[red]No serial interface '{name}'.[/red] (have: {have})")
        raise typer.Exit(1)


def _resolve_interface(cfg: "_config.TargetConfig", interface: Optional[str]) -> "_config.SerialInterface":
    try:
        return cfg.serial_interface(interface)
    except ValueError as exc:
        err.print(f"[red]{exc}.[/red] Run: paniolo serial setup")
        raise typer.Exit(1)


@serial_app.command("dtr")
def serial_dtr(
    target: Annotated[Optional[str], typer.Argument()] = None,
    ms: Annotated[int, typer.Option("--ms", help="Duration of the DTR pulse in milliseconds")] = 200,
    interface: Annotated[
        Optional[str],
        typer.Option("--interface", "-i", help="Serial interface name (default: power_serial_interface or the only one)"),
    ] = None,
) -> None:
    """Pulse the DTR line (J2 power button header) on a serial interface.

    Short pulse (≤500 ms) delivers a power-button event to the OS.
    Long pulse (≥3000 ms) triggers a hard PMIC power-off.

    With --interface/-i, any configured serial interface can be targeted.
    Without it, defaults to the target's power_serial_interface (if set),
    then falls back to the only configured interface."""
    cfg = _resolve(target)
    iface_name = interface or cfg.power_serial_interface
    _dtr_button(cfg, iface_name, ms, f"DTR pulse on {cfg.name}")
    console.print("[green]Done.[/green]")


@serial_app.command("reset")
def serial_reset(
    target: Annotated[Optional[str], typer.Argument()] = None,
    ms: Annotated[int, typer.Option("--ms", help="Press duration in milliseconds")] = 200,
    interface: Annotated[
        Optional[str],
        typer.Option("--interface", "-i", help="Serial interface name (default: power_serial_interface or the only one)"),
    ] = None,
) -> None:
    """Send a soft-reset signal via a brief J2 power button press.

    The OS receives a power-button event and responds according to its policy
    (typically a graceful reboot or halt)."""
    cfg = _resolve(target)
    iface_name = interface or cfg.power_serial_interface
    console.print(f"[dim]Soft reset[/dim] [bold]{cfg.name}[/bold]")
    _dtr_button(cfg, iface_name, ms, f"Soft reset on {cfg.name}")
    console.print("[green]Reset signal sent.[/green]")


@serial_app.command("connect")
def serial_connect(
    target: Annotated[Optional[str], typer.Argument()] = None,
    interface: Annotated[Optional[str], typer.Option("--interface", "-i", help="Interface name (default: the only one)")] = None,
) -> None:
    """Open an interactive serial console to a target (via tio)."""
    cfg = _resolve(target)
    iface = _resolve_interface(cfg, interface)
    if not _serial.tio_binary():
        err.print("[red]tio not found in PATH.[/red] Install it (e.g. brew install tio).")
        raise typer.Exit(1)
    cmd = _serial.connect_cmd(iface.device, iface.baud)
    os.execvp(cmd[0], cmd)


@serial_app.command("watch")
def serial_watch(
    target: Annotated[Optional[str], typer.Argument()] = None,
    port: Annotated[int, typer.Option("--port")] = 8724,
) -> None:
    """Start the serialcap daemon (owning every configured interface) so serial
    appears on the video dashboard and is captured for `serial log`."""
    cfg = _resolve(target)
    if not cfg.serial_interfaces:
        err.print(
            f"[red]No serial interfaces configured for '{cfg.name}'.[/red] "
            "Run: paniolo serial setup"
        )
        raise typer.Exit(1)

    url = _serial.daemon_url()
    if url:
        console.print(f"[dim]Serial daemon already running at[/dim] {url}")
        return

    if not _serial.serialcap_binary():
        err.print("[red]serialcap not found.[/red] Build: cargo build --release in serialcap/")
        raise typer.Exit(1)

    _serial.start_daemon(cfg.serial_interfaces, port)
    names = ", ".join(i.name for i in cfg.serial_interfaces)
    console.print(f"[dim]Starting serial daemon for[/dim] {len(cfg.serial_interfaces)} interface(s): {names}…")

    url = None
    for _ in range(50):
        time.sleep(0.1)
        url = _serial.daemon_url()
        if url:
            break

    if url:
        console.print(f"[green]Serial daemon started.[/green] {url}")
        console.print("Open the dashboard with: [bold]paniolo console[/bold]")
    else:
        err.print("[red]Serial daemon did not start within 5 s.[/red]")
        raise typer.Exit(1)


@serial_app.command("stop")
def serial_stop() -> None:
    """Stop the running serialcap daemon."""
    binary = _serial.serialcap_binary()
    if not binary:
        err.print("[red]serialcap not found.[/red]")
        raise typer.Exit(1)

    result = subprocess.run([binary, "stop"], check=False)
    if result.returncode == 0:
        console.print("[green]Serial daemon stopped.[/green]")
    else:
        raise typer.Exit(result.returncode)


@serial_app.command("devices")
def serial_devices() -> None:
    """List available serial devices."""
    devices = _serial.list_serial_devices()
    if not devices:
        console.print("No serial devices found.")
        return
    for d in devices:
        console.print(f"  {d}")


@serial_app.command("log")
def serial_log(
    interface: Annotated[Optional[str], typer.Option("--interface", "-i", help="Interface name (default: the only captured one)")] = None,
    tail: Annotated[Optional[int], typer.Option("--tail", "-n", help="Show only the most recent N lines")] = None,
    from_seq: Annotated[Optional[int], typer.Option("--from", help="Lowest line sequence number (inclusive)")] = None,
    to_seq: Annotated[Optional[int], typer.Option("--to", help="Highest line sequence number (inclusive)")] = None,
    since: Annotated[Optional[int], typer.Option("--since", help="Only lines newer than this sequence number")] = None,
    raw: Annotated[bool, typer.Option("--raw", help="Keep raw bytes (ANSI/control) instead of cleaning")] = False,
    as_json: Annotated[bool, typer.Option("--json", help="Emit JSON Lines instead of formatted text")] = False,
    no_pending: Annotated[bool, typer.Option("--no-pending", help="Exclude the current unterminated line")] = False,
) -> None:
    """Print captured serial output, timestamped and addressable by line range.

    Thin passthrough to `serialcap log`, which reads the daemon's on-disk capture
    log directly — so this works whether or not the daemon is currently running.
    With multiple interfaces, pass --interface to choose one."""
    binary = _serial.serialcap_binary()
    if not binary:
        err.print("[red]serialcap not found.[/red] Build: cargo build --release in serialcap/")
        raise typer.Exit(1)
    cmd = _serial.log_cmd(
        binary,
        interface=interface,
        tail=tail,
        from_seq=from_seq,
        to_seq=to_seq,
        since=since,
        raw=raw,
        as_json=as_json,
        no_pending=no_pending,
    )
    result = subprocess.run(cmd, check=False)
    if result.returncode != 0:
        raise typer.Exit(result.returncode)


@serial_app.command("show")
def serial_show(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Show the serial interfaces configured for a target, and daemon status."""
    cfg = _resolve(target)
    if not cfg.serial_interfaces:
        console.print(f"No serial interfaces configured for '{cfg.name}'. Run: paniolo serial setup")
        return
    url = _serial.daemon_url()
    t = Table(show_header=False, box=None, padding=(0, 2))
    for iface in cfg.serial_interfaces:
        label = f"{iface.device} @ {iface.baud}"
        if iface.power_sense_signal:
            label += f"  [dim](sense: {iface.power_sense_signal})[/dim]"
        t.add_row(iface.name, label)
    t.add_row("daemon", f"[green]running[/green] at {url}" if url else "[dim]stopped[/dim]")
    console.print(t)


# ── hid ───────────────────────────────────────────────────────────────────────


def _open_rig() -> "_hid.HidRig":
    cfg = _hid.load_hid_config()
    if not cfg:
        err.print("[red]No HID control board configured.[/red] Run: paniolo hid setup")
        raise typer.Exit(1)
    try:
        return _hid.HidRig(cfg.port)
    except (RuntimeError, OSError, ValueError) as exc:
        err.print(f"[red]Could not open {cfg.port}:[/red] {exc}")
        raise typer.Exit(1)


@hid_app.command("setup")
def hid_setup(
    port: Annotated[Optional[str], typer.Option("--port", help="Data CDC port; auto-suggested if omitted")] = None,
) -> None:
    """Detect and save the control board's data serial port."""
    if port is None:
        ports = _hid.list_serial_ports()
        if not ports:
            err.print("[red]No USB serial ports found.[/red] Is the control board plugged in?")
            raise typer.Exit(1)
        if len(ports) == 1:
            port = ports[0]
        else:
            console.print("Candidate ports (the data port is usually the higher-numbered):")
            for p in ports:
                console.print(f"  {p}")
            port = typer.prompt("Enter the data port", default=_hid.guess_data_port())
    _hid.save_hid_config(_hid.HidConfig(port=port))
    console.print(f"[green]HID control board configured:[/green] {port}")


@hid_app.command("type")
def hid_type(
    text: Annotated[list[str], typer.Argument(help="Text to type")],
) -> None:
    """Type a string."""
    rig = _open_rig()
    try:
        rig.type(" ".join(text))
    finally:
        rig.close()


@hid_app.command("key")
def hid_key(name: Annotated[str, typer.Argument(help="Keycode name, e.g. ENTER")]) -> None:
    """Tap a key (press + release)."""
    rig = _open_rig()
    try:
        rig.key(name)
    finally:
        rig.close()


@hid_app.command("releaseall")
def hid_releaseall() -> None:
    """Release all held keys and mouse buttons."""
    rig = _open_rig()
    try:
        rig.releaseall()
    finally:
        rig.close()


@hid_app.command("combo")
def hid_combo(
    names: Annotated[list[str], typer.Argument(help="Keycode names, e.g. LEFT_CONTROL C")],
) -> None:
    """Chord: press all keys, then release all."""
    rig = _open_rig()
    try:
        rig.combo(*names)
    finally:
        rig.close()


@hid_app.command("click")
def hid_click(
    button: Annotated[str, typer.Argument(help="left | right | middle")] = "left",
) -> None:
    """Click a mouse button."""
    rig = _open_rig()
    try:
        rig.click(button)
    finally:
        rig.close()


@hid_app.command("move", context_settings={"ignore_unknown_options": True})
def hid_move(
    dx: Annotated[str, typer.Argument()],
    dy: Annotated[str, typer.Argument()],
) -> None:
    """Relative mouse move (auto-split into HID steps on the board)."""
    rig = _open_rig()
    try:
        rig.move(int(dx), int(dy))
    finally:
        rig.close()


@hid_app.command("scroll", context_settings={"ignore_unknown_options": True})
def hid_scroll(amount: Annotated[str, typer.Argument()]) -> None:
    """Scroll the wheel (positive = up, negative = down)."""
    rig = _open_rig()
    try:
        rig.scroll(int(amount))
    finally:
        rig.close()


@hid_app.command("run")
def hid_run(
    file: Annotated[Path, typer.Argument(help="Command file (one per line; # comments; delay/sleep directives)")],
    delay: Annotated[int, typer.Option("--delay", help="Default ms between commands")] = 0,
) -> None:
    """Run a sequence of commands from a file, with optional timing."""
    if not file.exists():
        err.print(f"[red]File not found:[/red] {file}")
        raise typer.Exit(1)
    steps = _hid.parse_sequence(file.read_text())
    rig = _open_rig()
    try:
        _hid.run_sequence(rig, steps, default_delay=delay / 1000.0)
    finally:
        rig.close()
    console.print(f"[green]Ran {len(steps)} step(s).[/green]")


@hid_app.command("show")
def hid_show() -> None:
    """Show the HID control board configuration."""
    cfg = _hid.load_hid_config()
    if not cfg:
        console.print("No HID control board configured. Run: paniolo hid setup")
        return
    present = Path(cfg.port).exists()
    t = Table(show_header=False, box=None, padding=(0, 2))
    t.add_row("port", cfg.port)
    t.add_row("device", "[green]present[/green]" if present else "[yellow]not found[/yellow]")
    console.print(t)


# ── setup ─────────────────────────────────────────────────────────────────────


def _user_in_group(group_name: str) -> bool:
    """Return True if the current user is a member of group_name."""
    try:
        gid = grp.getgrnam(group_name).gr_gid
    except KeyError:
        return True  # group doesn't exist on this system
    return gid in os.getgroups() or gid == os.getgid()


def _ensure_linux_groups() -> bool:
    """Add the current user to dialout and video groups if needed.

    Returns True if any group changes were made (meaning a re-login is needed
    for them to take effect).
    """
    _REQUIRED_GROUPS = [
        ("dialout", "serial port access (/dev/ttyUSB*, /dev/ttyACM*)"),
        ("video",   "V4L2 capture device access (/dev/video*)"),
    ]
    username = pwd.getpwuid(os.getuid()).pw_name
    changed = False
    for group, reason in _REQUIRED_GROUPS:
        try:
            grp.getgrnam(group)
        except KeyError:
            continue  # group not present on this system, skip
        if _user_in_group(group):
            console.print(f"  [green]✓[/green] {group:12s} already a member")
        else:
            result = subprocess.run(
                ["sudo", "usermod", "-aG", group, username],
                capture_output=True,
                text=True,
            )
            if result.returncode == 0:
                console.print(f"  [green]✓[/green] {group:12s} added ({reason})")
                changed = True
            else:
                err.print(
                    f"  [red]✗[/red] {group}: could not add user "
                    f"({result.stderr.strip() or result.stdout.strip()})"
                )
    return changed


@app.command()
def setup() -> None:
    """Install system tools and build/install paniolo's binaries.

    Builds hdmicap and serialcap (cargo install) into ~/.cargo/bin so the
    daemons resolve from a stable installed path, not the in-repo build tree.
    On macOS, also installs the visionocr OCR helper (swiftc) and tftp-now
    (Homebrew).  On Linux, DHCP and TFTP are pure-Python; no extra tools needed.
    """
    repo = Path(__file__).parent.parent.parent
    cargo_bin = Path.home() / ".cargo" / "bin"

    # 1. macOS system tool: tftp-now via Homebrew.
    #    On Linux, DHCP and TFTP are built into paniolo as pure-Python servers;
    #    no external TFTP binary is needed.
    if sys.platform == "darwin":
        if not shutil.which("brew"):
            err.print("[red]Homebrew not found.[/red] Install it: https://brew.sh")
            raise typer.Exit(1)
        tftp = shutil.which("tftp-now") or next(
            (str(p) for d in _netboot._BREW_PATHS if (p := Path(d) / "tftp-now").exists()),
            None,
        )
        if tftp:
            console.print(f"  [green]✓[/green] tftp-now     {tftp}")
        else:
            console.print("  [dim]…[/dim] installing tftp-now via brew")
            try:
                subprocess.run(["brew", "install", "tftp-now"], check=True)
            except subprocess.CalledProcessError:
                err.print(
                    "[yellow]tftp-now not in default tap.[/yellow] "
                    "Try: brew tap curl/curl && brew install tftp-now"
                )
                raise typer.Exit(1)
    else:
        console.print(
            "  [dim]ℹ[/dim]  Linux: DHCP+TFTP are built-in. "
            "Before building, ensure system packages are installed:\n"
            "    sudo apt-get install build-essential pkg-config libudev-dev libclang-dev"
        )
        console.print("\n[dim]Checking group membership…[/dim]")
        needs_relogin = _ensure_linux_groups()
        if needs_relogin:
            console.print(
                "\n[yellow]Note:[/yellow] Group changes take effect after you log out and back in "
                "(or run [bold]newgrp dialout[/bold] in the current shell)."
            )

    # 2. Rust daemons: cargo install into ~/.cargo/bin.
    cargo = shutil.which("cargo")
    if not cargo:
        err.print(
            "  [yellow]✗[/yellow] cargo not found — install Rust (https://rustup.rs) "
            "to build hdmicap/serialcap"
        )
    else:
        for crate in ("hdmicap", "serialcap"):
            crate_dir = repo / crate
            if not (crate_dir / "Cargo.toml").exists():
                console.print(f"  [yellow]…[/yellow] {crate}: source not found at {crate_dir}, skipping")
                continue
            console.print(f"  [dim]building {crate} (cargo install — may take a few minutes)…[/dim]")
            try:
                subprocess.run([cargo, "install", "--path", str(crate_dir), "--force"], check=True)
                console.print(f"  [green]✓[/green] {crate:12s} {cargo_bin / crate}")
            except subprocess.CalledProcessError:
                err.print(f"  [red]✗[/red] {crate}: cargo install failed")
                raise typer.Exit(1)

    # 3. OCR helper: visionocr on macOS, linuxocr on Linux.
    if sys.platform == "darwin":
        try:
            dest = cargo_bin / "visionocr"
            _ocr.build_visionocr(dest)
            console.print(f"  [green]✓[/green] visionocr    {dest}")
        except (FileNotFoundError, subprocess.CalledProcessError) as exc:
            console.print(f"  [yellow]…[/yellow] visionocr: skipped ({exc})")
    else:
        try:
            dest = cargo_bin / "linuxocr"
            _ocr.install_linuxocr(dest)
            console.print(f"  [green]✓[/green] linuxocr     {dest}")
        except (FileNotFoundError, OSError) as exc:
            console.print(f"  [yellow]…[/yellow] linuxocr: skipped ({exc})")
        if not shutil.which("tesseract"):
            console.print(
                "  [yellow]![/yellow] tesseract not found — install it for OCR:\n"
                "    sudo apt-get install tesseract-ocr"
            )

    console.print("\n[green]Setup complete.[/green]")
    if cargo and str(cargo_bin) not in os.environ.get("PATH", "").split(os.pathsep):
        console.print(f"[yellow]Note:[/yellow] add {cargo_bin} to your PATH so the daemons resolve.")
````