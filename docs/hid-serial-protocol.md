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

# HID serial protocol, version 1

A device-independent text protocol for USB HID input injectors: a
microcontroller (or bridge chip wrapper) that presents a USB HID keyboard +
mouse to a *target* machine and accepts injection commands from a *control
host* over a serial byte stream.

This document is **normative**. Implementations:

| Implementation | Status |
|---|---|
| `hidrig/firmware/` — Adafruit KB2040, CircuitPython | Reference implementation |
| WCH CH9329 bridge (see [ch9329-spec.md](ch9329-spec.md)) | Deferred — would need a host-side shim speaking this protocol |

The host-side client is the `hidrig` CLI (`hidrig/src/`), which works against
any conforming device.

---

## 1. Transport

- Any bidirectional byte stream. The reference implementation uses a UART:
  **115200 baud, 8N1, no flow control, 3.3 V logic**. USB CDC or a TCP
  socket are equally valid carriers.
- Encoding is **UTF-8**.
- No transport-level framing, checksums, or escaping: the stream is assumed
  reliable (it is a wire on a bench).

## 2. Framing and flow control

- A **command** is a single line terminated by `\n` (LF). Implementations
  MUST tolerate and strip a trailing `\r` (CR).
- A **response** is a single line, also LF-terminated:
  - `OK` — success.
  - `OK <data>` — success with payload (e.g. `version`).
  - `ERR <message>` — failure; `<message>` is free-form human-readable text.
- Exactly one response per command, in order. The device MUST NOT emit
  unsolicited lines.
- The host MUST wait for the response before sending the next command — the
  one-command-in-flight rule is the protocol's only flow control. Responses
  to long-running commands (`type` of a long string, a large `move`) arrive
  only after the last HID report is submitted.
- A device that cannot parse or execute a command MUST reply `ERR ...` and
  keep running; a malformed line never kills the session.
- Empty lines are ignored (no response).

## 3. Commands

All commands and key/button names are case-insensitive; the canonical
spelling below is lowercase for verbs, UPPERCASE for key names. Arguments
are separated by single spaces.

| Command | Reply | Effect |
|---|---|---|
| `type <text>` | `OK` | Type `<text>` (everything after the first space, verbatim) as keystrokes |
| `key <NAME>` | `OK` | Tap (press + release) one key |
| `combo <NAME>...` | `OK` | Chord: press all named keys, then release all |
| `down <NAME>` | `OK` | Press and hold a key |
| `up <NAME>` | `OK` | Release a held key |
| `releaseall` | `OK` | Release all held keys |
| `move <dx> <dy>` | `OK` | Relative mouse move; signed decimal integers |
| `click <button>` | `OK` | Tap (press + release) a mouse button |
| `mdown <button>` | `OK` | Press and hold a mouse button |
| `mup <button>` | `OK` | Release a held mouse button |
| `scroll <amount>` | `OK` | Scroll wheel; signed decimal integer, positive = up |
| `ping` | `OK` | No-op liveness check |
| `version` | `OK <ver> <impl>` | Protocol version (decimal integer) + implementation id (free-form) |

- `<button>` is `left`, `right`, or `middle`.
- `move` / `scroll` values may exceed one HID report's range (int8 for
  boot-protocol relative mice); the device MUST split them into multiple
  reports transparently.
- `type` text is the remainder of the line after `type ` — it may contain
  spaces and `#`; no quoting or escaping exists. Characters outside the
  device's keyboard layout (reference: US) may be typed approximately or
  rejected with `ERR`.
- `version` for this spec replies `OK 1 <impl>`, e.g.
  `OK 1 kb2040-circuitpython/1.0`. Hosts use it to detect protocol
  compatibility; `<impl>` is informational only.

### Key names

`<NAME>` values are USB HID keyboard usage names in the `adafruit_hid`
`Keycode` convention. A conforming implementation MUST accept at least:

- Letters `A`–`Z`; digits `ZERO`–`NINE` (top row); `KEYPAD_ONE`-style names
  are optional.
- `ENTER`, `TAB`, `SPACE`, `ESCAPE`, `BACKSPACE`, `DELETE`, `INSERT`,
  `HOME`, `END`, `PAGE_UP`, `PAGE_DOWN`.
- `UP_ARROW`, `DOWN_ARROW`, `LEFT_ARROW`, `RIGHT_ARROW`.
- `LEFT_CONTROL`, `LEFT_SHIFT`, `LEFT_ALT`, `LEFT_GUI` and the `RIGHT_*`
  forms.
- `F1`–`F12`.
- `MINUS`, `EQUALS`, `LEFT_BRACKET`, `RIGHT_BRACKET`, `BACKSLASH`,
  `SEMICOLON`, `QUOTE`, `GRAVE_ACCENT`, `COMMA`, `PERIOD`,
  `FORWARD_SLASH`, `CAPS_LOCK`, `PRINT_SCREEN`.

An unknown name is an `ERR`. (The reference implementation accepts the full
`adafruit_hid.Keycode` table; non-CircuitPython implementations map the
names to HID usage IDs themselves.)

## 4. Device behavior

- **Boot:** the device begins serving the protocol as soon as it is ready;
  there is no banner or greeting (the host would have no way to distinguish
  it from a stale buffer anyway). Hosts should `ping` to detect liveness.
- **Target not enumerated:** commands that need USB may block until
  enumeration or fail with `ERR`; the device MUST NOT crash. (The reference
  implementation blocks at startup until the target enumerates, and replies
  `ERR` on later send failures, e.g. target suspend.)
- **Power:** an injector powered from the target's USB port reboots with the
  target; held keys cannot survive a target power cycle. Hosts must tolerate
  serial silence while the target is off.
- **State:** the only session state is the set of held keys/buttons
  (`down`/`mdown`). `releaseall` clears held keys. There is no reset
  command; power-cycling the device is the reset.

## 5. Reserved extensions

Future versions may add (implementations MUST `ERR` on these today, which
v1 hosts treat as "not supported"):

- `moveabs <x> <y>` — absolute mouse positioning in a logical
  `0..32767` space (requires an absolute-axis HID report descriptor).
- `consumer <NAME>` — consumer-control usages (volume, media keys).

Protocol changes that break v1 semantics bump the `version` integer.

## 6. Conformance checklist for a new implementation

1. Serve the byte stream (UART/CDC/TCP) with LF-terminated lines, CR
   tolerated, UTF-8.
2. Implement every command in §3; reply `ERR` (never crash, never silence)
   on anything unparseable.
3. One response per command, in order, only after the HID effect is fully
   submitted.
4. Accept the §3 key-name set case-insensitively.
5. Split oversized `move`/`scroll` into multiple HID reports.
6. Reply `OK 1 <your-impl-id>` to `version`.
7. Verify against the host tool: `hidrig -d <port> ping`, `version`, a
   `type` round-trip, and `hidrig run` of a sequence file.
