<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# Openterface KVM-Go — architecture and paniolo support

*Status: **bench-verified 2026-08-24.** Both paniolo channels work with no code
changes. Sibling of the
[Mini-KVM deep-control notes](openterface-deep-control.md), which this device
supersedes in several important ways.*

The [Openterface KVM-Go](https://openterface.com/product/kvm-go/) is the
successor to the Openterface Mini-KVM: a keychain-sized KVM-over-USB with a
built-in male video connector (HDMI, DisplayPort, or VGA — this doc covers the
HDMI variant). Hardware design files are published under CERN-OHL-S v2 at
[TechxArtisanStudio/Openterface_KVM-GO_Hardware](https://github.com/TechxArtisanStudio/Openterface_KVM-GO_Hardware).

**Source discipline** (unchanged from the Mini-KVM notes): everything below
comes from the open *hardware* design files, the vendor datasheet, the vendor's
published documentation, or our own bench probing and USB captures. The
Openterface host applications were **not** read — they are AGPL, and paniolo
must not derive its implementation from them.

## Bottom line for paniolo

| Channel | Helper | Status |
|---|---|---|
| `video` | `hdmicap` (AVFoundation / UVC) | ✅ works unmodified |
| `hid` | `ch9329` | ✅ works unmodified |
| microSD | — | enumerates; mux control not yet driven by paniolo |

Wiring it into a target needs nothing new:

```bash
paniolo video set -t <target> --device "<uniqueID from hdmicap devices>"
paniolo hid   set -t <target> --cmd "ch9329 -d /dev/cu.usbmodemXXXXX"
```

## Architecture

Two boards joined by headers H1/H2 — `KVM_Go_HDMI` (video) and `KVM_Go_KM`
(keyboard/mouse + storage):

```
Host USB-C ─┬─ HUB1 (SL2.1s) ─┬─ CH32V208  USB1 → CDC-ACM control (1a86:fe0c)
            │                 └─ FSUSB42MUX side A ─┐
            └─ MS2130S ─── SuperSpeed direct to host │  (UVC video + UAC audio)
                                                     ├── GL823K ── microSD
Target USB-C ── HUB2 (SL2.1s) ─┬─ CH32V208 USB2 → HID "KeyMod" (1a86:fe00)
                               └─ FSUSB42MUX side B ─┘
```

USB identities observed on the bench:

| Role | Product string | VID:PID | Where |
|---|---|---|---|
| Host control | `USB Serial` (CDC-ACM) | `1a86:fe0c` | behind HUB1 |
| Host video | `Openterface` (UVC + UAC + HID cfg) | `345f:2132` | SuperSpeed, direct |
| Host storage | `USB Storage` (GL823K) | `05e3:0751` | behind HUB1, when mux'd host-side |
| **Target** | `KeyMod` (emulated kbd/mouse) | `1a86:fe00` | behind HUB2 |
| Both hubs | `USB2.0 HUB` (SL2.1s) | `1a40:0101` | — |

Key parts (from the published BOMs):

- **U10 `MS2130S`** (video board) — USB 3.x UVC capture, replacing the
  Mini-KVM's USB 2.0 MS2109. Config storage moved from an AT24C16 I²C EEPROM to
  a **W25Q16 SPI NOR** (U4).
- **U1 `CH32V208GBU6`** (KM board) — one RISC-V MCU does *both* the host CDC
  control interface and the target HID. Replaces the Mini-KVM's CH340C +
  CH9329 pair.
- **HUB1/HUB2 `SL2.1s`** — same hub as the Mini-KVM, one per side.
- **U4 `GL823K`** + `CARD1` — onboard microSD reader on an **`FSUSB42MUX`**
  (U5), switchable between host and target.
- Extras: **`DS18B20`** 1-Wire temperature sensor (U8), an RGB LED, and a
  **BLE antenna** (the CH32V208 has BLE 5.3 on-die; the vendor lists a future
  native iPadOS app). `SW1` is wired to `BOOT0` — ISP entry at reset, mux
  toggle at runtime.

## Differences from the Mini-KVM that matter

1. **No CH340, so no modem-line tricks.** The Mini-KVM's `DTR → SW_GND`
   (A-port disconnect) and `RTS → HIDRESET` findings — items 2 and 3 in the
   [deep-control notes](openterface-deep-control.md) — **do not apply here**.
   DTR/RTS are now a CDC class request (`SET_CONTROL_LINE_STATE`) that the
   firmware may act on or ignore. The upside: opening the control tty no longer
   yanks a USB device off the bus, which was the load-bearing constraint on the
   Mini-KVM.

2. **The USB mux select is on the MCU, not the video chip.** The KM schematic
   labels it plainly: `USB_SW` → FSUSB42 `Sel`, driven by the CH32V208.
   Also `SDPOWER_SW` → SY6280 enable (SD power) and `SD_STATE` ← GL823K
   activity. On the Mini-KVM the equivalent `Sel` hung off the MS2109 GPIO and
   was unreachable, which is what dead-ended **OTF-3**. Here it sits behind a
   serial port paniolo already talks to.

3. **The switched device is onboard** — a microSD reader rather than an
   external USB-A port. True hands-free virtual media with no stick to plug.
   The vendor documents the behaviour: host and target share the card but
   never simultaneously, it **defaults to the host at power-on**, and it can be
   switched by the physical button or by the host app.

4. **SD power is gated from *target* VBUS** (`TVBUS` → SY6280AAC, enabled by
   `SDPOWER_SW`). Bench-confirmed: the card reader only appears once the target
   side is powered.

## The CH9329 protocol works, with one difference

The CH32V208 speaks the CH9329 frame protocol, so the existing
[`ch9329`](../ch9329/README.md) helper drives it as-is:

```
$ ch9329 -d /dev/cu.usbmodem51201 info
chip_version=0x01 target_connected=true num_lock=false caps_lock=false scroll_lock=false baud=115200
```

`chip_version` reports **`0x01`**, not the real CH9329's `0x38` — the MCU
emulates the protocol rather than being the chip. `target_connected` is live
and accurate: it flips to `true` the moment the target enumerates the emulated
HID.

**Verified end-to-end into a Raspberry Pi 5** via the lock-LED round trip —
sending `CAPS_LOCK` flipped `caps_lock` `false` → `true` → `false`, proving the
keystroke reached the target's USB HID stack and the LED state came back over
the CDC link. `move`, `moveabs`, `key`, and `combo` all work.

## There is no proprietary initialization

This was verified directly rather than assumed. A Cynthion capture of the
vendor Linux app driving the device — 5.4 M packets covering enumeration,
app start, and a working picture — contains **zero vendor-type control
requests**:

```bash
tshark -r capture.pcap -Y "usb.bmRequestType.type == 2"   # → 0 hits
```

Everything the app does is standard USB: hub port class requests, MSC
`GET_MAX_LUN`, UVC control enumeration (`GET_INFO`/`GET_DEF`/`GET_CUR`/
`GET_MIN`/`GET_MAX`/`GET_RES`), UVC `VS_PROBE`/`VS_COMMIT`, HID `SET_IDLE`,
and CDC `SET_LINE_CODING`/`SET_CONTROL_LINE_STATE`.

The committed video format was:

```
bmHint=0x0000  bFormatIndex=1  bFrameIndex=1
dwFrameInterval=333333        → 30.000 fps
dwMaxVideoFrameSize=4147200   = 1920×1080×2
dwMaxPayloadTransferSize=16384
```

**Consequence:** the capture chip is a plain UVC device. paniolo needs no
reverse-engineered handshake, and the clean-room wall never has to be crossed
for the video path.

Incidental CDC observations from the same capture: the app sets
`SET_LINE_CODING` to 9600 8N1, later to 115200 8N1, and asserts DTR+RTS
(`SET_CONTROL_LINE_STATE wValue=0x0003`). Consistent with `ch9329`'s existing
115200-then-9600 autodetect; nothing to change.

## Operational notes

### A blank capture is usually a sleeping display

**Check this first.** A target whose display has blanked and a capture chip
with no HDMI input are indistinguishable downstream — both yield a flat field
(this device fills black on some links and uniform grey on others). Wake the
target over the `hid` channel *before* investigating the capture path:

```bash
ch9329 -d <dev> move 60 40 ; ch9329 -d <dev> move -60 -40
ch9329 -d <dev> key LEFT_SHIFT
```

During bring-up this cost most of a session and produced three separate wrong
diagnoses (a required vendor init, 4K mode selection, and a NV12-vs-YUY2
format mismatch) before a single mouse wiggle produced `signal=stable` and a
clean 1080p screenshot. `ch9329 info` reporting `target_connected=true`
confirms the wiggle will actually land.

### When the signal goes away, frames stop

Unlike the MS2109 — which streams black frames when idle — this device stops
delivering frames entirely when the HDMI source disappears (display sleeps,
cable pulled, device unplugged). `hdmicap`'s watchdog treats that as a fault:

```
capture stalled (4s with no new frames) — exiting for restart
no frames in 12s after device open — exiting for restart
```

The daemon then exits. Under paniolo supervision this becomes a restart loop
for as long as the target's screen is asleep. It is *correct* fault detection —
50 minutes of continuous clean streaming was observed once a signal was
present — but the no-signal state arguably wants to be a state rather than a
fatal stall for this device. **Not yet addressed.**

### USB 2.0 fallback

If the capture chip enumerates at High Speed rather than SuperSpeed, the
advertised mode set shrinks sharply — max 1920×1080, no 4K, no 120/144/240 fps,
no `yuvs` at 1080p — and it appears *behind the internal hub*
(`locationID 0x514…`) instead of on its own SuperSpeed root port
(`0x520…`). This is the MS2130S falling back to the USB 2.0 video lines that
run through header H2 to the host-side hub, exactly as the schematic describes.

Usual cause is a **USB 2.0-only USB-C cable**. It is also expected and
unavoidable when the device is routed through a Cynthion, which is USB 2.0
only. 1080p is fine for console work; use a USB 3 cable on a SuperSpeed port to
regain the full mode set.

### 4K is experimental

The vendor lists 4K as an experimental feature that "may generate additional
heat", and ships a documented default of **1080p60 for optimal stability**.
`hdmicap`'s `pick_format()` selects the highest pixel count available, so on a
SuperSpeed link it *will* choose 3840×2160. That has not been shown to cause a
problem, but it is an accidental default rather than a considered one, and
worth a deliberate decision before relying on it.

## Not yet done

- [ ] Drive the microSD mux from paniolo (`USB_SW` via the CH32V208). This is
      the Mini-KVM's blocked OTF-3 made reachable — the command is not in the
      vendor's public documentation, so it needs USB capture of the app
      toggling the switch, not a source read.
- [ ] Decide `hdmicap` format-selection policy (see "4K is experimental").
- [ ] Decide whether the no-signal stall should exit or become a reported
      state.
- [ ] Read the DS18B20 temperature sensor and RGB LED, if the MCU exposes them.
- [ ] Characterise the BLE interface (vendor lists a future iPadOS app).
- [ ] Confirm whether `SET_CONTROL_LINE_STATE` (DTR/RTS) has any hardware
      effect on this design, as it did on the Mini-KVM.
