# Paniolo — System architecture

> The whole design of paniolo in its **current state**. Start here for the big picture, then
> drop into a [subsystem guide](README.md) for command-level detail. Internal module-by-module
> notes for contributors/agents live in [`AGENTS.md`](../AGENTS.md); the forward-looking
> hardware-CI design lives under [`ci-integration/`](ci-integration/).
>
> Keep this in sync as the system changes.

---

## 1. What paniolo is

Paniolo is an **agent-controlled target-machine wrangler** for low-level software development
(bootloaders, firmware, OS bring-up). It gives an AI agent (or a human, or a script) the
physical controls of a target board: **netboot it, watch its output, send it input, power-cycle
it** — without a person at the bench each iteration.

It is deliberately a **device-control / "wrangling" layer**, not a test orchestrator. It owns
power, serial, deploy (netboot), video, and HID. It does *not* decide what tests to run or
produce verdicts — when integrated with hardware-CI ecosystems those concerns sit *above* it
(see [`ci-integration/`](ci-integration/)).

## 2. Deployment model

```
  ┌─────────────────────┐         ┌──────────────────────── control host ────────────────────────┐
  │  dev machine / agent │  SSH    │  paniolo CLI  +  per-subsystem daemons                        │
  │  (you, or an agent)  │ ──────► │                                                               │
  └─────────────────────┘         │   USB-Ethernet ─────────────────┐                             │
                                   │   USB serial (FTDI) ────────────┤                             │
                                   │   USB HDMI capture ─────────────┤                             │
                                   │   USB HID rig (KB2040) ─────────┤                             │
                                   └─────────────────────────────────┼─────────────────────────────┘
                                                                      ▼
                                                          ┌──────────────────────┐
                                                          │   target board (DUT) │
                                                          └──────────────────────┘
```

- The **control host** is physically wired to one or more **targets** and runs paniolo.
- The simplest driver is an **agent or script that SSHes into the control host** and runs
  `paniolo …` commands (the remote-control pattern in the root [`README.md`](../README.md)).
- **Or point paniolo at a [lab file](distributed-control.md)** (`--lab` / `PANIOLO_LAB`): the dev
  machine then drives a target on its control host *transparently* — commands re-exec over SSH and
  `console` tunnels the dashboard back — so you don't SSH by hand. The dev machine is the data-plane
  hub; control hosts hold only runtime state. See §5 "Distributed control".
- Runs on **macOS 10.14+** and **Linux** (x86-64/arm64). Platform differences are isolated to a
  handful of spots (§8).

## 3. Process architecture

Paniolo today is **"one daemon per subsystem, no central server"** (called *Option A* in
`AGENTS.md`). There is no long-running parent process; the `paniolo` binary runs, does its work,
and exits. Per-subsystem **daemons** are backgrounded subprocesses that own a piece of hardware
and persist between CLI invocations. State lives in plain files, not memory.

| Component | Language | Role |
|---|---|---|
| `paniolo` CLI | Python 3.11+ (Typer) | The single entry point; spawns/queries daemons, edits config, runs scripts. `typer` is the only core dependency; stdlib otherwise. |
| `serialcap` | Rust (tokio/axum) | Daemon that **exclusively owns** a target's serial ports; fans output out to a WebSocket + a timestamped capture log; accepts keystrokes back. |
| `hdmicap` | Rust (tokio/axum, nokhwa) | "Warm-stream" daemon that keeps the USB HDMI capture device open and serves frames + the combined dashboard over HTTP. |
| `netbootd` | Rust (tokio) | The default single-binary DHCP+TFTP netboot engine. Privilege-separated `/dev/bpf` send path on macOS via a setuid `netbootd-bpf-helper`. |
| `_dhcp` / `_tftp` | Python modules | Legacy netboot DHCP and TFTP servers (`--engine python`), run as `python -m paniolo._dhcp` / `._tftp` subprocesses. The implementation `netbootd` was ported from; kept as a fallback. |
| `visionocr` / `linuxocr` | Swift / shell+Tesseract | On-device OCR helpers invoked by `hdmicap` and `paniolo video read`. |
| HID rig firmware | CircuitPython | Two KB2040 boards that turn text commands into USB HID events (see [`hidrig/`](../hidrig/README.md)). |

A future *Option B* — a single long-running Rust server with socket RPC for inter-subsystem
coordination — is noted in `AGENTS.md` but **not** implemented; the dashboard's hdmicap→serialcap
link (§7) is the only cross-subsystem coupling today.

## 4. Configuration & state model

