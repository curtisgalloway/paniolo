# Serial console

paniolo has two serial console modes:

| Mode | Command | What you get |
|---|---|---|
| Interactive | `serial connect` | A direct terminal through `tio`, in the foreground |
| Daemon | `serial watch` | The serialcap daemon: a timestamped rolling capture log and a WebSocket terminal in the dashboard |

The port is exclusive, so the two modes conflict. Run one at a time.

---

## Setup

Add a serial interface to a target:

```bash
paniolo serial add console -t target-machine \
    --device /dev/tty.usbserial-0001 \
    --baud 115200

# Optional: also wire a power sense signal on this interface (see power.md)
paniolo serial set console -t target-machine --sense cts

# Optional: declare this interface's DTR line is wired to the J2 power button,
# which enables `serial dtr` / `serial reset` on it (opt-in; see power.md)
paniolo serial set console -t target-machine --power-button
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

## Virtual machine consoles

A VM's serial console is a pseudo-terminal (pty), so it works as a paniolo
serial interface like any other port. Use it to watch a guest while nothing
else can reach it: firmware, the bootloader, an unattended installer, before
SSH or a guest agent exists.

Find the device path. The VM gets a new one on every boot:

```bash
utmctl attach <vm-name>          # UTM: prints "PTTY: /dev/ttys006"
                                 # qemu: -serial pty prints the path at startup
