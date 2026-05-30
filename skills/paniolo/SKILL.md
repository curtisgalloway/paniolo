---
name: paniolo
description: Control a physical target machine (SBC, e.g. a Raspberry Pi) with the paniolo CLI during low-level bring-up — netboot it over a direct USB-Ethernet link, watch and OCR its HDMI screen, drive its serial console, and power-cycle it. Use when you need to boot, observe, type into, screenshot, read the screen of, or power a target/board through paniolo.
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
uv tool install .          # installs the `paniolo` CLI into ~/.local/bin
paniolo setup              # builds/installs hdmicap, serialcap, netbootd, visionocr;
                           # on macOS also installs netbootd-bpf-helper setuid-root (one sudo)
```

`uv tool install .` is required first — without it the `paniolo` command doesn't
exist yet. Run both steps once per machine. Make sure `~/.local/bin` (uv tools)
and `~/.cargo/bin` (Rust daemons) are on your `PATH`.

To pick up Python code changes after pulling or editing:

```
cd ~/src/paniolo && uv tool install --reinstall .
```

## Configure a target

```
paniolo target set <name> --interface <iface> \
    [--tftp-root <dir>] \
    [--ha-power-entity <switch.entity>]
```

- `--interface` auto-detects a USB-Ethernet adapter if omitted.
- Serial consoles are configured separately with `paniolo serial setup` (a target
  can have several named interfaces); they're preserved across `target set` runs.
- Inspect or remove: `paniolo target show` / `paniolo target clear <name>`.

## Netboot (DHCP + TFTP)

Boot a board over the direct USB-Ethernet link:

```
paniolo netboot start [target]            # serve DHCP + TFTP on the interface (python engine)
paniolo netboot start [target] --engine rust  # experimental single-binary netbootd
paniolo netboot tftp-root [target]        # print where to drop boot files
paniolo netboot status [target]
paniolo netboot logs -f [target]          # follow the combined log
paniolo netboot stop [target]
```

`start` refuses an interface that carries the system default route (a primary
NIC) — the netboot link must be a dedicated USB-Ethernet adapter. The default
engine is the pure-Python DHCP+TFTP pair; `--engine rust` runs the experimental
`netbootd` binary (opt-in, for validation). On macOS the rust engine's BPF send
path uses the setuid `netbootd-bpf-helper` installed by `paniolo setup`.

Put boot files in the target's TFTP root (for a Raspberry Pi 5, the kernel goes
in as `kernel_2712.img`). Needs passwordless `sudo` for `ifconfig` (it assigns
the interface's static IP).

## Video — capture, preview, OCR

```
paniolo video setup                   # detect + save the capture device
paniolo video watch                   # start the capture daemon (background)
paniolo video preview                 # open the dashboard in a browser
paniolo video shot [--stable] [--out frame.png]   # one lossless PNG
paniolo video read [--stable]         # OCR the current screen, print text
paniolo video show                    # device + daemon status
paniolo video stop
```

- The **dashboard** (default `http://127.0.0.1:8723/`) shows live video on top, a
  serial terminal below, and an **OCR button** that reads the current screen.
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
paniolo serial setup [target] --name console   # add/update a named interface
                                               #   (--device auto-detected if omitted)
paniolo serial setup [target] --name bmc --device /dev/ttyUSB1 --baud 9600
paniolo serial remove <name> [-t target]       # drop a named interface
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
`paniolo target set <name> --power-cycle-cmd <script>`.
The script is responsible for the full off→on sequence (HA API, PDU relay, GPIO, etc.).

DTR commands drive the target's physical power button via an FTDI serial
adapter wired to the Pi J2 header. A ≤500 ms pulse is a soft button event; ≥3000 ms
is a hard PMIC power-off. Set the default interface with
`paniolo target set <name> --power-serial console`.

## Driving it remotely over SSH

The control host runs paniolo; you can operate it from anywhere:

```
ssh control "paniolo netboot start fortune"
TFTP=$(ssh control "paniolo netboot tftp-root fortune")
scp kernel.img control:"$TFTP/kernel_2712.img"
ssh control "paniolo netboot logs -f fortune"
ssh control "paniolo power-cycle fortune"
ssh control "paniolo netboot stop fortune"
```

## Quick reference — gotchas

- Serial port is exclusive: one of `connect` / `watch` / external `tio`/`screen`.
- `~/.local/bin` (uv tool) and `~/.cargo/bin` (Rust daemons) must be on `PATH`.
- `paniolo console` auto-starts both daemons if they aren't running.
- Netboot requires passwordless `sudo` (`ip` on Linux, `ifconfig` on macOS).
- OCR is strongest on large text; tiny console fonts may misread some characters.

---

Licensed under the Apache License, Version 2.0.
