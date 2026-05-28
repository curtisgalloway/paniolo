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
