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

# ch9329 — CH9329 USB-HID injector for paniolo

`ch9329` is a paniolo `hid` helper (the keyboard and mouse "hands" of a target)
for the **WCH CH9329**, a chip that turns commands on a UART (serial line) into
USB keyboard and mouse input (HID) on the target. It exposes the **same CLI
surface** as [`hidrig`](../hidrig/README.md) (the KB2040 injector client), so it
drops straight into a paniolo `hid` channel and `paniolo hid send` drives it the
same way.

| Device | How it connects | Status |
|---|---|---|
| **[Openterface Mini-KVM](https://openterface.com/)** (its keyboard/mouse half) | a real CH9329 behind its CH340 USB-serial adapter | bench-verified |
| **Openterface KVM-Go** | a CH32V208 emulating the CH9329 protocol over its own USB-CDC port; reports `chip_version=0x01` instead of a real chip's `0x38` (see [`notes/openterface-kvm-go.md`](../notes/openterface-kvm-go.md)) | bench-verified, unmodified helper |
| **Sipeed NanoKVM-USB** | same protocol, linked at 57600 baud | should work; not bench-verified here |

The difference is underneath. `hidrig` composes HID reports on the host and
sends them to its own KB2040 boards. The CH9329 *is itself* the USB HID device,
with a fixed command set, so `ch9329` parses each command and speaks the chip's
**binary frame protocol** directly (`HEAD 57 AB · ADDR · CMD · LEN · DATA · SUM`). The
protocol facts are the clean-room reference in
[`notes/ch9329-spec.md`](../notes/ch9329-spec.md).

## Wiring it into a target

```bash
# Build/install with everything else (lands in ~/.local/libexec/paniolo/bin):
make install            # from the repo root; or: make rust

# Bind the CH9329 to a target's hid channel (use its CH340 serial device):
paniolo hid set -t winbox --cmd "ch9329 -d /dev/cu.usbserial-4120"

# Drive it — identical to any hid helper:
paniolo hid send -t winbox type "hello world"
paniolo hid send -t winbox combo LEFT_CONTROL S
paniolo hid send -t winbox moveabs 16384 16384   # center; 0..32767 logical
paniolo hid send -t winbox click left
paniolo hid send -t winbox ping
```

Pair it with a `video` channel pointed at the same Openterface's MS2109 capture
(`paniolo video set -t winbox --device "<id>"`) and the one box is a full KVM:
`paniolo video shot` for eyes, `ch9329` for hands.

## Commands

The full [HID serial protocol](../docs/dev/hid-serial-protocol.md) §3 surface:
`type`, `key`, `combo`, `down`, `up`, `releaseall`, `move`, `moveabs`, `click`,
`mdown`, `mup`, `scroll`, `ping`, `version`, and `run <file>` for command
sequences.

- **Key names** are `adafruit_hid` Keycode names (`A`–`Z`, `ENTER`,
  `LEFT_CONTROL`, `FORWARD_SLASH`, `F1`…`F12`, `PRINT_SCREEN`, `SCROLL_LOCK`,
  `PAUSE`, `NUM_LOCK`, `APPLICATION`).
- **`type`** assumes a **US layout**. It rejects a character it can't type
  with `ERR` rather than dropping it silently.
- **`combo`** chords at most 6 keys at once (the boot-protocol report's key
  slots; modifiers don't count) and refuses a larger chord with `ERR`.
- **`version`** reports `1 ch9329/0.1.0 moveabs`. It advertises the
  **`moveabs`** capability: the CH9329 has a true absolute pointer, so
  click-where-you-point works. It deliberately does *not* advertise `baud`
  (see [`baud`](#extras-beyond-hidrig) below).

### Extras beyond hidrig

- **`ch9329 -d <dev> info`**: CH9329 `GET_INFO`. Reports firmware version,
  whether the target has enumerated the emulated HID (`target_connected`),
  lock-LED state, and the negotiated baud. Useful for `paniolo doctor`-style
  checks; the KB2040 can't report target enumeration, the CH9329 can.
- **`ch9329 -d <dev> usb host|target|state`**: switch or query the USB mux that
  shares one onboard microSD reader between the host and the target. Present on
  the **Openterface KVM-Go**, whose CH32V208 drives the mux select line; a real
  CH9329 does not implement it. Wired into paniolo as the `usb` channel
  (`paniolo usb attach-host|attach-target|state`, docs/usb.md); protocol in
  notes/openterface-usb-mux-spec.md.
  - The device replies with the *resulting* mux position, not a success code.
    `host`/`target` compare it against what was asked and fail if the mux did
    not move. All three print the resulting side.
  - A device without a mux does not answer at all (the protocol has no negative
    ack for an unknown opcode), so the wait times out. The daemon reports that
    plainly as "this device does not support USB mux switching", not as a
    transport failure. So a `usb state` query against ordinary hardware never
    trips the reopen logic (each reopen briefly toggles DTR/RTS, which resets a
    KVM-Go's MCU).
- **`ch9329 -d <dev> baud <rate>`**: **persistently** set the chip's serial
  baud (`SET_PARA_CFG` → flash → `RESET` to activate), then reconnect at the new
  rate. Datasheet range 1200..=115200 (Openterface default 115200; NanoKVM-USB
  57600; factory chips 9600). Use it, for example, to bring a factory-9600 chip
  up to 115200. The `RESET` makes the chip re-enumerate its USB HID, so the
  target briefly sees the keyboard/mouse disconnect and reconnect.
  - This is *persistent*, unlike the protocol's `baud`, which is a *transient*
    renegotiation that reverts on power-cycle. That is why `version` does not
    advertise a `baud` capability and the daemon never auto-invokes it: a host
    must not call it expecting transient behavior.
- **`-b/--baud <rate>`**: force the link rate for *this* connection without
  changing the chip (default: autodetect 115200, 57600, then 9600). Needed to
  reconnect after `baud` set a rate outside that autodetect list.

## Status and limitations

**Verified end to end into a real target.** A chip 0x38 over a CH340 at 115200,
into a Raspberry Pi OS desktop on a Pi 5: typing (US layout), special keys
(`ENTER`/`ESCAPE`/`CAPS_LOCK`, confirmed via the lock-LED round trip in
`GET_INFO`), absolute pointer positioning, clicking, and right-click all drive
the desktop correctly. The **Openterface KVM-Go** is bench-verified end to end
too (`chip_version=0x01`, 115200 — see
[`notes/openterface-kvm-go.md`](../notes/openterface-kvm-go.md)). The
NanoKVM-USB is not bench-verified here.

**Baud changing is hardware-verified.** The `baud` command (the `SET_PARA_CFG`
flash-and-reset procedure, `notes/ch9329-spec.md` §5) round-tripped
115200 → 9600 → 115200 on real hardware, and the change survived a fresh
process (it's persisted to flash).

**Lock keys are held for 200 ms.** A tap or chord involving a **lock key**
(`CAPS_LOCK` etc.) is held for 200 ms before release, because macOS debounces
short lock-key presses. Against a macOS 15 target at the bench, 30 ms never
registered, 60 ms did, and 200 ms toggled 10/10.

**Two CH9329-on-Linux quirks are worked around in `session.rs`:**

- *Clicks go through the **relative** report, not the absolute one.* libinput
  (the Linux input stack) coalesces a button transition in an absolute-pointer
  report at an unchanged coordinate, so it never registers as a click. A
  relative `BTN` report (zero motion) always processes, and clicks wherever the
  pointer currently is, so `moveabs` then a separate `click` lands correctly.
- *`moveabs` nudges one unit first.* The absolute device coalesces a report
  whose coordinates equal its previous one, so re-sending the same position
  after a relative move was a no-op (the cursor wouldn't snap back). Sending a
  one-unit-off report, then the exact target, forces a real move.

**The KVM daemon (`serve`/`stop`) is hardware-verified.** `ch9329 serve` owns
the UART and re-exposes the protocol over a localhost WebSocket (`GET /hid`)
plus a `POST /send` one-shot endpoint. It publishes the
`/tmp/paniolo-<uid>/hid/<target>/daemon.json` discovery file (port, plus the
per-start token every request must carry) that paniolo's `console` reads, so
the web-console "Capture input" KVM works with the Openterface.

- While a daemon runs, one-shot `paniolo hid send` invocations route through it
  automatically. The CLI and the browser never contend for the UART, and their
  injections intermix.
- The UART is driven by the blocking `serialport` path on a dedicated thread,
  bridged to the async server, because tokio-serial's async reads are
  unreliable on a macOS tty.

**Held state (`down`/`up`/`mdown`/drag) persists across commands only through
the daemon**, because its one long-lived session carries the report. This was
verified by holding `LEFT_SHIFT` across three separate CLI invocations and
getting uppercase output. *Without* a daemon, a direct one-shot
`ch9329 down A` resets per process (the CH9329 has no "read current report"
command). `combo` and `run` sequences still compose within one process.

**Recovery clears the chip's own report, not just the daemon's.** The chip
remembers its last HID report independent of the daemon process. So a fresh
`open()` (first start, or a reopen after a transport error) pushes an all-zero
keyboard and mouse report once `GET_INFO` confirms the chip is there, rather
than trusting a blank in-memory `Session` to mean nothing is held on the
target.

- A single lost reply is retried once in place before the daemon gives up and
  reopens (each reopen briefly toggles DTR/RTS, which resets a KVM-Go's MCU).
- `tap`/`combo`/`type` make a best-effort attempt to release whatever they
  pressed if a later step in the same call fails, so a mid-sequence transport
  hiccup does not leave a key down.
- Graceful shutdown (`SIGTERM`/Ctrl-C) releases every held key, modifier and
  mouse button before the daemon exits. It never touches the USB mux.

## License

Apache 2.0 — see the repository [LICENSE](../LICENSE).
