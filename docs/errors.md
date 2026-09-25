<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# Exit status and errors

A script or agent driving paniolo can tell *what kind* of failure happened from
the exit status alone, and get the details as one line of JSON, without
matching any English message text.

**Terms.**

- **Kind:** the class of a failure (`not_configured`, `daemon_down`, …). Each
  kind has one exit code.
- **Control host:** the machine a target's hardware is plugged into; paniolo
  re-runs a command there over SSH when the target is not local
  ([distributed control](distributed-control.md)).
- **Hook:** a script from the lab file that paniolo runs for a channel
  (`cycle_cmd`, a power `on_cmd`, a USB or HID `cmd`).
- **Helper:** a program paniolo ships or runs for a channel (`serialcap`,
  `hdmicap`, a power helper such as `shellyplug`).
- **Daemon:** a helper that keeps running in the background and answers
  requests (`serialcap`, `hdmicap`, `netbootd`).
- **Passthrough:** a command whose job is to run another program and hand back
  *its* exit status (listed [below](#passthrough-commands)).

## Exit codes

| Code | Kind | Meaning |
|---|---|---|
| 0 | — | Success. |
| 1 | — | The command ran and the answer is "no": `doctor` found problems. No other paniolo command exits 1 (a [passthrough](#passthrough-commands) may, for its program), and **no error exits 1**. |
| 2 | `usage` | Bad flags or arguments. |
| 3 | `not_configured` | No lab file, an invalid one, or an unknown target, channel, interface or host; a hook or helper that is missing or not executable. Fix the lab or install the helper; retrying will not help. |
| 4 | `unreachable` | The control host could not be reached over SSH. The command may not have run, or may have been killed part way (OpenSSH reports both the same way). |
| 22 | `timeout` | No answer within a deadline: a daemon still starting, a daemon request, an SSH port forward. The outcome is unknown, so check state before repeating something that changes it (a power cycle). |
| 100 | `daemon_down` | The channel's daemon is not running and the command needs it, or its record is stale. Start it with `paniolo serial watch` or `paniolo video watch`; `paniolo daemons` lists what is running. |
| 101 | `helper_failed` | A hook or helper ran and failed: it exited non-zero (its code is in `child_exit`), a daemon exited during startup, or a running daemon refused the request (its reason is the message). |
| 109 | `internal` | A failure paniolo does not classify yet. Treat it as a bug report waiting to happen; the message says what went wrong. |

Codes follow the tens-digit bands: 2–9 will fail the same way again, 20s may
succeed on a retry, 100 and up are specific to paniolo. Nothing exits 125 or
above except a [passthrough](#passthrough-commands) reporting its child.

A command that re-runs on a control host exits with the **remote** paniolo's
code, so a missing channel on `bench1` is 3 on your machine too. Only an SSH
failure is reported locally, as 4.

## The JSON error object

Ask for it by setting `PANIOLO_JSON_ERRORS=1`, or with `--json-errors` placed
**before** any trailing arguments: `paniolo --json-errors hid send …`. Commands
that pass their remaining arguments on (`hid send`, `adb run`, `adb input`,
`helper`) treat a `--json-errors` after those arguments as one of them, so
the variable is the safer choice in scripts. On failure, paniolo then prints the usual message and,
as the **last line of stderr**, exactly one JSON object:

```console
$ paniolo --json-errors power-cycle nosuch
target 'nosuch' not found in lab
{"error":{"channel":null,"child_exit":null,"code":3,"host":null,"kind":"not_configured","message":"target 'nosuch' not found in lab","target":"nosuch"}}
$ echo $?
3
```

| Field | Value |
|---|---|
| `kind` | One of the kinds in the table above. |
| `code` | The exit status, repeated. |
| `message` | The same text printed on the line(s) above it. For a command-line parse error (`usage`) it is only the first sentence of the usage error, without its `error: ` prefix. |
| `target` | The target name, when the failure is about one. |
| `channel` | The channel kind (`power`, `serial`, `video`, `hid`, `usb`, `netboot`, …). |
| `host` | The control host, for `unreachable`. |
| `child_exit` | The hook's or helper's own exit code, for `helper_failed`; null when a signal killed it. |

Every field is always present, `null` when it does not apply. Key order is not
part of the contract. Stdout is never
changed, so `--json` output from commands such as `discover` or `serial log
--json` stays parseable.

Parse only the last line of stderr: anything a hook or helper prints comes
before it. The variable is not passed on to hooks, helpers or daemons, so a hook
that itself runs paniolo does not add a second object; a hook that wants one
sets the variable itself.

**Version skew.** A control host running paniolo 0.4.x or older exits 1 for
every error and does not know `--json-errors`. With the flag, such a host
rejects the command with exit 2 (`unexpected argument '--json-errors'`) rather
than running it. Upgrade the host (`apt upgrade`, then `paniolo daemons restart
--stale`).

## Passthrough commands

These commands run another program for you and exit with **that program's**
status, the way `ssh host cmd` does, so paniolo's codes do not apply once the
program has started:

- `paniolo helper <name> …`
- `paniolo config edit` (your `$EDITOR`)
- `paniolo video shot`, `paniolo video devices` (the `hdmicap` helper)
- `paniolo adb run`, `paniolo adb input`, `paniolo adb devices`
- `paniolo serial connect` (tio) and `paniolo adb shell`, the interactive
  consoles
- `paniolo setup --host <host>` (the remote setup)

A program killed by a signal exits 128+N, as in a shell. paniolo's own failures
*before* the program starts (unknown target, unreachable host) still use the
table above.

## A stopped serial daemon

`paniolo serial log` reads the capture file from disk, so it still works when
`serialcap` is stopped, but shows nothing captured since it stopped. In that
case it prints a `warning:` line on stderr and exits 0. Pass `--require-live`
to make it fail with 100 (`daemon_down`) instead, when a stale log would be a
wrong answer.