```

Configure it as an ordinary interface and use daemon mode:

```bash
paniolo serial add console -t winvm --device /dev/ttys006 --baud 115200
paniolo serial watch winvm
paniolo serial log winvm --tail 50
```

After a VM restart the pty number changes, so re-point the interface:

```bash
paniolo serial set console -t winvm --device "$(utmctl attach winvm | sed -n 's/^PTTY: //p')"
```

Things to know:

- **`--baud` is recorded but not applied.** A pty has no line rate, and macOS
  sets one with an ioctl that pseudo-terminals reject. serialcap detects a pty
  and opens it without a rate; the daemon logs `is a pty — opening without a
  line rate`. Set the baud your hardware would use; nothing depends on it.
- **`--device` can be a symlink to the pty.** serialcap resolves symlinks
  before checking for a pty, so a stable symlink works the same as the raw
  path. The `hidrig` console bridge publishes a DUT's serial console this way,
  because its pty path changes across daemon restarts (see
  [hid-dual-board-design.md](dev/hid-dual-board-design.md)).
- **Both console modes work.** `serial connect` gives an interactive `tio`
  terminal. `serial watch` gives the capture log and dashboard, which suits an
  install you are waiting on. As with any device, run one at a time.
- **Writing to the console is a real keypress.** Firmware reads the pty as
  console input. On an EDK2 (UEFI firmware) guest, the virt machine's PL011
  UART is registered as ConIn (the UEFI console input), so `serial send`
  answers a boot prompt with no display attached. This gets you past bootmgr's
  "Press any key to boot from CD or DVD" headlessly. The window is short and
  its timing varies by tens of seconds (firmware USB enumeration), so send in a
  loop instead of sleeping a fixed time and sending once:

  ```bash
  for i in $(seq 120); do paniolo serial send winvm " "; sleep 1; done
  ```

- **The input window closes early.** It covers firmware only: bootloader
  menus, the UEFI Shell, boot prompts. On a Windows guest, UEFI stops reading
  ConIn as soon as WinPE starts, which is before Windows Setup runs. Do not plan
  on typing into a guest past its bootloader.

You can also wire a VM's lifecycle to the [power](power.md) hooks
(`on_cmd`/`off_cmd`/`state_cmd`): `utmctl start|stop`, with a `state_cmd` that
translates `utmctl status` into the `on`/`off` the contract requires.

---

## Interactive mode

```bash
paniolo serial connect [-i console] [target-machine]
```

Opens a direct `tio` terminal session in the foreground. Exit with Ctrl+T Q.
It holds the serial port exclusively, so it conflicts with the daemon.

---

## Daemon mode

```bash
paniolo serial watch [target-machine]   # start serialcap daemon
paniolo serial stop  [target-machine]   # stop it (on the target's host)
```

There is **one serialcap daemon per target**. It owns all of that target's
serial interfaces, writes a timestamped rolling capture log, and serves a
WebSocket terminal for the [dashboard](dashboard.md). Because each target has
its own daemon, several targets can capture at once on one host. The daemon
prints its URL on start; the dashboard shows it too.

**Stale daemons.** After an upgrade or rebuild, a daemon still running the old
binary shows as **stale** in `paniolo serial show` and `paniolo daemons`.
`paniolo serial watch` restarts a stale daemon automatically, or run
`paniolo daemons restart serialcap` (see [architecture](dev/architecture.md)).

**Untracked daemons.** An *untracked* daemon is one that outlived its
discovery file, which is paniolo's only record of a running daemon. On Linux
the file sits in `/tmp`, which systemd cleans by age. Debian's stock policy is
`q /tmp 1777 root root 10d`, so a daemon that runs for ten days with no
command against it loses its file. It keeps running and keeps the serial ports,
while `serial show` reports the channel stopped and `serial watch` starts a
replacement that cannot open the port.

paniolo handles this:

- `serial show` reports `running, untracked (pid N)`.
- `paniolo daemons` lists it under **Untracked daemons**.
- `serial watch` and `serial stop` reap it (`SIGTERM`, then `SIGKILL`). Its
  port and token were lost with the file, so a signal is the only handle left.

paniolo matches an untracked daemon to the channel by the devices on its
command line (serialcap's repeated `--interface NAME=DEVICE@BAUD[:SENSE]`).
Holding any one of the target's ports is enough, since that is the port a
replacement would fail to open. See [video.md](video.md) for why the file goes
missing and the `tmpfiles.d` drop-in that prevents it.

---

## Querying captured output

`paniolo serial log` reads the capture log straight from disk, with no daemon
round trip. The log persists across daemon restarts.

The target may be positional or `-t`. You can omit the target and `-i` when
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

Each line carries a monotonic sequence number (`seq`, stable across log
rotation) and a UTC timestamp (`ts_ms`). `--since` returns lines with `seq`
greater than the last value you saw, so scripts can re-run it safely.

**When the daemon is stopped**, `serial log` still works but shows nothing
captured since it stopped. It prints a `warning:` line on stderr and exits 0.
Add `--require-live` to fail with exit 100 (`daemon_down`) instead, when a
stale log would be a wrong answer ([exit codes](errors.md)).

**Pending lines.** The last, unterminated line lives in a pending-line
sidecar. Completed records take precedence over a stale sidecar, including when
applying `--tail` and sequence filters. A pending line can change without its
sequence changing, so advance a polling cursor only past completed records, or
use `--no-pending` when polling with `--since`.

Each interface has its own capture directory, so logs never mix:
`/tmp/paniolo-<uid>/serialcap/<target>/capture/<name>/serial.jsonl`.

---

## Sending input

`paniolo serial send` writes a line of input through the **running daemon**.
Capture keeps going: no `serial stop`, no exclusive re-open, and output still
flows to `serial log` and the dashboard. (`serial connect`, by contrast, holds
the port and cannot run alongside the daemon.) The daemon must be running
(`paniolo serial watch`) and the interface connected.

With two positionals the first is the target (`serial send <target> <text>`).
With one, it is the text and the sole target is implied. `-t` also works.

```bash
# Send a command (a carriage return is appended by default)
paniolo serial send target-machine -i console "iochk --live-dangerously /block/000"

