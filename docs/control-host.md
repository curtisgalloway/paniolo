<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# Standing up a control host

This page takes a blank machine to a working **control host** in one human
action: flash a card, plug it in, power on. The box comes up on the network
with your key authorized, the right groups set, and `paniolo` on `PATH`.
Everything after that happens over SSH.

A control host is the small Linux box physically cabled to your targets. It
holds the serial adapters, the HDMI capture dongle, the HID (USB
keyboard/mouse) rig, the netboot link, and the power switching. Your dev
machine talks to it over SSH, and [distributed control](distributed-control.md)
ships it the slice of the lab file it needs at command time.

It holds no durable target config, so a control host is **disposable**:
re-image it, re-run `paniolo setup`, and it resumes its role from the lab
file with nothing to restore. This page is also the re-image path.

## Picking the hardware

The workload is lighter than it looks. For one or two targets:

| Subsystem | Cost on the host |
|---|---|
| `serialcap` | 11.5 KB/s per 115200-baud console, at most |
| `netbootd` | bursty — kernel and initramfs at line rate for a few seconds per boot, then idle |
| `hdmicap` | MJPEG dongles are teed compressed, without re-encode: a few percent of one core |
| `video shot` | one JPEG decode, on demand |
| `video read` | the only heavy operation — decode plus OCR, roughly 1 to 3 s on a Pi 4 and about 1 s on a Pi 5, as per-call latency rather than sustained load |

- **A Raspberry Pi 4 with 4 GB is the sensible floor. Use a Pi 5 if OCR
  latency matters.** 2 GB works.
- **Any 64-bit UEFI x86 box qualifies.** The floor is roughly anything sold as
  a mini-PC in the last decade.
- **Pi 3 and Zero 2 are excluded.** 100 Mbps USB-attached Ethernet would make
  netboot crawl, 1 GB of RAM is tight, and the `.deb` is arm64 only.
- **Storage endurance is the Pi's real risk**, not CPU. A box that runs
  24/7 writing capture logs wants a quality SD card, or a USB SSD; on a Pi 5,
  an NVMe HAT (an add-on board for an M.2 SSD).
- **Plan for two network interfaces.** `netbootd` refuses to serve on the
  primary NIC, so the DUT (device under test) link needs its own adapter
  alongside the uplink.

OCR (reading text off the captured screen) *accuracy* depends on the
platform, not the hardware size. Linux hosts use Tesseract however fast the
box is, and it is weaker than Apple Vision on small console fonts.

## Raspberry Pi, from blank card to first SSH

Raspberry Pi OS has used cloud-init (the standard Linux first-boot configuration
tool) as its native first-boot mechanism since Trixie (Debian 13). No
installer runs and no custom image gets built: the stock image is the disk,
and four files on its boot partition configure it.

**The step-by-step procedure lives in the bundled `control-host` agent
skill**, written to be executed rather than skimmed:

```bash
paniolo skill control-host
```

It is one file, readable by a person or an agent, with the exact commands
for both macOS and Linux. They live only there on purpose: a flashing
procedure copied into two documents drifts. You can also
[read it on GitHub](https://github.com/curtisgalloway/paniolo/blob/main/skills/control-host/SKILL.md).

The shape of it:

1. **Download** Raspberry Pi OS Lite arm64, Trixie or newer.
2. **Identify the card.** This step can destroy data: `dd` to the wrong
   device overwrites a disk silently, and external and backup drives appear in
   the same listing as the card. Confirm the device node by size and
   removability before writing anything.
3. **Write the image**, then mount the FAT boot partition.
4. **Install the seed** from
   [`packaging/host-seed/pi-sd/`](https://github.com/curtisgalloway/paniolo/tree/main/packaging/host-seed/pi-sd):
   three cloud-init files, plus an empty `ssh` file, which is what actually
   enables sshd (the SSH server). Names must be exact: cloud-init silently
   ignores a misnamed file.
5. **Render the five placeholders** in `user-data` (hostname, account, GECOS
   full-name field, public key, paniolo version) and verify
   none survived.
6. **Eject and boot.** First boot takes several minutes: it waits for the
   network, updates apt, downloads the `.deb`, and reboots itself. Then
   `ssh <user>@<hostname> paniolo --version` should answer.
7. **Enroll it**: `paniolo host add`, then `paniolo configure <target> -H
   <name>` to get a proposed target block you review and commit.

A seeded host should not need `paniolo setup`. On Linux its packaged mode
does two things: add your account to the `dialout` and `video` groups, and
warn if Tesseract is missing. The seed grants both groups at account creation
and installs `tesseract-ocr`, so setup finds nothing to fix.

The seed carries no lab configuration, no target wiring, and no credential
beyond that one public key. Those belong in the lab file, where a human
reviews them.

## When a boot does not come up

Get on the console (HDMI, or the Pi's serial UART) and ask cloud-init:

```bash
cloud-init status --long
journalctl -u cloud-init -u cloud-init-local -b
```

**`extended_status: degraded done` is normal here.** cloud-init warns that it
cannot find `cc_netplan_nm_patch` (a Raspberry Pi OS packaging wart), but
every stage still completes. Judge the run by `errors` and
`recoverable_errors`, not by the word "degraded".

Two failures to recognize on sight:

- **SSH refuses the connection** even though the boot finished. The empty
  `ssh` flag file is missing. Pi OS ships sshd disabled, and the
  `enable_ssh: true` cloud-config key does not turn it on: it is a downstream
  extension that was a silent no-op on the tested image.
- **The boot hangs at "Local Stage (pre-network)"** and only a power-cut
  recovers it. Something in `bootcmd` is waiting on the network, which is
  waiting on the stage that is blocked. The seed's `systemctl enable ssh` is
  written without `--now` for exactly this reason.

If you edit the seed on a card that has **already booted once**, also change
`instance_id` in `meta-data`. cloud-init caches per-instance state on the root
filesystem; with an unchanged id, your edited `user-data` never re-runs.

## x86 mini-PCs

Not built yet. Until it exists, install Ubuntu Server by hand and apply the
same account, group, and package steps the Pi seed performs.

The planned design is Ubuntu Server autoinstall wrapping the same cloud-init
core, delivered as a bootable USB installer. It is written up, with the
alternatives considered and rejected, in
[the design note](https://github.com/curtisgalloway/paniolo/blob/main/notes/control-host-provisioning.md).
