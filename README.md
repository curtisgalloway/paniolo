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
cd ~/src/paniolo
make install           # Python CLI + Rust daemons + OCR helper, in one step
```

`make install` runs `uv tool install --reinstall .` for the Python CLI, then
`paniolo setup`, which compiles and installs the Rust daemons (`hdmicap`,
`serialcap`, `netbootd`) and the OCR helper (`visionocr` on macOS via `swiftc`,
`linuxocr` on Linux) into `~/.cargo/bin`. Netboot is served by the
single-binary `netbootd` (Rust) engine. (On macOS, `setup` also installs
`netbootd-bpf-helper` setuid-root — one sudo — for the `netbootd` raw-frame
send path.)

> **Rust control plane:** the CLI itself is being rewritten in Rust (the
> `cli/` crate; see [docs/config-redesign.md](docs/config-redesign.md)).
> `paniolo setup` from the Rust CLI installs the daemons *and* the Rust
> `paniolo` binary into `~/.cargo/bin` — a single static binary per control
> host, no Python environment needed. Configuration moves to one CLI-managed
> lab file (`~/.config/paniolo/lab.toml`).

To pick up code changes after pulling or editing, just re-run it:

```bash
make install           # rebuilds and reinstalls everything (idempotent)
```

Or target one layer while iterating: `make python` (CLI only), `make rust`
(the Rust crates only, skipping OCR/setuid), `make native` (`paniolo setup`
only). `make help` lists every target. The underlying commands still work
directly if you prefer:

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
ssh control-mac "paniolo target add target-machine"
ssh control-mac "paniolo netboot set -t target-machine --interface en3 --tftp-root ~/pxe"
ssh control-mac "paniolo power set -t target-machine --cycle-cmd /path/to/power-cycle.sh"

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

A *target* is a named machine you want to control. Configuration lives in a
single CLI-managed **lab file** (`~/.config/paniolo/lab.toml`, or `--lab` /
`PANIOLO_LAB`): hosts plus targets, each target's hardware described as
*channels* (`netboot`, `serial`, `power`, `video`) bound to the host they're
physically attached to. No daemon required. If exactly one target is
configured it is the default and can be omitted from every command.

See [docs/config-redesign.md](docs/config-redesign.md) for the model and
[Target configuration](docs/netboot.md#target-configuration) for the fields.

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
