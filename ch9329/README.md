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

`ch9329` is a paniolo `hid` helper for the **WCH CH9329** UART→USB-HID bridge —
the keyboard/mouse half of an **[Openterface Mini-KVM](https://openterface.com/)**
(reached through its CH340 USB-serial adapter). It is a sibling of
[`hidrig`](../hidrig/README.md) (the KB2040 injector client): it exposes the
**same CLI surface**, so it drops straight into a paniolo `hid` channel and
`paniolo hid send` drives it identically.

The difference is underneath. `hidrig` forwards the line-based
[HID serial protocol](../docs/hid-serial-protocol.md) over a UART to a KB2040
running firmware that does the HID. The CH9329 *is itself* the USB HID device,
so `ch9329` parses each command and speaks the chip's **binary frame protocol**
directly (`HEAD 57 AB · ADDR · CMD · LEN · DATA · SUM`). The protocol facts are
the clean-room reference in [`docs/ch9329-spec.md`](../docs/ch9329-spec.md).

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

The full [HID serial protocol](../docs/hid-serial-protocol.md) §3 surface:
`type`, `key`, `combo`, `down`, `up`, `releaseall`, `move`, `moveabs`, `click`,
`mdown`, `mup`, `scroll`, `ping`, `version`, and `run <file>` for command
sequences. Key names are `adafruit_hid` Keycode names (`A`–`Z`, `ENTER`,
`LEFT_CONTROL`, `FORWARD_SLASH`, `F1`…`F12`); `type` assumes a **US layout**.

Extras beyond hidrig's surface:

- `ch9329 -d <dev> info` — CH9329 `GET_INFO`: firmware version, whether the
  target has enumerated the emulated HID (`target_connected`), lock-LED state,
  and the negotiated baud. Useful for `paniolo doctor`-style checks; the KB2040
  can't report target enumeration, the CH9329 can.
- `ch9329 -d <dev> baud <rate>` — **persistently** set the chip's serial baud
  (`SET_PARA_CFG` → flash → `RESET` to activate), then reconnect at the new
  rate. Datasheet range 1200..=115200 (Openterface default 115200; factory
  chips 9600). Use it to bring a factory-9600 chip up to 115200, for example.
  Note the `RESET` makes the chip re-enumerate its USB HID, so the target
  briefly sees the keyboard/mouse disconnect and reconnect.
- `-b/--baud <rate>` — force the link rate for *this* connection without
  changing the chip (default: autodetect 115200 then 9600). Needed to reconnect
  after `baud` set a rate other than 115200/9600.

`version` reports `1 ch9329/0.1.0 moveabs` — it advertises the **`moveabs`**
capability (the CH9329 has a true absolute pointer, so click-where-you-point
works). It deliberately does *not* advertise `baud`: the protocol's `baud` is a
*transient* renegotiation that reverts on power-cycle, but the CH9329's is
persistent (above), so a host should not auto-invoke it expecting transient
behavior.

## Status and limitations

- **One-shot injection is implemented and verified end-to-end into a real
  target** (chip 0x38 over a CH340 at 115200, into a Raspberry Pi OS desktop on
  a Pi 5): typing (US layout), special keys (`ENTER`/`ESCAPE`/`CAPS_LOCK` —
  confirmed via the lock-LED round trip in `GET_INFO`), absolute pointer
  positioning, clicking, and right-click all drive the desktop correctly.
- **Two CH9329-on-Linux quirks are worked around in `session.rs`** (both would
  have bitten a naive port — marion's mouse code, never hardware-tested, has
  neither):
  - *Clicks go through the **relative** report, not the absolute one.* A button
    transition in an absolute-pointer report at an unchanged coordinate is
    coalesced by libinput and never registers as a click; a relative `BTN`
    report (zero motion) always processes — and clicks wherever the pointer
    currently is, so `moveabs` then a separate `click` lands correctly.
  - *`moveabs` nudges one unit first.* The absolute device coalesces a report
    whose coordinates equal its previous one, so re-sending the same position
    after a relative move was a no-op (the cursor wouldn't snap back). Sending a
    one-unit-off report then the exact target forces a real move.
- **The KVM daemon (`serve`/`stop`) is implemented and hardware-verified.**
  `ch9329 serve` owns the UART and re-exposes the protocol over a localhost
  WebSocket (`GET /hid`) plus a `POST /send` one-shot endpoint, publishing the
  `/tmp/paniolo-<uid>/hid/daemon.json` discovery file paniolo's `console` reads
  — so the web-console "Capture input" KVM works with the Openterface. While a
  daemon runs, one-shot `paniolo hid send` invocations route through it
  automatically, so the CLI and the browser never contend for the UART and
  their injections intermix. (The UART is driven by the blocking `serialport`
  path on a dedicated thread bridged to the async server — tokio-serial's async
  reads are unreliable on a macOS tty.)
- **Held state (`down`/`up`/`mdown`/drag) persists across commands through the
  daemon**, because its one long-lived session carries the report — verified by
  holding `LEFT_SHIFT` across three separate CLI invocations and getting
  uppercase output. *Without* a daemon, a direct one-shot `ch9329 down A` resets
  per process (the CH9329 has no "read current report" command), so held state
  is per-invocation there; `combo` and `run` sequences still compose within one
  process.
- **Baud changing is implemented and hardware-verified** via the `baud`
  command (the `SET_PARA_CFG` flash-and-reset procedure, `docs/ch9329-spec.md`
  §5) — round-tripped 115200 → 9600 → 115200 on real hardware, the change
  surviving a fresh process (it's persisted to flash). It is *persistent*, not
  the protocol's transient renegotiation, so it is not advertised as a `baud`
  capability and the daemon never auto-invokes it.

## License

Apache 2.0 — see the repository [LICENSE](../LICENSE).