# Send without the trailing carriage return
paniolo serial send target-machine -i console --no-newline "partial"
```

**What success means.** The serial driver accepted every byte. It does not
confirm the target ran the command. A disconnect fails pending writes, and
unsent bytes are not replayed on reconnect. An error can follow partial
delivery, so check the console before retrying.

### Pacing a slow console (`--pace-ms`)

Some consoles are **polled**: the CPU reads the UART's receive register only
when its loop comes around. With **no hardware flow control**, such a console
silently drops input. Bytes arrive at full line rate, the receive FIFO
overflows while the CPU is busy, and the lost bytes are gone. This is common in
early bring-up (e.g. a Zircon polled console).

`--pace-ms` stands in for the missing flow control. The daemon sends the bytes
one at a time, that many milliseconds apart, so each is read before the next
arrives. About 8 ms/byte is a known-good value for a 115200-baud polled
console.

```bash
# Drip one byte every 8 ms — slow but overflow-proof
paniolo serial send -i console --pace-ms 8 "iochk --live-dangerously /block/000"
```

- A paced send of N bytes takes at least `(N - 1) * pace_ms` ms and blocks
  until the driver accepts the whole line.
- Pacing is applied after each successful driver write, so a stalled writer
  cannot build up a burst of queued bytes.
- Requests run in FIFO order, including dashboard input.
- `--pace-ms 0` (the default) sends at full rate. That is fine for an
  interrupt-driven console or one with flow control wired.

> **Why not RTS/CTS or XON/XOFF instead?** Hardware RTS/CTS *is* the proper fix,
> but it needs the target's UART to enable auto-flow-control and the right pins
> wired. The Pi 5 debug header is TX/RX/GND only, with no flow-control pins, so
> it can't be relied on during bring-up. Software flow control (XON/XOFF) is
> worse: sending XOFF needs the same CPU attention the polled console isn't
> giving the UART, so it reacts too late, and it corrupts binary streams.
> Pacing needs nothing from the target, so it always works. RTS/CTS may be added
> later as a per-interface opt-in for well-behaved consoles.

Under the hood this is `POST /input?interface=NAME[&pace_ms=N]` on the daemon,
with the raw bytes as the request body (see [HTTP API](#http-api-serialcap-daemon)).

---

## Integration with the video dashboard

`paniolo console` opens the combined hdmicap dashboard in a browser and starts
both daemons if they aren't running. The page embeds an xterm.js terminal that
connects across ports to serialcap's WebSocket (`/stream`). A target with no
serial channel still opens: you get video (and KVM input, if the target has a
hid channel) with no terminal pane. You can also start the daemons separately:

```bash
paniolo video watch [target-machine]    # hdmicap — serves the page
paniolo serial watch [target-machine]   # serialcap — backs the terminal
paniolo console [-i <interface>] # open in browser (auto-starts both)
```

- **Scrollback.** On WebSocket connect, serialcap replays up to 64 KB of
  scrollback, so the terminal isn't blank mid-session.
- **Input.** Keystrokes in the terminal go to the serial port in real time.
- **Multiple interfaces.** The dashboard shows one terminal pane per interface,
  side by side. `paniolo console -i <name>` opens a single pane pinned to one
  interface.
- **Slow clients.** If a client falls too far behind (a slow network link, a
  busy browser tab), the daemon drops that client's oldest buffered output
  instead of blocking everyone else. Only that client's view is affected, never
  the capture log or other connections. The gap is marked in that client's
  stream, in the same styled, timestamped form as the
  connect/disconnect/button markers:
  `── serial client lagged, dropped N chunks [HH:MM:SS UTC] ──`.
- **Authentication.** The page uses serialcap's own token. `paniolo console`
  reads it from the daemon's discovery file and puts it in the `?serialws=` URL
  it gives the page. serialcap echoes only that loopback origin in its CORS
  header (never `*`), so the cross-port connection needs no proxy and no other
  page can make it.

See [dashboard.md](dashboard.md) for layout options and other URL parameters.

---

## DTR power control (FTDI wiring)

If an FTDI USB-serial adapter is wired to the target's J2 power button header,
the same interface can press the power button by toggling its DTR signal. This
is **opt-in**: the interface must declare `power_button = true`
(`paniolo serial set <iface> --power-button`), or these commands error.

```bash
paniolo serial dtr [--ms 200] [-i console] [target-machine]   # pulse DTR
paniolo serial reset [-i console] [target-machine]             # soft reset (200 ms)
```

> **This is a hardware reset, not a console `reboot`.** "Reboot over the serial
> console" means typing `reboot` into a logged-in shell — `paniolo serial send
> <target> "reboot"` — which is unrelated to the DTR power-button toggle above.

**Errors.** A failed DTR assertion or release returns an error. The daemon
always attempts the release, then closes and reopens a failed serial handle
with DTR deasserted. An error can follow a partial or completed physical press,
so check the target before retrying a power operation.

**A press does not close and reopen the port.** The daemon pulses DTR on the
port it already has open. This matters because the OS raises DTR on open and
drops it on close (Linux's tty core always does). Closing and reopening around
a press would add a second, driver-timed press right after the deliberate one,
and lose anything the target sent while the port was closed. For the same
reason, every open (daemon start, and each reconnect) briefly asserts DTR. The
daemon opens with DTR de-asserted to avoid adding a press, but cannot suppress
the transition the kernel makes during the open call itself.

> This has not been confirmed against real target hardware. To verify: watch
> the DTR line with a scope or an LED across a `paniolo serial dtr` call —
> there should be exactly one pulse, not two — or, if `--sense` is wired,
> watch `power_on` in `paniolo serial show`/`GET /status` settle once rather
> than flickering.

See [power.md](power.md) for wiring diagrams, the generic power hooks
(`cycle_cmd`/`on_cmd`/`off_cmd`/`state_cmd`), and a full command reference.

---

## Runtime paths

| Purpose | Path |
|---|---|
| serialcap discovery | `/tmp/paniolo-<uid>/serialcap/<target>/daemon.json` (`{pid, port, token, interfaces:[...]}`; owner-only, it holds the token) |
| serialcap advisory lock | `/tmp/paniolo-<uid>/serialcap/<target>/daemon.lock` |
| serialcap stderr log | `/tmp/paniolo-<uid>/serialcap/<target>/daemon.log` (truncated on each start; shown on start timeout) |
| Capture log (per interface) | `/tmp/paniolo-<uid>/serialcap/<target>/capture/<name>/serial.jsonl(.1..)` |
| Pending (unterminated) line | `/tmp/paniolo-<uid>/serialcap/<target>/capture/<name>/pending.json` |

The `<target>` segment is there because the daemon is per target. The runtime
base honors `$PANIOLO_RUNTIME_BASE` (default `/tmp`).

---

## HTTP API (serialcap daemon)

All per-interface endpoints take `?interface=NAME`, defaulting to the first
configured interface.

**Every request needs the daemon's token.** The daemon makes a fresh one on
each start and publishes it as `token` in its discovery file, readable only by
the operator's uid. Send it as `Authorization: Bearer <token>` or as a
`?token=<token>` query parameter (the form the dashboard's WebSocket uses).

The daemon also requires a loopback `Host` and, when a browser sends one, a
loopback `Origin`. So a web page in your browser cannot reach it even though it
listens on 127.0.0.1. No token gets 401; a foreign Host or Origin gets 403.
By hand:

```bash
d=/tmp/paniolo-$(id -u)/serialcap/target-machine/daemon.json
curl -s -H "Authorization: Bearer $(jq -r .token "$d")" \
    "http://127.0.0.1:$(jq -r .port "$d")/status"
