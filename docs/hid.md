# USB HID injection

paniolo can inject keyboard and mouse events into the target through a USB HID
injector: a device plugged into the target that presents itself as a keyboard
and mouse (HID, Human Interface Device, is the USB class for those). A helper
tool on the control host drives the injector. paniolo reaches the helper
through a per-target **hid channel**, an opaque command prefix like the power
hooks: paniolo appends arguments to it and runs it, so it never needs to know
which device is attached.

---

## Choosing a helper

paniolo never talks to injector hardware itself, so **the hardware attached to
your control host decides which helper you use**. There is no default. Two
helpers ship in-tree, as peers:

| Helper | Drives | Bind with |
|---|---|---|
| [`ch9329`](https://github.com/curtisgalloway/paniolo/blob/main/ch9329/README.md) | Off-the-shelf KVM-over-USB devices (keyboard/video/mouse control of another machine) whose keyboard/mouse half speaks the WCH CH9329 protocol (a serial-to-USB-HID chip): the **Openterface Mini-KVM**, the **Openterface KVM-Go** (a CH32V208 emulating the protocol rather than the chip — see the [KVM-Go notes](https://github.com/curtisgalloway/paniolo/blob/main/notes/openterface-kvm-go.md)), and the **Sipeed NanoKVM-USB**. | `--cmd "ch9329 -d <uart>"` |
| [`hidrig`](https://github.com/curtisgalloway/paniolo/blob/main/hidrig/README.md) | The DIY dual-board KB2040 "dumb pipe" rig described below. Build-it-yourself hardware, and the only option that also bridges the DUT's serial console and switches its power on the same USB device. | `--cmd "hidrig -d <data-cdc>"` |

Any other injector drops in the same way without changing paniolo: implement
the command vocabulary (`type`, `key`, `moveabs`, …) and point the channel at
it. The vocabulary is the device-independent
[HID serial protocol](dev/hid-serial-protocol.md).

**Baud rates differ by device.** The `ch9329` helper autodetects among 115200
(Openterface), 57600 (Sipeed NanoKVM-USB) and 9600 (a factory CH9329).
`-b <rate>` forces one, which is worth doing: autodetect costs a probe on every
session open.

---

## The dual-board KB2040 rig (`hidrig`)

Two KB2040 boards (Adafruit RP2040 microcontroller boards). The host composes
the HID report bytes; the boards relay them without interpreting any HID
meaning. The **control** board faces the control host over USB-CDC (a USB
virtual serial port) and is the I2C1 controller. The **target** board faces
the DUT (device under test) as a USB HID device and is the I2C1 peripheral.

```
[Control host]
      |  USB-CDC (hidrig writes binary HID frames to the data endpoint)
      v
[Control KB2040]  -- I2C1 controller, routes frames by type byte
      |  I2C1: GP10 = SDA, GP19 = SCL, GND   (target addr 0x41, 4.7 kΩ pull-ups)
      v
[Target KB2040]   -- I2C1 peripheral; relays report bytes to send_report
      |  built-in USB (HID keyboard + absolute mouse)
      v
[Target / DUT]
```

In this rig the HID serial protocol is only the *external* interface. `hidrig`
reads it, composes HID reports itself, and writes binary frames to the control
board's data CDC endpoint; the line protocol never travels on a wire.

- Design and frame format: [`hid-dual-board-design.md`](dev/hid-dual-board-design.md)
- Host CLI: [`hidrig/README.md`](https://github.com/curtisgalloway/paniolo/blob/main/hidrig/README.md)
- Wiring and firmware bring-up: the [`paniolo-hardware`](https://github.com/curtisgalloway/paniolo-hardware)
  repo's [`hidrig-kb2040/`](https://github.com/curtisgalloway/paniolo-hardware/tree/main/hidrig-kb2040)

---

## Setup

```bash
# Build and install the helpers (once per control host; libexec, off PATH).
# `make install` rebuilds everything via `paniolo setup`; to do just one:
cargo install --path ch9329 --root ~/.local/libexec/paniolo
cargo install --path hidrig --root ~/.local/libexec/paniolo

# Bind a helper to the target in the lab file — pick the one matching your
# hardware. For ch9329, -d is the device's control UART:
paniolo hid set -t nuc --cmd "ch9329 -d /dev/ttyUSB0 -b 57600"

# For hidrig, -d is the control board's DATA CDC port (the second usbmodem
# of its pair):
paniolo hid set -t pi5 --cmd "hidrig -d /dev/cu.usbmodemXXXX"

# Channel on a remote control host
paniolo hid set -t pi5 --cmd "hidrig -d /dev/ttyACM1" --host bench1

# Remove the channel
paniolo hid rm -t pi5
```

A bare helper name in the cmd string works because paniolo puts its libexec
dir (`~/.local/libexec/paniolo/bin`) first on PATH when running the hook. Run
a helper by hand with `paniolo helper <name> …`.

`paniolo doctor` checks the channel on the channel's host: an absolute-path
helper is checked for existence, and a bare name is probed with `command -v`
using the same libexec-then-PATH lookup.

---

## Commands

`paniolo hid send` appends its arguments to the configured command and runs
it (over SSH when the channel is on a remote control host):

```bash
paniolo hid send -t pi5 type hello world     # type a string
paniolo hid send -t pi5 key ENTER            # tap (press+release) a key
paniolo hid send -t pi5 combo LEFT_CONTROL C # chord: press all, release all
paniolo hid send -t pi5 releaseall           # release any held keys
paniolo hid send -t pi5 click left           # click left/right/middle
paniolo hid send -t pi5 move 300 -50         # relative mouse move
paniolo hid send -t pi5 moveabs 16000 8000   # absolute move (0..32767 logical)
paniolo hid send -t pi5 scroll -3            # scroll wheel (negative = down)
paniolo hid send -t pi5 ping                 # injector liveness check
```

With a single target in the lab, `-t` may be omitted. Everything after `send`
(minus `-t`) is the helper's CLI; see `hidrig --help` for the full set.

**Key names** are `adafruit_hid` Keycode names: `A`–`Z`, `ENTER`, `TAB`,
`ESCAPE`, `BACKSPACE`, `DELETE`, `UP_ARROW`, `DOWN_ARROW`, `LEFT_ARROW`,
`RIGHT_ARROW`, `LEFT_CONTROL`, `LEFT_SHIFT`, `LEFT_ALT`, `LEFT_GUI`,
`F1`–`F12`, `PRINT_SCREEN`, `SCROLL_LOCK`, `PAUSE`, `NUM_LOCK`,
`APPLICATION` (alias `MENU`), etc.

**Negative arguments.** `move` and `scroll` take negative values directly
(`paniolo hid send -t pi5 move 50 -30`). Put `-t` before the helper arguments.

**`combo` holds at most 6 keys at once**, the key slots in a boot-protocol
keyboard report. Modifiers like `LEFT_CONTROL` don't count. A chord that needs
more, counting keys already held with `down`, fails with `ERR` instead of
silently dropping the extras.

**`type` is exact.** It keeps trailing spaces (only the command's own line
ending is stripped). A character outside the US layout fails with `ERR`
instead of typing part of the string and silently skipping the rest.

---

## Command files

A command file is a plain text file with one protocol command per line. Blank
lines and `# comments` are ignored. It also supports two timing directives,
`delay` (milliseconds) and `sleep` (seconds):

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
hidrig -d /dev/cu.usbmodemXXXX run boot-sequence.txt
hidrig -d /dev/cu.usbmodemXXXX run - < boot-sequence.txt   # via stdin
```

Sequencing and timing live on the host; the firmware stays dumb. `delay` and
`sleep` must be finite and between 0 and one hour. A negative, infinite,
`nan`, or multi-hour value is rejected when the file is parsed, not accepted
and hung on later.

---

## KVM mode: type and click from the web console

When the target has a `hid` channel, `paniolo console` turns the dashboard into
a KVM.

1. Click **⌨ Capture input** in the video overlay. It becomes **⌨ Capturing**.
2. Your keyboard and mouse now drive the target. Keys go over as HID events.
   The mouse is **absolute**: the target cursor lands where you point inside
   the video.
3. Click the button again to release.

Details:

- Your own cursor stays visible as a crosshair over the video. There is **no
  pointer lock**, so you trade a little feedback lag for never losing your
  pointer.
- Clicking the video sends a real click to the target. The overlay buttons
  never inject.
- Losing window focus releases capture and clears held keys, so nothing sticks
  down on the target.
- Absolute positioning needs the `moveabs` capability (the KB2040 reference
  firmware advertises it in its `version` reply). A relative-only injector
  still works as a console keyboard, but click-where-you-point needs `moveabs`.

### The hid daemon

KVM mode runs on the **hid daemon**. The helper owns the control link and
exposes the command vocabulary over a localhost WebSocket (the
[HID serial protocol](dev/hid-serial-protocol.md) §1 carrier). `paniolo
console` starts it on demand, and the browser streams
`moveabs`/`down`/`up`/`scroll` commands to it. The daemon serializes every
command, from the browser *and* the CLI, onto the one wire, so
`paniolo hid send` injections mix cleanly with what you type in the console:

```bash
paniolo console pi5                    # KVM dashboard (auto-starts the hid daemon)
paniolo hid serve pi5                  # warm the daemon ahead of time (idempotent)
paniolo hid send  -t pi5 type "while console is open"   # intermixes with the browser
paniolo hid stop pi5                   # stop the daemon (positional target —
                                       #   only `hid send`/`set`/`rm` use -t)
```

While a daemon runs for a device, `hidrig -d <device> …` one-shots route
through it automatically (the UART has a single owner), so the CLI and the web
console never fight over the port.

**The daemon's API needs its token.** Like serialcap and hdmicap, the hid
daemon makes a token on each start and publishes it in its discovery file
(`/tmp/paniolo-<uid>/hid/<target>/daemon.json`, owner-only). Every request
must carry it as `Authorization: Bearer <token>` or `?token=<token>`, and only
loopback `Host`/`Origin` values are accepted. One-shots routed through the
daemon send it automatically, and `paniolo console` puts it in the `?hidws=`
URL it gives the dashboard. So a web page in your browser cannot inject
keystrokes into the target. A daemon from an older paniolo has no token;
`paniolo daemons restart --stale` replaces it.

**Stopping.** `hid stop` (and the standalone `ch9329 stop` / `hidrig stop`)
shuts the daemon down through its authenticated `POST /stop`, never by
signaling the PID in the discovery file. A record left behind by a crash can
name a PID the kernel has since given to an unrelated process. A daemon too
old to have the endpoint must be stopped with `paniolo daemons stop hid`,
which checks the process identity first.

**Limits.**

- One `type` command takes at most 4096 characters.
- A `move` or `scroll` moves at most 32 767 units per axis per call (`ch9329`).
- A `/send` body or WebSocket message holds exactly one full `type` line (the
  4096 characters plus the `type ` verb), so the documented `type` length is
  accepted in full.
- A command the injector never answers fails after 30 s instead of stalling
  every other client behind it.

**Reliability.**

- A keystroke, click or move is not idempotent, so a lost reply on an input
  command surfaces as the 30 s timeout. It is not retried, since a retry would
  inject the command twice. Only pure status reads (`ping`, `info`,
  `version`) are retried once.
- A command whose client has already given up (its 30 s elapsed) is dropped,
  not injected late.
- On shutdown the daemon releases every held key, modifier and mouse button so
  nothing is left stuck down on the target. If a transport error had dropped
  the link, it reopens it first, since the CH9329 chip holds its last report on
  its own.

**Latency.**

- HID frames are fire-and-forget over the USB-CDC link (no per-frame round
  trip), so cursor streaming stays responsive.
- The dashboard **coalesces mouse moves** to one `moveabs` per animation frame
  (newest position only, not every `mousemove`).
- The control board is a USB-CDC device, so there is no baud negotiation; USB
  sets the rate.
- The remaining floor is the target's USB interrupt `bInterval` (the polling
  interval the host uses for the device): ~8 ms per report on the
  CircuitPython firmware.

---

## Lab file shape

```toml
[targets.nuc.hid]
cmd = "ch9329 -d /dev/ttyUSB0 -b 57600"

[targets.pi5.hid]
cmd = "hidrig -d /dev/cu.usbmodemXXXX"
# host = "bench1"            # if the injector hangs off a remote control host
```

---

## Host testing tools

**Injection itself is cross-platform.** Both shipped helpers build and run on
macOS, Linux and Windows. What is platform-specific is the optional *bench*:
tooling to exercise the whole pipeline with no DUT, by plugging the **target**
board into the control host and capturing the HID reports it sends while you
inject.

Capturing those reports means taking the device away from the host's own HID
stack, which is OS-specific. The tools in `hidrig/host/` do it for macOS. The
equivalents elsewhere are described below but are **not shipped**.

### macOS

Build with `cd hidrig/host && make`, then plug the target board into the same
Mac that drives the control link.

`hidrig/host/hid_capture_usb.m` is the **leak-safe** tool. It detaches the
target board from the macOS HID stack using IOUSBHost whole-device capture and
prints timestamped interrupt-IN reports. Injected input reaches only the tool,
not the focused app or the real cursor.

```bash
sudo ./hid_capture_usb         # start this BEFORE injecting
# second terminal:
hidrig -d /dev/cu.usbmodemXXXX moveabs 16383 16383
```

> The older `hid_seize_reports.c` (`IOHIDDeviceOpen(..SeizeDevice)`) is
> **non-exclusive** on Darwin 24/25 — injected moves still move the real
> cursor — so use it only as a passive tap. `leak_check.py` (which imports
> Quartz, so it is macOS-only too) checks that nothing leaked into the live
> session; see `hidrig/host/README.md`.

### Linux

No capture tool ships. The equivalent is to read the target board's reports
from its `/dev/hidraw*` node (`usbhid-dump` or a few lines of Python will do)
and to unbind it from `usbhid` first if you need the leak-safe property that
`hid_capture_usb` gives on macOS. Injection, the daemon, and `hid_bench.py`
(below) all work; only the receive side is missing.

### Windows

No capture tool ships, and no equivalent has been worked out. Injection works;
verify against a real DUT instead of a loopback bench.

### Measuring latency

`hidrig/host/hid_bench.py` sends `moveabs` commands stamped with wall-clock
send times. It is plain `pyserial`, so it runs anywhere. Matching those sends
to arrivals needs a capture tool on the receive side, which today means macOS.

On macOS the daemon drops the serial read-latency timer (`IOSSDATALAT`) to its
floor when it opens the control CDC endpoint. The default added ~230 ms to
each control-frame round trip (`ping`/`version`). HID frames are
fire-and-forget and don't pay it, but the floor keeps liveness checks prompt
(`ping` ~3 ms).

A mouse move injects in ~8 ms; the target's USB interrupt endpoint's 8 ms
`bInterval` is the floor.
