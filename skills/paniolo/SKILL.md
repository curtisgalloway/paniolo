---
name: paniolo
description: Control a physical target machine (SBC, e.g. a Raspberry Pi) with the paniolo CLI during low-level bring-up — netboot it over a direct USB-Ethernet link, watch and OCR its HDMI screen, drive its serial console, power-cycle it, switch the link between netboot and ffx, and drive targets on remote control hosts transparently via a single git-tracked lab file over SSH. Use when you need to boot, observe, type into, screenshot, read the screen of, power, or remotely control a target/board through paniolo — including across multiple control hosts.
---

# Paniolo — controlling a target machine

Paniolo drives a physical target machine (a single-board computer such as a
Raspberry Pi) during firmware/OS bring-up. It runs on the control host that is
physically wired to the target (USB-Ethernet for netboot, an HDMI capture
dongle for video, a USB-serial adapter for the console, a smart plug for power).

Almost every command operates on a named **target**. If exactly one target is
configured, the name can be omitted.

## Drive through paniolo — don't touch its devices directly

Paniolo owns the hardware it's configured for, and a background **daemon** holds
most of it open: netbootd (the netboot interface + DHCP/TFTP), serialcap (each
serial port), hdmicap (the capture device), the hid injector. Those daemons
track state, and the netif/netboot commands assume they're the only ones
touching the link and ports. So **always go through paniolo commands; never
reconfigure or open a paniolo-managed device by hand**:

- **Don't** run `ifconfig` / `ip` / `networksetup` / `ethtool` on the netboot
  interface — use `paniolo netif mode …` (and `paniolo netboot start/stop`). A
  stray address or an admin-down behind paniolo's back desyncs what `netif
  status` reports and can silently break DHCP/TFTP.
- **Don't** open a serial port with `screen` / `tio` / `minicom` while a
  `serial watch` daemon holds it (the port is exclusive) — read with `paniolo
  serial log` and write with `paniolo serial send`.
- **Don't** `kill` a daemon or its helper by PID — use `paniolo daemons stop`
  (or the per-subsystem `stop`).

If a paniolo command for what you want doesn't seem to exist, run `paniolo
--help` or re-read this skill before reaching for the raw device — it almost
certainly does (e.g. `netif mode link`/`off` to toggle the bare link, `daemons`
to find and clear a stuck port).

## First-time setup

```
cd ~/src/paniolo
make install               # cargo-installs the `paniolo` CLI, then `paniolo setup`
                           # builds/installs hdmicap, serialcap, netbootd, cambrionix,
                           # visionocr; on macOS also installs netbootd-bpf-helper
                           # setuid-root (one sudo)
