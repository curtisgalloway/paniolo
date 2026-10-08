<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-->

# jetkvm — JetKVM network KVM helper for paniolo

`jetkvm` is a paniolo `hid` helper for the [JetKVM](https://jetkvm.com/), a
network KVM that plugs into the target over USB and HDMI and is reached over
the network. It has the **same CLI surface** as [`ch9329`](../ch9329/README.md)
and [`hidrig`](../hidrig/README.md) (`type`, `key`, `combo`, `down`, `up`,
`releaseall`, `move`, `moveabs`, `click`, `mdown`, `mup`, `scroll`, `ping`,
`version`, `run`, `serve`, `stop`), so it drops into a `hid` channel:

```
paniolo hid set -t target-machine --cmd "jetkvm -d 192.0.2.10"
```

It is written in Go because the device's firmware (0.5.9) exposes keyboard and
mouse **only over WebRTC**, and the Go WebRTC stack
([pion](https://github.com/pion/webrtc)) is the practical way to speak it.
Milestone 1 covers keyboard and mouse injection; video, virtual media and
power are not part of this helper.

## How it talks to the device

1. `POST http://<host>/auth/login-local` with the password; the reply sets an
   `authToken` cookie.
2. A WebSocket to `/webrtc/signaling/client` (with that cookie) carries the
   WebRTC offer and answer.
3. One data channel labeled `rpc` carries JSON-RPC 2.0 text messages:
   `keyboardReport`, `absMouseReport`, `relMouseReport`, `wheelReport`,
   `ping`, `getUSBState`, `getVideoState`, `getKeyboardLedState`. No media
   tracks are negotiated.

## The one-session rule

JetKVM allows **one session at a time**. A new offer closes the current
session, which is told with an `otherSessionConnected` event. Consequences:

- Opening the JetKVM web UI while `jetkvm serve` is running kicks the daemon,
  and a `jetkvm` command kicks your browser tab.
- `jetkvm serve` holds one session and every one-shot routes through it, so
  many commands cost one login, not one each.
- When the daemon is kicked it logs that clearly and does **not** reconnect on
  its own (that would fight the person using the browser). The next command
  you send reconnects. A plain network drop is retried in the background with
  backoff (1 s up to 30 s).

## Password

Never a flag value. In order, the first source that is set wins:

1. `JETKVM_PASSWORD` in the environment
2. `--password-file <path>` (one trailing newline dropped; a warning if the
   file is readable by group or others)
3. `--password-command '<cmd>'` (run with `sh -c`, 30 s limit; stdout is the
   password), e.g. `--password-command 'op read op://vault/jetkvm/password'`

A missing password or a rejected one exits 3.

## `ok` is not "delivered"

The firmware answers every HID call with success even when the target's USB
host has not enumerated the device (target off, in firmware, cable out). Use
`jetkvm info` to see the USB state, video state and keyboard LEDs.

## Commands

See [docs/hid.md](../docs/hid.md) and the
[HID serial protocol](../docs/dev/hid-serial-protocol.md). JetKVM-specific:

| Command | What it does |
|---|---|
| `info` | Prints `usb=… video=… leds=…` from the device |
| `version` | `1 jetkvm/<ver> moveabs` (needs no device or password) |

`click`, `mdown` and `mup` use the relative-mouse report with zero motion, so
they act wherever the pointer already is; a preceding `moveabs` picks the
spot.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | the command failed (device error, unreachable, timeout) |
| 2 | usage error |
| 3 | no password, or the device rejected it |

## Build and test

```
cd jetkvm
go build .
gofmt -l . ; go vet ./... ; go test -race ./...
```

The tests include a fake JetKVM (login, signaling and a pion answerer) driven
end to end. `paniolo setup` builds this helper into the private libexec dir
when `go` is on PATH. Release packaging (the `.deb`, tarball, zip and
Homebrew keg) is not wired up yet.