```

A daemon started by a paniolo older than the token has none and accepts
unauthenticated requests; `paniolo daemons restart --stale` replaces it.

`serialcap stop` uses the authenticated `POST /stop`, so a stale discovery
file cannot make it kill an unrelated process that reused the PID. It never
falls back to signaling the PID. To replace an older daemon that lacks this
endpoint, run `paniolo daemons stop serialcap`, then start capture again.

| Method | Path | Purpose |
|---|---|---|
| POST | `/stop` | Authenticated daemon shutdown; never signals a discovery-file PID |
| GET | `/stream` | Bidirectional WebSocket: serial output (binary) + client keystrokes |
| GET | `/status` | One interface (`?interface=`) or all; `{name, device, baud, connected, power_on}` |
| GET | `/interfaces` | All interfaces and their status |
| GET | `/devices` | Serial devices on the host |
| POST | `/button` | Pulse DTR for `?ms=N`; 503 if assertion or release fails; see [power.md](power.md) |
| POST | `/input` | Write the request body to the port; `?pace_ms=N` drips one byte per N ms |

**`POST /input`** writes through the port the daemon already owns, so input
coexists with live capture.

- Paced and unpaced requests both wait until the driver accepts all bytes.
  Paced writes wait at least `pace_ms` between driver writes; reads and DTR
  requests are still serviced meanwhile.
- The body is capped at 64 KiB and `pace_ms` at 10 000.
- Returns 200 on completion, 400 for a pace past the ceiling, 404 for an
  unknown interface, 413 for an oversized body, and 503 for a disconnected
  interface or interrupted write.
- A failed write may have partly reached the target. Remaining bytes are
  discarded, not replayed after reconnect.

`/stream` messages from the client are also capped at 64 KiB and go through
the same FIFO writer. A WebSocket write failure closes that connection.
