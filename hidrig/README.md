# KB2040 HID Injector

A USB keyboard/mouse injector for automated testing of a target machine
(SBC, e.g. a Raspberry Pi). A single **Adafruit KB2040** presents itself to
the target as a plain USB HID keyboard + mouse; the control host drives it
with line-based text commands over a UART on the board's TX/RX pins,
typically through a USB-serial adapter.

```
[Control host] --USB-serial adapter--+
                                     | UART (TX/RX/GND, 115200 8N1, 3.3 V)
                                     v
                                 [KB2040] --built-in USB (HID device)--> [Target]
```

The wire protocol is the **HID serial protocol v1** — see
[`docs/hid-serial-protocol.md`](../docs/hid-serial-protocol.md) for the
normative spec. This directory holds the reference firmware implementation
(CircuitPython) and the host-side CLI (`hidrig`, Rust), which works against
any device implementing the spec.

## Hardware

- 1x Adafruit KB2040 (any CircuitPython-capable RP2040 board with free
  UART pins works with minor pin edits)
- 1x 3.3 V USB-serial adapter (FTDI, CP2102, ...) on the control host
- 3 jumper wires: adapter TX -> board RX, adapter RX -> board TX, GND -> GND

The board is powered by the **target's** USB port, so it reboots with the
target — held keys can never survive a target power cycle, and the UART is
silent while the target is off.

Pins (KB2040): `TX`/`D0` (GPIO0) and `RX`/`D1` (GPIO1), with GND adjacent.
`D2` is the dev-mode jumper (below).

## Firmware setup

The board runs **CircuitPython 9.x**. See [`SETUP.md`](SETUP.md) for the
full runbook; in short:

1. Install CircuitPython 9.x (hold BOOT, copy the UF2).
2. `uvx circup --path /Volumes/CIRCUITPY install adafruit_hid`
3. Copy `firmware/boot.py` and `firmware/code.py` to `CIRCUITPY/`.
4. Hard-reset (replug). The board now enumerates as **HID only** — no
   CIRCUITPY drive, no REPL.

**Dev mode:** jumper `D2` to GND (adjacent pins) and reset — CIRCUITPY and
the REPL come back so you can edit the firmware. Plug into a dev machine
(not the target) for that.

Status NeoPixel: blinking red = waiting for the target's USB to enumerate;
green blip = up and serving; solid red = last command failed.

## Host CLI

`hidrig` (this crate) drives any conforming injector:

```bash
cargo install --path hidrig

hidrig -d /dev/cu.usbserial-XXXX ping            # liveness
hidrig -d /dev/cu.usbserial-XXXX version         # protocol + impl + capabilities
hidrig -d /dev/cu.usbserial-XXXX type "hello world"
hidrig -d /dev/cu.usbserial-XXXX key ENTER
hidrig -d /dev/cu.usbserial-XXXX combo LEFT_CONTROL C
hidrig -d /dev/cu.usbserial-XXXX move 300 -50      # relative
hidrig -d /dev/cu.usbserial-XXXX moveabs 16000 8000 # absolute (0..32767 logical)
hidrig -d /dev/cu.usbserial-XXXX click right
hidrig -d /dev/cu.usbserial-XXXX scroll -3
hidrig -d /dev/cu.usbserial-XXXX run boot-seq.txt   # command file; '-' = stdin
```

### Daemon mode (`serve`) — the KVM path

The control UART can have only one owner, so a streaming web console and CLI
one-shots can't both open it. `hidrig serve` resolves that: it owns the UART
and re-exposes the protocol over a localhost WebSocket (`GET /hid`) plus
`POST /send`, serializing every command onto the one wire.

```bash
hidrig -d /dev/cu.usbserial-XXXX serve            # owns the UART, runs until stopped
hidrig -d /dev/cu.usbserial-XXXX type hi          # auto-routes through the daemon
hidrig stop                                       # stop the daemon
```

While a daemon for a device is running, every `hidrig -d <device> …` one-shot
routes through it automatically (over `POST /send`), so the CLI and the web
console never contend for the port. `paniolo console` starts this daemon on
demand and the dashboard streams keyboard + absolute-mouse events to it — see
[`docs/hid.md`](../docs/hid.md).

For throughput, the daemon **negotiates the UART up** from the 115200 boot rate
to 460800 (the firmware advertises a `baud` capability and switches only after
acking at the old rate, so a naive connect always works and a power-cycle
re-syncs). `hidrig -d <device> baud <rate>` exercises the switch by hand.

