# Power control

paniolo provides two power control mechanisms:

- **DTR via FTDI** — drives the target's J2 power button header directly over the
  serial cable. Generic and wiring-based; no external services required. It is
  **opt-in per serial interface** (`power_button = true`): wiring the FTDI DTR
  line to J2 is the rare exception, so `serial dtr` / `serial reset` refuse any
  interface that hasn't declared it (rather than toggle a possibly-unwired line).
- **Generic power hooks** — four optional shell commands (`on_cmd`, `off_cmd`,
  `cycle_cmd`, `state_cmd`) wired via `paniolo power set`. Write any command
  or point to a standalone helper binary; paniolo calls it via `sh -c`.

**Design principle:** device-specific control logic never goes in the core
crates. It lives in standalone helper binaries wired in via these generic
hooks. The `cambrionix` helper described below is the canonical example.
To add support for new power-switching hardware, follow the
[power-helper recipe](dev/adding-power-helpers.md).

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
# Add a serial interface. --power-button declares that this interface's DTR line
# is wired to the J2 power button (this is what enables `serial dtr`/`reset`);
# --sense records the modem-control input wired for power sensing (optional).
paniolo serial add console -t target-machine \
    --device /dev/tty.usbserial-0001 \
    --baud 115200 \
    --power-button \
    --sense cts

# Only needed when a target has MORE THAN ONE power_button interface: pick which
# one DTR commands default to.
paniolo power set -t target-machine --serial-interface console
```

To enable (or revoke) DTR on an interface you added earlier:

```bash
paniolo serial set console -t target-machine --power-button         # enable
paniolo serial set console -t target-machine --power-button false   # revoke
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

**DTR is opt-in.** `serial dtr` / `serial reset` only act on an interface whose
`power_button = true`. Interface resolution is: an explicit `-i`, else the power
channel's `serial_interface`, else the sole `power_button` interface. If the
chosen interface hasn't opted in — or none has — the command **errors** with a
hint (it never falls back to a lone console, which might be unwired and would
silently no-op) pointing at the target's real power method and the
console-reboot path.

> **"Reboot over serial" is not a DTR reset.** Typing `reboot` at a logged-in
> serial console is a *software* reboot: `paniolo serial send <target>
> "reboot"`. `serial reset` / `serial dtr` are a *hardware* DTR power-button
> toggle that needs the J2 wiring above. When a request says "use serial to
> reboot" without naming DTR / the wire / the power button, default to the
> console `reboot` (or the configured `paniolo power-cycle`) — not DTR.

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
    [--serial-interface <name>]   # default DTR interface when several opt in
    [--host <labhost>]
```

Each hook is run via `sh -c <cmd>`. Exit code determines success or failure.
Hooks can be any shell command, script path, or standalone helper binary.
Besides the dedicated helpers documented below (`cambrionix`, `zigplug`,
`shellyplug`, `amt`), the dual-board `hidrig` control board can
switch a DUT power relay (`hidrig power off|on|cycle`) behind these same
hooks — one USB device for HID, console, and power (see
[`hidrig/README.md`](https://github.com/curtisgalloway/paniolo/blob/main/hidrig/README.md)).

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

`paniolo doctor` probes every hook (over SSH for remote hosts): an absolute
path with `test -e`, a bare name with `command -v` under the same resolution
the hooks get at runtime — the per-user libexec dir
(`~/.local/libexec/paniolo/bin`), then the system package dir
(`/usr/libexec/paniolo/bin`), then PATH. It reports which hooks are configured
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
cambrionix -d <device> on <port>          # mode c (charging/on), confirm by read-back
cambrionix -d <device> off <port>         # mode o (off), confirm by read-back
cambrionix -d <device> cycle <port> [--delay-ms 3000]
                                          # off → confirm → delay → restore prior mode → confirm
```

Ports 1–15 are accepted. Port 0 is the hub's own host/system row (read-only in
the table output). `cycle` restores the previous mode: Sync (`s`) if it was
Sync, otherwise charging (`c`).

