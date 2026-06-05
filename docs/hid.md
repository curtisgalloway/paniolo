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
# Build and install the helper (once per control host; libexec, off PATH)
cargo install --path hidrig --root ~/.local/libexec/paniolo   # or just `make install`

# Bind the helper to the target in the lab file
paniolo hid set -t pi5 --cmd "hidrig -d /dev/cu.usbserial-XXXX"

# Channel on a remote control host
paniolo hid set -t pi5 --cmd "hidrig -d /dev/ttyUSB0" --host bench1

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
hidrig -d /dev/cu.usbserial-XXXX run boot-sequence.txt
hidrig -d /dev/cu.usbserial-XXXX run - < boot-sequence.txt   # via stdin
```

Sequencing and timing live on the host; the firmware stays dumb.

---

## KVM mode — type and click from the web console

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

Under the hood this is the **hid daemon**: the helper owns the UART and
re-exposes the protocol over a localhost WebSocket (the
[HID serial protocol](hid-serial-protocol.md) §2 carrier). `paniolo console`
starts it on demand; the browser streams `moveabs`/`down`/`up`/`scroll`
commands to it. Because the daemon serializes every command — from the browser
*and* from the CLI — onto the one wire, `paniolo hid send` injections intermix
cleanly with what you type in the console:

```bash
paniolo console pi5                    # KVM dashboard (auto-starts the hid daemon)
paniolo hid serve -t pi5               # warm the daemon ahead of time (idempotent)
paniolo hid send  -t pi5 type "while console is open"   # intermixes with the browser
paniolo hid stop  -t pi5               # stop the daemon
```

Absolute positioning requires the `moveabs` capability (the KB2040 reference
firmware advertises it in its `version` reply). A relative-only injector still
works as a console keyboard, but click-where-you-point needs `moveabs`.

When a daemon is running for a device, `hidrig -d <device> …` one-shots route
through it automatically (the UART has a single owner), so the CLI and the web
console never contend for the port.

**Latency.** Each command is a serial round-trip, so cursor streaming is
sensitive to per-command cost and event rate. Two things keep it responsive:
the dashboard **coalesces mouse moves** to one `moveabs` per animation frame
(newest position only, instead of every `mousemove`), and the daemon
**negotiates the UART up** from the 115200 boot rate to 460800 when the device
advertises the `baud` capability (the firmware boots at 115200 so a naive
connect always works, and returns to it on power-cycle). One-shots stay at
115200 — a single command doesn't need the speed.

---

## Lab file shape

```toml
[targets.pi5.hid]
cmd = "hidrig -d /dev/cu.usbserial-XXXX"
# host = "bench1"            # if the injector hangs off a remote control host
```

---

## Host testing tools (macOS)

To exercise the full pipeline without a target, plug the injector's USB into
the same Mac that drives the UART and capture its HID reports while injecting.
Build with `cd hidrig/host && make`.

`hidrig/host/hid_capture_usb.c` (`.m`) is the **leak-safe** tool: it detaches
the injector from the macOS HID stack via IOUSBHost whole-device capture and
prints timestamped interrupt-IN reports, so injected input reaches only the
tool — not the focused app or the real cursor.

```bash
sudo ./hid_capture_usb         # start this BEFORE injecting
# second terminal:
hidrig -d <adapter> moveabs 16000 8000
```

> The older `hid_seize_reports.c` (`IOHIDDeviceOpen(..SeizeDevice)`) is
> **non-exclusive** on Darwin 24/25 — injected moves still move the real
> cursor — so use it only as a passive tap. `hid_bench.py` (latency/throughput)
> and `leak_check.py` round out the bench; see `hidrig/host/README.md`.

### Latency note

On macOS the daemon drops the serial read-latency timer (`IOSSDATALAT`) to its
floor on open; the default added ~230 ms per command, which had dominated the
felt KVM latency. With the fix a mouse move injects in ~8 ms (the USB endpoint's
8 ms `bInterval` is the floor); `ping` is ~3 ms.
