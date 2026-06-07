# Power control

paniolo provides two power control mechanisms:

- **DTR via FTDI** — drives the target's J2 power button header directly over the
  serial cable. Generic and wiring-based; no external services required.
- **Generic power hooks** — four optional shell commands (`on_cmd`, `off_cmd`,
  `cycle_cmd`, `state_cmd`) wired via `paniolo power set`. Write any command
  or point to a standalone helper binary; paniolo calls it via `sh -c`.

**Design principle:** device-specific control logic never goes in the core
crates. It lives in standalone helper binaries wired in via these generic
hooks. The `cambrionix` helper described below is the canonical example.
To add support for new power-switching hardware, follow the
[power-helper recipe](adding-power-helpers.md).

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
paniolo serial add console -t target-machine \
    --device /dev/tty.usbserial-0001 \
    --baud 115200 \
    --sense cts             # whichever modem-control input is wired

# Tell the target which interface to use as the default for power commands
paniolo power set -t target-machine --serial-interface console
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

If no `-i` is given, DTR commands use `serial_interface` from the target's
power channel. If that's not set, they fall back to the target's only configured
serial interface (or fail if multiple are configured without an explicit choice).

---

## Generic power hooks

For cases where DTR isn't wired (or where you want full software-defined
control), configure one or more shell-command hooks on the target's power
channel. All four are optional and independent:

```bash
paniolo power set -t <target> \
    [--cycle-cmd <cmd>]   \   # paniolo power-cycle
    [--on-cmd    <cmd>]   \   # paniolo power on
    [--off-cmd   <cmd>]   \   # paniolo power off
    [--state-cmd <cmd>]   \   # paniolo power-state (stdout: "on" or "off")
    [--serial-interface <name>]   # default interface for DTR commands
    [--host <labhost>]
```

Each hook is run via `sh -c <cmd>`. Exit code determines success or failure.
Hooks can be any shell command, script path, or standalone helper binary.

### Commands backed by hooks

```bash
paniolo power on  [target]        # run on_cmd; error with config hint when unset
paniolo power off [target]        # run off_cmd; error with config hint when unset
paniolo power-cycle [target]      # run cycle_cmd
paniolo power-state [target]      # state_cmd if set; else serial sense-line
```

**`power-state` precedence:** if `state_cmd` is set, paniolo runs it and reads
the first whitespace-delimited token of its stdout. The token must be `on` or
`off` (case-insensitive); any other output is an error. If `state_cmd` is not
set, paniolo falls back to the existing serial sense-line path (requires the
sense signal to be wired and the serialcap daemon to be running).

### `paniolo doctor` hook probing

`paniolo doctor` probes every hook whose value is an absolute path with
`test -e` (over SSH for remote hosts) and reports which hooks are configured
by name, e.g. `cycle_cmd,on_cmd,off_cmd,state_cmd`.

### Example: Home Assistant script (cycle_cmd)

The following shows `cycle_cmd` wired to a Home Assistant API — a valid
generic-hook example that doesn't require any device-specific helper:

```bash
paniolo power set -t target-machine \
    --cycle-cmd /Users/you/.config/paniolo/scripts/power-cycle-target-machine.sh
```

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

Runs `cycle_cmd` and exits with its return code. No built-in timing or
sense-signal logic — the script is responsible for the full sequence.

---

## Cambrionix hub control

The `cambrionix` standalone binary drives a Cambrionix USB hub's control UART
(115200 8N1, `>>` prompt, commands `mode c|s|o <port>` / `state`). It wires
cleanly into paniolo's generic power hooks.

### Installation

`cambrionix` is built and installed by `make install` / `paniolo setup`
alongside the other crates. It lands in the private libexec dir
(`~/.local/libexec/paniolo/bin`), not on PATH — hook strings still reference
it by bare name (paniolo resolves libexec first); to run it by hand, go
through `paniolo helper cambrionix …`.

### Commands

