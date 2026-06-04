---
name: paniolo
description: Control a physical target machine (SBC, e.g. a Raspberry Pi) with the paniolo CLI during low-level bring-up — netboot it over a direct USB-Ethernet link, watch and OCR its HDMI screen, drive its serial console, power-cycle it, switch the link between netboot and ffx, and drive targets on remote control hosts transparently via a single git-tracked lab file over SSH. Use when you need to boot, observe, type into, screenshot, read the screen of, power, or remotely control a target/board through paniolo — including across multiple control hosts.
---

# Paniolo — controlling a target machine

Paniolo drives a physical target machine (a single-board computer such as a
Raspberry Pi) during firmware/OS bring-up. It runs on the control host that is
physically wired to the target (USB-Ethernet for netboot, an HDMI capture
dongle for video, a USB-serial adapter for the console, a smart plug for power).

Almost every command operates on a named **target**. If exactly one target is
configured, the name can be omitted.

## First-time setup

```
cd ~/src/paniolo
make install               # cargo-installs the `paniolo` CLI, then `paniolo setup`
                           # builds/installs hdmicap, serialcap, netbootd, visionocr;
                           # on macOS also installs netbootd-bpf-helper setuid-root (one sudo)
```

Run once per machine; re-run after pulling or editing (it's a full rebuild).
Everything lands in `~/.cargo/bin` — make sure it's on `PATH`. If a `paniolo`
from the retired Python CLI shadows it (uv-tools shim in `~/.local/bin`),
remove it with `uv tool uninstall paniolo`; `make install` warns when it
detects a shadow.

## Configure a target

Config lives in one CLI-managed **lab file** (`~/.config/paniolo/lab.toml`, or
`--lab`/`PANIOLO_LAB`). A target's hardware is described as *channels*:

```
paniolo target add <name> [--host <labhost>] [--note <text>]
paniolo netboot set -t <name> --interface <iface> [--tftp-root <dir>] [--host-ip <ip>]
paniolo serial add console -t <name> --device <path> [--baud 115200] [--sense cts]
paniolo power set -t <name> [--cycle-cmd <script>] [--serial-interface console]
paniolo video set -t <name> --device "<capture id or name>"
```

- `paniolo netboot devices` lists candidate USB-Ethernet interfaces (the
  primary NIC is excluded); `paniolo discover` lists all lab-relevant hardware;
  `paniolo configure <name> -H <host>` proposes a whole block to paste in.
- Inspect: `paniolo config show` (whole lab) / `paniolo target show <name>`.
- Remove: `paniolo target rm <name>`, or per channel (`netboot rm`,
  `serial rm <iface> -t <name>`, `power rm`, `video rm`).
- `paniolo doctor` probes every configured channel against reality (devices
  exist, over SSH for remote hosts).

## Netboot (DHCP + TFTP)

Boot a board over the direct USB-Ethernet link:

```
paniolo netboot start [target]            # serve DHCP + TFTP (netbootd)
paniolo netboot tftp-root [target]        # print where to drop boot files
paniolo netboot status [target]
paniolo netboot logs -f [target]          # follow the combined log
paniolo netboot stop [target]
```

`start` refuses an interface that carries the system default route (a primary
NIC) — the netboot link must be a dedicated USB-Ethernet adapter. The default
engine is the single-binary `netbootd` (Rust); `--engine python` selects the
legacy DHCP+TFTP subprocess pair it was ported from. On macOS the rust engine's
BPF send path uses the setuid `netbootd-bpf-helper` installed by `paniolo setup`.

Put boot files in the target's TFTP root (for a Raspberry Pi 5, the kernel goes
in as `kernel_2712.img`). Needs passwordless `sudo` for `ifconfig` (it assigns
the interface's static IP).

## Link mode — switch between netboot and ffx

The same USB-Ethernet link can't be in netboot mode and ffx-over-network mode at
once (they want incompatible host addressing). Use `netif` to flip between them
atomically instead of doing it by hand:

```
paniolo netif mode netboot [target]        # IPv4 + DHCP + TFTP (= netboot start)
paniolo netif mode ffx [target]            # stop netboot, add host fe80::1/64 for ffx
paniolo netif mode off [target]            # tear down both
paniolo netif status [target]              # which mode is active, addresses, peer
```

- **Switching to ffx stops netboot first** — otherwise a power-cycle TFTP-boots a
  stale image instead of falling through to the SD card. This is the safe way to
  hand the link off to `ffx`.
- `mode ffx` adds the host-side `fe80::1/64` that `ffx` needs (nothing else sets
  it up, and it's lost on a control-host reboot). Re-run `mode ffx` any time to
  re-add it — every mode is idempotent.
- `netif status` in ffx mode reads the IPv6 neighbor table and prints a
  ready-to-paste `ffx target add fe80::…%<iface>` for the discovered device. If
  no peer shows, power-cycle the target and wait for it to finish SLAAC.

Typical ffx hand-off: `paniolo netif mode ffx fortune` → `paniolo power-cycle
fortune` → `paniolo netif status fortune` (grab the address) → `ffx target list`.
Go back to TFTP bring-up with `paniolo netif mode netboot fortune`. Same
passwordless `sudo` requirement as netboot (`ip` on Linux, `ifconfig` on macOS).

## Video — capture, preview, OCR

```
paniolo video devices                 # list capture devices (with stable ids)
paniolo video set -t <target> --device "<id-or-name>"   # configure the video channel
paniolo video watch                   # start the capture daemon (background)
paniolo video preview                 # open the dashboard in a browser
paniolo video shot [--stable] [--out frame.png]   # one lossless PNG
paniolo video read [--stable]         # OCR the current screen, print text
paniolo video show                    # device + daemon status
paniolo video stop
```

- The **dashboard** (the video daemon's URL — ports are OS-assigned, printed by
  `video watch`/`console`) shows live video on top, a
  serial terminal below, and an **OCR button** that reads the current screen.
- The `device` may be a **stable id** (preferred — `video devices` prints
  `id=…`: the AVFoundation uniqueID on macOS, the `/dev/v4l/by-path` symlink on
  Linux), a name substring, or a `/dev/video*` path. Ids are derived from USB
  port topology, so they survive reboots and tell two identical dongles apart;
  replugging into a different port changes the id. A name substring matching
  more than one device is an error (listing the candidates' ids), not a silent
  first-match guess.
- `--stable` waits for a steady frame before capturing (useful right after a mode
  switch or reboot).
- **OCR** (`video read` and the dashboard button) is on-device (Apple Vision). It
  reads large boot-screen / BIOS text well; very small console fonts can produce
  a few character confusions (e.g. `1`/`l`, `2`/`Z`).

## Serial console

A target can have **several named serial interfaces** (e.g. a main `console` and
a `bmc`). Each port is **exclusive** — only one consumer can hold it at a time —
but a single `watch` daemon owns *all* of them at once.

```
paniolo serial add console -t <target> --device <path>   # add a named interface
paniolo serial add bmc -t <target> --device /dev/ttyUSB1 --baud 9600
paniolo serial set console -t <target> --sense cts        # update fields
paniolo serial rm <name> -t <target>                      # drop a named interface
paniolo serial connect [target] [-i name]      # interactive terminal (tio) in your shell
paniolo serial watch [target]                  # run the daemon for ALL interfaces;
                                               #   they appear in the dashboard pane
paniolo serial log [-i name] [options]         # print captured output (timestamped)
paniolo serial show [target]                   # list interfaces + daemon status
paniolo serial stop                            # release the ports
paniolo serial devices                         # list serial devices on the host
paniolo serial dtr [target] [-i name] [--ms N] # pulse DTR line (J2 power button header)
paniolo serial reset [target] [-i name]        # soft reset via brief DTR pulse
```

`--name` defaults to `console`, so a single-interface setup needs no flags. With
one interface, `-i`/`--interface` can be omitted everywhere. Don't run `connect`
and `watch` (or an external `screen`/`tio`) on the **same device** at once —
start one, or `stop`/close the other first.

### Reading captured output

While `watch` is running, the daemon keeps a **rolling, timestamped capture log**
per interface (persisted on disk, so it survives a daemon restart and you can read
it even after `stop`). The live view stays in the dashboard; use `serial log` when
you want to *read back* what scrolled past:

```
paniolo serial log                    # most recent ~200 lines (sole interface)
paniolo serial log -i bmc --tail 50   # most recent 50 lines of the 'bmc' interface
paniolo serial log --since 1840       # only lines after sequence #1840 (polling)
paniolo serial log --from 1800 --to 1860   # a specific line range
paniolo serial log --json             # JSON Lines (seq, ts_ms, text) for parsing
paniolo serial log --raw              # keep ANSI colors / control bytes
```

With more than one interface configured, pass `-i <name>` to choose one (omitting
it errors and lists the names). Each line is shown as `[<UTC timestamp>] #<seq>
<text>`. The `seq` is a stable, monotonic line number — note it, then come back
later with `--since <seq>` to get only what's new, or `--from/--to` to re-read an
exact span. Output is ANSI-stripped by default; a `*` after the sequence number
marks the current unterminated line (e.g. a `login:` prompt with no newline yet).

## Power control

```
paniolo power-cycle [target]           # run the target's power_cycle_cmd script
paniolo power-state [target]           # show power state (requires sense signal + daemon)
paniolo serial dtr [target] [--ms N]   # pulse DTR line on J2 header (soft/hard press)
paniolo serial reset [target]          # soft reset via brief DTR pulse
```

`power-cycle` runs the shell script set with
`paniolo power set -t <name> --cycle-cmd <script>`.
The script is responsible for the full off→on sequence (HA API, PDU relay, GPIO, etc.).

DTR commands drive the target's physical power button via an FTDI serial
adapter wired to the Pi J2 header. A ≤500 ms pulse is a soft button event; ≥3000 ms
is a hard PMIC power-off. Set the default interface with
`paniolo power set -t <name> --serial-interface console`.

## Targets on a remote control host (a "lab")

When the machine wired to the target isn't the one you're running paniolo on,
describe your **lab** in one git-tracked TOML file and point paniolo at it with
`--lab <file>` or `PANIOLO_LAB`. Each target's `host` names a control host;
commands then run **transparently on that host over SSH** — you don't ssh by hand.

```toml
# mylab.toml
[hosts.bench1]
ssh = "curtisg@bench1.local"     # ssh destination ("local" = this machine)
# identity = "~/.ssh/lab_key"    # set this if your ssh-agent offers many keys
#                                  (avoids "Too many authentication failures")
# paniolo_cmd = "/Users/curtisg/.local/bin/paniolo"  # if paniolo isn't on the
#                                  host's non-interactive ssh PATH

[targets.fortune]
host = "bench1"

[targets.fortune.netboot]
interface = "enx00e04c08d9a0"
tftp_root = "/home/curtisg/tftp/fortune"

[[targets.fortune.serial]]
name = "console"
device = "/dev/serial/by-id/usb-FTDI_FT232R_USB_UART_BG00W7NY-if00-port0"

[targets.fortune.power]
cycle_cmd = "/home/curtisg/scripts/power-cycle.sh"
```

```
export PANIOLO_LAB=~/labs/mylab.toml
paniolo netboot start fortune      # runs on bench1, transparently
paniolo power-cycle fortune
paniolo netboot logs -f fortune    # streams back live
paniolo serial connect fortune     # interactive tio over ssh -t
paniolo console fortune            # dashboard tunnelled to your browser; Ctrl-C to close
```

Notes:
- With **no** `--lab`/`PANIOLO_LAB`, paniolo uses the default lab at
  `~/.config/paniolo/lab.toml` (auto-created on first write).
- The lab file is **CLI-managed and hand-edit friendly**: all the config
  commands edit it surgically (your comments survive), and it stays your
  git-tracked source of truth.
- **Channels can live on different hosts** (per-channel `--host`): each command
  runs on the host of the channel it touches. Composites (`console`) need
  their channels co-located on one host.
- Still over plain SSH if you prefer: `ssh bench1 "paniolo …"` works too, but the
  lab makes location transparent.

### Authoring a lab (discovery + provisioning)

After hand-writing a host's connection info (`[hosts.bench1] ssh = …`), let
paniolo discover its hardware and propose the target block:

```
paniolo discover                                  # list THIS host's hardware
paniolo --lab mylab.toml configure fortune --host bench1
```

`configure` runs discovery on `bench1` over SSH and prints a proposed
`[targets.fortune]` block — best-guessing the USB-Ethernet interface and serial
device, listing other candidates as comments. It **writes nothing**: you review
it, paste it into the lab file, and commit (the lab is your reviewed source of
truth). Edit the guesses as needed; add `tftp_root`/`power` (not discoverable).

Provision a control host's daemons from your dev machine:

```
paniolo --lab mylab.toml setup --host bench1       # runs `paniolo setup` on bench1
```

(The host needs the paniolo CLI + source already present; this builds the Rust
daemons there over an ssh PTY.)

## Quick reference — gotchas

- Serial port is exclusive: one of `connect` / `watch` / external `tio`/`screen`.
- `~/.cargo/bin` (the CLI and daemons) must be on `PATH`; a stale Python
  `paniolo` in `~/.local/bin` shadows it (`uv tool uninstall paniolo`).
- `paniolo console` auto-starts both daemons if they aren't running.
- Netboot requires passwordless `sudo` (`ip` on Linux, `ifconfig` on macOS).
- netboot and ffx are mutually exclusive on the link — use `paniolo netif mode`
  to switch; entering ffx mode stops netboot so a power-cycle boots from SD.
- Remote (lab) targets: if ssh fails with "Too many authentication failures",
  set the host's `identity` (agent key-spray); if the remote can't find paniolo,
  set its `paniolo_cmd` to an absolute path.
- OCR is strongest on large text; tiny console fonts may misread some characters.

---

Licensed under the Apache License, Version 2.0.
