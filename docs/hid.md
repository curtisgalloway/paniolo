# USB HID injection

paniolo can inject keyboard and mouse events into the target through any
helper tool that drives a USB HID injector — by default the KB2040 rig in
[`hidrig/`](https://github.com/curtisgalloway/paniolo/blob/main/hidrig/README.md). The integration is a generic per-target
**hid channel**, an opaque command prefix exactly like the power hooks:
paniolo appends arguments to it and runs it, staying agnostic to the device.

---

## Architecture

The default injector is the **dual-board KB2040 "dumb pipe"** rig: two KB2040s
where the host composes the HID report bytes and the boards relay them without
interpreting any HID semantics. The host-facing **control** board faces the
control host over USB-CDC and is the I2C1 controller; the **target** board
faces the DUT over USB-HID and is the I2C1 peripheral.

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

The command vocabulary (`type`, `key`, `moveabs`, …) is the device-independent
[HID serial protocol](hid-serial-protocol.md), but in this rig it is the
*external* interface only: `hidrig` consumes it and composes HID reports
itself, then writes binary frames to the control board's data CDC endpoint —
the line protocol never travels on a wire. See
[`hid-dual-board-design.md`](hid-dual-board-design.md) for the design and the
frame format, and [`../hidrig/README.md`](https://github.com/curtisgalloway/paniolo/blob/main/hidrig/README.md) for wiring and bring-up.
Because the interface above `hidrig` is just the helper's CLI, any other
injector (another microcontroller, a CH9329 shim) drops in through the same
generic `hid` channel without touching paniolo.


---

## Setup

```bash
# Build and install the helper (once per control host; libexec, off PATH)
cargo install --path hidrig --root ~/.local/libexec/paniolo   # or `make install`, which
                                                              # rebuilds everything via `paniolo setup`

# Bind the helper to the target in the lab file
# -d is the control board's DATA CDC port (the second usbmodem of its pair)
paniolo hid set -t pi5 --cmd "hidrig -d /dev/cu.usbmodemXXXX"

# Channel on a remote control host
paniolo hid set -t pi5 --cmd "hidrig -d /dev/ttyACM1" --host bench1

# Remove the channel
paniolo hid rm -t pi5
```

The bare `hidrig` in the cmd string resolves because paniolo prepends its
libexec dir (`~/.local/libexec/paniolo/bin`) to PATH when running the hook;
run the helper by hand with `paniolo helper hidrig …`. `paniolo doctor`
checks the channel: an absolute-path helper is probed for existence on the
channel's host; bare names are probed with `command -v` under the same
libexec-then-PATH resolution.

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
paniolo hid send -t pi5 moveabs 16000 8000   # absolute move (0..32767 logical)
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
hidrig -d /dev/cu.usbmodemXXXX run boot-sequence.txt
hidrig -d /dev/cu.usbmodemXXXX run - < boot-sequence.txt   # via stdin
```

Sequencing and timing live on the host; the firmware stays dumb.

---

## KVM mode: type and click from the web console

`paniolo console` turns the dashboard into a KVM when the target has a `hid`
channel. Click the **⌨ Capture input** button in the video overlay to engage
(it becomes **⌨ Capturing**; click again to release). While engaged your
keyboard and mouse drive the target: keys are forwarded as HID events and the
mouse is **absolute**, so the target cursor lands where you point inside the
video. Your own cursor stays visible as a crosshair over the video — there is
**no pointer lock**, so you trade a little feedback lag for never losing your
pointer. Clicking the video sends a real click to the target; the overlay
buttons themselves never inject. Losing window focus auto-releases and clears
held keys so nothing sticks on the target.

Under the hood this is the **hid daemon**: the helper owns the control link and
re-exposes the command vocabulary over a localhost WebSocket (the
[HID serial protocol](hid-serial-protocol.md) §2 carrier). `paniolo console`
starts it on demand; the browser streams `moveabs`/`down`/`up`/`scroll`
commands to it. Because the daemon serializes every command — from the browser
*and* from the CLI — onto the one wire, `paniolo hid send` injections intermix
cleanly with what you type in the console:

```bash
paniolo console pi5                    # KVM dashboard (auto-starts the hid daemon)
paniolo hid serve pi5                  # warm the daemon ahead of time (idempotent)
paniolo hid send  -t pi5 type "while console is open"   # intermixes with the browser
paniolo hid stop pi5                   # stop the daemon (positional target —
                                       #   only `hid send`/`set`/`rm` use -t)
```

Absolute positioning requires the `moveabs` capability (the KB2040 reference
firmware advertises it in its `version` reply). A relative-only injector still
works as a console keyboard, but click-where-you-point needs `moveabs`.

When a daemon is running for a device, `hidrig -d <device> …` one-shots route
through it automatically (the UART has a single owner), so the CLI and the web
console never contend for the port.

**Latency.** HID frames are fire-and-forget over the USB-CDC link (no
per-frame round-trip), so cursor streaming stays responsive; the dashboard also
**coalesces mouse moves** to one `moveabs` per animation frame (newest position
only, instead of every `mousemove`). The control board is a USB-CDC device, so
there is no baud negotiation — USB sets the rate. The remaining floor is the
target's USB interrupt `bInterval` (~8 ms per report on the CircuitPython
firmware).

---

## Lab file shape

```toml
[targets.pi5.hid]
cmd = "hidrig -d /dev/cu.usbmodemXXXX"
# host = "bench1"            # if the injector hangs off a remote control host
```

---

## Host testing tools (macOS)

To exercise the full pipeline without a DUT, plug the **target** board's USB
into the same Mac that drives the control link and capture its HID reports while
injecting. Build with `cd hidrig/host && make`.

`hidrig/host/hid_capture_usb.m` is the **leak-safe** tool: it detaches
the target board from the macOS HID stack via IOUSBHost whole-device capture and
prints timestamped interrupt-IN reports, so injected input reaches only the
tool — not the focused app or the real cursor.

```bash
sudo ./hid_capture_usb         # start this BEFORE injecting
# second terminal:
hidrig -d /dev/cu.usbmodemXXXX moveabs 16383 16383
```

> The older `hid_seize_reports.c` (`IOHIDDeviceOpen(..SeizeDevice)`) is
> **non-exclusive** on Darwin 24/25 — injected moves still move the real
> cursor — so use it only as a passive tap. `hid_bench.py` (latency/throughput)
> and `leak_check.py` round out the bench; see `hidrig/host/README.md`.

### Latency note

On macOS the daemon drops the serial read-latency timer (`IOSSDATALAT`) to its
floor when it opens the control CDC endpoint; the default added ~230 ms to each
control-frame round trip (`ping`/`version`). HID frames are fire-and-forget so
they don't pay it, but the floor keeps liveness checks prompt (`ping` ~3 ms). A
mouse move injects in ~8 ms — the target's USB interrupt endpoint's 8 ms
`bInterval` is the floor.
