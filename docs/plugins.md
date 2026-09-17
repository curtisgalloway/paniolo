# Hardware plugins — private and unreleased hardware

Bench hardware that paniolo has never heard of — a custom front-panel fixture
that presses an unreleased board's buttons, a strap or DIP-switch controller,
a JTAG mux, a fixture that taps a test point — can still be driven through
paniolo, by an agent, without publishing anything about it. A **plugin** is an
out-of-tree command you own. paniolo records it on the target, runs it on the
right control host with your arguments appended, and asks it to describe
itself so an agent can learn its verbs. paniolo never learns the protocol.

This is the same principle as the [power hooks](power.md) and the
[hid channel](hid.md): device-specific logic lives outside the core, in a
helper. The difference is that a plugin's vocabulary is *its own*. Power hooks
have four fixed verbs and `usb` has three; `hid send` passes arguments through
but expects the [HID serial protocol](dev/hid-serial-protocol.md). A plugin is
for the hardware that fits none of those — and a target may carry several.

---

## Wiring one up

```bash
paniolo plugin add panel -t board1 \
    --cmd "panel-ctl -d /dev/ttyACM3" \
    --description "front-panel buttons" \
    [--host <labhost>]
```

- **`panel`** is the plugin's name. It is how `plugin run`/`describe` pick it
  (`-n panel`), and it must be unique per target. Names follow the same rule
  as serial interface names: letters, digits, `.`, `_`, `-`.
- **`--cmd`** is the command paniolo runs, with the caller's arguments
  appended, via `sh -c` on the plugin's host. A bare name resolves through
  paniolo's helper dirs first (`~/.local/libexec/paniolo/bin`, then
  `/usr/libexec/paniolo/bin`), then `PATH`; an absolute path works too. Keep
  device paths and per-unit options here, so the plugin binary itself stays
  configuration-free.
- **`--description`** is a one-liner shown by `plugin list` and `target show`.
  It is how an agent tells `panel` from `straps` before running `describe`.
- **`--host`** binds the plugin to a remote control host, like every other
  channel. Install the tool on *that* host.

In the lab file this is a `[[plugin]]` array, like `[[serial]]`:

```toml
[[targets.board1.plugin]]
name = "panel"
cmd = "panel-ctl -d /dev/ttyACM3"
description = "front-panel buttons"

[[targets.board1.plugin]]
name = "straps"
cmd = "/opt/rig/straps.sh"
host = "bench2"
```

`plugin set <name> -t <target> [--cmd …] [--description …] [--host …]` changes
only what you pass; `plugin rm <name> -t <target>` removes one.

## Using it

```bash
paniolo plugin list [board1]                  # what's configured — runs nothing
paniolo plugin describe board1 -n panel       # what can it do? runs `<cmd> describe`
paniolo plugin run -t board1 -n panel press reset            # runs `<cmd> press reset`
paniolo plugin run -t board1 -n panel press boot --hold-ms 3000
```

`-n` may be omitted when the target has exactly one plugin. With several,
paniolo refuses and names them — it never guesses. Keep `-t`/`-n` *first* on
`plugin run`: everything after them belongs to the plugin, hyphens included
(`-- --flag` if the first plugin argument starts with a dash).

The plugin's stdout and stderr pass straight through, and its exit code is
paniolo's exit code. On a remote host, the command re-execs there over SSH
exactly like `hid send` — nothing to install beyond the tool itself.

The typical agent loop, then, is:

1. `paniolo target show board1` — sees `plugin panel @bench1 … description=front-panel buttons`.
2. `paniolo plugin describe board1 -n panel` — reads the verbs.
3. `paniolo plugin run -t board1 -n panel press recovery` — acts.

`paniolo doctor` probes that each plugin's program exists on its host (an
absolute path via `test -e`, a bare name via `command -v` under the same
helper-dirs-then-PATH resolution the run uses), the way it probes power hooks.

---

## The plugin contract

paniolo asks exactly one thing of a plugin, and it is optional:

| paniolo command | what runs |
|---|---|
| `paniolo plugin describe … -n X` | `<cmd> describe` |
| `paniolo plugin run … -n X <args>` | `<cmd> <args>` |

**`describe`** should print, on stdout, what the plugin can do — one verb per
line with its arguments and a short gloss is the convention, in whatever form
you like (plain text is fine; an agent reads it, not a parser). A plugin that
does not implement it simply fails `paniolo plugin describe`, and an agent falls
back to the description in the lab file and whatever documentation you gave
it. Implement it: it is the difference between an agent that can use the
hardware cold and one that needs to be told.

**Every other verb is yours.** Some conventions worth following so plugins read
alike across a bench — all optional:

```
<cmd> describe                      # list the verbs (above)
<cmd> press <button> [--hold-ms N]  # momentary press; hold for N ms (default: a short tap)
<cmd> hold <button> / release <button>
<cmd> set <switch> on|off           # a strap, a DIP switch, a mux position
<cmd> state                         # read back what the hardware can report
```

Two rules from the power-helper recipe apply with full force:

- **Confirm by read-back wherever the hardware can report.** A `press` that
  the fixture silently ignored costs a whole debugging session; exit non-zero
  on a mismatch, and never report a guess from `state`.
- **Exit non-zero on failure, and say why on stderr.** paniolo propagates
  the code; an agent reads the message.

Environment the plugin can rely on, every invocation:

| Variable | Value |
|---|---|
| `PATH` | paniolo's helper dirs prepended, so a bundled helper (`ch9329`, `hidrig`, …) is callable by bare name from inside a plugin |
| `PANIOLO_TARGET` | the target's name (`board1`) |
| `PANIOLO_PLUGIN` | the plugin entry's name (`panel`) |
| `PANIOLO_STATE_DIR` | `~/.config/paniolo/helpers/<program>/<target>/` — durable state (calibration, pairing), pre-created |
| `PANIOLO_RUNTIME_DIR` | `/tmp/paniolo-<uid>/<program>/<target>/` — locks, logs, discovery, pre-created; wiped on reboot |

`<program>` is the basename of the cmd's first token (`panel-ctl`, `straps.sh`),
so one plugin binary serving several targets keeps their state apart. A
private script that serves a whole bench can key its own config on
`PANIOLO_TARGET`/`PANIOLO_PLUGIN` instead of repeating device paths in every
`cmd` string. Never write unnamespaced files into `~/.config/paniolo/` itself —
that is where the lab file lives.

One-shot, stateless, exclusive: each invocation opens the device, acts, exits.
If the transport is an exclusive-open serial port, two concurrent invocations
collide — the same trade-offs as for
[power helpers](dev/adding-power-helpers.md#7-field-notes-earned-the-hard-way),
including when a small daemon is the right answer.

---

## A minimal example

A plugin can be a shell script. This one drives a hypothetical panel MCU that
takes one-line commands over a serial port (`PRESS <name> <ms>`, `STATE`) and
echoes `OK`:

```sh
#!/bin/sh
# panel-ctl — private front-panel fixture for board1. Not for publication.
set -eu
dev=/dev/ttyACM3
[ "${1:-}" = "-d" ] && { dev=$2; shift 2; }
send() { printf '%s\r\n' "$1" > "$dev"; head -n1 < "$dev" | tr -d '\r'; }
case "${1:-}" in
  describe)
    echo "press <reset|boot|recovery> [--hold-ms N]  -- press a panel button"
    echo "state                                      -- LED readback";;
  press)
    btn=$2; ms=200
    [ "${3:-}" = "--hold-ms" ] && ms=$4
    reply=$(send "PRESS $btn $ms")
    [ "$reply" = OK ] || { echo "panel refused: $reply" >&2; exit 1; }
    echo "pressed $btn (${ms} ms)";;
  state) send STATE;;
  *) echo "usage: panel-ctl [-d DEV] describe|press|state" >&2; exit 2;;
esac
```

Install it where the plugin's host can run it — `~/.local/libexec/paniolo/bin/`
keeps it off `PATH` but resolvable by bare name (and `paniolo helper` lists it
alongside the shipped helpers), or use an absolute path in `--cmd`. Then:

```bash
paniolo plugin add panel -t board1 --cmd "panel-ctl -d /dev/ttyACM3" --description "front-panel buttons"
paniolo plugin describe board1
paniolo plugin run -t board1 press reset
```

Keep the script in your private hardware repo. Nothing about it — the
protocol, the device, the board — needs to reach this one.

---

## When a plugin is the wrong tool

- **It switches the target's power.** Use the [power hooks](power.md):
  `power-cycle`, `power on/off`, `power-state` are what every other part of
  paniolo (the dashboard, the skills, the eval scenarios) reaches for, and a
  plugin named `power` is invisible to them.
- **It types or clicks.** Use the [hid channel](hid.md) with a helper speaking
  the HID serial protocol; the KVM path and `paniolo console` build on it.
- **It routes a USB device between host and target.** That is the
  [usb channel](usb.md).
- **It is a reset line on the console adapter's DTR.** That is `serial dtr` /
  `serial reset` with `--power-button` on the interface (see power.md).

A plugin is for everything else — and for the first cut of something that may
later deserve a channel of its own.