```

Run once per machine; re-run after pulling or editing (it's a full rebuild).
Only the `paniolo` CLI lands in `~/.cargo/bin` — make sure that's on `PATH`.
The helpers (hdmicap, serialcap, netbootd, cambrionix, hidrig, zigplug, the
OCR tool) live in the private libexec dir `~/.local/libexec/paniolo/bin`,
off PATH; paniolo resolves them itself, and `paniolo helper [NAME] [ARGS…]`
lists or runs one directly. If a different `paniolo` shadows the CLI on PATH
(e.g. a Homebrew keg from the tap winning over `~/.cargo/bin`), put
`~/.cargo/bin` first or remove the other copy; `make install` warns when it
detects a shadow.

## Configure a target

Config lives in one CLI-managed **lab file** (`~/.config/paniolo/lab.toml`, or
`--lab`/`PANIOLO_LAB`). A target's hardware is described as *channels*:

```
paniolo target add <name> [--host <labhost>] [--description <text>]
paniolo netboot set -t <name> --interface <iface> [--tftp-root <dir>] [--host-ip <ip>] [--boot-file grubaa64.efi] [--http-port 80]
paniolo serial add console -t <name> --device <path> [--baud 115200] [--sense cts]
paniolo power set -t <name> [--cycle-cmd C] [--on-cmd C] [--off-cmd C] [--state-cmd C] [--serial-interface console]
paniolo video set -t <name> --device "<capture id or name>"
paniolo hid set -t <name> --cmd "hidrig -d <uart>"   # USB HID injection helper
paniolo adb set -t <name> [--serial <adb-id>]        # an Android DUT over adb
```

- `paniolo netboot devices` lists candidate USB-Ethernet interfaces (the
  primary NIC is excluded); `paniolo discover` lists all lab-relevant hardware;
  `paniolo configure <name> -H <host>` proposes a whole block to paste in.
- Inspect: `paniolo config show` (whole lab) / `paniolo target show <name>`.
- Remove: `paniolo target rm <name>`, or per channel (`netboot rm`,
  `serial rm <iface> -t <name>`, `power rm`, `video rm`, `hid rm`, `adb rm`).
- `paniolo doctor` probes every configured channel against reality (devices
  exist, over SSH for remote hosts).

## Netboot (DHCP + TFTP + HTTP)

Boot a board over the direct USB-Ethernet link:

```
paniolo netboot start [target]            # serve DHCP + TFTP + HTTP (netbootd)
paniolo netboot tftp-root [target]        # print where to drop boot files
paniolo netboot status [target]
paniolo netboot logs -f [target]          # follow the combined log
paniolo netboot stop [target]
```

`start` refuses an interface that carries the system default route (a primary
NIC) — the netboot link must be a dedicated USB-Ethernet adapter. Netboot is
served by the single-binary `netbootd` (Rust). On macOS its BPF send path uses
the setuid `netbootd-bpf-helper` installed by `paniolo setup`.

Put boot files in the target's TFTP root (for a Raspberry Pi 5, the kernel goes
in as `kernel_2712.img`). Needs passwordless `sudo` for `ifconfig` (it assigns
the interface's static IP).

**UEFI clients (PXE / HTTP Boot over IPv4).** netbootd reads the client's DHCP
vendor class and serves the matching path from one config: a `PXEClient` gets
the bootfile over TFTP (with a `PXEClient` echo); an `HTTPClient` gets an
`http://…/<boot_file>` URL and the file over HTTP. Set `--boot-file` to the UEFI
NBP (e.g. `grubaa64.efi`, `ipxe.efi`). **HTTP Boot is preferred** for UEFI — it
runs over kernel TCP (robust under load) and skips the macOS BPF/ARP machinery
the silent Pi bootloader needs. IPv4 + plain HTTP only (no IPv6/HTTPS yet).

## Link mode — netboot · link · ffx · off

`paniolo netif` owns the host side of the USB-Ethernet link and puts it in one of
four mutually-exclusive modes. Use it to flip between them atomically instead of
configuring the interface by hand:

```
paniolo netif mode netboot [target]        # IPv4 + DHCP + TFTP (= netboot start)
paniolo netif mode link [target]           # bare link UP: host IP only, no daemon
paniolo netif mode ffx [target]            # stop netboot, add host fe80::1/64 for ffx
paniolo netif mode off [target]            # soft DOWN: release host IP (+ ffx LL)
paniolo netif down-hard [target]           # hard DOWN: also kill WoL + admin-down the iface
paniolo netif status [target]              # mode, carrier, addresses, ffx peer
```

**Testing the link up/down.** To check that the link itself comes up and drops —
without serving anything — toggle `link` ↔ `off` and read `netif status`:

```
paniolo netif mode link [target]    # up:   host IP assigned, no DHCP/TFTP
paniolo netif status [target]       #       mode=link, carrier up
paniolo netif mode off [target]     # down: host IP released
paniolo netif status [target]       #       mode=off
```

Caveat — **soft "down" (`mode off`) only releases the host IP; it does not force
the carrier down.** `netif status` shows `carrier` separately from `mode` because
a NIC with **Wake-on-LAN** enabled keeps the PHY energized even when the
interface is down, so the peer's link LED stays lit and `carrier` can still read
`up` in `off` mode. **When you need the target to actually *detect* link loss,
use `paniolo netif down-hard`** — it does `mode off` and then disables WoL
(`ethtool -s <iface> wol d` on Linux) and admin-downs the interface, so the peer
sees carrier go away. Bring the link back with `netif mode link` (or `netboot`);
WoL stays off until you re-enable it (`ethtool -s <iface> wol g`) or replug the
adapter. (macOS WoL is a system-wide pref, so there `down-hard` relies on the
admin-down; unplugging the cable is always the unambiguous drop.)

- **Switching to ffx stops netboot first** — otherwise a power-cycle TFTP-boots a
  stale image instead of falling through to the SD card. This is the safe way to
  hand the link off to `ffx`.
