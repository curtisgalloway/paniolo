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
| [Power control](docs/power.md) | `paniolo button/reset/power-cycle/power-state` | Hardware power button via FTDI DTR line wired to Pi J2 header |
| [HID injection](docs/hid.md) | `paniolo hid` | USB keyboard/mouse injection via a two-board KB2040 rig |
| [Dashboard](docs/dashboard.md) | `paniolo console` | Combined video + serial web UI; `-i <name>` preselects a serial interface |
| [HA power switch](docs/power.md#home-assistant-power-switch) | `paniolo power-switch` | Cut/restore power via a Home Assistant smart switch |

---

## Requirements

- macOS 10.14 (Mojave) or later
- Python 3.11+
- [uv](https://docs.astral.sh/uv/) (`brew install uv`)
- [Homebrew](https://brew.sh)
- Rust toolchain (for hdmicap, serialcap — `brew install rustup`)

---

## Installation

```bash
git clone https://github.com/curtisgalloway/paniolo ~/src/paniolo
uv tool install ~/src/paniolo
paniolo setup          # installs dnsmasq, tftp-now, hdmicap, serialcap, visionocr
```

`paniolo setup` compiles and installs the Rust daemons (`hdmicap`, `serialcap`)
and the Swift OCR helper (`visionocr`) into `~/.cargo/bin`, and installs the
TFTP and DHCP servers via Homebrew.

To pick up code changes after pulling or editing:

```bash
uv tool install --reinstall ~/src/paniolo
cargo install --path ~/src/paniolo/hdmicap    # if hdmicap changed
cargo install --path ~/src/paniolo/serialcap  # if serialcap changed
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
    --ha-power-entity switch.my_plug"

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
| HA config | `~/.config/paniolo/ha.toml` |
| HID config | `~/.config/paniolo/hid.toml` |
| Netboot daemon state | `~/.local/share/paniolo/<name>/netboot.json` |
| hdmicap discovery | `$TMPDIR/hdmicap/daemon.json` |
| serialcap discovery | `$TMPDIR/serialcap/daemon.json` |
| Serial capture logs | `$TMPDIR/serialcap/capture/<name>/serial.jsonl` |

---

## License

Apache 2.0 — see [LICENSE](LICENSE).
