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
| [Power control](docs/power.md) | `paniolo power on/off`, `paniolo power-cycle`, `paniolo power-state`, `paniolo serial dtr/reset` | DTR-based hardware power button (J2 header) and generic shell-command hooks (on/off/cycle/state) |
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
- [Homebrew](https://brew.sh) (macOS only — Linux uses the system package manager)
- Rust toolchain (`brew install rustup` on macOS, or `rustup.rs` on Linux)

---

## Installation

```bash
git clone https://github.com/curtisgalloway/paniolo ~/src/paniolo
cd ~/src/paniolo
make install           # paniolo CLI + daemons + OCR helper, in one step
```

`make install` bootstraps the CLI with `cargo install --path cli`, then runs
`paniolo setup`, which compiles and installs all of paniolo's binaries — the
`paniolo` CLI itself plus the daemons and helpers (`hdmicap`, `serialcap`, `netbootd`, `cambrionix`) —
and the OCR helper (`visionocr` on macOS via `swiftc`, `linuxocr` on Linux)
into `~/.cargo/bin`. One static binary per component, no Python environment.
Netboot is served by the single-binary `netbootd` (Rust) engine. (On macOS,
`setup` also installs `netbootd-bpf-helper` setuid-root — one sudo — for the
`netbootd` raw-frame send path.) Configuration is one CLI-managed lab file
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

Or iterate faster with `make rust` (cargo-install the crates only, skipping
the OCR/setuid steps). `make help` lists every target. The underlying commands
still work directly if you prefer:

```bash
cargo install --path ~/src/paniolo/cli        # if the CLI changed
cargo install --path ~/src/paniolo/hdmicap    # if hdmicap changed
cargo install --path ~/src/paniolo/serialcap  # if serialcap changed
cargo install --path ~/src/paniolo/netbootd   # if netbootd changed (re-run `paniolo setup` to re-setuid the helper on macOS)
cargo install --path ~/src/paniolo/cambrionix # if cambrionix changed
```

The USB HID commands (`paniolo hid`) still live in the legacy Python CLI
pending the Rust port (see [docs/hid.md](docs/hid.md)); installing it via uv
recreates the PATH shadow above, so prefer a throwaway run
(`uv run --with pyserial paniolo hid ...`) if you need them.

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
| hdmicap discovery | `/tmp/paniolo-<uid>/hdmicap/daemon.json` |
| serialcap discovery | `/tmp/paniolo-<uid>/serialcap/daemon.json` |
| Serial capture logs | `/tmp/paniolo-<uid>/serialcap/capture/<name>/serial.jsonl` |

---

## License

Apache 2.0 — see [LICENSE](LICENSE).