Command files take one protocol command per line; blank lines and
`# comments` are skipped, and `delay <ms>` / `sleep <seconds>` pause between
commands (sequencing lives on the host; the firmware stays dumb).

Key names are `adafruit_hid` Keycode names (`A`–`Z`, `ENTER`, `TAB`,
`ESCAPE`, `LEFT_CONTROL`, `LEFT_SHIFT`, `UP_ARROW`, `F1`–`F12`, ...).

## paniolo integration

paniolo calls the tool through the generic per-target `hid` channel — an
opaque command prefix, exactly like the power hooks:

```bash
paniolo hid set -t pi5 --cmd "hidrig -d /dev/cu.usbserial-XXXX"
paniolo hid send -t pi5 type hello
paniolo hid send -t pi5 key ENTER
```

`paniolo hid send` appends its arguments to the configured command and runs
it on whichever control host owns the channel (transparently over SSH for
remote hosts). See [`docs/hid.md`](../docs/hid.md).

## Host testing tools (macOS)

To verify the full pipeline end-to-end, plug the injector's USB into the same
Mac that drives the UART and capture its HID reports while you inject. Build
with `cd hidrig/host && make`.

**`host/hid_capture_usb.m` — leak-safe capture (use this one).** Takes the
injector away from the macOS HID stack entirely via IOUSBHost whole-device
capture (`IOUSBHostObjectInitOptionsDeviceCapture`, root passes the gate), then
prints each interrupt-IN report with timestamps. Because the device is detached,
injected keystrokes and mouse moves reach **only** this tool — they never touch
the focused app or the real cursor.

```bash
sudo ./hid_capture_usb            # defaults to the injector serial
sudo ./hid_capture_usb <serial>   # if you have more than one KB2040 attached
```

In another terminal: `hidrig -d /dev/cu.usbserial-XXXX moveabs 16000 8000` and
watch the report bytes. **Start the capture tool before injecting** or the
reports leak into your live session.

> `host/hid_seize_reports.c` (the older `IOHIDDeviceOpen(..SeizeDevice)` tool)
> is **non-exclusive** on modern macOS (Darwin 24/25): the seize succeeds and
> reports arrive, but the system event path is not detached, so injected moves
> still move the real cursor. Keep it only as a passive raw-report tap; use
> `hid_capture_usb` when you need true exclusivity.

`host/hid_bench.py` measures latency/throughput (modes `latency`/`rr`/`pipe`)
and `host/leak_check.py` asserts no cursor leak — both via `uv run --with
pyserial …`. See `host/README.md`.

### macOS serial latency

The daemon drops the macOS serial read-latency timer (`IOSSDATALAT`) to its
floor when it opens the FTDI adapter (`proto.rs`). The default timer adds
~230 ms to every command's round trip — the dominant HID-path latency until
this was fixed. With it, a mouse move injects in ~8 ms (the USB interrupt
endpoint's 8 ms `bInterval` is then the floor).

## Files

```
hidrig/
  src/main.rs           # `hidrig` CLI: one-shots, `run`, `serve`/`stop`
  src/proto.rs          # line protocol client + sequence parser; macOS low-latency open
  src/uart.rs           # daemon: the single UART owner (serializes all commands)
  src/server.rs         # daemon: axum WebSocket /hid + POST /send
  src/daemon.rs         # daemon: lock, discovery file, lifecycle
  firmware/boot.py      # USB identity: HID-only, absolute-pointer descriptor
  firmware/code.py      # UART line protocol -> USB HID (reference impl)
  host/hid_capture_usb.m    # macOS leak-safe HID capture (IOUSBHost device-capture)
  host/hid_seize_reports.c  # macOS passive raw-report tap (non-exclusive on Darwin 24/25)
  host/hid_bench.py         # latency/throughput bench
  host/leak_check.py        # asserts injection does not leak to the live session
  host/Makefile
  host/README.md
```

## History

The first version of this rig used two boards (a USB-CDC control board
relaying I2C packets to a USB-HID target board) because the control link was
the board's own USB port. Moving the control link to the UART eliminated the
second board, the binary I2C protocol, and the duplicated opcode tables —
see git history (`hidrig/control/`, `hidrig/target/`) if you need the old
design.
