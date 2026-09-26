# Paniolo — System architecture

> The whole design of paniolo in its **current state**. Start here for the big picture, then
> go to a [subsystem guide](../README.md) for command-level detail. Module-by-module notes for
> contributors and agents are in [`AGENTS.md`](https://github.com/curtisgalloway/paniolo/blob/main/AGENTS.md); the planned
> hardware-CI design is under [`ci-integration/`](../README.md#hardware-ci-integration-in-design).
>
> Keep this in sync as the system changes.

---

## 1. What paniolo is

Paniolo is an **agent-controlled target-machine wrangler** for low-level software development
(bootloaders, firmware, embedded software, OS bring-up). It gives an AI agent, a human, or a
script the physical controls of a target board: **netboot it, watch its output, send it input,
power-cycle it**, with no person at the bench for each iteration.

It is a **device-control ("wrangling") layer**, not a test orchestrator. It owns power, serial,
deploy (netboot), video, HID (keyboard/mouse input), and adb (Android Debug Bridge). It does
*not* decide what tests to run or produce verdicts. In a hardware-CI setup those concerns sit
*above* it (see [`ci-integration/`](../README.md#hardware-ci-integration-in-design)).

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

- The **control host** is physically wired to one or more **targets** (DUTs, devices under
  test) and runs paniolo.
- The simplest driver is an **agent or script that SSHes into the control host** and runs
  `paniolo …` commands (the remote-control pattern in the root [`README.md`](https://github.com/curtisgalloway/paniolo/blob/main/README.md)).
- **Or point paniolo at a [lab file](../distributed-control.md)** (`--lab` / `PANIOLO_LAB`). The
  dev machine then drives a target on its control host *transparently*: commands re-exec over SSH
  and `console` tunnels the dashboard back, so you don't SSH by hand. The dev machine is the
  data-plane hub; control hosts hold only runtime state. See §5 "Distributed control".
- Runs on **macOS 10.14+** and **Linux** (x86-64/arm64). Platform differences are confined to a
  few spots (§8).

## 3. Process architecture

Paniolo is **"one daemon per subsystem, no central server"** (*Option A* in `AGENTS.md`). There
is no long-running parent process: the `paniolo` binary runs, does its work, and exits.
Per-subsystem **daemons** are background subprocesses that own a piece of hardware and persist
between CLI invocations. State lives in plain files, not memory.

| Component | Language | Role |
|---|---|---|
| `paniolo` CLI | Rust (clap) | The single entry point; spawns/queries daemons, edits the lab file, dispatches remote commands over SSH. |
| `serialcap` | Rust (tokio/axum) | Daemon that **exclusively owns** a target's serial ports; fans output out to a WebSocket + a timestamped capture log; accepts keystrokes back. |
| `hdmicap` | Rust (tokio/axum; ObjC AVFoundation layer on macOS, v4l on Linux) | "Warm-stream" daemon that keeps the USB HDMI capture device open and serves frames + the combined dashboard over HTTP. |
| `netbootd` | Rust (tokio) | The single-binary DHCP+TFTP+HTTP netboot engine (the only one). Privilege-separated `/dev/bpf` send path on macOS via a setuid `netbootd-bpf-helper`. |
| `cambrionix` | Rust | Standalone power helper: Cambrionix USB-hub port control, wired in via the generic power hooks. |
| `zigplug` | Python (uv tool, zigpy-znp) | Standalone power helper: Zigbee smart-plug control through a CC2652 coordinator; one-shots proxy through an auto-spawned daemon that owns the ZNP session. |
| `shellyplug` | Rust (ureq) | Standalone power helper: Shelly Gen2+ smart plugs/relays over the device's local HTTP RPC — one-shot, stateless. |
| `amt` | Rust (ureq) | Standalone power helper: Intel AMT/vPro (the out-of-band management engine in Intel PCs) power over WS-Management (port 16992, HTTP Digest), with true power-state readback from the ME (Management Engine). |
| `hidrig` | Rust | HID-injection helper: protocol client + `serve` daemon for the KB2040 injector, wired in via the generic `hid` channel. |
| `ch9329` | Rust | The other HID-injection helper: same CLI surface + `serve` daemon, speaking the CH9329 (a serial-to-USB-HID chip) binary frame protocol for Openterface Mini-KVM / KVM-Go and Sipeed NanoKVM-USB devices. |
| `visionocr` / `linuxocr` | Swift / shell+Tesseract | On-device OCR (text recognition) helpers invoked by `hdmicap` (`GET /ocr`, wrapped by `paniolo video read` and the dashboard OCR button). |
| HID rig firmware (separate repo) | CircuitPython | Two KB2040 (Adafruit RP2040 microcontroller) boards — the dual-board "dumb pipe" that relays host-composed HID reports to the DUT as USB keyboard + mouse events. Custom hardware, not part of this repo: see [`paniolo-hardware`](https://github.com/curtisgalloway/paniolo-hardware)'s [`hidrig-kb2040/`](https://github.com/curtisgalloway/paniolo-hardware/tree/main/hidrig-kb2040), driven by the `hidrig` row above. |

Only `paniolo` itself lands on PATH (`~/.cargo/bin`). Every helper and daemon installs into the
private libexec dir `~/.local/libexec/paniolo/bin`, where paniolo finds them itself:
`paniolo helper <name>` runs one directly, and `paniolo daemons` lists/stops/restarts the running
ones.

A future *Option B*, a single long-running Rust server with socket RPC between subsystems, is
noted in `AGENTS.md` but **not** implemented. The dashboard's hdmicap→serialcap link (§7) is the
only cross-subsystem coupling today.

## 4. Configuration and state model

**All configuration lives in one CLI-managed lab file**: `~/.config/paniolo/lab.toml`, or
`--lab` / `PANIOLO_LAB` to point elsewhere (e.g. a git-tracked file). It names the **hosts** and
the **targets**. Each target's hardware is described as *channels* (`netboot`, `serial`, `power`,
`video`, `hid`, `adb`), each bound to the host it is physically attached to. Reading it needs no
daemon. If exactly one target is configured, it is the default and may be omitted from every
command. The schema (`cli/src/model.rs`):

```toml
[hosts.mac1]                     # optional — "local" is implicit; entries name remote hosts
ssh = "user@control-mac"         # how others reach it; identity / paniolo_cmd / control_path /
                                 #   description optional
# hostname = "mac1.local"        # this box's FQDN — set it so the host recognizes itself when one
#                                  shared lab file is run from any machine (matched vs `hostname -f`)

[targets.target-machine]
[targets.target-machine.netboot]
interface = "en3"                # USB-Ethernet interface for netboot
host_ip = "192.168.99.1"         # static IP on that interface; also the TFTP/HTTP server address
tftp_root = "/path/to/pxe"       # required to start netboot
# boot_file = "grubaa64.efi"     # UEFI NBP for PXE / HTTP Boot clients (http_port,
#                                #   content_type optional; see netboot.md)

[[targets.target-machine.serial]] # repeatable — a target may have several named consoles
name = "console"
device = "/dev/serial/by-id/…"   # stable symlink preferred (Linux): by-id names the
                                 #   adapter; by-path when it has no serial number
baud = 115200
power_sense_signal = "cts"       # optional; cts|dsr|dcd|ri — modem-control input wired to the rail
# power_button = true            # opt-in: DTR is wired to the board's power button
#                                #   (required by `serial dtr` / `serial reset`)

[targets.target-machine.power]   # generic hooks — device-specific logic lives in helpers
cycle_cmd = "zigplug … cycle …"  # `paniolo power-cycle`
on_cmd = "zigplug … on …"        # `paniolo power on`   (off_cmd / state_cmd likewise)
serial_interface = "console"     # default interface for DTR power commands

[targets.target-machine.video]
device = "USB Video"             # HDMI capture device for hdmicap

[targets.target-machine.hid]
cmd = "hidrig -d /dev/…"         # opaque helper prefix; `paniolo hid send` appends args

[targets.target-machine.adb]     # an Android DUT reached over adb
serial = "33271JEGR02033"        # `adb -s <serial>`; omit for the sole device
```

Every channel also takes an optional `host = "<name>"` to bind it to a remote control host
(§5 "Distributed control"). The file is edited through the CLI (`paniolo target add`,
`netboot set`, `serial add`, `power set`, `video set`, `hid set`, …), never by daemons. The
per-target files from the pre-lab-file layout (`~/.config/paniolo/targets/<name>.toml`,
`video.toml`) are not read.

**Runtime state, discovery, and capture** live outside the config tree:

- **Discovery and auth.** Each daemon writes a **discovery file** (pid + port + a per-start
  bearer **token**, owner-only) and holds an **advisory lock**. Every request to a daemon must
  carry that token (`Authorization: Bearer` from the CLI, `?token=` from the dashboard), and the
  daemons accept only loopback `Host`/`Origin` values. Binding 127.0.0.1 keeps other machines
  out; the token and origin checks keep other *web pages* out (`cli/src/daemons.rs` `Endpoint`,
  each daemon's `auth.rs`).
- **Staleness.** At spawn the CLI records the daemon's binary identity (`binmeta.json`). A daemon
  still running an older binary after an upgrade/rebuild is flagged **stale** in
  `paniolo daemons` and healed by `paniolo daemons restart`.
- **Instances.** The per-target capture daemons (serialcap/hdmicap/hid) run one instance **per
  target**, so several targets can capture at once on one host. Host-singleton daemons
  (zigplug/cambrionix/netbootd) run one per host.
- **Helper env vars.** Helpers receive two pre-created directories on every invocation:
  `PANIOLO_STATE_DIR` (durable state, `~/.config/paniolo/helpers/<name>`) and
  `PANIOLO_RUNTIME_DIR` (`/tmp/paniolo-<uid>/<name>`; the capture daemons append a `/<target>`
  segment). The runtime base honors `$PANIOLO_RUNTIME_BASE` (default `/tmp`)
  (`cli/src/daemons.rs`; contract in [`adding-power-helpers.md`](adding-power-helpers.md)).
- **Private runtime directory.** The runtime directory is created 0700. An existing one must be
  a real directory owned by the current user with no group/other bits: a too-open one we own is
  tightened; a symlink or another owner's is refused. The discovery-file readers apply the same
  check before trusting a `daemon.json` beneath it, and the SSH ControlMaster (OpenSSH's shared
  connection) socket dir (`cli/src/ssh.rs`) is created through it too. Daemon stderr logs and
  serialcap's capture files are created 0600.

The paths:

| Purpose | Path |
|---|---|
| Lab file (all config) | `~/.config/paniolo/lab.toml` (or `--lab` / `PANIOLO_LAB`) |
| Helper durable state (`PANIOLO_STATE_DIR`) | `~/.config/paniolo/helpers/<name>/` (e.g. zigplug's `zigbee.db`) |
| Netboot state (pids, uptime) | `~/.local/share/paniolo/<name>/netboot.json` |
| Netboot combined log | `~/.local/share/paniolo/<name>/netboot.log` |
| hdmicap discovery / lock (per target) | `/tmp/paniolo-<uid>/hdmicap/<target>/{daemon.json, daemon.lock}` |
| serialcap discovery / lock (per target) | `/tmp/paniolo-<uid>/serialcap/<target>/{daemon.json, daemon.lock}` |
| hid daemon discovery (channel name, any injector, per target) | `/tmp/paniolo-<uid>/hid/<target>/daemon.json` |
| zigplug daemon discovery (host singleton) | `/tmp/paniolo-<uid>/zigplug/daemon.json` |
| serialcap capture log (per interface) | `/tmp/paniolo-<uid>/serialcap/<target>/capture/<name>/serial.jsonl(.1..)` |
| serialcap pending (unterminated) line | `/tmp/paniolo-<uid>/serialcap/<target>/capture/<name>/pending.json` |

Nothing rewrites these runtime files after the daemon that publishes them starts. A `/tmp` age
policy (Debian's stock `q /tmp 1777 root root 10d`) therefore deletes them out from under a
long-running daemon, which keeps its device while paniolo reports the channel stopped. The
`.deb` ships `/usr/lib/tmpfiles.d/paniolo.conf` (`x /tmp/paniolo-*`) to prevent that, and
`paniolo daemons` / `video show` / `video watch` recognize and reap an orphan that happens anyway
(GitHub #187).

## 5. Subsystems

### Netboot / deploy ([`netboot.md`](../netboot.md))
A minimal **DHCP + TFTP + HTTP** server over a **direct USB-Ethernet link**, with no router,
switch, or upstream DHCP. It is the single-binary `netbootd` (Rust), running all three as tokio
tasks. (DHCP hands out the address; TFTP, the Trivial File Transfer Protocol, serves boot files.)
No external daemons (`dnsmasq` etc.) are required at runtime.

- **Start.** `paniolo netboot start` assigns the static `host_ip` to the interface, then spawns
  `netbootd`. On Linux it is prefixed with `sudo`, because ports 67/69 and 80 need root; macOS
  10.14+ allows them rootless.
- **DHCP** hands the target a fixed lease and points it at the TFTP root via BOOTP `siaddr` +
  DHCP option 66. **TFTP** is read-only (RFC 1350 + blksize/tsize).
- **UEFI clients.** netbootd reads the DHCP vendor class and dispatches. A `PXEClient` (PXE: the
  standard firmware netboot) gets the configured `boot_file` over TFTP. An `HTTPClient` gets an
  `http://…/<boot_file>` URL and the file over HTTP (HTTP Boot — see
  [`netboot.md`](../netboot.md)).

**Interface safety checks.** `paniolo netboot start` refuses:

- an interface that carries the system default route (a primary NIC). It reconfigures the
  interface to the static `host_ip`, so the netboot link must be a dedicated secondary
  (USB-Ethernet) interface.
- a second target on an interface that another target's live netbootd already serves (one
  netboot per interface).

The address assignment underneath (`netif::configure_interface`) refuses a /24 that any *other*
interface on the host already holds (one subnet per link — see docs/netboot.md). The same rule is
applied by `LabFile::set_netboot` to the lab file at edit time (judging only the link being
edited, so a lab with two clashing pairs stays repairable), and `doctor` reports it as
`CONFLICT`. After spawning, `netboot start` watches the daemon for ~2 s. If netbootd exits during
startup, it fails with the tail of the log rather than recording a dead daemon.

**Listening and privilege.** `netbootd` only ever hears the netboot link. Every listener (DHCP,
TFTP, HTTP) is pinned to the interface (`IP_BOUND_IF` / `SO_BINDTODEVICE`) before it is bound,
and a pin that fails is fatal (`--interface` is required). Startup is validate → bind and pin →
drop root → serve. On Linux the daemon drops from `sudo`'s root to the invoking user
(`SUDO_UID`/`SUDO_GID`) as soon as the listeners are bound, so nothing that parses packets or
serves files runs privileged. (Its later `sudo ip …` shell-outs rely on passwordless sudo.) The
client lease is derived from `host_ip` (same /24, last octet `100`). An HTTP bind failure only
disables HTTP Boot; DHCP and TFTP keep serving.

**One client.** `netbootd` enforces one *client*, not just one lease:

- DHCP locks onto the MAC of the first DISCOVER/REQUEST it sees, for the life of the process,
  and silently ignores any other MAC. A second device on the link never gets an OFFER/ACK and
  can't steal the lease, the ARP pin, or the MAC handed to TFTP's raw-frame sender.
- TFTP accepts RRQs (read requests) only from that leased IP, and caps concurrent transfers at a
  small fixed `MAX_TRANSFERS` via a semaphore. A flood of RRQs from unique source ports therefore
  cannot grow tasks/sockets/files without bound.

Neither gate has an in-process reset: a new client means restarting `netbootd` (i.e. `paniolo
netboot stop` then `start`).

**macOS BPF helper.** On macOS, `netbootd`'s raw-frame send path (the Sequoia workaround) gets a
`/dev/bpf` (Berkeley Packet Filter, raw network access) descriptor from a setuid-root
`netbootd-bpf-helper` over `SCM_RIGHTS`. The daemon itself stays unprivileged; the helper is the
only root component, installed by `paniolo setup`. The descriptor is write-only with a reject-all
filter (it can inject, never capture), and the helper serves only the user who installed it and
never the default-route interface.

### Link mode: netboot · link · ffx · off ([`netif.md`](../netif.md))
The same USB-Ethernet link serves **mutually-exclusive** roles:

| Mode | Host side | Use |
|---|---|---|
| netboot | IPv4 + DHCP + TFTP + HTTP | the target TFTP-boots |
| `link` | host IP up, no daemon | link up/down testing |
| ffx | host IPv6 link-local `fe80::1`/64 | the target boots from SD and is reached over `ffx` (Fuchsia's host tool) at `fe80::…%<iface>` |
| `off` | host IP released | |

`paniolo netif mode <netboot|link|ffx|off>` (`cli/src/netif.rs`) makes the switch atomic. `ffx`
runs `netboot stop` first, so a power-cycle falls through to SD rather than TFTP-booting a stale
image, and adds the host `fe80::1` that ffx needs but nothing else sets up. Each mode is
idempotent: the ephemeral IPv6 LL is re-added on demand.

The active mode is **probed** (running daemons + interface addresses), not stored, so
`paniolo netif status` stays correct across control-host reboots. In ffx mode it also reports the
device's discovered link-local peer (`ip -6 neigh`) as a paste-ready `ffx target add`.

`paniolo netif down-hard` goes beyond `mode off` for cases where the target must *detect* link
loss. It disables Wake-on-LAN (which otherwise keeps the PHY energized) and admin-downs the
interface so the peer sees carrier drop. Privileged steps reuse netboot's `sudo` path; there is
no new privilege model.

### Serial console ([`serial.md`](../serial.md))
The `serialcap` daemon is the heart of the design. One daemon **per target exclusively owns all
of that target's serial interfaces** (two targets on one host run two serialcap daemons). For
each interface, a *supervisor* task owns the port (with a reconnect loop) and **fans every byte
out three ways**:

1. broadcast to live WebSocket clients (`/stream`);
2. a 64 KB scrollback ring for instant replay;
3. a tee to a capture thread that writes **timestamped, sequence-numbered JSONL** lines to disk
   (rotating, survives restarts).

Writes flow the other way: WebSocket clients send bytes that the supervisor injects into the
port. `paniolo serial log` reads the on-disk JSONL **directly** (no daemon round-trip), so it
works whether or not the daemon is running. A separate, dependency-light **interactive** path
(`paniolo serial connect`) execs `tio` for a foreground terminal. It holds the port exclusively,
so it conflicts with the daemon.

### Power control ([`power.md`](../power.md))
Two mechanisms, both driven through serial/config:

- **DTR via FTDI.** The serial adapter's DTR line (a modem-control output) is wired to the
  board's J2 power-button header: `serial dtr`/`serial reset`, ≤500 ms soft / ≥3 s hard PMIC off.
- **Generic power hooks.** Arbitrary shell commands on the target's `power` channel
  (`cycle_cmd`/`on_cmd`/`off_cmd`/`state_cmd`, run by `paniolo power-cycle`, `power on/off`,
  `power-state`).

Device-specific logic lives in standalone helper binaries wired through those hooks, never in
the core. `cambrionix` (Cambrionix hub ports), `zigplug` (Zigbee smart plugs), `shellyplug`
(Shelly Gen2+ plugs/relays), and `amt` (Intel AMT/vPro) ship with paniolo, and the dual-board
`hidrig` control board can switch a DUT relay (`hidrig power`) behind the same hooks. When no
`state_cmd` is set, `paniolo power-state` falls back to an optional **power-sense** signal (a
modem-control input wired to the target rail) via the serialcap daemon's `/status`.

### Video + OCR ([`video.md`](../video.md))
`hdmicap` keeps a UVC (standard USB video class) HDMI capture device open continuously, avoiding
multi-second reopen latency per capture, and serves the current frame as PNG/MJPEG plus the
dashboard over HTTP. `paniolo video read` (wrapping hdmicap's `GET /ocr`) and the dashboard OCR
button run **on-device OCR** on the warm frame: Apple Vision (`visionocr`) on macOS, Tesseract
(`linuxocr`) on Linux. Both are tuned for thin console fonts (2× upscale, black-pad,
`.fast`/lowered min text height).

### HID injection ([`hid.md`](../hid.md))
The dual-board "dumb pipe" rig presents to the DUT as a USB HID keyboard + mouse. `hidrig`
composes HID reports on the host and writes binary frames to the **control** board's USB-CDC
(USB serial) port, which relays them over I2C1 to the **target** board that injects them
([hid-dual-board-design.md](hid-dual-board-design.md)). The command vocabulary is the
device-independent [HID serial protocol](hid-serial-protocol.md), but that is the *external*
interface only: `hidrig` consumes it and composes the reports. paniolo integrates the helper
through the generic per-target `hid` channel, an opaque command prefix (`paniolo hid send`
appends arguments), exactly like the power hooks.

### adb / Android targets ([`adb.md`](../adb.md))
When the target is an Android device, the per-target `adb` channel binds it (by
`adb -s <serial>`) to the control host it's plugged into. That gives paniolo the same verbs over
one transport:

- console: `paniolo adb shell` (interactive), `adb run` (one-shot);
- screen: `adb screencap`, via `adb exec-out screencap -p` → PNG;
- input: `adb input` → `adb shell input`.

adb is a generic transport like SSH, not a device-specific helper, so it lives in the core CLI
(`cli/src/adb.rs`). It shells out to the host's `adb` binary and is routed per-channel by the
same dispatch as every other channel. Reboot/power needs no new code: wire `adb reboot` through
the generic power hooks.

### Dashboard ([`dashboard.md`](../dashboard.md))
`paniolo console` opens hdmicap's `GET /`: a two-pane web UI with live video on top and xterm.js
(a browser terminal emulator) terminal(s) below. See §7 for how the two daemons connect.

### Distributed control ([`distributed-control.md`](../distributed-control.md))
Drives targets on **remote control hosts** from the dev machine, over SSH only, with no agent or
coordinator daemon.

- The lab file names the hosts and binds each target's channels to one (`cli/src/model.rs`).
  `cli/src/ssh.rs` is the transport (per-host ControlMaster,
  `run`/`forward`/`run_interactive`).
- Commands that touch a remote channel **re-exec** on its host: `cli/src/dispatch.rs` ships the
  relevant lab slice as a temp file and re-invokes with `--lab`.
- `console` **tunnels** the daemon ports back and stitches them together with the dashboard's
  `?serialws=` override.
- `setup --host` provisions a host; `discover`/`configure` propose a lab block from discovered
  hardware for the human to review and commit.
- Channels of one target may live on different hosts; each command routes per-channel. Only the
  composite `console` still requires its channels co-located.

`console --detach` and locking remain design-only (see
[`distributed-control-plan.md`](https://github.com/curtisgalloway/paniolo/blob/main/notes/distributed-control-plan.md)).

## 6. Representative data flows

- **Boot-and-watch:** agent `scp`s an image into `tftp_root` → `netboot start` → target PXE-boots
  over the USB-Ethernet link → boot output streams through `serialcap` to the JSONL log → agent
  polls `serial log --since` (or watches the dashboard / OCRs the screen).
- **Serial round-trip:** UART bytes → supervisor → {WebSocket clients, scrollback, JSONL capture
  thread}; dashboard keystrokes → WebSocket → supervisor → UART.
- **Power-cycle from the dashboard:** browser → hdmicap `POST /power-cycle` → `paniolo
  power-cycle <target>` (target from `PANIOLO_TARGET`) → the target's `cycle_cmd` hook.

## 7. Cross-subsystem coupling (the dashboard)

The dashboard is the **only** place two subsystems interlock, and they stay decoupled. hdmicap
**serves the page** but references serialcap **only by URL**:

- `paniolo console` passes a complete loopback WebSocket URL, with serialcap's own token inside,
  as `?serialws=` (for a remote target, the tunnel's local port). hdmicap's token rides as
  `?token=`.
- The page refuses a non-loopback URL. Its built-in `ws://<host>:8724/stream` fallback applies
  only when the page is opened by hand.
- The page fetches serialcap's `/interfaces` and builds one xterm.js terminal per interface.

No daemon learns another's token: each token travels only in the URL the page uses to reach
that daemon. xterm.js is **vendored, not loaded from a CDN**, so the dashboard works on an
isolated lab network. The power on/off toggle and cycle button appear only when hdmicap was
started with a target, which keeps them safe on shared dashboards. Their availability probe
(`GET /power`) performs no power action.

## 8. Host-OS differences (macOS vs Linux)

Core power/serial/netboot works on both. The platform-specific spots:

| Area | macOS | Linux |
|---|---|---|
| Netboot ports 67/69 | rootless (10.14+) | `sudo` (auto-prepended); netbootd drops root to `SUDO_UID` once bound |
| Listener pinning to the netboot interface | `IP_BOUND_IF` | `SO_BINDTODEVICE` (set while still root) |
| Interface config | `networksetup` / `ifconfig` | `ip addr`/`ip link` (iproute2) |
| ARP pinning | `arp -s` | `ip neigh replace … nud permanent` |
| TFTP egress workaround | BPF raw frames (`/dev/bpf*`) for Sequoia routing | normal `sendto()` |
| BPF descriptor access (rust engine) | setuid `netbootd-bpf-helper` passes the fd (daemon stays unprivileged) | n/a (kernel send path) |
| OCR backend | Apple Vision (`visionocr`, `swiftc`) | Tesseract (`linuxocr`, `tesseract-ocr` pkg) |
| Serial device discovery | `/dev/tty.usb*` | `/dev/serial/{by-id,by-path}/*` grouped per port, by-id preferred → `/dev/ttyUSB*`/`ACM*` |
| `paniolo setup` extras | compiles `visionocr` (`swiftc`) + installs setuid `netbootd-bpf-helper` (one sudo) | `linuxocr` (needs `tesseract-ocr` pkg) |

For headless CI (see `ci-integration/`): the core path is clean on Linux, and the macOS-only
bits (Vision OCR, BPF) don't apply there.

## 9. Lifecycle & exclusivity notes

Serialcap, hdmicap, ch9329, and hidrig all accept an authenticated `POST /stop`; zigplug's
daemon accepts the same over its own HTTP API. Each helper's stop command uses this endpoint
rather than signaling the PID from the discovery file, which avoids hitting a reused PID. The
request and OS signals go through the same discovery cleanup and exit path.

- **Serial ports are exclusive.** Only one of `serialcap` / `tio` / `screen` can hold a port, so
  `serial watch` and `serial connect` conflict on the same device.
- **Daemons hard-exit on SIGTERM.** Both serve infinite responses (`/preview` MJPEG, `/stream`
  WebSocket), so each removes its discovery file, waits ~300 ms, then calls `exit(0)`; the OS
  releases the device.
- **Daemon APIs are authenticated** (see §4): a fresh token per daemon start, published only in
  the owner-only discovery file; loopback-only `Host`/`Origin`; no
  `Access-Control-Allow-Origin: *`. A daemon started by an older paniolo has no token and is
  replaced by `paniolo daemons restart --stale`.
- **Interface configuration needs root.** NOPASSWD sudo is the practical setup for unattended
  agent use.
- **Link modes (netboot / link / ffx / off) are mutually exclusive on the link.** netboot and
  ffx in particular want incompatible host addressing (IPv4 + DHCP/TFTP vs. IPv6 link-local).
  `paniolo netif mode` enforces this: entering one mode tears down the other.

## 10. Where this is going

Paniolo is **already in day-to-day use** for real low-level hardware bring-up: an agent iterates
on bootloader/firmware/OS code and uses paniolo to deploy, boot, observe, and power-cycle the
target without a human at the bench each cycle.

The active work makes paniolo's primitives consumable by hardware-CI orchestrators
(KernelCI/LAVA, the Linux kernel's board-test lab system; Fuchsia/botanist, Fuchsia's
on-device test launcher): a stable, ecosystem-agnostic device-control API plus thin
adapters. The discrete power verbs (`power on/off`) and agent write-to-serial (`serial send`)
have landed. Raw serial passthrough as a TCP socket + PTY and a JTAG extension point remain. The
design and its rationale are in [`ci-integration/design.md`](ci-integration/design.md) and
[`ci-integration/gap-analysis.md`](ci-integration/gap-analysis.md); progress is tracked in
[`requirements.md`](requirements.md).

## 11. Prior art & why paniolo

The closest existing tool is **labgrid** (Pengutronix). Like paniolo, it is a device-control
layer (labgrid in Python, paniolo in Rust) that sits *under* a test framework and produces no
verdicts of its own.

- **labgrid** is the mature, broad, **distributed** board-farm standard
  (coordinator/exporter/client over gRPC, a large driver catalog, multi-user
  reservations/locking) and is **Linux-only**. For a multi-board, multi-user farm, it is the
  right tool.
- **Paniolo** targets the niche labgrid under-serves: the single-target bring-up loop. It is a
  **single control host, zero-infrastructure, agent-in-the-loop** tool that adds capabilities
  labgrid lacks (on-device **OCR** of the screen, a **USB-HID injection** rig, a combined
  video+serial dashboard) and runs first-class on **macOS** as well as Linux.

Where the two overlap (raw-socket serial, discrete power verbs, a driver/protocol abstraction),
labgrid's design independently *validates* the direction of paniolo's CI-integration work. Full
comparison: [`ci-integration/related-work.md`](ci-integration/related-work.md).
