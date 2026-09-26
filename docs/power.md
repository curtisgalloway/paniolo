# Power control

paniolo has two ways to switch a target's power:

- **DTR via FTDI**: the serial cable's DTR line drives the target's J2
  power-button header. Wiring only; no external services. **Opt-in per serial
  interface** (`power_button = true`), because wiring DTR to J2 is rare:
  `serial dtr` / `serial reset` refuse an interface that has not declared it
  rather than toggle a possibly unwired line.
- **Generic power hooks**: four optional shell commands (`on_cmd`, `off_cmd`,
  `cycle_cmd`, `state_cmd`), set with `paniolo power set` and run via `sh -c`.
  Use them for everything else: a smart plug, a USB hub, Intel AMT, a script.

**Design principle:** device-specific control logic never goes in the core
crates. It lives in standalone helper binaries behind the generic hooks
(`cambrionix` is the canonical example). To support new hardware, follow the
[power-helper recipe](dev/adding-power-helpers.md).

---

## DTR power control (FTDI J2 wiring)

DTR (Data Terminal Ready) is a control output on a USB serial adapter such as
an FTDI cable; wired to J2, it "presses" the Raspberry Pi 5's power button.

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

The same FTDI adapter should also carry the target's serial console. The DTR
and sense signals share that USB serial port.

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

DTR commands live under `paniolo serial`, since the DTR line belongs to the
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

**Which interface is pressed.** `serial dtr` and `serial reset` pick, in order:

1. an explicit `-i`;
2. the power channel's `serial_interface`;
3. the only interface with `power_button = true`.

If the chosen interface lacks `power_button = true`, or none has it, the
command **errors** with a hint pointing at the target's real power method and
the console-reboot path. It never falls back to a lone console, which might be
unwired and would silently do nothing.

**If a DTR press fails** (when serialcap handles DTR), the command fails
rather than report a press. Release is attempted even after a failed
assertion, and the daemon reconnects the handle with DTR deasserted. Check the
target before retrying: the physical press may have happened.

> **"Reboot over serial" is not a DTR reset.** Typing `reboot` at a logged-in
> serial console is a *software* reboot: `paniolo serial send <target>
> "reboot"`. `serial reset` / `serial dtr` are a *hardware* DTR power-button
> toggle that needs the J2 wiring above. When a request says "use serial to
> reboot" without naming DTR, the wire, or the power button, default to the
> console `reboot` (or the configured `paniolo power-cycle`) — not DTR.

---

## Generic power hooks

Set one or more shell-command hooks on the target's power channel. All four
are optional and independent:

```bash
paniolo power set -t <target> \
    [--cycle-cmd <cmd>]   \   # paniolo power-cycle
    [--on-cmd    <cmd>]   \   # paniolo power on
    [--off-cmd   <cmd>]   \   # paniolo power off
    [--state-cmd <cmd>]   \   # paniolo power-state (stdout: "on" or "off")
    [--serial-interface <name>]   # default DTR interface when several opt in
    [--host <labhost>]
```

Each hook runs via `sh -c <cmd>`; its exit code decides success. A hook can be
any shell command, script path, or helper binary. Ready-made helpers:

