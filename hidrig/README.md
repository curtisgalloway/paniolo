# KB2040 Dual-Board HID Injector

A USB keyboard/mouse injector for automated testing of a target machine
(SBC, e.g. a Raspberry Pi). Two **Adafruit KB2040** boards form a **"dumb
pipe"**: the host-side `hidrig` tool composes the HID report bytes in Rust,
and the two boards relay those bytes to the target without interpreting any
HID semantics.

- The **control** board faces the control host over **USB-CDC** and is the
  I2C1 **controller**.
- The **target** board faces the device-under-test (DUT) over **USB-HID** and
  is the I2C1 **peripheral**.

The control board also bridges the DUT's **serial console** (its hardware UART)
and switches **DUT power** through a relay, so one USB-attached device drives the
target's HID, console, *and* power together (design §6–§7).

`hidrig` turns each command (`type`, `key`, `moveabs`, …) into HID report
bytes, wraps them in binary frames, and writes them to the control board's
data CDC endpoint. The control board relays HID frames verbatim over I2C1 to
the target board, which calls `send_report` — so neither board parses
keycodes or mouse math. The design and rationale live in
[`../docs/hid-dual-board-design.md`](../docs/hid-dual-board-design.md); the
firmware bring-up runbook is in
[`firmware/dual/README.md`](firmware/dual/README.md).

```
[Control host]
      |  USB-CDC (hidrig writes binary HID frames to the data endpoint)
      v
[Control KB2040]  -- routes frames by type byte; UART console bridge; power relay
      |  I2C1:  GP10 = SDA, GP19 = SCL, GND   (target addr 0x41, 4.7 kΩ pull-ups)
      |  UART0: GP0 = TX, GP1 = RX         <--->  DUT serial console
      |  GP5  -> relay / load-switch        --->  DUT power
      v
[Target KB2040]   -- I2C1 peripheral; relays report bytes to send_report
      |  built-in USB (HID keyboard + absolute mouse)
      v
[Target / DUT]
```

The command vocabulary (`type`, `key`, `combo`, `moveabs`, …) is the
device-independent **HID serial protocol v1**
([`../docs/hid-serial-protocol.md`](../docs/hid-serial-protocol.md)) — but in
this rig that protocol is the *external* interface only: `hidrig` (and the
`serve` daemon) consume it and compose reports themselves, so the line
protocol never travels on a wire. The boards see only binary frames.

## Wire protocol (host ↔ rig)

`hidrig` writes length-prefixed binary frames to the control board's data CDC
endpoint; the control board relays HID frames over I2C1 to the target. One
uniform frame format on both legs:

```
[type][b1][len][payload .. len bytes]
  0x01  rid  N   N HID report bytes   (rid 1 = keyboard / 8 B, 2 = abs mouse / 6 B)
  0x02  cmd  N   N arg bytes          (cmd 1 = ping, 2 = version, 3 = power)
  0x03  port N   N raw console bytes  (DUT serial console, both directions)
```

- **HID frames (`0x01`) are fire-and-forget** — no per-frame ack. `hidrig`
  paces them to the downstream USB poll interval (`bInterval`).
- **Control frames (`0x02`) are request/reply** — `ping`, `version`, and
  `power` draw a `[0x02][cmd][len][payload]` reply. `version` returns the
  control board's implementation id (`dual-control/1`); `power` acks before
  acting (a `power cycle` blocks the board only for its off-time).
- **Console frames (`0x03`) are a bidirectional byte pipe** to/from the DUT's
  serial console on the control board's hardware UART. They are fire-and-forget;
  the `serve` daemon demuxes inbound console output and re-exports it as a PTY
  (see *DUT serial console* below).

Because the host composes reports, its composer must match the target board's
HID **descriptor** exactly (report IDs, field order, the 0..32767 absolute
range). That descriptor lives in `firmware/dual/target/boot.py` and is the
host↔rig contract. See `src/compose.rs` for the composition and framing.

## Hardware

- 2× Adafruit KB2040 (any CircuitPython-capable RP2040 board with a free I2C1
  works with minor pin edits; the target also needs CircuitPython's
  `i2ctarget` module).