Every transition is confirmed by re-reading the port table — the hub accepts
a `mode` command without acknowledging it, so the read-back is the only
evidence it took. `on` requires the port to report mode `C` or `S`, `off`
requires `O`, and `cycle` checks both phases; a mismatch exits non-zero
instead of printing success. `state <port>` maps `C`/`S`/`I` to `on` and `O`
to `off`, and errors on any other mode letter rather than guessing. Each
response is bounded (3 s, 64 KiB) and a line the hub flags as an error fails
the command, so a `-d` that points at some other chatty UART fails fast
instead of hanging the hook.

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
and shows up in `paniolo daemons`. The discovery file also carries a per-run
bearer token (file mode 0600) that every request must present, so nothing
else on the host — another user, a web page — can drive the coordinator
through the daemon. Manual control: `zigplug serve` / `stop` / `status`
(`stop` and `status` need no `-d`); `--no-daemon` forces the legacy direct
path (debugging only).

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
zigplug -d <device> serve                 # start the daemon by hand (automatic otherwise)
zigplug stop                              # stop the daemon (no -d: one daemon per host)
zigplug status                            # daemon + network status (no -d)
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

## Shelly smart plug control (shellyplug)

The `shellyplug` standalone helper switches **Shelly Gen2+ smart plugs and
relays** (Plus, Pro, Gen3, Gen4) over each device's **local HTTP RPC API** —
no cloud account, no Home Assistant, no Matter controller. Pure Rust via
[ureq](https://crates.io/crates/ureq). Because the transport is a stateless
HTTP request/response, it is a plain **one-shot** helper (no daemon, unlike
`zigplug`): each invocation makes one `GET /rpc/<Method>` call and exits.

- **Supported:** Gen2/3/4 devices, which speak the JSON-RPC API
  (`Switch.Set`, `Switch.GetStatus`, `Shelly.GetDeviceInfo`). The original
  Gen1 devices use a different REST API (`/relay/0?turn=on`) and are **not**
  supported.
- **Auth:** only devices with authentication **disabled** (`auth_en: false`,
  the factory default) are supported for now. An auth-enabled device answers
  HTTP 401; the helper reports that clearly rather than guessing.

### Installation

`shellyplug` is built and installed by `make install` / `paniolo setup`
alongside the other crates, into the private libexec dir
(`~/.local/libexec/paniolo/bin`), not on PATH — hook strings reference it by
bare name (paniolo resolves libexec first). Run it by hand via
`paniolo helper shellyplug …`.

### Addressing

- **`-d <host>`** is the device's address on your network: a bare IP or
  hostname (`10.0.0.5`, `shelly.local`), optionally with a scheme or port
  (`http://10.0.0.5:8080`). A Shelly advertises an mDNS name like
  `shellyplugusg4-<mac>.local`; either pin its IP with a **DHCP reservation**
  or use that `.local` name in the hook string so a DHCP lease change doesn't
  break the hook.
- **`[id]`** is the switch component id, default `0`. Single-outlet plugs only
  have switch `0`; multi-channel devices (e.g. a Pro 4PM) use `0..N`.

### Commands

```bash
shellyplug -d <host> status [id]          # device info + switch state and power metering
shellyplug -d <host> state  [id]          # print exactly "on" or "off" (state_cmd contract)
shellyplug -d <host> on     [id]          # switch on, confirm by read-back
shellyplug -d <host> off    [id]          # switch off, confirm by read-back
shellyplug -d <host> cycle  [id] [--delay-ms 3000]
                                          # off → confirm → delay → on → confirm
```

### Wiring into paniolo power hooks

```bash
paniolo power set -t target-machine \
    --cycle-cmd "shellyplug -d 10.0.0.5 cycle 0" \
    --on-cmd    "shellyplug -d 10.0.0.5 on 0" \
    --off-cmd   "shellyplug -d 10.0.0.5 off 0" \
    --state-cmd "shellyplug -d 10.0.0.5 state 0"
```

After this, `paniolo power on/off`, `paniolo power-cycle`, and
`paniolo power-state` drive the plug with no further setup.

### Gotchas

- **macOS Local Network privacy gates the helper (the big one).** Every other
  paniolo helper talks over a serial port or to a `127.0.0.1` daemon —
  loopback is exempt from macOS's Local Network privacy gate. `shellyplug` is
  the **first helper to reach a device on the LAN**, so it is the first to hit
  that gate. On macOS (Sequoia and later) access to the local subnet is
  permitted **per-binary**, attributed to the controlling app. Symptom: the
  helper fails with **`No route to host` (EHOSTUNREACH)** even though a browser
  and `curl` reach the same device fine — Apple-signed system binaries like
  `curl` are exempt; a freshly built `shellyplug` is not. Fix: grant the app
  that launches the hook **Local Network** access (System Settings → Privacy &
  Security → Local Network — enable your terminal, e.g. iTerm2/Terminal). The
  first LAN access from that app usually triggers the one-time prompt. The
  binary connecting to the public internet but not the LAN is the tell.
- **A plug's IP can change.** Use a DHCP reservation or the device's `.local`
  mDNS name in the hook string (see Addressing).
- **`state` is cheap and honest.** It reads `Switch.GetStatus` live every call
  (no caching) and fails loudly if the device is unreachable, rather than
  reporting a stale guess — it is the hook agents poll.

---

## Intel AMT power control (amt)

The `amt` standalone helper switches **Intel AMT (vPro) machines** over
**WS-Management** (SOAP over HTTP on port 16992) — no smart plug at all: the
power switch is the machine's own Management Engine, reached over the regular
network. Pure Rust via [ureq](https://crates.io/crates/ureq); one-shot and
stateless like `shellyplug`.

Two properties make AMT the preferred backend where the hardware has it:

- **True power-state readback.** The ME runs on standby power and answers
  with the host on, off, sleeping, or bare-metal with no OS installed —
  `state` is a genuine sensor, not an outlet-side guess. (An HA/smart-plug
  cycle hook can't report state at all.)
- **Per-target control with zero extra hardware** — no outlet, no relay, no
  wiring; just the onboard NIC.

Mechanically it acts on `CIM_PowerManagementService.RequestPowerStateChange`
and reads back `CIM_AssociatedPowerManagementService.PowerState`.

### Requirements

- AMT provisioned and enabled in MEBx (Ctrl-P at boot), with network access
  to port 16992 on the AMT NIC. Works with the machine in any state — even
  with no OS on disk.
- **Digest-only auth is handled natively.** AMT 11+ advertises HTTP Digest
  as the *only* supported auth and rejects plaintext (this is why Debian's
  `amtterm` cannot talk to modern AMT). The helper implements the RFC 2617
  digest handshake itself.
- **TLS-provisioned AMT is not supported.** A machine provisioned for TLS
  serves WS-Man only on port 16993; the helper speaks the plain port and
  reports this clearly rather than guessing.

### Credentials

**The password never appears in the lab file, a flag, or any repository.**
The helper reads it from the **`AMT_PASSWORD` environment variable** only;
the lab file carries just the address and username. Inject it at call time —
for example with the 1Password CLI:

```bash
# .env:  AMT_PASSWORD=op://<vault>/<item>/password
op run --env-file .env -- bash -c 'paniolo power-state <target>'
```

Single quotes matter: the parent shell must not expand `$AMT_PASSWORD`
before `op run` sets it (see the Generic power hooks section for the same
gotcha with `HA_TOKEN`). Without the variable, every subcommand fails with a
message saying exactly this.

### Setting up the credential source

`AMT_PASSWORD` is deliberately the whole interface: the helper neither knows
nor cares where the secret is kept. **Any secret manager works** — 1Password,
HashiCorp Vault, `pass`, systemd credentials, a cloud secrets service — as
long as the variable is present in the environment of the `paniolo`
invocation whose hook needs it. To make that repeatable rather than
hand-typed, set up one of these **next to whatever invokes paniolo**:

- **A reference file + run wrapper**, when the secret manager has an
  `op run`-style launcher that resolves references into env vars at call
  time. The reference (`op://<vault>/<item>/password` above) is a pointer,
  not a secret — it is safe to commit to the private repo that holds your
  automation; the value itself never lands in a file.
- **A small fetch-and-exec wrapper** for managers without such a launcher
  (a 1Password Connect fetcher, `vault kv get`, `pass show`, …):

  ```sh
  #!/bin/sh
  # with-amt-password — run a command with AMT_PASSWORD in its environment
  AMT_PASSWORD="$(fetch-secret amt/password)" || exit 1
  export AMT_PASSWORD
  exec "$@"
  ```

  Commit the wrapper alongside your automation (it names *where* the secret
  lives, never the secret) and invoke hooks through it:
  `with-amt-password paniolo power-cycle <target>`.
- **An interactive export** for one-off manual use:
  `read -rs AMT_PASSWORD && export AMT_PASSWORD` keeps the value out of the
  command line and shell history.

**Placement rule:** the variable must exist where the *hook runs* — the host
that owns the target's power channel. Locally that is simply the environment
of your `paniolo` command. For a target driven through a remote lab host,
plain SSH does not carry your local environment across, so install the
wrapper/reference file on the control host itself (or use the sshd
`AcceptEnv` forwarding pattern shown under Generic power hooks).

### Commands

```bash
amt -d <host> status                 # firmware identity + power state detail
amt -d <host> state                  # print exactly "on" or "off" (state_cmd contract)
amt -d <host> on                     # power on, confirm by read-back
amt -d <host> off                    # power off (hard), confirm by read-back
amt -d <host> cycle [--delay-ms 3000]  # off → confirm → delay → on → confirm
```

- `-d <host>` is a hostname, IPv4 address, or bracketed IPv6 literal
  (`[fe80::1]`), optionally with a port (default 16992); an `http://` prefix
  is tolerated. Anything else URL-shaped — a path, query, userinfo, or an
  unbracketed IPv6 address — is rejected with a clear error rather than sent
  as part of the request. `-u <user>` sets the Digest username (default
  `admin`).
- `state` prints `on` only when the host is running (PowerState 2); sleep,
  hibernate, and soft-off all print `off`. Any other reported PowerState
  (`Other`, or one of the transitional power-cycle/reset values) is an error
  naming the raw value — the hook never guesses.
- `off` is the CIM "Off - Soft" **unconditional power-off** — equivalent to
  holding the power button, not a graceful OS shutdown — and is confirmed by
  waiting for the ME to report Off - Soft.
- `cycle` is built as off → confirm → delay → on → confirm rather than the
  fixed CIM power-cycle state, so the off-hold matches the other helpers'
  `--delay-ms` semantics. The off phase runs for any host **not already at
  Off - Soft** — a sleeping (S3) or hibernating host is powered off and held,
  not merely resumed — and each phase is confirmed by read-back, so the
  result is a genuine cold boot (POST). Only a host already soft-off skips
  straight to power-on.

### Wiring into paniolo power hooks

```bash
paniolo power set -t target-machine \
    --cycle-cmd "amt cycle -d 10.0.0.5 -u admin --delay-ms 5000" \
    --on-cmd    "amt on -d 10.0.0.5 -u admin" \
    --off-cmd   "amt off -d 10.0.0.5 -u admin" \
    --state-cmd "amt state -d 10.0.0.5 -u admin"
```

Remember the hooks run in paniolo's environment: `paniolo power …` /
`power-cycle` / `power-state` must themselves be invoked with `AMT_PASSWORD`
set (the `op run … bash -c '…'` pattern above).

### Gotchas

- **The AMT NIC drops link around power transitions.** For a few seconds as
  the host powers on or off, the shared NIC's PHY renegotiates and WS-Man
  requests fail with "no route to host" (observed on a Dell OptiPlex 7060:
  the power-on succeeded but the immediate read-back could not connect). The
  helper absorbs this: power requests and read-back polling retry transient
  transport errors (connection failures, I/O timeouts, DNS) within a 20 s
  budget that also bounds each individual attempt. Deterministic failures —
  a bad address, an unparseable response, a proxy problem — fail
  immediately. If a machine stays unreachable longer than 20 s, treat it as
  real.
- **`state` reflects the host, not the outlet.** Sleep (S3) and hibernate
  report `off` — the OS isn't running — even though the PSU has power. `on`
  from any of those states boots/wakes the machine; `cycle` from any of them
  holds the machine off first, so it cold-boots rather than resumes.
- **BIOS "AC Recovery" is irrelevant here** (unlike outlet-based helpers):
  AMT's power-on is an explicit command to the ME, not a power restore, so
  it works regardless of the AC-recovery BIOS setting.
- **`status` is the debugging view**: it prints the AMT firmware identity
  (from the HTTP `Server:` header) and the raw CIM PowerState name/number.
