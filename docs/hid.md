# USB HID injection

paniolo injects keyboard and mouse events into the target through a USB HID
injector: a device plugged into the target that acts as a keyboard and mouse.
A helper tool on the control host drives it. The target's **hid channel** is
a command prefix: paniolo appends arguments and runs it.

---

## Choosing a helper

Pick the helper that matches your injector hardware. There is no default.

| Helper | Drives | Bind with |
|---|---|---|
| [`ch9329`](https://github.com/curtisgalloway/paniolo/blob/main/ch9329/README.md) | KVM-over-USB devices speaking the WCH CH9329 protocol (serial-to-HID chip): the **Openterface Mini-KVM**, the **Openterface KVM-Go** (see the [KVM-Go notes](https://github.com/curtisgalloway/paniolo/blob/main/notes/openterface-kvm-go.md)), and the **Sipeed NanoKVM-USB**. | `--cmd "ch9329 -d <uart>"` |
| [`hidrig`](https://github.com/curtisgalloway/paniolo/blob/main/hidrig/README.md) | The DIY dual-board KB2040 rig below. The only option that also bridges the DUT's serial console and switches its power. | `--cmd "hidrig -d <data-cdc>"` |

Any other injector works if it implements the command vocabulary (`type`,
`key`, `moveabs`, …) of the [HID serial protocol](dev/hid-serial-protocol.md).

**Set the baud rate.** `ch9329` autodetects among 115200 (Openterface), 57600
(Sipeed NanoKVM-USB) and 9600 (a factory CH9329), which costs a probe per
session. `-b <rate>` skips it.

---

## The dual-board KB2040 rig (`hidrig`)

Two KB2040 boards (Adafruit RP2040 boards) relay HID reports the host
composes. The **control** board faces the control host over USB-CDC (USB
virtual serial) and is the I2C1 controller. The **target** board faces the
DUT as a USB HID device and is the I2C1 peripheral.

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

`hidrig` accepts the HID serial protocol and writes binary frames to the
control board; the line protocol never travels on a wire.

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

paniolo puts `~/.local/libexec/paniolo/bin` first on PATH when running the
hook, so bare helper names work. Run a helper by hand with
`paniolo helper <name> …`. `paniolo doctor` checks the helper exists on the
channel's host (`command -v` for a bare name).

---

## Commands

`paniolo hid send` appends its arguments to the configured command and runs
it (over SSH for a remote channel):

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

With one target, `-t` may be omitted. Put `-t` before the helper arguments.
See `hidrig --help` for the full set.

**Key names** are `adafruit_hid` Keycode names: `A`–`Z`, `ENTER`, `TAB`,
`ESCAPE`, `BACKSPACE`, `DELETE`, `UP_ARROW`, `DOWN_ARROW`, `LEFT_ARROW`,
`RIGHT_ARROW`, `LEFT_CONTROL`, `LEFT_SHIFT`, `LEFT_ALT`, `LEFT_GUI`,
`F1`–`F12`, `PRINT_SCREEN`, `SCROLL_LOCK`, `PAUSE`, `NUM_LOCK`,
`APPLICATION` (alias `MENU`), etc.

**`combo` holds at most 6 non-modifier keys**, including keys already held
with `down`. More fails with `ERR`.

**`type` is exact.** It keeps trailing spaces. A character outside the US
layout fails with `ERR` before typing anything.

---

## Command files

One protocol command per line. Blank lines and `# comments` are ignored.
`delay` takes milliseconds, `sleep` seconds:

```
# boot-sequence.txt
type root
key ENTER
delay 500        # wait 500 ms
type ls /
key ENTER
sleep 1.5        # wait 1.5 seconds
```

Run it (the file must be on the host that owns the channel):

```bash
hidrig -d /dev/cu.usbmodemXXXX run boot-sequence.txt
hidrig -d /dev/cu.usbmodemXXXX run - < boot-sequence.txt   # via stdin
```

`delay` and `sleep` must be between 0 and one hour. A negative, infinite or
`nan` value is rejected when the file is parsed.

---

## KVM mode: type and click from the web console

With a `hid` channel, `paniolo console` turns the dashboard into a KVM:

1. Click **⌨ Capture input** in the video overlay. It becomes **⌨ Capturing**.
2. Your keyboard and mouse drive the target. The mouse is **absolute**: the
   target cursor lands where you point.
3. Click the button again to release.

- There is **no pointer lock**; your cursor stays visible as a crosshair.
- Clicking the video clicks the target. Overlay buttons never inject.
- Losing window focus releases capture and all held keys.
- Click-where-you-point needs the `moveabs` capability (advertised in the
  firmware's `version` reply). A relative-only injector still works as a
  keyboard.

### The hid daemon

KVM mode runs on the **hid daemon**, which owns the control link and serves
the command vocabulary over a localhost WebSocket (the
[HID serial protocol](dev/hid-serial-protocol.md) §1 carrier). It serializes
commands from the browser *and* the CLI:

```bash
paniolo console pi5                    # KVM dashboard (auto-starts the hid daemon)
paniolo hid serve pi5                  # warm the daemon ahead of time (idempotent)
paniolo hid send  -t pi5 type "while console is open"   # intermixes with the browser
paniolo hid stop pi5                   # stop the daemon (positional target —
                                       #   only `hid send`/`set`/`rm` use -t)
```

While a daemon runs, `hidrig -d <device> …` one-shots route through it
automatically.

**The daemon's API needs its token**, from its discovery file
(`/tmp/paniolo-<uid>/hid/<target>/daemon.json`, owner-only). Send it as
`Authorization: Bearer <token>` or `?token=<token>`; only loopback
`Host`/`Origin` values are accepted. Routed one-shots send it automatically,
and `paniolo console` passes it in the `?hidws=` URL. A daemon from an older
paniolo has no token; `paniolo daemons restart --stale` replaces it.

**Stopping.** `hid stop` (and `ch9329 stop` / `hidrig stop`) uses the
authenticated `POST /stop` and never signals the discovery-file PID. Stop a
daemon too old for the endpoint with `paniolo daemons stop hid`.

**Limits.**

- One `type` command takes at most 4096 characters.
- A `move` or `scroll` moves at most 32 767 units per axis per call (`ch9329`).
- A `/send` body or WebSocket message holds one full `type` line.
- A command the injector never answers fails after 30 s.

**Reliability.**

- Input commands are never retried, so a lost reply surfaces as the 30 s
  timeout. Only `ping`, `info` and `version` are retried once.
- A command whose client already timed out is dropped, not injected late.
- On shutdown the daemon releases every held key and button, reopening the
  link first if needed.

**Latency.**

- HID frames are fire-and-forget over USB-CDC, with no baud negotiation.
- The dashboard sends at most one `moveabs` per animation frame.
- The floor is the target's USB `bInterval` (polling interval): ~8 ms per
  report on the CircuitPython firmware.

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

Both helpers run on macOS, Linux and Windows. The optional *bench* tests the
pipeline with no DUT: plug the **target** board into the control host and
capture the reports it sends. Only macOS tools ship, in `hidrig/host/`.

### macOS

Build with `cd hidrig/host && make`, then plug the target board into the same
Mac that drives the control link.

`hidrig/host/hid_capture_usb.m` is **leak-safe**: it detaches the target board
from the macOS HID stack, so injected input reaches only the tool, not the
real cursor.

```bash
sudo ./hid_capture_usb         # start this BEFORE injecting
# second terminal:
hidrig -d /dev/cu.usbmodemXXXX moveabs 16383 16383
```

> The older `hid_seize_reports.c` (`IOHIDDeviceOpen(..SeizeDevice)`) is
> **non-exclusive** on Darwin 24/25: injected moves still move the real
> cursor. Use it only as a passive tap. `leak_check.py` (macOS-only) checks
> that nothing leaked; see `hidrig/host/README.md`.

### Linux

No capture tool ships. Read the target board's `/dev/hidraw*` node
(`usbhid-dump` works), and unbind it from `usbhid` first for leak safety.
Injection, the daemon and `hid_bench.py` work.

### Windows

No capture tool ships. Verify against a real DUT.

### Measuring latency

`hidrig/host/hid_bench.py` (plain `pyserial`) sends timestamped `moveabs`
commands; matching them to arrivals needs a macOS capture tool.

On macOS the daemon lowers the serial read-latency timer (`IOSSDATALAT`) to
its floor, cutting `ping`/`version` round trips from ~230 ms to ~3 ms. A mouse
move injects in ~8 ms, the target's `bInterval` floor.
