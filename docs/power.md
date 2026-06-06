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
