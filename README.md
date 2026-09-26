# paniolo

<img src="docs/paniolo-on-horse.png" align="left" width="240" alt="A robot paniolo in a cowboy hat, on horseback, holding the reins">

Agent-controlled target machine wrangler for low-level software development.

"Paniolo" is the Hawaiian word for cowboy. When you write bootloaders,
firmware, or OS bring-up code, paniolo lets an AI agent take the reins: it can
netboot the target, watch its output, send it input, and power-cycle it, with
no human needed at each iteration.

<br clear="left">

## See it work

An agent on a laptop drives an Intel NUC's firmware setup on a CI rack in
another room. It exits setup, catches `F2` during POST (the firmware's
power-on self-test) through the KVM's emulated keyboard, confirms it is back in
setup by OCR (reading text off the captured screen), then navigates with an
absolute mouse. What the dashboard saw:

![NUC BIOS, driven over the KVM from another room](docs/demo/nuc-bios-puppet-screen.gif)

```console
$ paniolo hid send -t lab-nuc-1 key ENTER          # discard & exit → the NUC reboots
$ for i in $(seq 30); do paniolo hid send -t lab-nuc-1 key F2 >/dev/null; done
$ until paniolo video read lab-nuc-1 | grep -q -i "bios version"; do sleep 2; done
$ paniolo hid send -t lab-nuc-1 moveabs 19583 5871 ; paniolo hid send -t lab-nuc-1 click left
```

