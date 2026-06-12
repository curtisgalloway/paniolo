# paniolo

Agent-controlled target machine wrangler for low-level software development.

"Paniolo" is the Hawaiian word for cowboy. The idea: an AI agent sits at the
reins while you're writing bootloaders, firmware, or OS bring-up code — paniolo
gives it the controls to netboot the target, watch its output, send it input,
and power-cycle it without human intervention at each iteration.

---

## Capabilities

| Subsystem | Commands | What it does |
|---|---|---|
| [Netboot](docs/netboot.md) | `paniolo netboot` | DHCP + TFTP netboot over a direct USB-Ethernet link |
| [Remote labs](docs/distributed-control.md) | `paniolo --lab …` | Drive targets on remote control hosts transparently over SSH; one git-tracked lab file |
| [Link mode](docs/netif.md) | `paniolo netif` | Atomically switch the link between netboot and ffx-over-IPv6 modes |
| [Video](docs/video.md) | `paniolo video` | HDMI capture via warm-stream daemon; on-device OCR |
| [Serial](docs/serial.md) | `paniolo serial` | Serial console — interactive (tio) or daemon-backed with timestamped rolling log |
| [Power control](docs/power.md) | `paniolo power on/off`, `paniolo power-cycle`, `paniolo power-state`, `paniolo serial dtr/reset` | DTR-based hardware power button (J2 header) and generic shell-command hooks (on/off/cycle/state); helpers: `cambrionix` (Cambrionix hub ports), `usbhub` (off-the-shelf USB hub ports), `zigplug` (Zigbee smart plugs), `shellyplug` (Shelly Gen2+ plugs/relays over local HTTP RPC) |
| [HID injection](docs/hid.md) | `paniolo hid` | USB keyboard/mouse injection via a generic helper hook (`hidrig` KB2040 injector); KVM input from the web console |
| [Dashboard](docs/dashboard.md) | `paniolo console` | Combined video + serial web UI; auto-starts daemons; `-i <name>` preselects a serial interface |

---

## Documentation

Full docs live in [`docs/`](docs/README.md). Start with the
[**architecture overview**](docs/architecture.md) for the whole-system design, then the
per-subsystem guides linked above. The [tested-hardware list](docs/hardware.md) covers the
bench gear each subsystem is verified with. Hardware-CI integration (KernelCI/LAVA,
Fuchsia/botanist) design and the project requirements tracker are under
[`docs/`](docs/README.md) as well.

---

## Requirements

- macOS 10.14 (Mojave) or later, or Linux (x86-64 / arm64)
- [Homebrew](https://brew.sh) (macOS only — Linux uses the system package manager)
- Rust toolchain (`brew install rustup` on macOS, or `rustup.rs` on Linux)
- On Linux: `sudo apt-get install pkg-config libudev-dev libclang-dev cmake nasm`
  (`make install` checks for these and tells you what's missing)

---

## Installation

On Linux, prebuilt packages (amd64/arm64 `.deb` and tarball, Debian 12+ /
Raspberry Pi OS) are attached to each
[GitHub Release](https://github.com/curtisgalloway/paniolo/releases) —
`sudo apt install ./paniolo_<version>_<arch>.deb`, then run `paniolo setup`
once for group membership and the optional zigplug helper. Or build from
source:

```bash
git clone https://github.com/curtisgalloway/paniolo ~/src/paniolo
cd ~/src/paniolo
make install           # paniolo CLI + daemons + OCR helper, in one step
```

`make install` bootstraps the CLI with `cargo install --path cli`, then runs
`paniolo setup`, which compiles and installs all of paniolo's binaries. Only
the `paniolo` CLI lands on PATH (`~/.cargo/bin`); the daemons and helpers
(`hdmicap`, `serialcap`, `netbootd`, `cambrionix`, `hidrig`, `usbhub`,
`shellyplug`) and the OCR
helper (`visionocr` on macOS via `swiftc`, `linuxocr` on Linux) install into
the private libexec dir `~/.local/libexec/paniolo/bin`, where paniolo finds
them without polluting your PATH — run one directly with
`paniolo helper <name> [args…]` (no name lists them). One static binary per
component; the core needs no Python environment. (The optional `zigplug`
Zigbee smart-plug helper is the one Python component — `setup` installs it as
a uv tool with its shim in the libexec dir.) Netboot is served by the
single-binary `netbootd` (Rust) engine. (On macOS, `setup` also installs
`netbootd-bpf-helper` setuid-root — one sudo — for the `netbootd` raw-frame
send path.) Configuration is one CLI-managed lab file
(`~/.config/paniolo/lab.toml`); see
[docs/config-redesign.md](docs/config-redesign.md).

> **Upgrading from the Python CLI?** The old `make install` registered the
> Python `paniolo` as a uv tool; its `~/.local/bin/paniolo` shim shadows the
> Rust binary in `~/.cargo/bin`. Remove it once: `uv tool uninstall paniolo`
> (`make install` warns if a shadow is detected).

To pick up code changes after pulling or editing, just re-run it:

```bash
make install           # rebuilds and reinstalls everything (idempotent)
```

Or iterate faster with `make rust` (build + install the Rust crates only,
skipping the OCR/setuid/zigplug steps). `make help` lists every target. The
underlying commands still work directly if you prefer — note the helpers
install with `--root` so they land in libexec, not on PATH:

```bash
cargo install --path ~/src/paniolo/cli        # if the CLI changed
cargo install --path ~/src/paniolo/hdmicap   --root ~/.local/libexec/paniolo  # if hdmicap changed
cargo install --path ~/src/paniolo/serialcap --root ~/.local/libexec/paniolo  # if serialcap changed
cargo install --path ~/src/paniolo/netbootd  --root ~/.local/libexec/paniolo  # if netbootd changed (re-run `paniolo setup` to re-setuid the helper on macOS)
cargo install --path ~/src/paniolo/cambrionix --root ~/.local/libexec/paniolo # if cambrionix changed
cargo install --path ~/src/paniolo/hidrig    --root ~/.local/libexec/paniolo  # if hidrig changed
cargo install --path ~/src/paniolo/usbhub    --root ~/.local/libexec/paniolo  # if usbhub changed
cargo install --path ~/src/paniolo/shellyplug --root ~/.local/libexec/paniolo # if shellyplug changed
UV_TOOL_BIN_DIR=~/.local/libexec/paniolo/bin uv tool install --force ~/src/paniolo/zigplug # if zigplug changed
```

USB HID injection (`paniolo hid`) shells out to a helper speaking the
[HID serial protocol](docs/hid-serial-protocol.md) — by default `hidrig`,
the client for the KB2040 injector (see [docs/hid.md](docs/hid.md)).

---

## Remote control pattern

The intended use is an AI agent or script on a dev machine SSHing into the
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

## License

Apache 2.0 — see [LICENSE](LICENSE).
