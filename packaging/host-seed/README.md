<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# Control-host seed files

Unattended-install files that take a blank Raspberry Pi to an
**agent-reachable paniolo control host** with one human action: flash a card,
plug it in, power on.

The user-facing walkthrough is [docs/control-host.md](../../docs/control-host.md).
This directory holds the files themselves. The design rationale and the
alternatives that were rejected are in
[notes/control-host-provisioning.md](../../notes/control-host-provisioning.md).

| Flavor | Directory | Status |
|---|---|---|
| `pi-sd` — Raspberry Pi 4/5, Raspberry Pi OS Trixie Lite arm64 | [`pi-sd/`](pi-sd) | **Hardware-validated** (Pi 5 8 GB, 2026-08-20) |
| `x86-usb` — UEFI mini-PC, Ubuntu Server autoinstall | — | Designed, not built |

There is no generator. These are files you copy and edit; the argument-taking
generator sketched in the design note has not earned its keep yet, and the
edit is four placeholders.

## What the seed does, and where it stops

It creates the operator account with the right groups, authorizes one SSH key,
disables password login, installs the paniolo `.deb` and the packages it wants,
and brings up the wired uplink on DHCP.

It deliberately does **not** carry lab configuration, target wiring, daemon
state, or any credential beyond that one public key. Those live in the lab
file, and baking them into the image would fork the source of truth. The
seed's job ends at first SSH.

## Using them

**The executable procedure is `paniolo skill control-host`** — downloading
the image, identifying the right removable device before writing to it,
flashing, mounting, installing these files, rendering them, and enrolling the
result. It has the exact commands for macOS and Linux, and it is the single
source of truth for the steps. The rationale, host sizing, and troubleshooting
are in [docs/control-host.md](../../docs/control-host.md).

In short: copy all three files to the FAT `bootfs` partition under exactly
these names, `touch` an empty `ssh` file beside them, render the placeholders,
and boot.

Only `user-data` has placeholders. Replace all five, then confirm none
survive. This check strips the file's own comments first, so a clean run means
the payload really is rendered:

```bash
grep -v '^[[:space:]]*#' /Volumes/bootfs/user-data | grep '<[^>]*>' \
  && echo "FAIL: unrendered placeholders above" \
  || echo "OK: fully rendered"
```

Do not simplify that to `grep '<[a-z-]*>' user-data`. It matches the
explanatory comments in the template, so it can never report success, and its
character class excludes spaces, so it silently misses a placeholder that
contains one.

| Placeholder | Value |
|---|---|
| `<hostname>` | the host's name, matching what the lab file's `ssh` field will resolve |
| `<user>` | operator account name |
| `<full-name>` | GECOS field, cosmetic |
| `<ssh-public-key>` | one full public key line, e.g. `ssh-ed25519 AAAA… you@dev` |
| `<version>` | paniolo release to install, without the `v` — e.g. `0.2.0`, appearing twice on the same line |

## Gotchas these files encode

Each of these cost a debug cycle on the first build. They are why the files
look the way they do — do not "simplify" them back.

- **sshd is off by default on Pi OS, and `enable_ssh: true` does not turn it
  on.** That key is a Pi OS downstream extension, absent from upstream
  `cc_raspberry_pi`, and on the tested image it was a silent no-op. What works
  is the classic empty **`ssh` flag file** on `bootfs`: `sshswitch.service`
  survives in Trixie, enables sshd, and consumes the file. The seed ships that,
  a `bootcmd` fallback, and `openssh-server` in `packages` — three mechanisms,
  because losing SSH on a headless box means a trip to the bench.
- **Never `systemctl enable --now ssh` in `bootcmd`.** `bootcmd` runs in the
  pre-network local stage. `--now` waits for `ssh.service`, which waits for the
  network, which waits for `cloud-init-local` — which is blocked in that very
  command. The boot deadlocks and only a power-cut recovers it. Plain
  `systemctl enable ssh` writes a symlink and returns instantly.
- **Re-seeding an already-booted card takes two edits**: change `user-data`
  *and* bump `instance_id` in `meta-data`. cloud-init caches per-instance
  state on the rootfs, so an unchanged id means the user, package, and runcmd
  stages never re-run and your edit appears to do nothing.
- **`optional: false` on the uplink** makes boot wait for the link. First boot
  fetches packages and the `.deb` over the network; without the wait those
  stages can run early and fail quietly.

## Bench note, not a seed concern

A board reused from netboot bring-up may have its EEPROM `BOOT_ORDER` set
network-first (`0xf12`), which costs roughly 40 seconds per boot waiting for a
PXE server that is not there. Set it SD-first (`0xf21`) with `rpi-eeprom-config
--edit` once the board becomes a control host.
