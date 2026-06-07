---
name: usbhub
description: Control per-port USB hub power — switch VBUS on individual hub ports with the `usbhub` CLI (a pure-Rust uhubctl alternative; macOS + Linux). Use when the user wants to power a USB device on/off/cycle by its hub port, find out which hub ports can actually cut power, build or edit a usbhub model profile (the `learn`/`probe` workflow), disambiguate multiple identical hubs, or wire USB hub power into a paniolo power hook. Covers the mental model (profiles, physical ports, the verify-or-refuse rule, both-buses-in-tandem), the human-in-the-loop `learn` workflow an agent drives, and the gotchas (hubs lie about switching; adb-phone vs charge-only probes; Linux udev).
---

<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-->

# usbhub — per-port USB hub power control

`usbhub` switches VBUS on individual ports of off-the-shelf USB hubs by issuing
USB hub-class control requests (the same mechanism as
[uhubctl](https://github.com/mvp/uhubctl)), in pure Rust via `nusb`. It works
on **macOS and Linux**, displaces no kernel driver, and claims no interface.

It ships inside the [paniolo](https://github.com/curtisgalloway/paniolo)
repo as a power-control helper but builds and runs entirely on its own. The CLI
itself is the best reference — **run `usbhub --help`** for the mental model and
workflow, and `usbhub <command> --help` for any subcommand. This skill is the
orientation an agent needs before reaching for it, especially the parts that
require a human at the bench.

## Mental model (read this first)

- **Hubs are addressed by a model PROFILE, not a bus address.** A consumer
  "hub" is a cascade of 2–3 hub chips, each appearing twice (a USB 3 device and
  its USB 2 companion). A profile (`<model>.toml`) records that internal shape;
  `usbhub` resolves it by matching the cascade against the live topology, so a
  command keeps working after the hub is replugged into a different host port.
  Pass `--model <name>` to every status/state/on/off/cycle command.
- **Ports are addressed by their PHYSICAL silkscreen number.** The profile maps
  each printed port number to its location on the hub's internal chips.
- **The two buses are controlled in tandem.** A port's USB 2 and USB 3 halves
  share one VBUS, and power usually only drops when *both* sides' power bits are
  cleared, so `on`/`off`/`cycle` act on both together. You map and verify a port
  once. (`--side usb3|usb2` targets one bus for debugging only.)
- **Switching is REFUSED unless a human has verified the port cuts power.**
  Hub chips routinely *claim* per-port switching they can't do — the port
  "turns off" in the status word while the device keeps drawing 5 V. A profile
  port needs `controllable = true`, which is recorded only by a human watching a
  device physically lose power (the `learn` workflow), never inferred from
  descriptors. Unlisted or unverified ports refuse with an explanatory error.

## Install / run

**Standalone** (anyone, no paniolo):

```bash
cargo install --git https://github.com/curtisgalloway/paniolo usbhub
usbhub --help
```

**Under paniolo:** built and installed by `make install` / `paniolo setup` into
the private libexec dir; run it by hand with `paniolo helper usbhub <args…>`.

**Linux permissions:** sending control requests to a hub needs write access to
its `/dev/bus/usb/...` node — install a udev rule (uhubctl's rule works: match
the hub by vendor id or `bDeviceClass==09`) or run as root. macOS needs nothing.

## Discover what's attached (read-only, safe any time)

```bash
usbhub probe                 # every hub, its port chains, and CLAIMED switching mode
usbhub models                # list known model profiles
usbhub --model M status      # the profile's ports: mapping, verdict, live power bits
```

`probe`'s "per-port (claimed)" is the chip's claim, **not** proof — it only
tells you where verification is worth trying.

## Drive power (needs a verified profile)

```bash
usbhub --model M state <port>          # prints exactly "on" or "off" (machine-readable)
usbhub --model M on <port>             # switch on  + read-back confirm
usbhub --model M off <port>            # switch off + read-back confirm
usbhub --model M cycle <port>          # off → delay → on → confirm  [--delay-ms 3000]
```

`<port>` is the physical silkscreen number. These refuse on a port the profile
doesn't mark `controllable = true`. If several identical hubs are attached,
resolution is ambiguous and the error prints ready-to-paste `--at
usb3=BUS:CHAIN,usb2=BUS:CHAIN` pins — add the right one to disambiguate.

## Building or editing a profile — the `learn` workflow

This is the part an agent must understand: **`usbhub learn` is agent-drivable,
but every step depends on a human physically acting at the bench.** The tool
observes USB enumeration; only a human can observe whether VBUS actually
dropped. The agent runs the command, relays the physical instruction to the
human, waits, and records what the human reports.

### Easiest: the guided wizard

```bash
usbhub learn run
```

It asks for a model name. A **new** name → it asks the port count, walks the
human through unplug/replug (to capture the cascade), then for each port has
them plug a probe in, detects where it landed, cuts power, and asks whether it
died. An **existing** profile name → it loads that profile (resolved against the
live hub, no replug) and goes straight to a review screen to re-verify or add
ports. Either way it ends at a review screen (type a port number to redo it, or
`save`). Prompts have ↑/↓ history. The wizard is interactive (a human types the
answers) — drive the discrete steps below when an agent needs to orchestrate.

### Discrete steps (agent-orchestrated)

Each step does one observation or records one human report, persists the
session, and prints a `Next:` line. Relay the physical instruction, then run the
next step:

```bash
usbhub learn edit [model]     # begin: load <model> to edit if it exists, else
                              #   snapshot the bus to capture a new one
                              #   → ask the human to UNPLUG the hub
usbhub learn unplugged        # snapshot after unplug → ask the human to PLUG IT BACK IN
usbhub learn plugged          # diff → captures the hub's full chip cascade (both buses)
usbhub learn port <n>         # ask the human to plug the PROBE into physical port n;
                              #   BLOCKS until the probe is seen (--timeout-secs 120)
usbhub learn verify <n>       # cuts power on port n (both buses); reports an
                              #   enumeration check, then ask the human: did it lose power?
usbhub learn verify <n> --result dead             # human says it died → controllable
usbhub learn verify <n> --result alive [--reason "ganged rail"]   # stayed powered → not
usbhub learn status           # progress + the suggested next step
usbhub learn abort            # discard the session (restores power if a verify is pending)
usbhub learn save --model <name>     # write the profile + print the wiring commands
```

The verify step has a built-in hint: after cutting power it re-enumerates and
reports whether the probe **disappeared** from the bus. Still present ⇒ the port
did NOT cut VBUS (record `alive`). Vanished ⇒ consistent with power loss, but a
data-only disconnect or a self-powered device looks identical — the human's eyes
on the device decide.

### Choosing a probe device (tell the human)

The verify question is "did it actually lose power?", so the probe needs a
visible power state:

- A device with a **power LED**, or a **phone** (watch the charging indicator).
  When the port really cuts VBUS, charging stops and the device drops off the bus.
- It only has to **enumerate** as a USB device. An Android with USB debugging
  (adb) enumerates fine; an iPhone or any phone in charge-only mode may present
  no data device, so the walk won't see it — unlock it / pick a data mode, or
  use another probe. (Detection is by bus topology, not USB speed, so a device
  the OS reports with no speed still maps.)
- Any probe on **either** bus is enough — the buses are controlled in tandem.

### Editing an existing profile needs the hub present

`learn edit <model>` and the wizard's edit path resolve the saved profile
against the **live hub**, so it must be plugged in (check `usbhub probe`). To
tweak a verdict/reason with the hub absent, hand-edit the TOML instead.

## Profiles: location and format

Profiles are TOML files named `<model>.toml` in the state dir, resolved as
`$USBHUB_STATE_DIR`, else `$XDG_CONFIG_HOME/usbhub`, else `~/.config/usbhub`
(under paniolo, `$PANIOLO_STATE_DIR` → `~/.config/paniolo/helpers/usbhub`).
`--profile-dir <DIR>` overrides per command. They're plain TOML — version them,
share them, or hand-edit (a `controllable = true` you add by hand is as valid as
one `learn` recorded, *as long as you actually verified it*).

## Wiring into a paniolo power hook

`usbhub` satisfies paniolo's generic power-hook contract (`state` prints
`on`/`off`; `cycle` owns the full off→delay→on→confirm). `learn save` prints the
exact block; wire a target with:

```bash
paniolo power set -t <target> \
    --cycle-cmd "usbhub --model <m> cycle <port>" \
    --on-cmd    "usbhub --model <m> on <port>" \
    --off-cmd   "usbhub --model <m> off <port>" \
    --state-cmd "usbhub --model <m> state <port>"
```

Add `--at usb3=BUS:CHAIN,usb2=BUS:CHAIN` to each only if several identical hubs
share the control host.

## Gotchas (earned at the bench)

- **Hub descriptors lie about per-port switching.** `probe`'s claim is a hint;
  only a human-watched verify (or a multimeter) settles it. This is the whole
  reason for the verify-or-refuse rule.
- **Both buses move together.** Don't try to power the USB 2 and USB 3 halves of
  a port independently — on these hubs VBUS only drops when both are cut, and
  `usbhub` doesn't support independent control. Hubs with independent per-bus
  VBUS or mismatched port numbering between the buses are unsupported.
- **Cutting power re-enumerates the device.** Anything on the port disconnects.
  A port that carries the control path for the hub being switched would saw off
  its own branch — mark such ports `controllable = false` with a reason.
- **Port power state resets when the hub loses power.** Replugging or
  power-cycling the whole hub turns every port back on (chip default).
- **`state` is cheap and honest** — it never caches and fails loudly rather than
  guessing; it's the hook agents poll.