| Helper | Switches | Section |
|---|---|---|
| `cambrionix` | A port on a Cambrionix USB hub | [Cambrionix hub control](#cambrionix-hub-control) |
| `zigplug` | A Zigbee smart plug | [Zigbee smart plug control](#zigbee-smart-plug-control-zigplug) |
| `shellyplug` | A Shelly Wi-Fi plug or relay | [Shelly smart plug control](#shelly-smart-plug-control-shellyplug) |
| `amt` | An Intel AMT (vPro) machine's own power | [Intel AMT power control](#intel-amt-power-control-amt) |

The helpers install into the private libexec dir
(`~/.local/libexec/paniolo/bin`), not onto `PATH`. Hook strings still name
them bare, because paniolo looks in libexec first. To run one by hand, use
`paniolo helper <name> …`.

The dual-board `hidrig` control board can also switch a DUT power relay
(`hidrig power off|on|cycle`) behind these hooks: one USB device for HID
(emulated keyboard and mouse), console, and power (see
[`hidrig/README.md`](https://github.com/curtisgalloway/paniolo/blob/main/hidrig/README.md)).

### Commands backed by hooks

```bash
paniolo power on  [target]        # run on_cmd; error with config hint when unset
paniolo power off [target]        # run off_cmd; error with config hint when unset
paniolo power-cycle [target]      # run cycle_cmd
paniolo power-state [target]      # state_cmd if set; else serial sense-line
```

**`power-cycle`** runs `cycle_cmd` with no built-in timing or sense-signal
logic; the script owns the whole sequence. If the script fails, paniolo exits
101 (`helper_failed`) and reports the script's own code as `child_exit` in the
`--json-errors` object. A script that is missing or not executable (shell code
127/126) exits 3 (`not_configured`). See [Exit status and errors](errors.md).

**`power-state`** runs `state_cmd` if it is set and reads the first
whitespace-delimited token of its stdout. That token must be `on` or `off`
(case-insensitive); anything else is an error. Without `state_cmd`, paniolo
falls back to the serial sense line, which needs the sense signal wired and
the serialcap daemon running.

### `paniolo doctor` hook probing

`paniolo doctor` probes every hook, over SSH for remote hosts. It checks an
absolute path with `test -e`, and a bare name with `command -v` using the same
lookup order the hooks get at runtime:

1. the per-user libexec dir (`~/.local/libexec/paniolo/bin`);
2. the system package dir (`/usr/libexec/paniolo/bin`);
3. `PATH`.

It reports which hooks are configured by name, e.g.
`cycle_cmd,on_cmd,off_cmd,state_cmd`.

### Example: Home Assistant script (cycle_cmd)

This wires `cycle_cmd` to a script that calls the Home Assistant API. No
device-specific helper is needed:

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

The script reads `HA_TOKEN` from the environment. Never hardcode it in the
script or the paniolo config. Ways to inject it at call time:

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

---

## Cambrionix hub control

The `cambrionix` helper drives the control UART (serial port) of a Cambrionix
programmable USB hub: 115200 8N1, `>>` prompt, commands `mode c|s|o <port>` and
`state`. It plugs into the generic power hooks.

### Installation

`make install` / `paniolo setup` builds and installs `cambrionix` with the
other crates, into the private libexec dir. Run it by hand via
`paniolo helper cambrionix …`.

### Commands

```bash
cambrionix -d <device> state              # table of all ports (volts, mA, attach/mode)
cambrionix -d <device> state <port>       # print exactly "on" or "off" (state_cmd contract)
cambrionix -d <device> on <port>          # mode c (charging/on), confirm by read-back
cambrionix -d <device> off <port>         # mode o (off), confirm by read-back
cambrionix -d <device> cycle <port> [--delay-ms 3000]
                                          # off → confirm → delay → restore prior mode → confirm
```

- **Ports:** 1–15. Port 0 is the hub's own host/system row, read-only in the
  table output.
- **`cycle`** restores the previous mode: Sync (`s`) if it was Sync, otherwise
  charging (`c`).
- **Every change is confirmed by re-reading the port table.** The hub accepts a
  `mode` command without acknowledging it, so the read-back is the only proof
  it worked. `on` requires mode `C` or `S`, `off` requires `O`, and `cycle`
  checks both phases. A mismatch exits non-zero instead of printing success.
- **`state <port>`** maps `C`/`S`/`I` to `on` and `O` to `off`, and errors on
  any other mode letter rather than guessing.
- **Bounded responses.** Each response is limited to 3 s and 64 KiB, and a line
  the hub flags as an error fails the command. A `-d` that points at some other
  chatty UART fails fast instead of hanging the hook.

### Wiring into paniolo power hooks

This example powers a Raspberry Pi 5 from hub port 4, with the hub's control
UART on `/dev/cu.usbserial-AA00BB11`:

```bash
paniolo power set -t pi5 \
    --cycle-cmd "cambrionix -d /dev/cu.usbserial-AA00BB11 cycle 4" \
    --on-cmd    "cambrionix -d /dev/cu.usbserial-AA00BB11 on 4" \
    --off-cmd   "cambrionix -d /dev/cu.usbserial-AA00BB11 off 4" \
    --state-cmd "cambrionix -d /dev/cu.usbserial-AA00BB11 state 4"
```

After this, `paniolo power on pi5`, `paniolo power off pi5`,
`paniolo power-cycle pi5`, and `paniolo power-state pi5` all work with no
further setup.

---

## Zigbee smart plug control (zigplug)

The `zigplug` helper switches Zigbee smart plugs through a CC2652-based
coordinator dongle (e.g. Sonoff ZBDongle-P). Zigbee is a low-power mesh radio
protocol; the coordinator is the USB stick that runs the network. zigplug uses
[zigpy-znp](https://github.com/zigpy/zigpy-znp) to talk to it, and plugs into
the generic power hooks like `cambrionix`.

Device interview data is kept in a sqlite DB at
`~/.config/paniolo/helpers/zigplug/zigbee.db` (`--db` to override). A DB at the
pre-0.3 top-level location is migrated automatically.

**Operations run through a persistent daemon** that owns the coordinator
session. The CLI starts it on first use and proxies to it, so hook strings stay
one-shot commands. The daemon is needed for correctness, not speed:

- **Opening the serial port resets the chip.** The CP2102N's DTR/RTS lines
  drive the stick's auto-bootloader circuit on every open. Depending on the
  line states at reset time, the chip occasionally boots into the bootloader
  instead of the app, and the session hangs forever.
- **Concurrent one-shots collide.** Two invocations interleaving frames on one
  stateful ZNP session wedge the coordinator. On real hardware, a pile-up of
  stuck `power-state` hooks wedged the dongle for hours and cost the formed
  network its NVRAM.

The daemon opens the port once, runs every operation in turn on one session,
and gives each a hard timeout, so a sick radio produces a fast error, never a
hung power hook.

It follows the standard daemon contract
(`/tmp/paniolo-<uid>/zigplug/daemon.json`, localhost HTTP, OS-assigned port)
and shows up in `paniolo daemons`. The discovery file also holds a per-run
bearer token (file mode 0600) that every request must present, so nothing else
on the host — another user, a web page — can drive the coordinator through the
daemon.

Manual control: `zigplug serve` / `stop` / `status` (`stop` and `status` need
no `-d`). `--no-daemon` forces the legacy direct path, for debugging only.

### Installation

`zigplug` is a Python project (`zigplug/`). `paniolo setup` / `make install`
installs it as a uv tool when `uv` is on `PATH`, with its shim in the private
libexec dir. Run it by hand via `paniolo helper zigplug …`.

```bash
# manual equivalent
UV_TOOL_BIN_DIR=~/.local/libexec/paniolo/bin uv tool install --force ~/src/paniolo/zigplug
```

### One-time setup: form the network

```bash
paniolo helper zigplug -d /dev/cu.usbserial-XXXX form              # channel picked by energy scan
paniolo helper zigplug -d /dev/cu.usbserial-XXXX form --channel 25 # or explicit (25-26 avoid Wi-Fi)
```

`form` is idempotent: if the dongle already has a network, it prints the
existing channel/PAN and exits.

**If formation fails with "too much RF interference":** put the dongle on a
USB 2.0 extension cable, away from USB 3.x ports and hubs and from
video-capture devices. This failure is real and seen on hardware: radiated USB
noise desensitizes the CC2652 radio until the coordinator refuses to start on
any channel. Cable placement is almost always the fix. To factory-reset stale
dongle state, run `python -m zigpy_znp.tools.nvram_reset <device>` from the
`zigplug/` project venv.

### Pairing plugs

```bash
paniolo helper zigplug -d <device> permit --time 120   # open a join window
# put the plug in pairing mode (hold button until LED blinks; factory-fresh
# plugs usually enter pairing mode on first power-up)
paniolo helper zigplug -d <device> list                # IEEE, NWK, manufacturer, model, state
```

`permit` prints each join and interview as it happens, and exits non-zero if
nothing paired. A plug previously paired to another hub needs a full factory
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

IEEE addresses (the plug's fixed hardware ID) are accepted with or without
`:`/`-` separators.

### Coordinator NVRAM recovery (backup/restore)

zigpy snapshots the full network state (PAN, channel, network key, frame
counters) into the device DB on every session. If the coordinator's NVRAM is
lost or corrupted — symptom: `coordinator has no Zigbee network` on a dongle
that was already formed — you can recover the network **without re-pairing**:

```bash
paniolo helper zigplug -d <device> stop      # restore needs the port exclusively
paniolo helper zigplug -d <device> restore   # newest auto-backup from zigbee.db
paniolo helper zigplug -d <device> list      # verify the plugs answer
```

`restore` bumps the network-key frame counter (`--counter-increment`, default
10000) past anything the old coordinator could have sent, so joined devices
accept the restored coordinator. A plug that was orphaned for hours may not
answer until it rescans. Power-cycling the plug at the wall forces an immediate
rejoin (and cycles whatever it powers). If the bench matters, keep an off-host
copy with `zigplug backup -o <file>`.

### Wiring into paniolo power hooks

```bash
paniolo power set -t target-machine \
    --cycle-cmd "zigplug -d /dev/cu.usbserial-XXXX cycle ff:ff:b4:0e:06:04:ea:b7" \
    --on-cmd    "zigplug -d /dev/cu.usbserial-XXXX on    ff:ff:b4:0e:06:04:ea:b7" \
    --off-cmd   "zigplug -d /dev/cu.usbserial-XXXX off   ff:ff:b4:0e:06:04:ea:b7" \
    --state-cmd "zigplug -d /dev/cu.usbserial-XXXX state ff:ff:b4:0e:06:04:ea:b7"
```

The first hook starts the daemon (a few seconds). After that, operations answer
in about a second, concurrent hooks queue safely on its single session, and
every operation has a hard timeout. `form`, `restore`, and `backup` (when no
daemon is running) open the port directly and refuse to run while the daemon
is up — run `zigplug stop` first.

## Shelly smart plug control (shellyplug)

The `shellyplug` helper switches **Shelly Gen2+ smart plugs and relays** (Plus,
Pro, Gen3, Gen4) over each device's **local HTTP RPC API**. It needs no cloud
account, Home Assistant, or Matter controller. It is pure Rust, using
[ureq](https://crates.io/crates/ureq). HTTP is stateless, so unlike `zigplug`
there is no daemon: each invocation makes one `GET /rpc/<Method>` call and
exits.

- **Supported:** Gen2/3/4 devices, which speak the JSON-RPC API
  (`Switch.Set`, `Switch.GetStatus`, `Shelly.GetDeviceInfo`). Original Gen1
  devices use a different REST API (`/relay/0?turn=on`) and are **not**
  supported.
- **Auth:** only devices with authentication **disabled** (`auth_en: false`,
  the factory default) are supported for now. An auth-enabled device answers
  HTTP 401, and the helper says so clearly rather than guessing.

### Installation

`make install` / `paniolo setup` builds and installs `shellyplug` with the
other crates, into the private libexec dir. Run it by hand via
`paniolo helper shellyplug …`.

### Addressing

- **`-d <host>`** is the device's network address: a bare IP or hostname
  (`10.0.0.5`, `shelly.local`), optionally with a scheme or port
  (`http://10.0.0.5:8080`). A Shelly advertises an mDNS name like
  `shellyplugusg4-<mac>.local`. Either pin its IP with a **DHCP reservation**
  or use that `.local` name in the hook string, so a DHCP lease change does not
  break the hook.
- **`[id]`** is the switch component id, default `0`. Single-outlet plugs have
  only switch `0`; multi-channel devices (e.g. a Pro 4PM) use `0..N`.

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

- **macOS Local Network privacy blocks the helper (the big one).**
  - *Why:* every other paniolo helper talks over a serial port or to a
    `127.0.0.1` daemon, and loopback is exempt from macOS's Local Network
    privacy gate. `shellyplug` is the **first helper to reach a device on the
    LAN**, so it is the first to hit that gate. On macOS Sequoia and later,
    local-subnet access is granted **per binary**, attributed to the app that
    launched it.
  - *Symptom:* the helper fails with **`No route to host` (EHOSTUNREACH)**
    while a browser and `curl` reach the same device fine. Apple-signed system
    binaries like `curl` are exempt; a freshly built `shellyplug` is not. The
    tell is a binary that reaches the public internet but not the LAN.
  - *Fix:* grant the app that launches the hook **Local Network** access
    (System Settings → Privacy & Security → Local Network; enable your
    terminal, e.g. iTerm2/Terminal). That app's first LAN access usually
    triggers the one-time prompt.
- **A plug's IP can change.** Use a DHCP reservation or the device's `.local`
  mDNS name in the hook string (see Addressing).
- **`state` is cheap and never stale.** It reads `Switch.GetStatus` live on
  every call (no caching) and fails loudly if the device is unreachable,
  rather than reporting a stale guess. It is the hook agents poll.

---

## Intel AMT power control (amt)

The `amt` helper switches **Intel AMT (vPro) machines** with no smart plug:
the power switch is the machine's own Management Engine (ME), a controller on
the motherboard that runs on standby power and answers on the regular network.
AMT (Active Management Technology) is Intel's name for this. The helper speaks
**WS-Management** (SOAP over HTTP on port 16992). Like `shellyplug`, it is pure
Rust via [ureq](https://crates.io/crates/ureq), one-shot and stateless.

AMT is the preferred backend where the hardware has it:

- **True power-state readback.** The ME answers whether the host is on, off,
  sleeping, or bare metal with no OS, so `state` is a real sensor, not an
  outlet-side guess. (A Home Assistant or smart-plug cycle hook cannot report
  state at all.)
- **No extra hardware:** no outlet, relay, or wiring; just the onboard NIC.

Under the hood it calls `CIM_PowerManagementService.RequestPowerStateChange`
and reads back `CIM_AssociatedPowerManagementService.PowerState`.

### Requirements

- AMT provisioned and enabled in MEBx (the ME's firmware setup screen, Ctrl-P
  at boot), with network access to port 16992 on the AMT NIC. It works with
  the machine in any state, even with no OS on disk.
- **Digest-only auth is handled natively.** AMT 11+ advertises HTTP Digest as
  its *only* auth and rejects plaintext, which is why Debian's `amtterm`
  cannot talk to modern AMT. The helper does the RFC 2617 digest handshake
  itself.
- **TLS-provisioned AMT is not supported.** A machine provisioned for TLS
  serves WS-Man only on port 16993. The helper speaks the plain port and says
  so clearly rather than guessing.

### Credentials

**The password never appears in the lab file, a flag, or any repository.**
The helper reads it only from the **`AMT_PASSWORD` environment variable**; the
lab file holds just the address and username. Inject it at call time, for
example with the 1Password CLI:

```bash
# .env:  AMT_PASSWORD=op://<vault>/<item>/password
op run --env-file .env -- bash -c 'paniolo power-state <target>'
```

The single quotes matter: the parent shell must not expand `$AMT_PASSWORD`
before `op run` sets it. (The same applies to `HA_TOKEN` under
[Generic power hooks](#generic-power-hooks).) Without the variable, every
subcommand fails with a message saying exactly this.

### Setting up the credential source

The helper does not care where the secret is kept. **Any secret manager
works** — 1Password, HashiCorp Vault, `pass`, systemd credentials, a cloud
secrets service — as long as `AMT_PASSWORD` is in the environment of the
`paniolo` command whose hook needs it. To make that repeatable, set up one of
these **next to whatever invokes paniolo**:

- **A reference file + run wrapper**, when the secret manager has an
  `op run`-style launcher that turns references into env vars at call time.
  The reference (`op://<vault>/<item>/password` above) is a pointer, not a
  secret, so it is safe to commit to the private repo that holds your
  automation.
- **A small fetch-and-exec wrapper** for managers without such a launcher
  (a 1Password Connect fetcher, `vault kv get`, `pass show`, …):

  ```sh
  #!/bin/sh
  # with-amt-password — run a command with AMT_PASSWORD in its environment
  AMT_PASSWORD="$(fetch-secret amt/password)" || exit 1
  export AMT_PASSWORD
  exec "$@"
  ```

  Commit the wrapper with your automation (it names *where* the secret lives,
  never the secret) and run hooks through it:
  `with-amt-password paniolo power-cycle <target>`.
- **An interactive export** for one-off manual use:
  `read -rs AMT_PASSWORD && export AMT_PASSWORD` keeps the value out of the
  command line and shell history.

**Placement rule:** `AMT_PASSWORD` only needs to be in the environment of the
local `paniolo` command you run. For a target behind a remote control host,
paniolo's dispatch carries it across over the remote command's **stdin**,
never its argv, so `ps` on the control host never shows it. Nothing needs
installing there, and plain SSH's "your local environment doesn't cross"
limit does not apply. Two limits:

- It covers only `AMT_PASSWORD` (the fixed forwarding list,
  `ssh::FORWARDED_ENV`). A *generic* hook secret like `HA_TOKEN` still needs
  one of the techniques under [Generic power hooks](#generic-power-hooks): a
  wrapper on the control host, or sshd `AcceptEnv`.
- It covers only non-interactive commands (`power-cycle`, `power
  on/off/state`). An interactive one (`serial connect`) forwards nothing by
  design: its stdin is your own terminal, not a channel to send a secret
  ahead of.

### Commands

```bash
amt -d <host> status                 # firmware identity + power state detail
amt -d <host> state                  # print exactly "on" or "off" (state_cmd contract)
amt -d <host> on                     # power on, confirm by read-back
amt -d <host> off                    # power off (hard), confirm by read-back
amt -d <host> cycle [--delay-ms 3000]  # off → confirm → delay → on → confirm

amt -d <host> kvm status             # is port 5900 open, is consent required
amt -d <host> kvm enable             # open 5900 to VNC clients, enable redirection
amt -d <host> kvm disable            # close 5900, keep the stored RFB password
```

- **`-d <host>`** is a hostname, IPv4 address, or bracketed IPv6 literal
  (`[fe80::1]`), optionally with a port (default 16992). An `http://` prefix
  is tolerated. Anything else URL-shaped — a path, query, userinfo, or an
  unbracketed IPv6 address — is rejected with a clear error rather than sent
  as part of the request.
- **`-u <user>`** sets the Digest username (default `admin`).
- **`state`** prints `on` only when the host is running (PowerState 2).
  Sleep, hibernate, and soft-off all print `off`. Any other PowerState
  (`Other`, or a transitional power-cycle/reset value) is an error naming the
  raw value; the hook never guesses.
- **`off`** is the CIM "Off - Soft" **unconditional power-off**, the same as
  holding the power button, not a graceful OS shutdown. It is confirmed by
  waiting for the ME to report Off - Soft.
- **`cycle`** runs off → confirm → delay → on → confirm rather than using the
  fixed CIM power-cycle state, so the off-hold matches the other helpers'
  `--delay-ms` behavior. The off phase runs for any host **not already at
  Off - Soft**: a sleeping (S3) or hibernating host is powered off and held,
  not just resumed. Each phase is confirmed by read-back, so the result is a
  true cold boot (POST). Only a host already at soft-off skips straight to
  power-on.

#### KVM redirection (the ME's built-in VNC server)

`kvm enable` turns an AMT machine into a **network KVM that any standard VNC
client can drive**: the screen and input of a *physical* box, with no capture
card or HID rig in the path. It is not a power command; it lives here because
the `amt` helper already speaks WS-Man to the ME. (VNC's wire protocol is
called RFB, hence "RFB password".)

The RFB password comes from **`AMT_RFB_PASSWORD`** in the environment, never a
flag. It is a different secret from `AMT_PASSWORD`, and AMT's rules for it are
strict:

- **exactly 8 characters**;
- at least one capital, one lowercase, one digit, and one special character;
- but **not** `"`, `,` or `:`, which AMT rejects even though they count as
  special characters.

The helper checks these rules itself before writing, because AMT **locks the
RFB password** after a few failed authentication attempts. A shell can also
silently eat the special character if you put it on a command line (`!` in
double quotes is history expansion). Re-Putting the password over WS-Man
clears a lock.

```bash
AMT_PASSWORD=… AMT_RFB_PASSWORD='Ab3!defG' amt -d <host> kvm enable
```

By default `kvm enable` sets `OptInPolicy=false` (no local user has to approve
the session; a headless bench target has nobody to click the prompt) and
`SessionTimeout=0`. Pass `--opt-in` to require consent, or `--session-timeout
<minutes>` for an idle drop.

Before relying on KVM:

- **KVM must already be enabled in MEBx.** `kvm status` reports it as
  `enabled in MEBx`. When that is `NO`, nothing this helper writes will open
  the port; only the firmware setup screen can change it.
- **Port 5900 is gone in newer firmware.** Intel removed it from Kaby Lake
  11.8.94, Cannon Lake 12.0.93, Comet Lake 14.1.70, Tiger Lake 15.0.45 and
  Alder/Raptor Lake 16.1.25 onward. Past those versions KVM is reachable only
  over the 16994/16995 redirection protocol, which no standard VNC client
  speaks and this helper does not implement. Check `amt -d <host> status`
  before planning around it.

### Wiring into paniolo power hooks

```bash
paniolo power set -t target-machine \
    --cycle-cmd "amt cycle -d 10.0.0.5 -u admin --delay-ms 5000" \
    --on-cmd    "amt on -d 10.0.0.5 -u admin" \
    --off-cmd   "amt off -d 10.0.0.5 -u admin" \
    --state-cmd "amt state -d 10.0.0.5 -u admin"
```

Hooks run in paniolo's environment, so run `paniolo power …`, `power-cycle`,
and `power-state` with `AMT_PASSWORD` set (the `op run … bash -c '…'` pattern
above) on the machine you type on, whether the power channel is local or on a
remote control host; see the placement rule above.

### Gotchas

- **The AMT NIC drops link around power transitions.** For a few seconds as
  the host powers on or off, the shared NIC renegotiates its link and WS-Man
  requests fail with "no route to host". (Seen on a Dell OptiPlex 7060: the
  power-on succeeded but the immediate read-back could not connect.) The
  helper absorbs this. Power requests and read-back polling retry transient
  transport errors (connection failures, I/O timeouts, DNS) within a 20 s
  budget, which also bounds each attempt. Deterministic failures — a bad
  address, an unparseable response, a proxy problem — fail immediately. If a
  machine stays unreachable longer than 20 s, treat it as real.
- **`state` reflects the host, not the outlet.** Sleep (S3) and hibernate
  report `off` because the OS is not running, even though the PSU has power.
  `on` from any of those states boots or wakes the machine; `cycle` from any
  of them holds the machine off first, so it cold-boots rather than resumes.
- **BIOS "AC Recovery" does not matter here** (unlike outlet-based helpers).
  AMT's power-on is an explicit command to the ME, not a power restore, so it
  works whatever the AC-recovery BIOS setting is.
- **`status` is the debugging view.** It prints the AMT firmware identity
  (from the HTTP `Server:` header) and the raw CIM PowerState name and number.
