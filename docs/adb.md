# adb (Android targets)

Use the **adb channel** when the target is a phone, tablet, or Android-based
SBC (single-board computer) that the Android Debug Bridge (`adb`) can reach. It
gives paniolo the target's console (`adb shell`), screen
(`adb exec-out screencap`), and input injection (`adb shell input`). These are
the same verbs the serial/video/hid channels provide for wired bring-up
hardware, through one transport.

`adb` is a *generic transport* like SSH, not a device-specific helper, so it
lives in the core CLI (`cli/src/adb.rs`) rather than a libexec helper. paniolo
shells out to the host's `adb` binary directly. The device is named by its
`adb -s <serial>` id and bound to the control host it is physically plugged
into. paniolo reaches that host (local, or over SSH) through the usual
per-channel [dispatch](distributed-control.md).

> **Scope.** The channel covers console, screen, and input. Reboot/power
> needs no adb-specific code. Wire `adb reboot` through the generic
> [power hooks](power.md): `paniolo power set -t pixel --off-cmd "adb -s <id> reboot -p" --cycle-cmd "adb -s <id> reboot"`.

---

## Setup

```bash
# Discover attached devices (their serials), locally or on a control host
paniolo adb devices
paniolo adb devices -H bench1          # over SSH on a remote control host

# Or let paniolo propose a whole target block from discovered hardware —
# `adb` appears alongside the serial/video/netboot it finds (authorized
# devices only). Review and paste it into the lab; paniolo never writes it.
paniolo configure pixel -H bench1
paniolo discover                       # raw inventory of the local host

# Bind a device to a target in the lab file
paniolo target add pixel
paniolo adb set -t pixel --serial 33271JEGR02033

# Sole attached device? Omit --serial.
paniolo adb set -t pixel

# Device on a remote control host (adb runs there; reached over SSH)
paniolo adb set -t pixel --serial 33271JEGR02033 --host bench1

# Pin a non-PATH adb binary
paniolo adb set -t pixel --adb /opt/platform-tools/adb

# Remove the channel
paniolo adb rm -t pixel
```

**Prerequisite:** the device must already be authorized for adb (USB
debugging on, host key accepted). paniolo does not manage pairing. `adb devices`
must show the device as `device`, not `unauthorized`/`offline`.

`paniolo doctor` checks the channel by running `adb get-state` on the channel's
host. It reports:

- `ok` when the device answers in the `device` state;
- `MISSING` when it does not;
- a distinct *"adb not installed"* note when the binary itself is absent (adb
  is a system tool on `PATH`, not a paniolo libexec helper).

---

## Commands

With a single target in the lab, the target argument may be omitted.

```bash
# Console
paniolo adb shell pixel                       # interactive `adb shell` (PTY)
paniolo adb run -t pixel getprop ro.product.model   # one-shot, captured
paniolo adb run -t pixel -- logcat -d -t 50         # `--` guards leading flags

# Screen
paniolo adb screencap pixel -o shot.png       # PNG to a file — always this machine's
paniolo adb screencap pixel -o - > shot.png   # PNG to stdout

# Input
paniolo adb input -t pixel keyevent KEYCODE_HOME
paniolo adb input -t pixel text "hello world"
paniolo adb input -t pixel tap 540 1200
paniolo adb input -t pixel swipe 540 1800 540 600 200

# Discovery (no configured channel required)
paniolo adb devices [-H <host>]
paniolo adb show pixel                         # config + live device state
```

- `shell` is the interactive console (like `serial connect`).
- `run` is the agent-friendly one-shot: it captures output and propagates the
  command's exit code.
- `run` and `input` take a free-form tail, so they name the target with
  `-t/--target` (like `hid send`), not a positional. Put `-t` first, and use
  `--` before any argument that starts with a dash.

`screencap` uses `adb exec-out screencap -p`, which is binary-clean (no CRLF
mangling). `-o <path>` always means **this machine's** filesystem, exactly
like `video shot`. For a remote channel, `-o` streams the PNG over SSH into a
local sibling temp file and renames it onto `-o`'s path only on success. A
failed or interrupted capture therefore never truncates or half-writes what
was already at that path. `-o -` (the default) streams the PNG to stdout
instead.

---

## Lab file shape

```toml
[targets.pixel.adb]
serial = "33271JEGR02033"   # adb -s id; omit for the sole attached device
# adb  = "/opt/platform-tools/adb"   # override the adb binary (default: adb on PATH)
# host = "bench1"                    # control host the device is plugged into
```

---

## Relationship to the other channels

| Capability | Wired bring-up channel | adb equivalent |
|---|---|---|
| Console (interactive) | `serial connect` | `adb shell` |
| Console (one-shot/agent) | `serial send` / `serial log` | `adb run` |
| Screen | `video shot` (UVC capture) | `adb screencap` |
| Input | `hid send` (USB HID rig) | `adb input` |
| Reboot / power | `power` hooks, DTR | `power` hooks (`adb reboot`) |

An adb target needs no capture card, HID rig, or serial adapter: one USB cable
to the control host carries all of it. The trade-off is that adb sees only the
running Android userspace, not the bootloader/firmware that a serial console
and a capture card observe. For those, give the target *both* an `adb` channel
and the wired channels.