- 3 jumper wires between the boards for I2C1 — **straight, not crossed** (I2C
  is a bus): `SDA→SDA`, `SCL→SCL`, `GND→GND`.
- **Pull-ups are required:** ~4.7 kΩ from SDA→3.3 V and SCL→3.3 V (one set, on
  either board). Without them the controller-mode `busio.I2C` rejects the bus
  ("No pull up found") and the control board blinks red. (`i2ctarget` does
  *not* check, so a target coming up is not by itself proof of pull-ups.)

I2C1 pins (KB2040 labels): **`D10` = GP10 = SDA**, **`MOSI` = GP19 = SCL**.
Target peripheral address **0x41**.

**DUT serial console** (control board): hardware **UART0**, **`TX` = GP0**,
**`RX` = GP1** — wire `TX → DUT RX`, `RX → DUT TX`, common `GND`, at the DUT's
console logic level (3.3 V; never RS-232 voltages without a level shifter).

**DUT power** (control board): a free GPIO, **`D5` = GP5** by default, drives a
relay / load-switch on the DUT's 5 V — a Pi 5 pulls ~5 A, so this is a real
switch, not the GPIO driving the rail. Active-high by default (`RELAY_PIN` /
`RELAY_ACTIVE_HIGH` in `control/code.py`).

The two boards sit in **different power domains** — the control board is
host-USB powered, the target board is DUT-USB powered. That's fine while both
are powered for bench bring-up; see design §7 for the back-powering caution
before this goes near a real DUT power cycle. Because the target board is
DUT-powered, a `power cycle` also resets it — it re-enumerates as the DUT boots.

## Firmware setup

Both boards run **CircuitPython 9.x**. See [`SETUP.md`](SETUP.md) for the full
runbook; in short:

1. Flash CircuitPython 9.x on both boards (hold BOOT, copy the UF2).
2. Target board: `uvx circup --path /Volumes/CIRCUITPY install adafruit_hid`
   is *not* needed (the dumb relay uses only core `usb_hid`); copy
   `firmware/dual/target/boot.py` + `code.py`.
3. Control board: copy `firmware/dual/control/boot.py` + `code.py`.
4. Hard-reset both (`boot.py` only runs on a hard reset).

**Target mode switching (dev vs HID-only).** The target's `boot.py` reads a
1-byte NVM flag: **dev** (CIRCUITPY drive + REPL + HID, for editing) vs
**HID-only** (only the keyboard + mouse the DUT sees — no drive, no console).
**Tap the BOOT button (GP11)** to toggle and reset. Grounding **D2 at reset**
forces dev mode regardless of the flag, as a hardware recovery fallback. In
HID-only mode the target drops its CIRCUITPY drive and console, so a power blip
that hard-resets it can make it "vanish" — that's mode, not a dead board.

Status NeoPixel: the target blips green per frame received; the control blips
green per frame relayed and **solid/blinking red** on I2C failure (target not
ACKing — check pull-ups / wiring / address / that the target code is running).

## Host CLI (`hidrig`)

`hidrig` composes HID and drives the rig. It installs into paniolo's private
libexec dir (off PATH) — `make install` does this, or manually with
`cargo install --path hidrig --root ~/.local/libexec/paniolo`. Run it directly
via `paniolo helper hidrig …`, or bare inside a `paniolo hid set --cmd` hook
string.

`-d` is the control board's **data CDC port** — the *second* `usbmodem` of the
pair the control board exposes (the first is the REPL console); on Linux it is
the higher-numbered `/dev/ttyACM*`.

```bash
hidrig -d /dev/cu.usbmodemXXXX ping              # liveness (control frame)
hidrig -d /dev/cu.usbmodemXXXX version           # -> dual-control/1
hidrig -d /dev/cu.usbmodemXXXX type "hello world"
hidrig -d /dev/cu.usbmodemXXXX key ENTER
hidrig -d /dev/cu.usbmodemXXXX combo LEFT_CONTROL C
hidrig -d /dev/cu.usbmodemXXXX move 300 -50       # relative
hidrig -d /dev/cu.usbmodemXXXX moveabs 16383 16383 # absolute (0..32767 logical; centre)
hidrig -d /dev/cu.usbmodemXXXX click right
hidrig -d /dev/cu.usbmodemXXXX scroll -3
hidrig -d /dev/cu.usbmodemXXXX power cycle         # DUT power off/on via the relay
hidrig -d /dev/cu.usbmodemXXXX power off           # off | on | cycle [secs]
hidrig -d /dev/cu.usbmodemXXXX run boot-seq.txt   # command file; '-' = stdin
```