The [demos page](https://curtisgalloway.github.io/paniolo/demos/) has more: a
live desktop puppeted over HID, out-of-band power over Intel AMT, and a Pi
cold-booted through a relay and caught on serial.

---

## Capabilities

| Subsystem | Commands | What it does |
|---|---|---|
| [Netboot](https://curtisgalloway.github.io/paniolo/netboot/) | `paniolo netboot` | DHCP + TFTP + HTTP netboot over a direct USB-Ethernet link (Raspberry Pi, plus UEFI PXE / HTTP Boot for EDK2 boards) |
| [Remote labs](https://curtisgalloway.github.io/paniolo/distributed-control/) | `paniolo --lab …` | Drive targets on remote control hosts transparently over SSH; one git-tracked lab file |
| [Control hosts](https://curtisgalloway.github.io/paniolo/control-host/) | `paniolo skill control-host` | Blank Raspberry Pi to agent-reachable control host in one human action: stock Raspberry Pi OS plus a hardware-validated cloud-init seed that brings it up with SSH authorized and paniolo installed; the bundled skill carries the flashing procedure |
| [Link mode](https://curtisgalloway.github.io/paniolo/netif/) | `paniolo netif` | Atomically switch the link between netboot, ffx-over-IPv6, bare-link (`link`), and `off` modes; toggle `link`/`off` to test the link up/down, or `down-hard` to force a real carrier drop (WoL off + admin-down) |
| [Video](https://curtisgalloway.github.io/paniolo/video/) | `paniolo video` | HDMI capture via warm-stream daemon; on-device OCR |
| [Serial](https://curtisgalloway.github.io/paniolo/serial/) | `paniolo serial` | Serial console — interactive (tio) or daemon-backed with timestamped rolling log |
| [Power control](https://curtisgalloway.github.io/paniolo/power/) | `paniolo power on/off`, `paniolo power-cycle`, `paniolo power-state`, `paniolo serial dtr/reset` | DTR-based hardware power button (J2 header; opt-in per serial interface) and generic shell-command hooks (on/off/cycle/state); helpers: `cambrionix` (Cambrionix hub ports), `zigplug` (Zigbee smart plugs), `shellyplug` (Shelly Gen2+ plugs/relays over local HTTP RPC), `amt` (Intel AMT/vPro over WS-Man, with true power-state readback) |
| [HID injection](https://curtisgalloway.github.io/paniolo/hid/) | `paniolo hid` | USB keyboard/mouse injection via a generic helper hook (`hidrig` KB2040 injector, or `ch9329` for Openterface Mini-KVM / KVM-Go and Sipeed NanoKVM-USB); KVM input from the web console |
| [Switchable USB media](https://curtisgalloway.github.io/paniolo/usb/) | `paniolo usb` | Route a shared USB device (an Openterface KVM-Go's onboard microSD card) to the control host or the target, for hands-free physical boot media that firmware can see |
| [adb (Android targets)](https://curtisgalloway.github.io/paniolo/adb/) | `paniolo adb` | Drive an Android DUT over adb — console (`shell`/`run`), screen (`screencap`), and input — one USB cable, no capture/HID/serial rig |
| [Dashboard](https://curtisgalloway.github.io/paniolo/dashboard/) | `paniolo console` | Combined video + serial web UI; auto-starts daemons; `-i <name>` preselects a serial interface |
| Agent skills | `paniolo skill` | List the bundled agent guides (driving a target, GUI puppeting, building a control host), or print one's `SKILL.md` for an agent to read |
| Lab config & diagnostics | `paniolo target`/`host`/`config`, `paniolo discover`, `paniolo configure`, `paniolo doctor`, `paniolo daemons` | CLI-managed lab file (targets, hosts, channels), hardware discovery with a proposed config block, config-vs-reality probing, and a one-view daemon inventory with stop/restart |
| [Exit status and errors](https://curtisgalloway.github.io/paniolo/errors/) | `paniolo --json-errors …` | Every failure exits with a code naming its kind (not configured, host unreachable, daemon down, hook failed, timeout); `--json-errors` adds a one-line JSON object so scripts and agents branch without parsing messages |

Terms used above: a **target** (or DUT, device under test) is the machine
being developed on; a **control host** is the machine cabled to it. **KVM** is
a keyboard-video-mouse device that captures a machine's screen and emulates
its keyboard and mouse. **HID** is the USB class for keyboards and mice.
**TFTP** and **PXE** are the protocols firmware uses to fetch a boot image
over the network. **Intel AMT** is the out-of-band management built into vPro
chipsets.

---

## Installation

### macOS: Homebrew

The Homebrew tap installs a prebuilt **universal** binary (one download, both
Apple Silicon and Intel). You need no Rust or Swift toolchain and there is no
compile step:

```bash
brew tap curtisgalloway/tap
brew install paniolo
```

New releases then arrive with `brew upgrade`. Run `paniolo setup` once after
installing. It setuid-installs `netbootd-bpf-helper` (one sudo, for the
netboot raw-frame send path) and installs the optional zigplug helper.

`brew install --HEAD paniolo` builds from source instead. It is the same path
as `make install` below, useful for tracking `main` between releases.

### Linux: apt repository

On Debian 12+ / Raspberry Pi OS (amd64/arm64), use the signed apt repository
served from the docs site. New releases then arrive with a normal
`apt upgrade`:

```bash
sudo install -d /etc/apt/keyrings
sudo curl -fsSL -o /etc/apt/keyrings/paniolo.asc https://curtisgalloway.github.io/paniolo/apt/paniolo.asc
sudo tee /etc/apt/sources.list.d/paniolo.sources >/dev/null <<'EOF'
Types: deb
URIs: https://curtisgalloway.github.io/paniolo/apt
Suites: stable
Components: main
Signed-By: /etc/apt/keyrings/paniolo.asc
EOF
sudo apt update && sudo apt install paniolo
```

### A new Raspberry Pi control host

If the Linux box does not exist yet (a blank Raspberry Pi that is to become a
control host), skip the steps above. A hardware-validated cloud-init seed
brings it up with SSH authorized and the release `.deb` installed, in one
human action: flash the stock Raspberry Pi OS image, drop four files on its
boot partition, and power on.

- Walkthrough: [standing up a control host](https://curtisgalloway.github.io/paniolo/control-host/).
- Exact commands, including how to identify the right SD card before writing
  to it: the bundled skill, `paniolo skill control-host`.

### Release packages

Each [GitHub Release](https://github.com/curtisgalloway/paniolo/releases) has
the same prebuilt packages (`.deb` and tarball) for direct install:
`sudo apt install ./paniolo_<version>_<arch>.deb`.

With the apt repository or a release package, run `paniolo setup` once after
installing. It sets group membership and installs the optional zigplug
helper.

### From source

Requirements:

- macOS 10.14 (Mojave) or later, or Linux (x86-64 / arm64)
- [Homebrew](https://brew.sh) (macOS only — Linux uses the system package manager)
- Rust toolchain (`brew install rustup` on macOS, or `rustup.rs` on Linux)
- On Linux: `sudo apt-get install pkg-config libudev-dev libclang-dev cmake nasm`
  (`make install` checks for these and tells you what's missing)

```bash
git clone https://github.com/curtisgalloway/paniolo ~/src/paniolo
cd ~/src/paniolo
make install           # paniolo CLI + daemons + OCR helper, in one step
```

`make install` bootstraps the CLI with `cargo install --path cli`, then runs
`paniolo setup`, which compiles and installs all of paniolo's binaries.

Where things land:

- **The `paniolo` CLI** is the only thing on PATH (`~/.cargo/bin`).
- **Daemons and helpers** (`hdmicap`, `serialcap`, `netbootd`, `cambrionix`,
  `hidrig`, `ch9329`, `shellyplug`, `amt`) and **the OCR helper** (`visionocr`
  on macOS via `swiftc`, `linuxocr` on Linux) go in the private libexec dir
  `~/.local/libexec/paniolo/bin`. paniolo finds them there without polluting
  your PATH. Run one directly with `paniolo helper <name> [args…]` (no name
  lists them).
- **Bundled agent skills** go in `~/.local/share/paniolo/skills` (the
  `.deb`/tarball ship them under `/usr/share/paniolo/skills`).
  `paniolo skill [NAME]` lists them or prints one for an agent to read.
- **On macOS**, `setup` also installs `netbootd-bpf-helper` setuid-root (one
  sudo) for the `netbootd` raw-frame send path.
- **Configuration** is one CLI-managed lab file
  (`~/.config/paniolo/lab.toml`); see
  [notes/config-redesign.md](notes/config-redesign.md).

Each component is one static binary, and the core needs no Python
environment. Netboot is served by the single-binary `netbootd` (Rust) engine.
The one Python component is the optional `zigplug` Zigbee smart-plug helper;
`setup` installs it as a uv tool with its shim in the libexec dir.

To pick up code changes after pulling or editing, re-run it:

```bash
make install           # rebuilds and reinstalls everything (idempotent)
```

For faster iteration, `make rust` builds and installs only the Rust crates,
skipping the OCR/setuid/zigplug steps. `make help` lists every target. The
underlying commands also work directly. Note that the helpers install with
`--root` so they land in libexec, not on PATH:

```bash
cargo install --path ~/src/paniolo/cli        # if the CLI changed
cargo install --path ~/src/paniolo/hdmicap   --root ~/.local/libexec/paniolo  # if hdmicap changed
cargo install --path ~/src/paniolo/serialcap --root ~/.local/libexec/paniolo  # if serialcap changed
cargo install --path ~/src/paniolo/netbootd  --root ~/.local/libexec/paniolo  # if netbootd changed (re-run `paniolo setup` to re-setuid the helper on macOS)
cargo install --path ~/src/paniolo/cambrionix --root ~/.local/libexec/paniolo # if cambrionix changed
cargo install --path ~/src/paniolo/hidrig    --root ~/.local/libexec/paniolo  # if hidrig changed
cargo install --path ~/src/paniolo/ch9329    --root ~/.local/libexec/paniolo  # if ch9329 changed
cargo install --path ~/src/paniolo/shellyplug --root ~/.local/libexec/paniolo # if shellyplug changed
cargo install --path ~/src/paniolo/amt       --root ~/.local/libexec/paniolo  # if amt changed
UV_TOOL_BIN_DIR=~/.local/libexec/paniolo/bin uv tool install --force ~/src/paniolo/zigplug # if zigplug changed
```

USB HID injection (`paniolo hid`) shells out to a helper that speaks the
[HID serial protocol](https://curtisgalloway.github.io/paniolo/dev/hid-serial-protocol/):
`hidrig`, the client for the KB2040 injector, or `ch9329` for CH9329-based KVM
devices (Openterface Mini-KVM / KVM-Go, Sipeed NanoKVM-USB). See the
[HID injection guide](https://curtisgalloway.github.io/paniolo/hid/).

---

## Remote control pattern

The intended use is an AI agent or script on a dev machine that SSHes into the
control Mac to drive the target:

```bash
# Configure target once
ssh control-mac "paniolo target add target-machine"
ssh control-mac "paniolo netboot set -t target-machine --interface en3 --tftp-root ~/pxe"
ssh control-mac "paniolo power set -t target-machine --cycle-cmd /path/to/power-cycle.sh"

# Deploy a new kernel and boot
TFTP_ROOT=$(ssh control-mac "paniolo netboot tftp-root target-machine")
scp out/kernel.img control-mac:"${TFTP_ROOT}/kernel_2712.img"
ssh control-mac "paniolo netboot start target-machine"
ssh control-mac "paniolo netboot logs -f target-machine"

# Interact with the console
ssh control-mac "paniolo serial log -t target-machine -i console --tail 50"

# Power cycle and repeat
ssh control-mac "paniolo power-cycle target-machine"
```

---

## Documentation

Full documentation is at
**[curtisgalloway.github.io/paniolo](https://curtisgalloway.github.io/paniolo/)**
and describes paniolo's verified current state. Its source is in
[`docs/`](docs/README.md).

- **User guides:** the per-subsystem guides linked in the
  [capabilities table](#capabilities).
- **[Demos](https://curtisgalloway.github.io/paniolo/demos/):** recorded runs
  from the CI rack.
- **[Tested hardware](https://curtisgalloway.github.io/paniolo/hardware/):**
  the bench gear each subsystem is verified with.
- **[Standing up a control host](https://curtisgalloway.github.io/paniolo/control-host/):**
  a blank Raspberry Pi to an agent-reachable host in one human action.
- **Developer documentation**, published alongside: start with the
  [**architecture overview**](https://curtisgalloway.github.io/paniolo/dev/architecture/)
  for the whole-system design, then the
  [requirements tracker](https://curtisgalloway.github.io/paniolo/dev/requirements/),
  the interface specs, and the Hardware-CI integration (KernelCI/LAVA,
  Fuchsia/botanist) design. Its source is [`docs/dev/`](docs/dev).

Point-in-time records (the design a feature was built from, a bring-up's
findings, a plan that has shipped) live in [`notes/`](notes/README.md). They
are kept for the record, are not maintained against the current code, and are
not published.

---

## License

Apache 2.0 — see [LICENSE](LICENSE).
