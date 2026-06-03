# Power control

paniolo provides two power control mechanisms:

- **DTR via FTDI** — drives the target's J2 power button header directly over the
  serial cable. Generic and wiring-based; no external services required.
- **`power_cycle_cmd`** — runs a configurable shell script. Write any script you
  like (HA switch, PDU relay, GPIO, etc.) and paniolo calls it.

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

## power_cycle_cmd — script-based power control

For cases where DTR isn't wired (or where you want full mains control), set a
shell script on the target:

```bash
paniolo power set -t target-machine \
    --cycle-cmd /Users/you/.config/paniolo/scripts/power-cycle-target-machine.sh
```

The script can do anything — call a Home Assistant API, drive a PDU relay, toggle
a GPIO, etc. paniolo runs it and reports success or failure based on the exit code.

### Example: Home Assistant script

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

Runs `power_cycle_cmd` and exits with its return code. No built-in timing or
sense-signal logic — the script is responsible for the full sequence.