`moveabs` takes absolute coordinates in `0..32767`; the host OS maps that range
across the full screen, so `moveabs 16383 16383` parks the cursor dead centre.
Key names are `adafruit_hid` Keycode names (`A`–`Z`, `ENTER`, `TAB`, `ESCAPE`,
`LEFT_CONTROL`, `LEFT_SHIFT`, `UP_ARROW`, `F1`–`F12`, …).

Command files take one command per line; blank lines and `# comments` are
skipped, and `delay <ms>` / `sleep <seconds>` pause between commands
(sequencing lives on the host; the firmware stays dumb).

### Daemon mode (`serve`) — the KVM path

The control link can have only one owner, so a streaming web console and CLI
one-shots can't both open it. `hidrig serve` resolves that: it owns the CDC
link, holds the composition state (held keys, virtual cursor), and re-exposes
the command vocabulary over a localhost WebSocket (`GET /hid`) plus
`POST /send`, serializing every command onto the one wire.

```bash
hidrig -d /dev/cu.usbmodemXXXX serve             # owns the link, runs until stopped
hidrig -d /dev/cu.usbmodemXXXX type hi           # auto-routes through the daemon
hidrig stop                                      # stop the daemon
```

While a daemon for a device is running, every `hidrig -d <device> …` one-shot
routes through it automatically (over `POST /send`), so the CLI and the web
console never contend for the port. `paniolo console` starts this daemon on
demand and the dashboard streams keyboard + absolute-mouse events to it — see
[`../docs/hid.md`](../docs/hid.md).

> The control board is a **USB-CDC** device, so there is **no baud
> negotiation** — USB sets the real rate and the nominal "baud" is ignored.
> (The retired single-board UART path used a 115200→460800 negotiation; that is
> gone.)

### DUT power and serial console

`hidrig power off|on|cycle [secs]` switches the DUT through the control board's
relay; it surfaces to paniolo as a normal power-helper behind the `power` hook,
and a `cycle` acks immediately, then the board holds power off for the given
seconds (firmware default 2 s).

When the `serve` daemon runs, it also bridges the DUT's serial console (control
board UART0) and **re-exports it as a PTY**, so paniolo's existing `serial`
channel attaches with no special handling. The daemon publishes a stable symlink
and records it in its discovery file; point a `serial` channel's `device =` at
that path (`/tmp/paniolo-<uid>/hid/console`):

```toml
[[targets.pi5.serial]]
name   = "console"
device = "/tmp/paniolo-501/hid/console"   # the hidrig daemon's console PTY
baud   = 115200                            # nominal; the UART rate is fixed in firmware
# no power_sense_signal — a PTY has no modem-control lines (use `hidrig power` instead)
```

Then `paniolo serial watch/connect/send/log` work as usual. The console exists
only while the daemon is serving, so bring the hid daemon up first (any
`paniolo hid …` or `paniolo console` for the target starts it). A PTY has no
DTR/CTS, so `serial dtr`/`reset` and `power_sense_signal` don't apply.

> **Status:** the console bridge and relay are not yet hardware-verified — in
> particular the PTY round trip through `tio`/serialcap should be confirmed on
> the bench before relying on it (design §6).

## paniolo integration

paniolo calls the tool through the generic per-target `hid` channel — an opaque
command prefix, exactly like the power hooks:

```bash
paniolo hid set -t pi5 --cmd "hidrig -d /dev/cu.usbmodemXXXX"
paniolo hid send -t pi5 type hello
paniolo hid send -t pi5 key ENTER
```

`paniolo hid send` appends its arguments to the configured command and runs it
on whichever control host owns the channel (transparently over SSH for remote
hosts). See [`../docs/hid.md`](../docs/hid.md).

## Host testing tools (macOS)

