<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# Standing up a control host

A **control host** is the small Linux box physically cabled to your targets —
it holds the serial adapters, the HDMI capture dongle, the HID rig, the
netboot link, and the power switching. Your dev machine talks to it over SSH,
and [distributed control](distributed-control.md) ships it the slice of the
lab file it needs at command time.

Because it holds no durable target config, a control host is **disposable**:
re-image it, re-run `paniolo setup`, and it resumes its role from the lab
file with nothing to restore. This page is the re-image path.

The contract is one human action. Flash a card, plug it in, power on — and the
box comes up on the network with your key authorized, the right groups set,
and `paniolo` on `PATH`. Everything after that happens over SSH.

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
  24/7 writing capture logs wants a quality SD card, or a USB SSD — on a Pi 5,
  an NVMe HAT.
- **Plan for two network interfaces.** `netbootd` refuses to serve on the
  primary NIC, so the DUT link needs its own adapter alongside the uplink.

OCR *accuracy* is a platform property rather than a sizing one: Linux hosts
use Tesseract regardless of how fast the box is, and it is weaker than Apple
Vision on small console fonts.

## Raspberry Pi, from blank card to first SSH

Raspberry Pi OS has shipped cloud-init as its native first-boot mechanism
since Trixie, so no installer runs and no custom image gets built — the stock
image is the disk, and three files on its boot partition configure it.

Those files live in
[`packaging/host-seed/pi-sd/`](https://github.com/curtisgalloway/paniolo/tree/main/packaging/host-seed/pi-sd).
They are the seed that was hardware-validated on a Pi 5, with every gotcha
from that bring-up encoded in them.

1. **Flash Raspberry Pi OS Lite arm64**, Trixie or newer, by any means. Skip
   Raspberry Pi Imager's customization screen — cloud-init supersedes it.

2. **Copy the three files** onto the FAT `bootfs` partition, keeping their
   names exactly. cloud-init silently ignores a misnamed file.

    ```bash
    cp packaging/host-seed/pi-sd/user-data \
       packaging/host-seed/pi-sd/meta-data \
       packaging/host-seed/pi-sd/network-config /Volumes/bootfs/
    ```

3. **Create an empty `ssh` file** on the same partition. This is what actually
   enables sshd, and it is not optional.

    ```bash
    touch /Volumes/bootfs/ssh
    ```

4. **Replace the placeholders** in `user-data` — hostname, account name and
   GECOS, your SSH public key, and the paniolo release to install. Then check
   that none are left:

    ```bash
    grep -n '<[a-z-]*>' /Volumes/bootfs/user-data     # must print nothing
    ```

5. **Eject the card, boot the Pi, and wait.** First boot updates apt and
   downloads the `.deb`, so allow a few minutes. Then confirm from your dev
   machine:

    ```bash
    ssh <user>@<hostname> paniolo --version
    ```

6. **Enroll the host** in the lab file, then propose its targets:

    ```bash
    paniolo host add <name> --ssh <user>@<hostname>
    paniolo configure <target> -H <name>
    ```

    `configure` runs discovery on the named host over SSH and prints a
    proposed `[targets.<target>]` block, best-guessing the serial device and
    USB-Ethernet interface. It writes nothing authoritative — you review it,
    paste it into the lab file, and commit.

You should not need `paniolo setup` on a seeded host. On Linux its packaged
mode does two things: add your account to the `dialout` and `video` groups,
and warn if Tesseract is missing. The seed grants both groups at account
creation and installs `tesseract-ocr`, so setup finds nothing to fix.

The seed carries no lab configuration, no target wiring, and no credential
beyond that one public key — all of which belong in the lab file, where a
human reviews them.

## When a boot does not come up

Get on the console (HDMI, or the Pi's serial UART) and ask cloud-init:

```bash
cloud-init status --long
journalctl -u cloud-init -u cloud-init-local -b
```

**`extended_status: degraded done` is normal here.** cloud-init warns that it
cannot find `cc_netplan_nm_patch`, a Raspberry Pi OS packaging wart, while
every stage still completes. Judge the run by `errors` and
`recoverable_errors`, not by the word "degraded".

Two failures are worth recognizing on sight:

- **SSH refuses the connection** even though the boot finished. The `ssh` flag
  file from step 3 is missing. Pi OS ships sshd disabled, and the
  `enable_ssh: true` cloud-config key does not turn it on — it is a downstream
  extension that was a silent no-op on the tested image.
- **The boot hangs at "Local Stage (pre-network)"** and only a power-cut
  recovers it. Something in `bootcmd` is waiting on the network, which is
  waiting on the stage that is blocked. The seed's `systemctl enable ssh` is
  written without `--now` for exactly this reason.

If you edit the seed on a card that has **already booted once**, change
`instance_id` in `meta-data` as well. cloud-init caches per-instance state on
the root filesystem, and an unchanged id means your edited `user-data` never
re-runs.

## x86 mini-PCs

Not built yet. The design — Ubuntu Server autoinstall wrapping the same
cloud-init core, delivered as a bootable USB installer — is written up in
[the design note](https://github.com/curtisgalloway/paniolo/blob/main/notes/control-host-provisioning.md),
along with the alternatives that were considered and rejected. Until it
exists, install Ubuntu Server by hand and apply the same account, group, and
package steps the Pi seed performs.
