<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
-->

# Openterface Mini-KVM deep control — findings & testing TODO

*Status: findings from published design files; **nothing bench-verified yet.**
Tracker: requirements §6.1 (OTF-1…6). 2026-08-16.*

The Openterface Mini-KVM's hardware is open source
([TechxArtisanStudio/Openterface_Mini-KVM_Hardware](https://github.com/TechxArtisanStudio/Openterface_Mini-KVM_Hardware),
CERN-OHL-S v2). Reading the v1.9 schematic, BOM, and datasheet shows the
device is considerably more automatable than "capture + HID": the switchable
USB-A port and the serial chip's modem lines form a small programmable USB
fixture. This doc records the findings and the bench tests needed before
paniolo grows verbs for them.

**Source discipline:** everything below comes from the open *hardware* design
files and the vendor datasheet. The Openterface host applications were not
read (copyleft); any protocol facts not derivable from the schematic must
come from the vendor's published docs or our own bench probing.

## Architecture (v1.9 schematic)

Two SL2.1s USB 2.0 hubs, one per side:

```
Host USB-C ── HUB1 ──┬─ MS2109  (HDMI→UVC video + HDMI audio; vendor cfg interface)
                     ├─ CH340C  (USB-serial → CH9329; modem lines repurposed)
                     ├─ FSUSB42MUX side A ─┐
                     └─ extension pads      │   USB-A female
                                            ├── (switchable port)
Target USB-C ─ HUB2 ──┬─ CH9329 (HID)      │
                      ├─ FSUSB42MUX side B ─┘
                      └─ extension pads
```

Both VBUS rails power it through CH213K current limiters — it survives
target power cycles while the host side is connected. The target always
enumerates an SL2.1s hub in front of the CH9329 (and the A-port when
switched over).

## Findings

1. **Software-switchable USB-A port.** The A-port routes through an
   FSUSB42MUX between the two hubs. The mux `Sel` node is shared between the
   physical slide switch and MS2109 GPIO pins (`GPIO0`/`SPDIFOUT`,
   schematic p2 `USB_SWITCH` block) — the host can plausibly flip it via the
   MS2109's vendor configuration interface. Use case: hands-free physical
   media (image a real stick host-side, present it to the target — visible
   to BIOS, unlike streamed virtual media).

2. **CH340 `DTR#` → net `USB_CTRL` → MOSFET Q11 → `SW_GND`** — the A-port
   connector's ground return (p1/p3). Toggling DTR floats/restores the
   plugged device's ground: a **software surprise-unplug/replug** of a real
   USB device. This is a hot-plug exerciser primitive for USB driver
   testing, on a stock unit.

3. **CH340 `RTS#` → net `HIDRESET` → CH9329 reset/config pin** (p3). A
   wedged HID chip is recoverable in software — the natural watchdog action
   for a CH9329-based `hid` backend. (A third line, `DATAFLIP`, runs to the
   CH9329 `DEF`/`UP` pins — semantics to map on the bench.)

4. **USB descriptors in a writable AT24C16 EEPROM** hanging off the MS2109's
   config I2C (p2, with a WP line). Per-unit serial strings are possible →
   stable `/dev/v4l/by-id` paths when a lab runs several units. (The CH340C
   has no serial number; its ambiguity remains — disambiguate by physical
   port path.)

5. **"Extension Pins" (datasheet ⑥, "for developer use")** expose a spare
   downstream port of *each* hub plus power (p1 `PROBE_USB`). The
   target-side one is a permanent slot for a gadget the target sees
   natively — e.g. a microcontroller mass-storage gadget (true virtual
   media) or a tap for a protocol analyzer.

6. **HDMI-embedded audio** is forwarded as a USB audio device (datasheet) —
   a future automated audio-output verification channel, already present in
   the capture hardware.

## Testing TODO (OTF-1 — do this before any code)

On the bench, with a sacrificial USB stick in the A-port and a scope/serial
console as needed:

- [ ] Identify our unit's PCB revision (v1.6 vs v1.9 — silk/sticker). The
      software-switch routing is confirmed from the v1.9 schematic only.
- [ ] Find the MS2109 GPIO write that drives the mux `Sel`
      (vendor-published protocol docs or generic MS2109 register tooling);
      confirm flip works with the slide switch in each position (does the
      switch override, or gate, the GPIO?).
- [ ] Measure mux-flip behavior: does the previously-attached side see a
      clean disconnect? Settle time before the new side enumerates?
- [ ] `TIOCMSET` DTR on the CH340: confirm assertion polarity, that the
      A-port device disconnects/re-enumerates, and minimum reliable
      hold time (`--hold-ms` default).
- [ ] `TIOCMSET` RTS: confirm it resets the CH9329 (chip re-announces on
      serial; keyboard re-enumerates target-side) and that it does NOT
      disturb the A-port or video.
- [ ] Map `DATAFLIP` (DSR/CTS side): status input from the CH9329, or a
      default-restore strap? Document.
- [ ] Dump the AT24C16 through the MS2109 path before any write; archive
      per-unit backups.
- [ ] Note interactions: video glitches during mux flips? CH340 open/close
      side effects on DTR/RTS state (classic serial-open DTR pulse — does
      merely opening the port replug the A-device?).

That last item is load-bearing: if opening the serial port pulses DTR, every
`paniolo hid` invocation would hot-unplug the A-port device unless the tty is
opened with modem lines held.

## Integration sketch (after OTF-1)

- The CH9329 `hid` backend (the `ch9329/` helper crate,
  [clean-room spec](ch9329-spec.md)) has shipped; add RTS hardware reset as
  its recovery verb (OTF-2).
- New channel verbs: `usb attach-host` / `usb attach-target` (OTF-3) and
  `usb replug [--hold-ms]` (OTF-4) on the same serial + MS2109 handles.
- EEPROM serial-stamping utility (OTF-5).
- Extension-pins gadget slot exploration (OTF-6, deferred).
