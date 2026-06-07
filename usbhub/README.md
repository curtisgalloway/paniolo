<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
-->

# usbhub

Per-port power control for off-the-shelf USB hubs — turn the VBUS on an
individual port on, off, or power-cycle it. Pure Rust via
[nusb](https://crates.io/crates/nusb); no kernel driver is displaced and no
interface is claimed. Works on **macOS and Linux**.

It uses the same mechanism as [uhubctl](https://github.com/mvp/uhubctl)
(USB hub-class `SET_FEATURE`/`CLEAR_FEATURE PORT_POWER` requests) but
addresses hubs and ports differently, to be safe and stable on a bench full
of identical hardware:

- **Hubs are addressed by a model profile, not a bus location.** A consumer
  "hub" is usually a cascade of 2–3 hub chips, each appearing twice (a USB 3
  device and its USB 2 companion). A profile captures that internal shape, and
  usbhub matches it against the live topology — so your command keeps working
  after you replug the hub into a different port. Only when several *identical*
  hubs share one host do you need to pin one (the error tells you how).
- **Ports are addressed by their physical silkscreen number.** The profile
  maps each printed port number to its location on both the USB 3 and USB 2
  sides, and `on`/`off`/`cycle` act on both — otherwise a device can fall back
  to the other topology and stay powered.
- **Switching is refused unless a human has verified the port.** Hub chips
  routinely *claim* per-port power switching they can't actually do — the port
  "turns off" in the status word while the device keeps drawing 5 V. usbhub
  never trusts that claim: a port is switchable only after a human watched it
  physically lose power (the `learn` workflow records this), so a power-cycle
  that silently does nothing can't slip into your automation.

## Install

```bash
cargo install --git https://github.com/curtisgalloway/paniolo usbhub
```

Needs a [Rust toolchain](https://rustup.rs). The binary lands in
`~/.cargo/bin/usbhub`.

> usbhub lives in the [paniolo](https://github.com/curtisgalloway/paniolo)
> repository (a bench-automation toolkit) but builds and runs entirely on its
> own — the command above pulls only what this crate needs.

### Linux permissions

Sending control requests to a hub needs write access to its
`/dev/bus/usb/...` node. Either run as root, or install a udev rule granting
your user access to USB hubs — uhubctl's
[rule](https://github.com/mvp/uhubctl#linux-usb-permissions) works (match the
hub by vendor id or `bDeviceClass==09`). macOS needs no special permissions.

## Quick start

**1. See what's on the bus** (read-only; safe any time):

```bash
usbhub probe
```

This lists every hub, its port chains, and what each hub *claims* about power
switching — a hint for where verification is worth trying, not a guarantee.

**2. Build a profile for your hub.** The interactive walkthrough cuts power on
each port and asks you to confirm the device actually died:

```bash
usbhub learn run
```

`run` is a guided wizard. It asks for a model name:

- **A new name** → it asks how many physical ports the hub has, walks you
  through unplugging/replugging it once (to capture the chip cascade), then for
  **each port** has you plug the probe in, detects where it landed, cuts the
  power, and asks whether it actually died.
- **An existing profile's name** → it loads that profile (resolving its chips
  against the live hub, no replug needed) and drops you straight into the
  review step, where you can re-verify or add any port.

Either way you finish at a **review** screen showing every port's verdict;
type a port number to (re)do it, or `save` to write the profile and print the
commands to drive the hub. Prompts have ↑/↓ history and line editing, and the
session is saved as you go, so you can quit (Ctrl-D) and resume any time.
Everything it does is also available as the discrete `usbhub learn <step>`
subcommands (below) — including `usbhub learn edit <model>` to load an existing
profile for hand editing.

**Picking a probe device.** The verify step cuts a port's power and asks you
whether the device *actually* lost power — so use something whose power state
you can see. Any probe that enumerates on *either* bus is enough (see the
limitation below); pick one with a visible power state:

- A device with a **power LED** is ideal — the LED answers the "did it really
  lose power?" question directly.
- A **phone** is excellent — its charging indicator is the signal. When the
  port really cuts VBUS, charging stops (visible on screen) and the phone drops
  off the bus. It just has to *enumerate* as a USB device: an Android with USB
  debugging (adb) on works well; an iPhone (or any phone) sitting in
  charge-only mode may not present a data device, so the walk won't see it —
  unlock it / pick a data mode, or use a different probe.
- A flash drive works but has no power indicator, so you're relying on the
  enumeration check alone.

Detection is by **bus topology**, not USB speed, so a probe whose connection
speed the OS doesn't report still maps correctly — that was the fix for phones
that enumerate without a speed.

To help, the verify step **re-enumerates the bus after cutting power** and tells
you whether the probe disappeared: if it's *still* on the bus, the port did not
cut VBUS (it recommends "alive / not controllable"). If it vanished, that's
consistent with power loss — but a data-only disconnect, or a self-powered
device (phone, powered hub) losing only its data link, looks identical, so your
eyes on the charging icon / LED / power meter remain the deciding vote.

**3. Drive it** by physical port number:

```bash
usbhub --model <name> status          # per-port table: mapping, verdict, live power
usbhub --model <name> state 7         # prints exactly "on" or "off"
usbhub --model <name> on 7
usbhub --model <name> off 7
usbhub --model <name> cycle 7         # off → 3 s → on → confirm (--delay-ms to change)
```

## Commands

```
usbhub probe                          read-only topology + claimed switching modes
usbhub models                         list known model profiles
usbhub --model M status               per-port table with live power bits
usbhub --model M state <port>         print exactly "on" or "off"
usbhub --model M on|off <port>        switch + read-back confirm
usbhub --model M cycle <port>         off → delay → on → confirm  [--delay-ms 3000]
usbhub learn run                      interactive profile builder (TTY)
usbhub learn <step>                   scriptable profile-builder steps (see below)
```

Add `--side usb3|usb2` to act on only one topology (debugging; default is
both). Add `--at usb3=BUS:CHAIN,usb2=BUS:CHAIN` only when several identical
hubs share the host — the ambiguity error prints the exact pins to paste.

### Scriptable learn steps

`learn run` drives the same discrete, resumable subcommands the wizard uses;
you can also run them directly from a script or an agent. Each does one
observation or records one human report, persists the session, and prints what
to do next:

```
usbhub learn edit [model]             load <model>'s profile to edit if it exists,
                                      else snapshot the bus to capture anew
usbhub learn unplugged                snapshot; then plug the hub back in
usbhub learn plugged                  diff → capture the hub's chip cascade
usbhub learn port <n>                 then plug the probe into physical port n
usbhub learn verify <n>               cut power; look at the probe
usbhub learn verify <n> --result dead|alive [--reason "..."]
usbhub learn status                   progress + suggested next step
usbhub learn save --model <name>      write the profile, print the commands
usbhub learn abort                    discard (restores power if mid-verify)
```

## Where things are stored

Profiles and the in-progress learn session live in:

- `$USBHUB_STATE_DIR` if set, else
- `$XDG_CONFIG_HOME/usbhub`, else `~/.config/usbhub`

Profiles are plain TOML under `profiles/<model>.toml` — version them, share
them, or hand-edit them (a `controllable = true` you add by hand is just as
valid as one `learn` recorded, as long as *you* verified it). `--profile-dir`
overrides the location per command.

## Limitation: the two buses are controlled in tandem

A USB 3 port is two logical devices — a USB 3 hub and its USB 2 companion —
but they share one physical VBUS, and on real hubs the power usually only
drops when *both* sides' power bits are cleared. usbhub therefore treats a
physical port as a single unit: you map and verify it once (with a probe on
whichever bus it happens to enumerate), and `on`/`off`/`cycle` act on the same
`(chip, port)` location on **both** buses together.

This assumes the USB 2 companion mirrors the USB 3 side's port numbering for a
given physical port — true for the cascaded Realtek-style hubs this was built
for. **Not supported:** hubs whose two buses expose independent VBUS, or number
their ports differently between the USB 2 and USB 3 sides. (`--side usb3|usb2`
on the control commands can target one bus for debugging, but it isn't a way to
power the two halves independently.)

## License

Apache-2.0. See [LICENSE](LICENSE).
