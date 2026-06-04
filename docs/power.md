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

If no `-i` is given, DTR commands use `power_serial_interface` from the target
config. If that's not set, they fall back to the target's only configured
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
alongside the other crates. It lands in `~/.cargo/bin`.

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
