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

It guides you through unplugging/replugging the hub (to capture its chip
cascade), plugging a probe device into each physical port (to map them), and a
power-off check per port (to verify controllability). At the end it writes a
profile and prints the exact commands to drive the hub.

The `learn>` prompt takes the **same commands as `usbhub learn <cmd>`** (the
`usbhub learn` prefix is optional and names can be abbreviated — `ver 7` works),
so the `Next:` hints it prints are typeable verbatim. Type `help` for the list
and `quit` to leave (the session is saved; resume any time).

**Picking a probe device.** The verify step cuts a port's power and asks you
whether the device *actually* lost power — so use something whose power state
you can see:

- A **USB 3 hub with a power LED** is the ideal probe: it enumerates on both
  the USB 3 and USB 2 sides at once, mapping both in a single plug, and its LED
  answers the "did it really lose power?" question.
- A **phone** is also excellent — its charging indicator is the signal. When
  the port really cuts VBUS, charging stops (visible on screen) and the phone
  drops off the bus. (A phone almost always enumerates as USB 2.0, so it maps
  only the USB 2 side; use the USB 3 hub if you want both sides in one plug.)
- A flash drive works but maps only its own side and has no power indicator.

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

`learn run` is a wrapper over discrete, resumable subcommands you can also
drive from a script or an agent — each does one observation or records one
human report, persists the session, and prints what to do next:

```
usbhub learn start                    snapshot the bus; then unplug the hub
usbhub learn unplugged                snapshot; then plug the hub back in
usbhub learn plugged                  diff → capture the hub's chip cascade
usbhub learn port <n>                 then plug the probe into physical port n
usbhub learn verify <n>               cut power; look at the probe
usbhub learn verify <n> --result dead|alive [--reason "..."]
usbhub learn status                   progress + suggested next step
usbhub learn finish --model <name>    write the profile, print the commands
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

## License

Apache-2.0. See [LICENSE](LICENSE).
