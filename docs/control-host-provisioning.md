# Provisioning a Linux control host

> **Status: design, not implemented.** This doc specifies the unattended-install
> recipe ("seed") for standing up a new Linux control host — an x86 mini-PC or a
> Raspberry Pi — from blank hardware to agent-reachable. It complements
> [pi4-control-host.md](pi4-control-host.md) (per-bench hardware, wiring, and
> UART/gadget configuration), and builds on the distributed-control principle
> that control hosts are disposable. Nothing here exists yet; the generator and
> templates land under `packaging/` when built.

## The problem

[Distributed control](distributed-control.md) principle 3 says control hosts are
**stateless executors**: "re-image it, re-run `paniolo setup` on it, and it
resumes its role from the lab file with nothing to restore." That is doctrine
without tooling — there is no actual re-image path. Standing up a control host
today means a manual OS install plus a checklist of hand-run steps, which is
exactly the kind of tedium the rest of paniolo exists to delete.

The paniolo-specific delta on a stock Linux install is deliberately tiny,
because the hard parts already shipped:

- The **Linux `.deb`** (built by `.github/workflows/release.yml` from
  [`packaging/nfpm.yaml`](https://github.com/curtisgalloway/paniolo/blob/main/packaging/nfpm.yaml),
  published for **amd64 and arm64**, smoke-tested via `apt-get install` in CI)
  carries the CLI, every helper daemon, linuxocr, and the agent skills.
- `paniolo setup` has a packaged-install mode (no checkout, no Rust toolchain)
  that finishes per-user platform steps.
- Everything host-specific lives in the **lab file**, not on the host.

So the recipe's real job is: get a blank box to the point where that existing
machinery can take over. Everything else would be maintaining a Linux distro
for an audience of two boxes — a full custom image pipeline (mkosi/debos/Yocto)
was considered and rejected as wrong-sized.

## The contract: the seed's job ends at first SSH

**Definition of done for the seed:** after one human action (flash media, plug
in, power on), the box comes up on the network with the operator's SSH key
authorized, the right groups set, and `paniolo` on `PATH`. An agent can do
everything after that through the existing front door:

1. Add a `[hosts.<name>]` entry to the lab file (`ssh`, `hostname`, `identity`).
2. `paniolo setup --host <name>` for any remaining per-user platform steps.
3. Discovery-assisted `paniolo configure` to wire targets, reviewed and
   committed by a human as usual.

What deliberately stays **out** of the seed: lab configuration, target wiring,
daemon state, credentials beyond the one SSH key. Baking any of that into the
install would fork the source of truth away from the lab file.

## Design: one cloud-init core, two wrappers

Ubuntu Server's unattended-install mechanisms converge on **cloud-init**, so
one shared `user-data` core serves both platforms with a thin delivery wrapper
each:

| Platform | Base OS | Mechanism | Delivery |
|---|---|---|---|
| x86 box (NUC-class or any UEFI PC) | Ubuntu Server 24.04 LTS amd64 | subiquity **autoinstall** (embeds the cloud-init core) | Generator emits a bootable USB installer; installs to internal disk unattended |
| Raspberry Pi 4 / 5 | Ubuntu Server 24.04 LTS arm64 **preinstalled image** | cloud-init **NoCloud** | Flash the stock image; generator writes `user-data` / `network-config` (and optional `config.txt` fragment) onto the FAT `system-boot` partition |

The Pi path is the lighter of the two — no installer even runs; the image is
the disk and cloud-init applies the seed on first boot.

### The shared cloud-init core

Sketch (the generator templates this from its inputs):

```yaml
hostname: <host>
users:
  - name: <user>
    groups: [sudo, dialout, video]     # dialout/video: what `paniolo setup` would ask for
    shell: /bin/bash
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    ssh_authorized_keys:
      - <operator pubkey>
ssh_pwauth: false
package_update: true
packages:
  - tesseract-ocr                      # the .deb only Recommends it; be explicit
runcmd:
  # Fetch the paniolo .deb for this architecture from the GitHub release
  # (latest by default; the generator can pin --version).
  - arch=$(dpkg --print-architecture)
  - curl -fsSL -o /tmp/paniolo.deb "<release asset URL for ${arch}>"
  - apt-get install -y /tmp/paniolo.deb
```

Notes:

- **Groups at user creation.** The `.deb` intentionally does not manage group
  membership (per-user concern, see the nfpm manifest comment); cloud-init's
  `users:` block does it declaratively, so first login already has
  serial/video access and `setup` finds nothing to fix.
- **No passwords anywhere.** Key-only SSH; passwordless sudo matches the
  bench-appliance role (single-operator lab hardware, not a multi-user host).
- **`tesseract-ocr` explicitly**, because apt installs Recommends by default
  but the seed shouldn't depend on that default surviving.

### Platform extras

- **x86 wrapper:** the autoinstall YAML adds `identity`/`storage` (whole-disk,
  direct layout) around the shared core, plus a late-command for the `.deb` if
  installing inside the target chroot is preferred over first-boot `runcmd`.
- **Pi wrapper:** optionally appends the control-host fragment to
  `/boot/firmware/config.txt` — `enable_uart=1` + `dtoverlay=disable-bt` for
  the PL011 GPIO console, `dtoverlay=dwc2,dr_mode=peripheral` for the future
  HID gadget — and disables the serial getty. These are only wanted when the
  bench uses the GPIO UART / gadget port, so they are generator flags, not
  defaults. Wiring, voltage-level, and topology guidance stays in
  [pi4-control-host.md](pi4-control-host.md).
- **Networking:** netboot's refusal to serve on the primary NIC means a
  control host needs **two** interfaces (uplink + DUT link). The seed can
  carry a netplan stanza for the DUT-link NIC when its name is known at
  generation time (USB-GbE adapters are predictable via `by-id`/MAC match),
  but this is optional — the link can equally be configured later through the
  normal lab-file flow. Wi-Fi-as-uplink on the Pi is supported by
  `network-config`, with the caveat that the PSK sits in plaintext on the
  boot partition.

## Host sizing

The control-host workload is lighter than it looks. Per subsystem, for one or
two DUTs:

| Load | Cost |
|---|---|
| `serialcap` | ≤ 11.5 KB/s per 115200-baud console — negligible |
| `netbootd` | bursty: kernel+initramfs at line rate for seconds per boot, then idle |
| `hdmicap` | MJPEG dongles are teed compressed to `/preview` **without re-encode** (Linux MJPEG tee, `hdmicap/src/capture.rs`) — a few % of one core |
| `video shot` | one turbojpeg decode on demand |
| `video read` | the only heavy op: decode + Tesseract, on the order of 1–3 s on a Pi 4, ~1 s on a Pi 5 — per-call latency, not sustained load |

Recommendations:

- **Raspberry Pi 4 (4 GB) is the sensible floor; Pi 5 if OCR latency matters.**
  2 GB works. Aggregate CPU of a Pi 4 is in the same class as a low-end x86
  mini-PC (Celeron N3060-class), which already qualifies.
- **Pi 3 / Zero 2 are excluded** as full hosts: 100 Mbps USB-attached
  Ethernet (netboot would crawl), 1 GB RAM, and the `.deb` is arm64-only.
  A serial-only role would work but isn't worth a special seed path.
- **Any 64-bit UEFI x86 box qualifies** — the floor is roughly "anything sold
  as a mini-PC in the last decade."
- **Storage endurance, not CPU, is the Pi's real risk** for an always-on box
  writing capture logs: use a quality SD card, or a USB SSD (Pi 5: NVMe HAT)
  for a bench that runs 24/7.
- OCR **accuracy** is a platform property, not a sizing one: Linux hosts use
  Tesseract (weaker than Apple Vision on small console fonts) regardless of
  how fast the box is.

## Generator shape

Start as a standalone script — `packaging/host-seed/` — with no runtime
dependency on the CLI; promote to a `paniolo host-seed` subcommand only if it
earns it. Inputs:

| Input | Meaning | Default |
|---|---|---|
| `--hostname` | host name for the box | required |
| `--user` / `--ssh-key` | operator account + authorized pubkey | required |
| `--flavor` | `x86-usb` (autoinstall USB) or `pi-sd` (NoCloud files for a flashed card) | required |
| `--paniolo-version` | release to fetch | latest |
| `--dut-nic` | netplan stanza for the DUT-link interface | none (configure later) |
| `--pi-uart` / `--pi-gadget` | Pi `config.txt` fragments | off |

Output is either a bootable USB image (x86) or a small set of files to drop on
`system-boot` after flashing the stock Pi image (`user-data`,
`network-config`, optional `config.txt` fragment) — the latter is trivially
scriptable around Raspberry Pi Imager or `dd`.

## Alternatives considered

- **Full custom image pipeline** (mkosi / debos / NixOS / Yocto): rejected —
  the paniolo delta is ~40 lines of declarative config, and an image pipeline
  buys bit-reproducibility (which stateless hosts don't need — first boot
  starts with `apt upgrade` anyway) at the cost of owning kernel/security
  updates and multi-GB build artifacts.
- **Raspberry Pi OS + Imager `custom.toml` / `firstrun.sh`**: rejected —
  Imager customization covers user/Wi-Fi/SSH but not arbitrary packages or
  first-boot commands without patching `firstrun.sh` by hand. Ubuntu on both
  platforms means one template, one mental model, one `.deb` pipeline. The
  `config.txt` mechanics from [pi4-control-host.md](pi4-control-host.md)
  transfer unchanged (`/boot/firmware/` layout is the same).
- **PXE-boot the installer via `netbootd`**: not the plan (bootstrap
  chicken-and-egg — it needs an existing control host), but the UEFI PXE path
  is hardware-verified, so a "paniolo installs its own next control host"
  flow falls out nearly free later. Noted for the demo pile.

## Open questions

- **Version pinning vs. latest** for the `.deb` fetch — latest is convenient;
  pinning + recorded sha256 is more reproducible. Lean: default latest, offer
  `--paniolo-version`.
- Whether the x86 flavor should install the `.deb` in the installer's
  late-commands (present in the installed system before first boot) or via
  first-boot `runcmd` like the Pi (one shared code path). Lean: shared
  `runcmd` path unless it proves flaky.
- Whether the seed should register the host in DNS / DHCP reservations, or
  leave addressing entirely to the network (current lean: not the seed's job;
  the lab file's `ssh` field is the contract).
