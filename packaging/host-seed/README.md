<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# Control-host seed files

Unattended-install files that turn a blank Raspberry Pi into an
**agent-reachable paniolo control host** with one human action: flash a card,
plug it in, power on. They are cloud-init files: cloud-init is the first-boot
setup tool that Raspberry Pi OS runs, reading `user-data`, `meta-data` and
`network-config` from the boot partition.

This directory holds only the files. The user-facing walkthrough is
[docs/control-host.md](../../docs/control-host.md); the design rationale and
rejected alternatives are in
[notes/control-host-provisioning.md](../../notes/control-host-provisioning.md).

| Flavor | Directory | Status |
|---|---|---|
| `pi-sd` — Raspberry Pi 4/5, Raspberry Pi OS Trixie Lite arm64 | [`pi-sd/`](pi-sd) | **Hardware-validated** (Pi 5 8 GB, 2026-08-20) |
| `x86-usb` — UEFI mini-PC, Ubuntu Server autoinstall | — | Designed, not built |

There is no generator. You copy these files and edit them; the edit is four
placeholders, so the generator sketched in the design note has not been worth
building yet.

## What the seed does, and where it stops

The seed:

- creates the operator account with the right groups
- authorizes one SSH key and disables password login
- installs the paniolo `.deb` and the packages it wants
- brings up the wired uplink on DHCP

It deliberately does **not** carry lab configuration, target wiring, daemon
state, or any credential beyond that one public key. Those live in the lab
file; baking them into the image would create a second source of truth. The
seed's job ends at first SSH.

## Using them

**Follow `paniolo skill control-host`.** It is the single source of truth for
the steps, with exact commands for macOS and Linux: download the image,
identify the right removable device before writing to it, flash, mount, install
these files, render them, and enroll the result. Rationale, host sizing, and
troubleshooting are in [docs/control-host.md](../../docs/control-host.md).

In short: copy all three files to the FAT `bootfs` partition under exactly
these names, `touch` an empty `ssh` file beside them, render the placeholders,
and boot.

Only `user-data` has placeholders. Replace all five (see the table below),
then confirm none survive. This check strips the file's own comments first, so a clean run means
the payload really is rendered:

```bash
grep -v '^[[:space:]]*#' /Volumes/bootfs/user-data | grep '<[^>]*>' \
  && echo "FAIL: unrendered placeholders above" \
  || echo "OK: fully rendered"
```

Do not simplify that to `grep '<[a-z-]*>' user-data`. It matches the
template's explanatory comments, so it can never report success. Its character
class also excludes spaces, so it silently misses a placeholder that contains
one.

| Placeholder | Value |
|---|---|
| `<hostname>` | the host's name, matching what the lab file's `ssh` field will resolve |
| `<user>` | operator account name |
| `<full-name>` | GECOS field, cosmetic |
| `<ssh-public-key>` | one full public key line, e.g. `ssh-ed25519 AAAA… you@dev` |
| `<version>` | paniolo release to install, without the `v` — e.g. `0.2.0`, appearing twice on the same line |

## Gotchas these files encode

Each of these cost a debug cycle on the first build. They explain why the
files look the way they do. Do not "simplify" them back.

- **sshd is off by default on Pi OS, and `enable_ssh: true` does not turn it
  on.** That key is a Pi OS downstream extension, absent from upstream
  `cc_raspberry_pi`, and on the tested image it was a silent no-op. What works
  is the classic empty **`ssh` flag file** on `bootfs`: `sshswitch.service`
  survives in Trixie, enables sshd, and consumes the file. The seed uses three
  mechanisms (that file, a `bootcmd` fallback, and `openssh-server` in
  `packages`), because losing SSH on a headless box means a trip to the bench.
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
network-first (`0xf12`). That costs roughly 40 seconds per boot waiting for a
PXE (network boot) server that is not there. Set it SD-first (`0xf21`) with `rpi-eeprom-config
--edit` once the board becomes a control host.