**Per-target config** is one TOML file per target at `~/.config/paniolo/targets/<name>.toml`.
No daemon is needed to read it; if exactly one target is configured it is the default and may be
omitted from every command. The current schema (`src/paniolo/_config.py`):

```toml
name = "target-machine"          # required
interface = "en3"                # required — USB-Ethernet interface for netboot
host_ip = "192.168.99.1"         # static IP on that interface; also the TFTP server address
tftp_root = "/path/to/pxe"       # optional; required to start netboot
power_cycle_cmd = "/path/cycle.sh"      # optional; shell command run by `paniolo power-cycle`
power_serial_interface = "console"      # optional; default interface for DTR power commands

[[serial]]                       # repeatable — a target may have several named consoles
name = "console"
device = "/dev/serial/by-path/…" # stable by-path symlink preferred (Linux)
baud = 115200
power_sense_signal = "cts"       # optional; cts|dsr|dcd|ri — modem-control input wired to the rail
```

> Legacy note: older single-serial fields (`serial_device`/`serial_baud`) are auto-migrated into
> a `[[serial]]` named `console`; a removed `ha_power_entity` field is silently dropped.

**Runtime state, discovery, and capture** live outside the config tree. Each daemon writes a
**discovery file** (pid + port) and holds an **advisory lock** so only one runs per host:

| Purpose | Path |
|---|---|
| Target / video configs (legacy Python) | `~/.config/paniolo/{targets/<name>.toml, video.toml}` |
| Netboot state (pids, uptime) | `~/.local/share/paniolo/<name>/netboot.json` |
| Netboot combined log | `~/.local/share/paniolo/<name>/netboot.log` |
| hdmicap discovery / lock | `/tmp/paniolo-<uid>/hdmicap/{daemon.json, daemon.lock}` |
| serialcap discovery / lock | `/tmp/paniolo-<uid>/serialcap/{daemon.json, daemon.lock}` |
| serialcap capture log (per interface) | `/tmp/paniolo-<uid>/serialcap/capture/<name>/serial.jsonl(.1..)` |
| serialcap pending (unterminated) line | `/tmp/paniolo-<uid>/serialcap/capture/<name>/pending.json` |

## 5. Subsystems

### Netboot / deploy ([`netboot.md`](netboot.md))
A minimal **pure-Python DHCP + TFTP** pair (`_dhcp.py`, `_tftp.py`) over a **direct
USB-Ethernet link** — no router, switch, or upstream DHCP. `paniolo netboot start` assigns the
static `host_ip` to the interface, then spawns the two servers (`python -m paniolo._dhcp` /
`._tftp`); on Linux these are prefixed with `sudo` (ports 67/69 need root; macOS 10.14+ allows
them rootless). DHCP hands the target a fixed lease and points it at the TFTP root via BOOTP
`siaddr` + DHCP option 66; TFTP is read-only (RFC 1350 + blksize/tsize). No external daemons
(`dnsmasq`/`tftp-now`) are required at runtime.

`paniolo netboot start` refuses an interface that carries the system default route (a primary
NIC), since it reconfigures the interface to the static `host_ip` — the netboot link must be a
dedicated secondary (USB-Ethernet) interface.

**Netboot engine** (default): a single `netbootd` binary (Rust) runs both servers as tokio tasks.
On macOS its raw-frame send path (the Sequoia workaround) gets a `/dev/bpf` descriptor from a
setuid-root `netbootd-bpf-helper` over `SCM_RIGHTS`, so the daemon itself stays unprivileged — the
helper is the only root component, installed by `paniolo setup`. `--engine python` selects the
legacy `_dhcp`/`_tftp` subprocess pair the Rust daemon was ported from, kept as a fallback.

### Link mode: netboot ↔ ffx ([`netif.md`](netif.md))
The same USB-Ethernet link serves two **mutually-exclusive** roles: netboot (IPv4 + DHCP + TFTP,
the target TFTP-boots) and ffx (host IPv6 link-local `fe80::1`/64, the target boots from SD and is
reached over `ffx` at `fe80::…%<iface>`). `paniolo netif mode <netboot|ffx|off>` (`_netif.py`)
makes the switch atomic: `ffx` runs `netboot stop` first (so a power-cycle falls through to SD
rather than TFTP-booting a stale image) and adds the host `fe80::1` that ffx needs but nothing else
sets up. Each mode is idempotent — the ephemeral IPv6 LL is re-added on demand. The active mode is
**probed** (running daemons + interface addresses), not stored, so `paniolo netif status` stays
correct across control-host reboots; in ffx mode it also reports the device's discovered
link-local peer (`ip -6 neigh`) as a paste-ready `ffx target add`. Privileged steps reuse the same
`sudo` path as netboot — no new privilege model.

