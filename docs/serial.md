# Serial console

paniolo supports two serial console modes: interactive (direct terminal via
`tio`) and daemon-backed (serialcap, with a timestamped rolling log and
WebSocket dashboard terminal).

---

## Setup

Add a serial interface to a target:

```bash
paniolo serial add console -t target-machine \
    --device /dev/tty.usbserial-0001 \
    --baud 115200

# Optional: also wire a power sense signal on this interface (see power.md)
paniolo serial set console -t target-machine --sense cts
```

A target can have several named interfaces (e.g. `console`, `bmc`). Remove
one with:

```bash
paniolo serial rm console -t target-machine
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

The serialcap daemon runs **one per target** and owns all of that target's
configured interfaces, provides a WebSocket terminal for the
[dashboard](dashboard.md), and writes a timestamped rolling capture log.
(Each target gets its own daemon, so several targets capture concurrently on
one host.)

```bash
paniolo serial watch [target-machine]   # start serialcap daemon
paniolo serial stop  [target-machine]   # stop it (on the target's host)
```

A target with multiple serial interfaces starts a single daemon that manages
all of them. The daemon's URL is printed on start — it also appears in the
dashboard.

After an upgrade or rebuild, a daemon still running the old binary is flagged
**stale** by `paniolo serial show` and `paniolo daemons`. Re-running
`paniolo serial watch` auto-restarts a stale daemon, or restart it explicitly
with `paniolo daemons restart serialcap` (see [architecture](architecture.md)).

---

## Querying captured output

The capture log persists across daemon restarts. `paniolo serial log` reads it
directly — no daemon round-trip needed.

The target may be positional or `-t`; both it and `-i` can be omitted when
there is only one.

```bash
# Tail the last 50 lines from the default interface
paniolo serial log target-machine -i console --tail 50

# Only lines newer than a previously-seen sequence number (poll mode)
paniolo serial log target-machine -i console --since 1840

# Specific sequence number range
paniolo serial log target-machine -i console --from 1000 --to 1200

# Keep ANSI escape codes (stripped by default)
paniolo serial log target-machine -i console --raw

# JSON Lines output (includes timestamp and sequence number)
paniolo serial log target-machine -i console --json
```

Each captured line carries a monotonic sequence number (`seq`, stable across
log rotation) and a UTC timestamp (`ts_ms`). The `--since` flag polls for lines
with `seq` greater than the last seen value — safe to re-run from scripts.

Each interface writes to its own capture directory so logs never conflate:
`/tmp/paniolo-<uid>/serialcap/<target>/capture/<name>/serial.jsonl`.

---

## Sending input

`paniolo serial send` injects a line of input through the **running daemon**, so
scripted input coexists with capture — no `serial stop`, no exclusive re-open,
and output keeps flowing to `serial log` and the dashboard. (Contrast
`serial connect`, which holds the port exclusively and can't run alongside the
daemon.) The daemon must be running (`paniolo serial watch`).

With two positionals the first is the target (`serial send <target> <text>`);
with one, it's the text and the sole target is implied. `-t` also works.

```bash
# Send a command (a carriage return is appended by default)
paniolo serial send target-machine -i console "iochk --live-dangerously /block/000"

# Send without the trailing carriage return
paniolo serial send target-machine -i console --no-newline "partial"
```

### Pacing a slow console (`--pace-ms`)

A target whose console is **polled** (the CPU only reads the UART RX register when
its loop comes around) with **no hardware flow control** will silently drop input
characters: bytes arrive at the full line rate, the RX FIFO overflows while the
CPU is busy, and the lost bytes are gone. This is common during early bring-up
(e.g. a Zircon polled console).

`--pace-ms` is the substitute for the missing flow control: the daemon drips the
bytes out one at a time, that many milliseconds apart, so each byte is consumed
before the next arrives. ~8 ms/byte is a known-good value for a 115200-baud
polled console.

```bash
# Drip one byte every 8 ms — slow but overflow-proof
paniolo serial send -i console --pace-ms 8 "iochk --live-dangerously /block/000"
```

A paced send of N bytes takes about `N * pace_ms` ms and blocks until the whole
line is written. With `--pace-ms 0` (the default) the line is sent at full rate,
which is fine for an interrupt-driven console or one with flow control wired.

> **Why not RTS/CTS or XON/XOFF instead?** Hardware RTS/CTS *is* the proper fix,
> but it needs the target's UART to enable auto-flow-control and the right pins
> wired (the Pi 5 debug header is TX/RX/GND only — no flow-control pins), so it
> can't be relied on during bring-up. Software flow control (XON/XOFF) is worse:
> emitting XOFF needs the same CPU attention the polled console isn't giving the
> UART, so it can't react in time, and it corrupts binary streams. Pacing depends
> on nothing from the target, so it's the universal floor. RTS/CTS may be layered
> in later as a per-interface opt-in for well-behaved consoles.

Under the hood this is `POST /input?interface=NAME[&pace_ms=N]` on the daemon,
with the raw bytes as the request body (see HTTP API below).

---

## Integration with the video dashboard

`paniolo console` opens the combined hdmicap dashboard in a browser, starting
both daemons if they aren't already running. That page embeds an xterm.js
terminal that connects cross-port to serialcap's WebSocket (`/stream`). The
daemons can also be started individually:

```bash
paniolo video watch [target-machine]    # hdmicap — serves the page
paniolo serial watch [target-machine]   # serialcap — backs the terminal
paniolo console [-i <interface>] # open in browser (auto-starts both)
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

See [power.md](power.md) for wiring diagrams, the generic power hooks
(`cycle_cmd`/`on_cmd`/`off_cmd`/`state_cmd`), and a full command reference.

---

## Runtime paths

| Purpose | Path |
|---|---|
| serialcap discovery | `/tmp/paniolo-<uid>/serialcap/<target>/daemon.json` (`{pid, port, interfaces:[...]}`) |
| serialcap advisory lock | `/tmp/paniolo-<uid>/serialcap/<target>/daemon.lock` |
| serialcap stderr log | `/tmp/paniolo-<uid>/serialcap/<target>/daemon.log` (truncated on each start; shown on start timeout) |
| Capture log (per interface) | `/tmp/paniolo-<uid>/serialcap/<target>/capture/<name>/serial.jsonl(.1..)` |
| Pending (unterminated) line | `/tmp/paniolo-<uid>/serialcap/<target>/capture/<name>/pending.json` |

The serialcap daemon is **per target** (the `<target>` segment); the runtime
base honors `$PANIOLO_RUNTIME_BASE` (default `/tmp`).

---

## HTTP API (serialcap daemon)

All per-interface endpoints take `?interface=NAME`, defaulting to the first
configured interface. Responses carry a permissive CORS header.

| Method | Path | Purpose |
|---|---|---|
| GET | `/stream` | Bidirectional WebSocket: serial output (binary) + client keystrokes |
| GET | `/status` | One interface (`?interface=`) or all; `{name, device, baud, connected, power_on}` |
| GET | `/interfaces` | All interfaces and their status |
| GET | `/devices` | Serial devices on the host |
| POST | `/button` | Pulse DTR for `?ms=N` (J2 power button); see [power.md](power.md) |
| POST | `/input` | Write the request body to the port; `?pace_ms=N` drips one byte per N ms |

`POST /input` writes through the port the daemon already owns, so input coexists
with live capture. A paced write (`pace_ms > 0`) blocks until the whole body is
sent (~`len × pace_ms` ms). Returns 200 on success, 404 for an unknown interface,
503 if the supervisor isn't running.