To verify the full pipeline end-to-end, plug the **target** board's USB into
the same Mac that drives the control link and capture its HID reports while you
inject. Build with `cd hidrig/host && make`.

**`host/hid_capture_usb.m` — leak-safe capture (use this one).** Takes the
target board away from the macOS HID stack entirely via IOUSBHost whole-device
capture (`IOUSBHostObjectInitOptionsDeviceCapture`, root passes the gate), then
prints each interrupt-IN report with timestamps. Because the device is
detached, injected keystrokes and mouse moves reach **only** this tool — they
never touch the focused app or the real cursor.

```bash
sudo ./hid_capture_usb            # defaults to the injector serial
sudo ./hid_capture_usb <serial>   # if more than one KB2040 is attached
```

In another terminal: `hidrig -d /dev/cu.usbmodemXXXX moveabs 16383 16383` and
watch the report bytes. **Start the capture tool before injecting** or the
reports leak into your live session.

> `host/hid_seize_reports.c` (the older `IOHIDDeviceOpen(..SeizeDevice)` tool)
> is **non-exclusive** on modern macOS (Darwin 24/25): the seize succeeds and
> reports arrive, but the system event path is not detached, so injected moves
> still move the real cursor. Keep it only as a passive raw-report tap; use
> `hid_capture_usb` when you need true exclusivity.

`host/hid_bench.py` measures latency/throughput and `host/leak_check.py`
asserts no cursor leak — both via `uv run --with pyserial …`. See
`host/README.md`.

### macOS serial latency

The host drops the macOS serial read-latency timer (`IOSSDATALAT`) to its floor
when it opens the control CDC endpoint (`proto.rs`). The default timer adds
~230 ms to a control-frame round trip (`ping`/`version`); HID frames are
fire-and-forget so they don't pay it, but the floor keeps liveness checks
prompt. A mouse move injects in ~8 ms (the target's USB interrupt endpoint's
8 ms `bInterval` is then the floor).

## Files

```
hidrig/
  src/main.rs       # `hidrig` CLI: one-shots, `run`, `serve`/`stop`
  src/compose.rs    # HID composition: command vocabulary -> report bytes -> binary frames
  src/proto.rs      # control-link transport (binary frames over CDC) + command-file parser
  src/uart.rs       # daemon: the single control-link owner (HID/control + console demux)
  src/pty.rs        # daemon: PTY that re-exports the DUT console into paniolo's serial channel
  src/server.rs     # daemon: axum WebSocket /hid + POST /send
  src/daemon.rs     # daemon: lock, discovery file, console PTY symlink, lifecycle
  firmware/dual/control/   # control board: routes by type byte; UART console bridge; power relay
  firmware/dual/target/    # target board: I2C1 peripheral -> USB-HID send_report (dumb relay)
  firmware/dual/host_send.py  # dependency-free poke tool (push raw frames to the control board)
  firmware/dual/README.md     # firmware bring-up runbook
  host/hid_capture_usb.m   # macOS leak-safe HID capture (IOUSBHost device-capture)
  host/hid_seize_reports.c # macOS passive raw-report tap (non-exclusive on Darwin 24/25)
  host/hid_bench.py        # latency/throughput bench
  host/leak_check.py       # asserts injection does not leak to the live session
  host/Makefile
  host/README.md
```

## History

The **first** dual-board version (`hidrig/control/`, `hidrig/target/` in git
history) used a role-based design where the boards parsed commands and held
duplicated opcode tables. That was replaced by a **single-board** rig: one
KB2040 running "smart" CircuitPython firmware that spoke the line-based HID
serial protocol over a UART (via a USB-serial adapter) and composed HID with
`adafruit_hid`. The single-board firmware still lives in
`firmware/{boot,code,config}.py`.

The **current** design returns to two boards but as a **dumb pipe**:
composition moved to the Rust host (`src/compose.rs`), the firmware relays raw
report bytes, and the host↔rig wire became the binary frame format above. No
duplicated opcode tables, no `adafruit_hid` on the target, and the external
command vocabulary is unchanged (design §7). The single board can later be
rebuilt as a dumb device on the same Rust composition, with frames over its
UART.