### Serial console ([`serial.md`](serial.md))
The `serialcap` daemon is the heart of the design. One daemon **exclusively owns all of a
target's serial interfaces**; per interface a *supervisor* task owns the port (with a reconnect
loop) and **fans every byte out three ways**: (1) broadcast to live WebSocket clients
(`/stream`), (2) a 64 KB scrollback ring for instant replay, and (3) a tee to a capture thread
that assembles **timestamped, sequence-numbered JSONL** lines on disk (rotating, survives
restarts). Writes flow the other way — WebSocket clients send bytes that the supervisor injects
into the port. `paniolo serial log` reads the on-disk JSONL **directly** (no daemon round-trip),
so it works whether or not the daemon is running. A separate, dependency-light **interactive**
path (`paniolo serial connect`) execs `tio` for a foreground terminal — it holds the port
exclusively and so conflicts with the daemon.

### Power control ([`power.md`](power.md))
Two mechanisms, both driven through serial/config: **DTR via FTDI** (the serial adapter's DTR
line wired to the board's J2 power-button header — `serial dtr`/`serial reset`, ≤500 ms soft /
≥3 s hard PMIC off), and **`power_cycle_cmd`**, an arbitrary shell script (`paniolo power-cycle`)
for HA switches / PDUs / GPIO. `paniolo power-state` reads an optional **power-sense** signal (a
modem-control input wired to the target rail) via the serialcap daemon's `/status`.

### Video + OCR ([`video.md`](video.md))
`hdmicap` keeps a UVC HDMI capture device open continuously (avoiding multi-second per-capture
reopen latency) and serves the current frame as PNG/MJPEG plus the dashboard over HTTP.
`paniolo video read` and the dashboard OCR button run **on-device OCR** on the warm frame —
Apple Vision (`visionocr`) on macOS, Tesseract (`linuxocr`) on Linux — tuned for thin console
fonts (2× upscale, black-pad, `.fast`/lowered min text height).

### HID injection ([`hid.md`](hid.md))
A single KB2040 presents as a USB HID keyboard + mouse to the DUT; the control host drives it
over a UART (USB-serial adapter) speaking the device-independent
[HID serial protocol](hid-serial-protocol.md). The `hidrig` helper CLI is the protocol client,
and paniolo integrates it through the generic per-target `hid` channel — an opaque command
prefix (`paniolo hid send` appends arguments), exactly like the power hooks.

### Dashboard ([`dashboard.md`](dashboard.md))
`paniolo console` opens hdmicap's `GET /` — a two-pane web UI (live video on top, xterm.js
terminal(s) below). See §7 for how the two daemons connect.

### Distributed control ([`distributed-control.md`](distributed-control.md))
Drives targets on **remote control hosts** from the dev machine, over SSH only — no agent or
coordinator daemon. A single git-tracked **lab file** (`--lab` / `PANIOLO_LAB`) names the hosts and
binds each target's resources to one (`_lab.py`); `_ssh.py` is the transport (per-host ControlMaster,
`run`/`forward`/`run_interactive`). One-shot commands **re-exec** on the target's host
(`@remote_capable` ships the config slice via `PANIOLO_TARGET_CONFIG`, `_remote.py`); `console`
**tunnels** both daemon ports back and stitches them with the dashboard's `?serialws=` override.
`setup --host` provisions a host; `discover`/`configure` propose a lab block from discovered
hardware for the human to review and commit. With no lab, everything runs locally exactly as before.
Shipped Phases 0–5; multi-host targets, `console --detach`, and locking remain design-only (see
[`distributed-control-plan.md`](distributed-control-plan.md)).

## 6. Representative data flows

- **Boot-and-watch:** agent `scp`s an image into `tftp_root` → `netboot start` → target PXE-boots
  over the USB-Ethernet link → boot output streams through `serialcap` to the JSONL log → agent
  polls `serial log --since` (or watches the dashboard / OCRs the screen).
- **Serial round-trip:** UART bytes → supervisor → {WebSocket clients, scrollback, JSONL capture
  thread}; dashboard keystrokes → WebSocket → supervisor → UART.
- **Power-cycle from the dashboard:** browser → hdmicap `POST /power-cycle` → `paniolo
  power-cycle <target>` (target from `PANIOLO_TARGET`) → `power_cycle_cmd`.

## 7. Cross-subsystem coupling (the dashboard)