```bash
cambrionix -d <device> state              # table of all ports (volts, mA, attach/mode)
cambrionix -d <device> state <port>       # print exactly "on" or "off" (state_cmd contract)
cambrionix -d <device> on <port>          # mode c (charging/on)
cambrionix -d <device> off <port>         # mode o (off)
cambrionix -d <device> cycle <port> [--delay-ms 3000]
                                          # off → delay → restore prior mode → confirm on
```

Ports 1–15 are accepted. Port 0 is the hub's own host/system row (read-only in
the table output). `cycle` restores the previous mode: Sync (`s`) if it was
Sync, otherwise charging (`c`).

### Wiring into paniolo power hooks

```bash
paniolo power set -t pi5 \
    --cycle-cmd "cambrionix -d /dev/cu.usbserial-DK0F9LZI cycle 4" \
    --on-cmd    "cambrionix -d /dev/cu.usbserial-DK0F9LZI on 4" \
    --off-cmd   "cambrionix -d /dev/cu.usbserial-DK0F9LZI off 4" \
    --state-cmd "cambrionix -d /dev/cu.usbserial-DK0F9LZI state 4"
```

This example wires a Raspberry Pi 5 powered from hub port 4, with the hub's
control UART on `/dev/cu.usbserial-DK0F9LZI`. After this config,
`paniolo power on pi5`, `paniolo power off pi5`, `paniolo power-cycle pi5`,
and `paniolo power-state pi5` all work without further setup.

---

## Zigbee smart plug control (zigplug)

