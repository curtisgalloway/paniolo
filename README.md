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
| [Video](docs/video.md) | `paniolo video` | HDMI capture via warm-stream daemon; on-device OCR |
| [Serial](docs/serial.md) | `paniolo serial` | Serial console — interactive (tio) or daemon-backed with timestamped rolling log |
| [Power control](docs/power.md) | `paniolo serial dtr/reset`, `paniolo power-cycle`, `paniolo power-state` | DTR-based hardware power button (J2 header) and script-based power cycling |
| [HID injection](docs/hid.md) | `paniolo hid` | USB keyboard/mouse injection via a two-board KB2040 rig |
| [Dashboard](docs/dashboard.md) | `paniolo console` | Combined video + serial web UI; auto-starts daemons; `-i <name>` preselects a serial interface |

---

## Documentation

Full docs live in [`docs/`](docs/README.md). Start with the
[**architecture overview**](docs/architecture.md) for the whole-system design, then the
per-subsystem guides linked above. Hardware-CI integration (KernelCI/LAVA, Fuchsia/botanist)
design and the project requirements tracker are under [`docs/`](docs/README.md) as well.

---

## Requirements

- macOS 10.14 (Mojave) or later, or Linux (x86-64 / arm64)
- Python 3.11+
- [uv](https://docs.astral.sh/uv/) (`brew install uv` on macOS, or the [uv installer](https://docs.astral.sh/uv/getting-started/installation/) on Linux)
- [Homebrew](https://brew.sh) (macOS only — Linux uses the system package manager)
- Rust toolchain (for hdmicap, serialcap — `brew install rustup` on macOS, or `rustup.rs` on Linux)

---

## Installation

```bash
git clone https://github.com/curtisgalloway/paniolo ~/src/paniolo
uv tool install ~/src/paniolo
paniolo setup          # builds hdmicap + serialcap; installs the OCR helper
```

`paniolo setup` compiles and installs the Rust daemons (`hdmicap`, `serialcap`,
`netbootd`) and the OCR helper (`visionocr` on macOS via `swiftc`, `linuxocr` on
Linux) into `~/.cargo/bin`. The default DHCP and TFTP netboot servers are
**pure-Python** — no external servers to install. (On macOS, `setup` also
installs the legacy `tftp-now` via Homebrew, and installs `netbootd-bpf-helper`
setuid-root — one sudo — for the experimental `--engine rust` netboot path.)

To pick up code changes after pulling or editing:

```bash
uv tool install --reinstall ~/src/paniolo
cargo install --path ~/src/paniolo/hdmicap    # if hdmicap changed
cargo install --path ~/src/paniolo/serialcap  # if serialcap changed
cargo install --path ~/src/paniolo/netbootd   # if netbootd changed (re-run `paniolo setup` to re-setuid the helper on macOS)
```

For the USB HID commands, install the optional `pyserial` extra:

```bash
uv tool install --with pyserial ~/src/paniolo
```

---

## Remote control pattern

The intended use is an AI agent or script on a dev machine SSHing into the
control Mac to drive the target:

```bash
# Configure target once
ssh control-mac "paniolo target set target-machine \
    --interface en3 \
    --tftp-root ~/pxe \
    --power-cycle-cmd /path/to/power-cycle.sh"

# Deploy a new kernel and boot
TFTP_ROOT=$(ssh control-mac "paniolo netboot tftp-root target-machine")
scp out/kernel.img control-mac:"${TFTP_ROOT}/kernel_2712.img"
ssh control-mac "paniolo netboot start target-machine"
ssh control-mac "paniolo netboot logs -f target-machine"

# Interact with the console
ssh control-mac "paniolo serial log -i console --since --tail 50 target-machine"

# Power cycle and repeat
ssh control-mac "paniolo power-cycle target-machine"
```

---

## Concepts

### Target

A *target* is a named machine you want to control. Its configuration lives in
`~/.config/paniolo/targets/<name>.toml`. One config file per target; no daemon
required. If exactly one target is configured it is the default and can be
omitted from every command.

See [`paniolo target set --help`](docs/netboot.md#target-configuration) for all fields.

### Runtime paths

| Purpose | Path |
|---|---|
| Target configs | `~/.config/paniolo/targets/<name>.toml` |
| Video config | `~/.config/paniolo/video.toml` |
| HID config | `~/.config/paniolo/hid.toml` |
| Netboot daemon state | `~/.local/share/paniolo/<name>/netboot.json` |
| hdmicap discovery | `$XDG_RUNTIME_DIR/hdmicap/daemon.json` (Linux) / `$TMPDIR/hdmicap/daemon.json` (macOS) |
| serialcap discovery | `$XDG_RUNTIME_DIR/serialcap/daemon.json` (Linux) / `$TMPDIR/serialcap/daemon.json` (macOS) |
| Serial capture logs | `$XDG_RUNTIME_DIR/serialcap/capture/<name>/serial.jsonl` (Linux) |

---

## License

Apache 2.0 — see [LICENSE](LICENSE).