The dashboard is the **only** place two subsystems interlock, and they stay decoupled: hdmicap
**serves the page** but references serialcap **only by URL** (`ws://<host>:8724/stream` by
default; override via `?serial=` / `?serialws=`). The page fetches serialcap's `/interfaces` and
builds one xterm.js terminal per interface. xterm.js is **vendored, not CDN**, so the dashboard
works on an isolated lab network. The power on/off toggle and cycle button appear only when
hdmicap was started with a target (so they are safe on shared dashboards); their availability
probe (`GET /power`) performs no power action.

## 8. Host-OS differences (macOS vs Linux)

Core power/serial/netboot works on both; the platform-specific spots are contained:

| Area | macOS | Linux |
|---|---|---|
| Netboot ports 67/69 | rootless (10.14+) | `sudo` (auto-prepended) |
| Interface config | `networksetup` / `ifconfig` | `ip addr`/`ip link` (iproute2) |
| ARP pinning | `arp -s` | `ip neigh replace … nud permanent` |
| TFTP egress workaround | BPF raw frames (`/dev/bpf*`) for Sequoia routing | normal `sendto()` |
| BPF descriptor access (rust engine) | setuid `netbootd-bpf-helper` passes the fd (daemon stays unprivileged) | n/a (kernel send path) |
| OCR backend | Apple Vision (`visionocr`, `swiftc`) | Tesseract (`linuxocr`, `tesseract-ocr` pkg) |
| Serial device discovery | `/dev/tty.usb*` | `/dev/serial/by-path/*` → `/dev/ttyUSB*`/`ACM*` |
| `paniolo setup` extras | installs `tftp-now` (Homebrew, legacy) + visionocr | none beyond the Rust build deps |

For headless CI the relevant takeaway (see `ci-integration/`): the core path is clean on Linux,
and the macOS-only bits (Vision OCR, BPF, `tftp-now`) are irrelevant there.

## 9. Lifecycle & exclusivity notes

- **Serial ports are exclusive** — only one of `serialcap` / `tio` / `screen` can hold a port.
  `serial watch` and `serial connect` conflict on the same device.
- **Daemons hard-exit on SIGTERM** — both serve infinite responses (`/preview` MJPEG, `/stream`
  WebSocket), so each removes its discovery file, waits ~300 ms, then `exit(0)`; the OS releases
  the device.
- **Interface configuration needs root** — NOPASSWD sudo is the practical setup for unattended
  agent use.
- **netboot and ffx are mutually exclusive on the link** — they want incompatible host addressing
  (IPv4 + DHCP/TFTP vs. IPv6 link-local). `paniolo netif mode` enforces the exclusivity: entering
  one mode tears down the other.

## 10. Where this is going

Paniolo is **already in day-to-day use** — driving an agent through real low-level hardware
bring-up, where the agent iterates on bootloader/firmware/OS code and uses paniolo to deploy,
boot, observe, and power-cycle the target without a human at the bench each cycle. The active
line of work builds on that: making paniolo's primitives consumable by hardware-CI orchestrators
(KernelCI/LAVA, Fuchsia/botanist) — a stable, ecosystem-agnostic device-control API (discrete
power verbs, raw serial passthrough as a TCP socket + PTY, agent write-to-serial, a JTAG
extension point) plus thin adapters. That design and its rationale are in
[`ci-integration/design.md`](ci-integration/design.md) and
[`ci-integration/gap-analysis.md`](ci-integration/gap-analysis.md); progress is tracked in
[`requirements.md`](requirements.md).

## 11. Prior art & why paniolo

The closest existing tool is **labgrid** (Pengutronix) — like paniolo, a Python device-control
layer that sits *under* a test framework and produces no verdicts of its own. labgrid is the
mature, broad, **distributed** board-farm standard (coordinator/exporter/client over gRPC, a
large driver catalog, multi-user reservations/locking) and is **Linux-only**. Paniolo
deliberately occupies a different niche: a **single control host, zero-infrastructure,
agent-in-the-loop** tool that adds capabilities labgrid lacks — on-device **OCR** of the screen,
a **USB-HID injection** rig, and a combined video+serial dashboard — and runs first-class on
**macOS** as well as Linux. Where the two overlap (raw-socket serial, discrete power verbs, a
driver/protocol abstraction), labgrid's design independently *validates* the direction of
paniolo's CI-integration work. For a multi-board, multi-user farm, labgrid is the right tool;
paniolo targets the single-target bring-up loop it under-serves. Full comparison:
[`ci-integration/related-work.md`](ci-integration/related-work.md).