The `zigplug` standalone helper switches Zigbee smart plugs through a
CC2652-based coordinator dongle (e.g. Sonoff ZBDongle-P) using
[zigpy-znp](https://github.com/zigpy/zigpy-znp). Like `cambrionix`, it wires
into paniolo's generic power hooks. Device interview data persists in a
sqlite DB at `~/.config/paniolo/helpers/zigplug/zigbee.db` (`--db` to
override; a DB at the pre-0.3 top-level location is migrated automatically).

**Operations run through a persistent daemon** that owns the coordinator
session. The CLI auto-spawns it on first use and proxies transparently, so
hook strings stay plain one-shot commands. This is not an optimization but a
correctness requirement, learned the hard way:

- **Opening the serial port resets the chip.** The CP2102N's DTR/RTS lines
  drive the stick's auto-bootloader circuit on every open; depending on the
  line states at reset-sampling time the chip occasionally boots into the
  bootloader instead of the app, and the session hangs forever.
- **Concurrent one-shots collide.** Two invocations interleaving frames on
  one stateful ZNP session wedge the coordinator (hardware-verified: a
  pile-up of stuck `power-state` hooks wedged the dongle for hours and cost
  the formed network its NVRAM).

The daemon opens the port once, serializes every operation on one session,
and bounds each with a hard timeout — a sick radio yields a fast error, never
a hung power hook. It follows the standard daemon contract
(`/tmp/paniolo-<uid>/zigplug/daemon.json`, localhost HTTP, OS-assigned port)
and shows up in `paniolo daemons`. Manual control: `zigplug serve` / `stop` /
`status`; `--no-daemon` forces the legacy direct path (debugging only).

### Installation

`zigplug` is a Python project (`zigplug/`), installed by `paniolo setup` /
`make install` as a uv tool when `uv` is on PATH. Its shim lands in the
private libexec dir (`~/.local/libexec/paniolo/bin`), not on PATH — hook
strings still use the bare name; run it by hand via `paniolo helper
zigplug …`.

```bash
# manual equivalent
UV_TOOL_BIN_DIR=~/.local/libexec/paniolo/bin uv tool install --force ~/src/paniolo/zigplug
```

### One-time setup: form the network

```bash
paniolo helper zigplug -d /dev/cu.usbserial-XXXX form              # channel picked by energy scan
paniolo helper zigplug -d /dev/cu.usbserial-XXXX form --channel 25 # or explicit (25-26 avoid Wi-Fi)
```

`form` is idempotent — if the dongle already has a network it prints the
existing channel/PAN and exits.

**If formation fails with "too much RF interference":** put the dongle on a
USB 2.0 extension cable away from USB 3.x ports/hubs and video-capture
devices. This is a real, hardware-verified failure mode — radiated USB noise
desensitizes the CC2652 radio enough that the coordinator refuses to start on
any channel. A factory reset of stale dongle state is
`python -m zigpy_znp.tools.nvram_reset <device>` (run from the `zigplug/`
project venv), but cable placement is almost always the actual fix.

### Pairing plugs

```bash
paniolo helper zigplug -d <device> permit --time 120   # open a join window
# put the plug in pairing mode (hold button until LED blinks; factory-fresh
# plugs usually enter pairing mode on first power-up)
paniolo helper zigplug -d <device> list                # IEEE, NWK, manufacturer, model, state
```

`permit` prints each join and interview as it happens and exits non-zero if
nothing paired. Plugs previously paired to another hub need a full factory
reset (often a ~10 s button hold), not just pairing mode.

### Commands

As hook strings (or after `paniolo helper` when run by hand):

```bash
zigplug -d <device> list                  # table of joined plugs + live state
zigplug -d <device> state <ieee>          # print exactly "on" or "off" (state_cmd contract)
zigplug -d <device> on <ieee>             # switch on, confirm by read-back
zigplug -d <device> off <ieee>            # switch off, confirm by read-back
zigplug -d <device> cycle <ieee> [--delay-ms 3000]
                                          # off → delay → on → confirm
zigplug -d <device> remove <ieee>         # unpair (ZDO leave + forget)
zigplug -d <device> serve|stop|status     # daemon lifecycle (serve is automatic)
zigplug -d <device> backup [-o FILE]      # network backup (key, counters) as JSON
zigplug -d <device> restore [-i FILE]     # write a backup into coordinator NVRAM
```

IEEE addresses are accepted with or without `:`/`-` separators.

### Coordinator NVRAM recovery (backup/restore)

zigpy automatically snapshots the full network state — PAN, channel, network
key, frame counters — into the device DB on every session. If the
coordinator's NVRAM is lost or corrupted (symptom: `coordinator has no
Zigbee network` on a previously formed dongle), the network is recoverable
**without re-pairing**:

```bash
paniolo helper zigplug -d <device> stop      # restore needs the port exclusively
paniolo helper zigplug -d <device> restore   # newest auto-backup from zigbee.db
paniolo helper zigplug -d <device> list      # verify the plugs answer
```

`restore` bumps the network-key frame counter (`--counter-increment`, default
10000) past anything the old coordinator could have transmitted, so joined
devices accept the restored coordinator. A plug that spent hours orphaned may
not answer until it rescans — power-cycling the plug at the wall forces an
immediate rejoin (note: whatever it powers cycles with it). Keep an off-host
copy with `zigplug backup -o <file>` if the bench matters.

### Wiring into paniolo power hooks

```bash
paniolo power set -t target-machine \
    --cycle-cmd "zigplug -d /dev/cu.usbserial-XXXX cycle ff:ff:b4:0e:06:04:ea:b7" \
    --on-cmd    "zigplug -d /dev/cu.usbserial-XXXX on    ff:ff:b4:0e:06:04:ea:b7" \
    --off-cmd   "zigplug -d /dev/cu.usbserial-XXXX off   ff:ff:b4:0e:06:04:ea:b7" \
    --state-cmd "zigplug -d /dev/cu.usbserial-XXXX state ff:ff:b4:0e:06:04:ea:b7"
```

Concurrency and latency are handled by the daemon: the first hook spawns it
(a few seconds), after which operations answer in about a second, concurrent
hooks serialize safely on its single session, and every operation has a hard
timeout — a wedged radio fails a hook fast instead of hanging it. `form`,
`restore`, and `backup` (when no daemon runs) open the port directly and
refuse to run while the daemon does (`zigplug stop` first).

---

## Per-port USB hub power control (usbhub)

The `usbhub` standalone helper switches VBUS on individual ports of
off-the-shelf USB hubs by issuing hub-class control requests
(`SET_FEATURE`/`CLEAR_FEATURE` `PORT_POWER` — the same mechanism as
[uhubctl](https://github.com/mvp/uhubctl)), in pure Rust via
[nusb](https://crates.io/crates/nusb). No kernel driver is displaced and no
interface is claimed; it works on macOS and Linux.

It differs from uhubctl in how hubs and ports are addressed:

- **Hubs are addressed by model profile, not bus location.** A profile
  describes the product's internal chip cascade (consumer hubs are usually
  2–3 cascaded hub chips, each appearing twice: a USB 3 device and its USB 2
  companion). Resolution is signature-first: the profile's chip tree is
  matched against the live topology, so the hook string survives replugging
  the hub into a different host port. Only when several identical hubs share
  one host is an `--at` pin needed (the ambiguity error prints ready-to-paste
  pins).
- **Ports are addressed by the physical silkscreen number.** The profile maps
  each physical port to its (chip, chip-port) location on both the USB 3 and
  USB 2 sides; `on`/`off`/`cycle` act on both sides, since cutting only one
  lets the device fall back to the other, still powered.
- **Switching is refused unless a human has verified the port.** Hub chips
  routinely claim per-port power switching with no VBUS MOSFETs behind it
  (the port "turns off" in the status word while the device keeps drawing
  5 V). A profile port entry needs `controllable = true` — recorded by the
  `learn` verification flow below, or hand-written by someone who physically
  verified — before `on`/`off`/`cycle` will act. `controllable = false`
  entries refuse with the recorded reason; unlisted ports refuse outright.

### Installation

`usbhub` is built and installed by `make install` / `paniolo setup` alongside
the other crates, into the private libexec dir. Run it by hand via
`paniolo helper usbhub …`.

It also builds and runs entirely standalone — useful for sharing the hub
control without the rest of paniolo (its own
[README](https://github.com/curtisgalloway/paniolo/tree/main/usbhub) covers
this audience):

```bash
cargo install --git https://github.com/curtisgalloway/paniolo usbhub
```

Standalone, profiles and learn sessions go under `$USBHUB_STATE_DIR`, else
`$XDG_CONFIG_HOME/usbhub`, else `~/.config/usbhub`. Under paniolo, the
helper honors `$PANIOLO_STATE_DIR` like every other helper, so its state
lives in `~/.config/paniolo/helpers/usbhub/` — no change to the paniolo path.

On Linux, sending control requests to a hub needs write access to its
`/dev/bus/usb/...` node: install a udev rule (uhubctl's rule works — match
the hub VID or `bDeviceClass==09` and grant your bench group write access),
or run as root. macOS needs nothing.

### Commands

```bash
usbhub probe                              # read-only topology dump: every hub, its
                                          # claimed switching mode, and port chains
usbhub models                             # list known model profiles
usbhub --model <m> status                 # per-port table: mappings, assertions, live bits
usbhub --model <m> state <port>           # print exactly "on" or "off" (state_cmd contract)
usbhub --model <m> on <port>              # switch + read-back confirm (refused if unverified)
usbhub --model <m> off <port>
usbhub --model <m> cycle <port> [--delay-ms 3000]
```

Profiles live in `<state-dir>/profiles/<model>.toml` (state-dir resolution
above; `~/.config/paniolo/helpers/usbhub/profiles/` under paniolo,
`~/.config/usbhub/profiles/` standalone). `--profile-dir` overrides per
command.

### Building a profile: the learn workflow

A profile is built at the bench with `usbhub learn` — a resumable session of
discrete steps. The division of labor: **the tool observes enumeration, the
human observes physics.** The tool can see a device appear at chip port 2, and
after cutting power it re-enumerates to see whether that device dropped off the
bus — but it cannot see whether VBUS actually dropped (a self-powered device,
or a data-only disconnect, vanishes from the bus without losing power). So the
tool offers the enumeration result as a hint, and every controllability record
still comes from a human watching the probe lose power.

Agent-driven (each step prints what happened and a `Next:` line):

```bash
usbhub learn start            # snapshot; then unplug the hub
usbhub learn unplugged        # snapshot; then plug the hub back in
usbhub learn plugged          # diff → the hub's full chip cascade, both sides
usbhub learn port 7           # then plug the probe into physical port 7;
                              # blocks until seen (--timeout-secs 120)
usbhub learn verify 7         # cuts power; look at the probe device
usbhub learn verify 7 --result dead         # probe died → controllable
usbhub learn verify 7 --result alive --reason "ganged rail"   # → not controllable
usbhub learn status           # progress + suggested next step
usbhub learn finish --model rsh-st10c-6     # write the profile, print wiring
usbhub learn abort            # discard (restores power if a verify is pending)
```

Human-driven: `usbhub learn run` wraps the same steps in an interactive TTY
loop, so a session started by an agent can be finished by hand and vice versa.
Its `learn>` prompt takes the **same commands as `usbhub learn <cmd>`** (the
`usbhub learn` prefix optional, names abbreviatable — `ver 7`), so the printed
`Next:` hints are typeable verbatim; `help` lists them and `quit` saves and
exits.

Probe-device tips:

- A small **USB 3 hub makes the best probe**: it enumerates on both sides at
  once, mapping a physical port's USB 3 and USB 2 locations in one plug. A
  flash drive maps only its own side; re-run `learn port <n>` with an
  other-speed device to fill in the gap (verify refuses single-side mappings
  by default, since cutting one side can leave the device powered via the
  other — `--allow-single-side` when the port genuinely has only one).
- Use a probe **whose power state you can see** — a power LED, or a phone
  (watch the charging indicator; note a phone usually enumerates USB 2.0, so
  it maps only that side). "Did it lose power" is the question, and the verify
  step's enumeration check only narrows it: a probe still on the bus proves
  the port did *not* cut power, but a probe that vanished could be either real
  power loss or a mere data disconnect.
- Ports already occupied by permanent bench fixtures can be mapped by
  unplugging and replugging the fixture itself during `learn port <n>`.

### Wiring into paniolo power hooks

```bash
paniolo power set -t pi5 \
    --cycle-cmd "usbhub --model rsh-st10c-6 cycle 9" \
    --on-cmd    "usbhub --model rsh-st10c-6 on 9" \
    --off-cmd   "usbhub --model rsh-st10c-6 off 9" \
    --state-cmd "usbhub --model rsh-st10c-6 state 9"
```

`learn finish` prints exactly this block for the model it just wrote. Add
`--at usb3=BUS:CHAIN,usb2=BUS:CHAIN` to each command only if several
identical hubs share the control host.

### Gotchas

- **Hub descriptors lie.** `usbhub probe` prints each hub's claimed power
  switching mode as a hint for where verification is worth trying; "per-port
  (claimed)" does not mean the MOSFETs exist. Only the learn verify pass (or
  a multimeter) settles it.
- **USB 3 hubs are two hubs.** Every physical port has independent PORT_POWER
  state on the USB 3 chip and its USB 2 companion. `usbhub` acts on both
  mapped sides by default; `--side usb3|usb2` exists for debugging.
- **Cut power means re-enumeration.** Anything on the port disconnects; a
  port that hosts the control path for the very hub being switched would saw
  off its own branch. Mark such ports `controllable = false` with a reason
  during learn.
- **Port power state does not survive the hub losing power.** Replugging or
  power-cycling the whole hub turns every port back on (chip default).