- `mode ffx` adds the host-side `fe80::1/64` that `ffx` needs (nothing else sets
  it up, and it's lost on a control-host reboot). Re-run `mode ffx` any time to
  re-add it — every mode is idempotent.
- `netif status` in ffx mode reads the IPv6 neighbor table and prints a
  ready-to-paste `ffx target add fe80::…%<iface>` for the discovered device. If
  no peer shows, power-cycle the target and wait for it to finish SLAAC.

Typical ffx hand-off: `paniolo netif mode ffx fortune` → `paniolo power-cycle
fortune` → `paniolo netif status fortune` (grab the address) → `ffx target list`.
Go back to TFTP bring-up with `paniolo netif mode netboot fortune`. Same
passwordless `sudo` requirement as netboot (`ip` on Linux, `ifconfig` on macOS).

## Video — capture, preview, OCR

```
paniolo video devices                 # list capture devices (with stable ids)
paniolo video set -t <target> --device "<id-or-name>"   # configure the video channel
paniolo video watch [target] [--restart]   # start the capture daemon (background);
                                           #   --restart force-restarts a stalled one
paniolo video preview                 # print the daemon's dashboard URL (no browser)
paniolo video shot [target] [--stable] [--out frame.png]   # one lossless PNG
paniolo video shot --changed-since <hex-hash> --timeout <ms>   # block until the
                                           #   frame differs from a previous shot's hash
paniolo video read [target] [--stable]   # OCR the current screen, print text
paniolo video show [target]           # device + daemon status
paniolo video stop [target]           # stop the daemon (on the target's host)
```

- The **dashboard** (the video daemon's URL — ports are OS-assigned, printed by
  `video watch`/`console`) shows live video on top, a
  serial terminal below, an **OCR button** that reads the current screen, and —
  when the target has a `hid` channel — a **⌨ Capture input** button that turns
  the page into a KVM (see HID injection below).
- The `device` may be a **stable id** (preferred — `video devices` prints
  `id=…`: the AVFoundation uniqueID on macOS, the `/dev/v4l/by-path` symlink on
  Linux), a name substring, or a `/dev/video*` path. Ids are derived from USB
  port topology, so they survive reboots and tell two identical dongles apart;
  replugging into a different port changes the id. A name substring matching
  more than one device is an error (listing the candidates' ids), not a silent
  first-match guess.
- `--stable` waits for a steady frame before capturing (useful right after a mode
  switch or reboot). `video shot` prints `signal=… hash=…` on stderr — feed that
  hash to `--changed-since` to wait efficiently for the screen to change.
- **OCR** (`video read`, which wraps the daemon's `GET /ocr`; also the
  dashboard button) is on-device (Apple Vision on macOS, Tesseract on Linux).
  It reads large boot-screen / BIOS text well; very small console fonts can
  produce a few character confusions (e.g. `1`/`l`, `2`/`Z`).

## Serial console

A target can have **several named serial interfaces** (e.g. a main `console` and
a `bmc`). Each port is **exclusive** — only one consumer can hold it at a time —
but a single `watch` daemon **per target** owns *all* of that target's interfaces
at once. (Each target gets its own daemon, so several targets capture concurrently
on one host.)

```
paniolo serial add console -t <target> --device <path>   # add a named interface
paniolo serial add bmc -t <target> --device /dev/ttyUSB1 --baud 9600
paniolo serial set console -t <target> --sense cts        # update fields
paniolo serial rm <name> -t <target>                      # drop a named interface
paniolo serial connect [target] [-i name]      # interactive terminal (tio) in your shell
paniolo serial watch [target]                  # run the daemon for ALL interfaces;
                                               #   they appear in the dashboard pane
paniolo serial send [target] "text"            # send one line of input through
                                               #   the running daemon (see below)
paniolo serial log [target] [-i name] [options] # print captured output (timestamped)
paniolo serial show [target]                   # list interfaces + daemon status
paniolo serial stop [target]                   # release the ports (on the target's host)
paniolo serial devices                         # list serial devices on the host
paniolo serial dtr [target] [-i name] [--ms N] # pulse DTR line (J2 power button header)
paniolo serial reset [target] [-i name]        # soft reset via brief DTR pulse
```

**Argument convention:** every runtime command takes the target as an
optional positional (omit it when the lab has one target); channel-config
commands (`add`/`set`/`rm`) take `-t`. `serial send`/`serial log` accept
`-t` too — `serial send` reads two positionals as `<target> <text>`, one as
just the text. The sole `-t`-only runtime command is `hid send`, whose
positional tail belongs to the helper.

`--name` defaults to `console`, so a single-interface setup needs no flags. With
one interface, `-i`/`--interface` can be omitted everywhere. Don't run `connect`
and `watch` (or an external `screen`/`tio`) on the **same device** at once —
start one, or `stop`/close the other first.

### Reading captured output

While `watch` is running, the daemon keeps a **rolling, timestamped capture log**
per interface (persisted on disk, so it survives a daemon restart and you can read
it even after `stop`). The live view stays in the dashboard; use `serial log` when
you want to *read back* what scrolled past:

```
paniolo serial log                    # most recent ~200 lines (sole interface)
paniolo serial log -i bmc --tail 50   # most recent 50 lines of the 'bmc' interface
paniolo serial log --since 1840       # only lines after sequence #1840 (polling)
paniolo serial log --from 1800 --to 1860   # a specific line range
paniolo serial log --json             # JSON Lines (seq, ts_ms, text) for parsing
paniolo serial log --raw              # keep ANSI colors / control bytes
```

With more than one interface configured, pass `-i <name>` to choose one (omitting
it errors and lists the names); with more than one target, name it
(`serial log pi5 …` or `-t pi5`). Each line is shown as `[<UTC timestamp>] #<seq>
<text>`. The `seq` is a stable, monotonic line number — note it, then come back
later with `--since <seq>` to get only what's new, or `--from/--to` to re-read an
exact span. Output is ANSI-stripped by default; a `*` after the sequence number
marks the current unterminated line (e.g. a `login:` prompt with no newline yet).

### Sending input

`paniolo serial send` injects one line through the **running daemon** (start
`watch` first), so input coexists with capture — what you send and what the
target echoes both land in the log:

```
paniolo serial send <target> "reboot"                # CR appended by default
paniolo serial send "reboot"                         # sole-target lab: text only
paniolo serial send --no-newline "partial input"     # suppress the CR
paniolo serial send --pace-ms 8 "long command"       # per-byte pacing for slow
                                                     #   polled consoles
```

Note the input only reaches the target if its console actually reads the UART
(a kernel with a broken serial driver logs output but ignores input).

## adb (Android targets)

When the target is an Android device, the `adb` channel gives you console,
screen, and input over one transport (see `docs/adb.md`). adb runs
on the control host the device is plugged into; paniolo routes per-channel like
any other.

```
paniolo adb devices [-H <host>]          # list attached devices (their serials)
paniolo adb set -t <target> [--serial <id>]   # bind a device (omit --serial = sole device)
paniolo adb show [target]                # config + live device state
paniolo adb shell [target]               # interactive `adb shell` in your terminal
paniolo adb run -t <target> <cmd...>     # one-shot command, output captured (agent path)
paniolo adb screencap [target] [-o frame.png]   # one PNG (exec-out screencap; -o - = stdout)
paniolo adb input -t <target> <args...>  # adb shell input: keyevent/text/tap/swipe
```

- `run`/`input` take `-t` (not a positional) because their tail is free-form;
  put `-t` first and use `--` before a leading-dash argument
  (`paniolo adb run -t pixel -- logcat -d -t 50`).
- `adb run -t <t> getprop ro.product.model` is the read workhorse for agents;
  `adb screencap -o -` pipes a PNG back (works on a remote control host too).
- Reboot/power: no adb-specific command — wire `adb reboot` through the power
  hooks (`paniolo power set -t <t> --cycle-cmd "adb -s <id> reboot"`).
- The device must be authorized (`adb devices` shows it as `device`, not
  `unauthorized`/`offline`); paniolo does not manage pairing.

## Power control

```
paniolo power on  [target]             # run on_cmd hook (error with hint if unset)
paniolo power off [target]             # run off_cmd hook (error with hint if unset)
paniolo power-cycle [target]           # run cycle_cmd hook
paniolo power-state [target]           # state_cmd stdout ("on"/"off") or serial sense-line
paniolo serial dtr [target] [--ms N]   # pulse DTR line on J2 header (opt-in; see below)
paniolo serial reset [target]          # soft reset via brief DTR pulse (opt-in)
```

Configure hooks with `paniolo power set`:

```
paniolo power set -t <name> \
    [--cycle-cmd <cmd>] [--on-cmd <cmd>] [--off-cmd <cmd>] [--state-cmd <cmd>] \
    [--serial-interface console]   # default DTR interface when several opt in
```

All four hooks are optional and run via `sh -c`. `power-state` uses `state_cmd`
if set (first whitespace token of stdout must be `on` or `off`); falls back to
the serial sense-line otherwise.

**"Reboot over serial" ≠ DTR reset — pick the right one.** Two unrelated things
share the word "serial/reset":

- **Console reboot (software):** type `reboot` into a logged-in console with
  `paniolo serial send <target> "reboot"`. This is what "use serial to reboot"
  almost always means. If you can't log in, use `paniolo power-cycle <target>`.
- **DTR reset (hardware):** `paniolo serial dtr` / `serial reset` toggle the
  FTDI DTR line wired to the board's J2 power button. A ≤500 ms pulse is a soft
  button event; ≥3000 ms is a hard PMIC power-off.

DTR is **opt-in per interface** — it works only where the interface declares
`power_button = true` (`paniolo serial set <iface> -t <name> --power-button`),
because DTR-to-J2 wiring is rare. On a target that hasn't opted in, `serial dtr`
/ `serial reset` **error** with a hint (they never toggle a lone console blindly).
So: unless DTR wiring is explicitly declared, reboot via the console `reboot` or
`power-cycle` — do **not** assume `serial reset` power-cycles the board.

### Cambrionix hub (example)

The `cambrionix` helper binary drives a Cambrionix USB hub's control UART and
satisfies the `state_cmd` contract (`state <port>` prints `on` or `off`):

```
paniolo power set -t pi5 \
    --cycle-cmd "cambrionix -d /dev/cu.usbserial-DK0F9LZI cycle 4" \
    --on-cmd    "cambrionix -d /dev/cu.usbserial-DK0F9LZI on 4" \
    --off-cmd   "cambrionix -d /dev/cu.usbserial-DK0F9LZI off 4" \
    --state-cmd "cambrionix -d /dev/cu.usbserial-DK0F9LZI state 4"
```

See `docs/power.md` for the full `cambrionix` command surface.

## HID injection — type and click into the target

The default injector is the dual-board KB2040 "dumb pipe": the host-side
`hidrig` composes HID reports and writes binary frames to the **control**
board's USB-CDC port, which relays them over I2C1 to the **target** board that
presents a USB keyboard + mouse to the DUT. The `hid` channel stores an opaque
helper command (the `hidrig` CLI by default); `paniolo hid send` appends its
arguments to it and runs it on the channel's host:

```
paniolo hid set -t <name> --cmd "hidrig -d /dev/cu.usbmodemXXXX" [--host <labhost>]
paniolo hid send -t <name> type hello world   # type a string
paniolo hid send -t <name> key ENTER          # tap a key (adafruit_hid Keycode names)
paniolo hid send -t <name> combo LEFT_CONTROL C
paniolo hid send -t <name> move 300 -50       # relative mouse move (negatives OK; keep -t first)
paniolo hid send -t <name> click left | scroll -3 | releaseall
paniolo hid send -t <name> ping               # injector liveness check
```

The same dual-board control board can also back this target's **serial** and
**power** channels: when `hidrig serve` runs it bridges the DUT's serial console
and re-exports it as a PTY (the hid daemon is per-target, so point a `serial`
channel's `device =` at `/tmp/paniolo-<uid>/hid/<target>/console`), and `hidrig
power off|on|cycle` switches DUT
power via a relay (behind the `power` hook) — one USB device for HID, console,
and power. (The relay/power path is hardware-verified, incl. state persistence
across a control-board reset; the console bridge is new and not yet verified.)

Absolute mouse (click-where-you-point): `paniolo hid send -t <name> moveabs
<x> <y>` positions the cursor in a 0..32767 logical space the OS maps across
the screen (the KB2040 firmware advertises the `moveabs` capability).

Sequences: `hidrig run <file>` runs a command file (one command per line,
`# comments`, `delay <ms>` / `sleep <seconds>`) — the file must be on the
channel's host. The target board is powered by the DUT's USB port, so its HID
goes silent while the DUT is off and reboots with it; the control board is
host-powered and independent. Command vocabulary:
`docs/hid-serial-protocol.md`; dual-board design + frame format:
`docs/hid-dual-board-design.md`.

**KVM in the console.** `paniolo console <name>` shows a **⌨ Capture input**
toggle button over the video when the target has a `hid` channel: click it to
drive the target with your own keyboard + mouse (absolute — the cursor follows
where you point; your local cursor stays visible as a crosshair), click again to
release. It auto-starts the hid daemon (`hidrig serve`), which owns the control
link and re-exposes the command vocabulary over a WebSocket; `paniolo hid send` injections intermix
with what you type in the browser. Manual daemon control: `paniolo hid
serve [target]` / `paniolo hid stop [target]` (positional target — only
`hid send` and `hid set/rm` use `-t`). When the target also has a `power` channel, the overlay
adds an on/off toggle switch + a separate cycle button (each confirms first).

For *operating a GUI* through the `video` + `hid` channels — clicking, typing,
and navigating a desktop/app/installer/BIOS you can only see over the capture
and only touch through emulated HID — read the companion **`kvm-puppeting`**
skill (the look-act-settle-verify discipline): `paniolo skill kvm-puppeting`.

## Companion skills

Paniolo ships its own agent skills; list them and read any one from the CLI,
even on a packaged install:

```
paniolo skill                  # list bundled skills with their descriptions
paniolo skill kvm-puppeting    # print a skill's SKILL.md (e.g. GUI puppeting)
paniolo skill usbhub           # per-port USB-hub power control
paniolo skill paniolo --path   # the file path, to open or Read it
```

## Targets on a remote control host (a "lab")

When the machine wired to the target isn't the one you're running paniolo on,
describe your **lab** in one git-tracked TOML file and point paniolo at it with
`--lab <file>` or `PANIOLO_LAB`. Each target's `host` names a control host;
commands then run **transparently on that host over SSH** — you don't ssh by hand.

```toml
# mylab.toml
[hosts.bench1]
ssh = "curtisg@bench1.local"     # ssh destination — how others reach it ("local" = this machine)
# description = "bench Mac mini"  # optional free-text label, shown in `config show` / `host show`
# hostname = "bench1.local"      # this box's FQDN; set it so bench1 recognizes itself when ONE
#                                  shared lab file is run from any machine (matched vs `hostname -f`)
# identity = "~/.ssh/lab_key"    # set this if your ssh-agent offers many keys
#                                  (avoids "Too many authentication failures")
# paniolo_cmd = "/Users/curtisg/.local/bin/paniolo"  # if paniolo isn't on the
#                                  host's non-interactive ssh PATH

[targets.fortune]
host = "bench1"
# description = "Pi 5 under test"  # optional free-text label (legacy key: `note`)

[targets.fortune.netboot]
interface = "enx00e04c08d9a0"
tftp_root = "/home/curtisg/tftp/fortune"

[[targets.fortune.serial]]
name = "console"
device = "/dev/serial/by-id/usb-FTDI_FT232R_USB_UART_BG00W7NY-if00-port0"

[targets.fortune.power]
cycle_cmd = "/home/curtisg/scripts/power-cycle.sh"
```

```
export PANIOLO_LAB=~/labs/mylab.toml
paniolo netboot start fortune      # runs on bench1, transparently
paniolo power-cycle fortune
paniolo netboot logs -f fortune    # streams back live
paniolo serial connect fortune     # interactive tio over ssh -t
paniolo console fortune            # dashboard tunnelled to your browser; Ctrl-C to close
```

Notes:
- With **no** `--lab`/`PANIOLO_LAB`, paniolo uses the default lab at
  `~/.config/paniolo/lab.toml` (auto-created on first write).
- The lab file is **CLI-managed and hand-edit friendly**: all the config
  commands edit it surgically (your comments survive), and it stays your
  git-tracked source of truth.
- **Channels can live on different hosts** (per-channel `--host`): each command
  runs on the host of the channel it touches. Composites (`console`) need
  their channels co-located on one host.
- Still over plain SSH if you prefer: `ssh bench1 "paniolo …"` works too, but the
  lab makes location transparent.

### Authoring a lab (discovery + provisioning)

After hand-writing a host's connection info (`[hosts.bench1] ssh = …`), let
paniolo discover its hardware and propose the target block:

```
paniolo discover                                  # list THIS host's hardware
paniolo --lab mylab.toml configure fortune --host bench1
```

`configure` runs discovery on `bench1` over SSH and prints a proposed
`[targets.fortune]` block — best-guessing the USB-Ethernet interface and serial
device, listing other candidates as comments. It **writes nothing**: you review
it, paste it into the lab file, and commit (the lab is your reviewed source of
truth). Edit the guesses as needed; add `tftp_root`/`power` (not discoverable).

Provision a control host's daemons from your dev machine:

```
paniolo --lab mylab.toml setup --host bench1       # runs `paniolo setup` on bench1
```

(The host needs the paniolo CLI + source already present; this builds the Rust
daemons there over an ssh PTY.)

## Background daemons — one view of everything

Paniolo runs several per-subsystem daemons (serialcap, hdmicap, the hid
injector, zigplug, netbootd). `paniolo daemons` shows all of them at once,
plus **stray helper processes** running out of the libexec dir (e.g. wedged
one-shot hooks — the thing to check when a serial-port-owning helper
misbehaves):

```
paniolo daemons                          # list: name, pid, port, detail + strays
paniolo daemons stop zigplug serialcap   # stop specific daemons (SIGTERM)
paniolo daemons stop --all               # stop every daemon + TERM strays
paniolo daemons stop --all --force       # …and SIGKILL whatever survives 3 s
paniolo daemons restart --stale          # restart capture daemons on an old binary
paniolo daemons restart serialcap        # restart a named capture daemon
paniolo daemons restart --all            # restart every serialcap/hdmicap
```

netbootd is stopped via its proper teardown (interface cleanup), everything
else by signal. Per-subsystem `stop` commands still work; this is the
sweep-the-bench view.

**Stale daemons after an upgrade.** A daemon keeps the binary it started from;
`make install` / `apt install` replaces the binary on disk but not the running
process. `paniolo daemons` flags such a daemon **stale** (also shown on
`serial show` / `video show`), and `paniolo daemons restart [--all|--stale|NAME]`
cleanly cycles serialcap/hdmicap from the current binary, reusing the lab's
channel config. `serial watch` / `video watch` also auto-restart a stale
instance. netbootd is excluded — cycle it via `paniolo netboot start/stop`.

## Quick reference — gotchas

- **Drive through paniolo, not around it.** Don't reconfigure or open a
  paniolo-managed device by hand (`ifconfig`/`ip`/`ethtool` on the netboot
  interface, `screen`/`tio` on a serial port, `kill` on a daemon) — it desyncs
  the daemon. Use `netif mode …`, `serial log`/`send`, `daemons stop`.
- Serial port is exclusive: one of `connect` / `watch` / external `tio`/`screen`.
- `~/.cargo/bin` (the CLI) must be on `PATH`, ahead of any other `paniolo`
  (e.g. a Homebrew keg from the tap can shadow it). The helper binaries are
  *not* on PATH — they live in `~/.local/libexec/paniolo/bin`
  (`paniolo helper <name> …` to run one by hand).
- `paniolo console` auto-starts both daemons if they aren't running. Local
  `console` passes the serialcap daemon's OS-assigned port as `?serial=PORT`
  so the dashboard's serial pane connects correctly.
- Netboot requires passwordless `sudo` (`ip` on Linux, `ifconfig` on macOS).
- netboot and ffx are mutually exclusive on the link — use `paniolo netif mode`
  to switch; entering ffx mode stops netboot so a power-cycle boots from SD.
- To test the bare link: `paniolo netif mode link` (up) / `mode off` (soft down).
  Soft "down" only releases the host IP — with Wake-on-LAN on, `carrier` can still
  read `up` (the PHY stays powered). For a drop the target actually detects, use
  `paniolo netif down-hard` (kills WoL + admin-downs the iface), or unplug.
- Remote (lab) targets: if ssh fails with "Too many authentication failures",
  set the host's `identity` (agent key-spray); if the remote can't find paniolo,
  set its `paniolo_cmd` to an absolute path.
- OCR is strongest on large text; tiny console fonts may misread some characters.

---

Licensed under the Apache License, Version 2.0.
