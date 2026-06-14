# adb (Android targets)

paniolo can drive an **Android target (DUT)** over the Android Debug Bridge.
When the target is a phone, tablet, or Android-based SBC reachable via `adb`,
the **adb channel** gives paniolo its console (`adb shell`), screen
(`adb exec-out screencap`), and input injection (`adb shell input`) — the same
verbs the serial/video/hid channels provide for wired bring-up hardware, but
through one transport.

adb is a *generic transport* like SSH, not a device-specific helper, so it
lives in the core CLI (`cli/src/adb.rs`) rather than a libexec helper — paniolo
shells out to the host's `adb` binary directly. The device is named by its
`adb -s <serial>` id and bound to the control host it is physically plugged
into; reaching that host (local, or over SSH) is the usual per-channel
[dispatch](distributed-control.md).

> **Scope.** The first cut covers console, screen, and input. Reboot/power
> needs no adb-specific code — wire `adb reboot` through the generic
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

`paniolo doctor` checks the channel by running `adb get-state` on the channel's
host: `ok` when the device answers in the `device` state, `MISSING` when it does
not, and a distinct *"adb not installed"* note when the binary itself is absent
(a system tool on `PATH`, not a paniolo libexec helper).

The device must already be authorized for adb (USB debugging on, host key
accepted). paniolo does not manage pairing — `adb devices` showing the device as
`device` (not `unauthorized`/`offline`) is the prerequisite.

---

## Commands

With a single target in the lab, the target argument may be omitted.

```bash
# Console
paniolo adb shell pixel                       # interactive `adb shell` (PTY)
paniolo adb run -t pixel getprop ro.product.model   # one-shot, captured
paniolo adb run -t pixel -- logcat -d -t 50         # `--` guards leading flags

# Screen
paniolo adb screencap pixel -o shot.png       # PNG to a file
paniolo adb screencap pixel -o - > shot.png   # PNG to stdout (works remote)

# Input
paniolo adb input -t pixel keyevent KEYCODE_HOME
paniolo adb input -t pixel text "hello world"
paniolo adb input -t pixel tap 540 1200
paniolo adb input -t pixel swipe 540 1800 540 600 200

# Discovery (no configured channel required)
paniolo adb devices [-H <host>]
paniolo adb show pixel                         # config + live device state
```

`shell` is the interactive console (analogous to `serial connect`); `run` is the
agent-friendly one-shot that captures output and propagates the command's exit
code. Because `run`/`input` take a free-form tail, they use `-t/--target` for the
target (like `hid send`) rather than a positional — put `-t` first, and use `--`
before any argument that starts with a dash.

`screencap` uses `adb exec-out screencap -p`, which is binary-clean (no CRLF
mangling). As with `video shot`, `-o -` streams the PNG back over SSH for a
remote channel; an `-o <file>` path is written on the **channel's host**.

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

An adb target needs no capture card, HID rig, or serial adapter — one USB cable
to the control host carries all of it. The trade-off is that adb only sees the
running Android userspace, not the bootloader/firmware a serial console and a
capture card observe; for that, a target can carry *both* an `adb` channel and
the wired channels.
