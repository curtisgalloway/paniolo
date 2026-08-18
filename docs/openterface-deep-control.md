<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
-->

# Openterface Mini-KVM deep control — findings & testing TODO

*Status: **partially bench-verified 2026-08-18.** DTR→`SW_GND` characterized on
hardware; EEPROM dumped; the MS2109 GPIO path is blocked. RTS→CH9329 and
`DATAFLIP` remain untested. Tracker: requirements §6.1 (OTF-1…6).*

The Openterface Mini-KVM's hardware is open source
([TechxArtisanStudio/Openterface_Mini-KVM_Hardware](https://github.com/TechxArtisanStudio/Openterface_Mini-KVM_Hardware),
CERN-OHL-S v2). Reading the v1.9 schematic, BOM, and datasheet shows the
device is considerably more automatable than "capture + HID": the switchable
USB-A port and the serial chip's modem lines form a small programmable USB
fixture. This doc records the findings and the bench tests needed before
paniolo grows verbs for them.

**Source discipline:** everything below comes from the open *hardware* design
files, the vendor datasheet, the vendor's published product documentation, or
our own bench probing. The Openterface host applications were not read
(copyleft).

**Measurement caveat — read before trusting any USB observation.** The lab
host (`dev`) is a QEMU/KVM VM, and the Openterface reaches it by *per-device*
USB passthrough (`qm config 102`: `host=1-1.1.2.1/.2/.3`). Passthrough puts
each forwarded device on its own emulated root port, so **the guest cannot see
the real topology** — from inside the VM the unit looks like three independent
host connections with no hub, which is an artifact. Passthrough binds by
physical port path, so genuine device changes *are* visible to the guest, but
topology, enumeration bursts, and bus-level errors are not. Ground truth is
`journalctl -k` on the hypervisor; **`dmesg` on proxmox is unusable** —
`usbhid-ups` floods it and the ring buffer holds only ~15 minutes. Guest
`dmesg` timestamps also lag `CLOCK_MONOTONIC` by ~13 s, so attribute events by
before/after counting rather than by timestamp comparison.

## Architecture (v1.9 schematic) — confirmed on hardware

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

Confirmed 2026-08-18 from the hypervisor: hub `1a40:0101` (Terminus
Technology — the SL2.1s) enumerates at `1-1.1.2`, with the MS2109 on its
port 1, the CH340 on port 2, and the switchable A-port device on port 3 — one
host cable feeding one hub, exactly as drawn.

Both VBUS rails power it through CH213K current limiters — it survives
target power cycles while the host side is connected. The target always
enumerates an SL2.1s hub in front of the CH9329 (and the A-port when
switched over).

## Findings

1. **Software-switchable USB-A port — the switch is *monitored*, not a
   control.** The A-port routes through an FSUSB42MUX between the two hubs.
   Per the vendor's
   [USB switch documentation](https://docs.openterface.com/products/minikvm/usb-switch/):
   *"The Hardware Switch, despite being physical, is monitored by software and
   does not directly control the circuit direction. Instead, the software
   interprets the switch position and manages the actual circuit switching."*
   The device **defaults to the host connection at startup**, and the software
   switch takes priority over the physical toggle (the vendor documents a
   four-state sync/out-of-sync table; inward = host, outward = target).

   Bench-confirmed 2026-08-18: with no Openterface host application running,
   moving the slide switch between "H" and "T" produced **no** electrical
   change — the A-port device stayed on the host in both positions, with zero
   USB events over 15 s of polling. This corrects the earlier schematic-derived
   reading that the mux `Sel` node is *shared* between the switch and MS2109
   GPIO: the MS2109 drives `Sel`, and the switch is an input it senses.

   Use case unchanged: hands-free physical media (image a real stick
   host-side, present it to the target — visible to BIOS, unlike streamed
   virtual media).

2. **CH340 `DTR#` → net `USB_CTRL` → MOSFET Q11 → `SW_GND`** — the A-port
   connector's ground return (p1/p3). **Confirmed on hardware**, with polarity:

   | DTR | A-port device |
   |---|---|
   | asserted (1) | disconnected |
   | deasserted (0) | present |

   Four deliberate transitions produced four matching kernel events, and eight
   further pulses behaved identically. Consequences:

   - **Merely opening the control tty unplugs the A-port device**, because the
     kernel asserts DTR (and RTS) at open; closing it plugs the device back in
     via `HUPCL`. Any `paniolo hid` invocation would do this. This is the
     load-bearing constraint for OTF-2/3/4: the replug and reset verbs must go
     through the existing session's owner thread, never a second `open()`.
   - **Replugged devices usually come back degraded.** The hypervisor
     negotiated *full-speed* (12 Mbps) on most ground-float replugs of a stick
     that first enumerated at high-speed (480 Mbps), with the occasional
     high-speed result — nondeterministic. A physical unplug/replug restores
     high-speed. Real bus behavior, verified host-side, not an emulation
     artifact.
   - **A pulse is not one clean replug.** The host log shows a *burst* of
     re-enumerations per pulse (four in four seconds in one case), which the
     guest coalesces into a single event.
   - **Repeated cycles destabilize the shared hub.** After ~9 cycles the
     hypervisor logged `device not accepting address, error -71` and
     `device descriptor read/64, error -71` storms on the A-port device, and
     the *sibling* CH340 on the same hub simultaneously began failing control
     transfers in the guest (`failed to send control message: -71`, then `EIO`
     on the tty). Neither USB re-authorize nor a root-port disable cycle
     recovered it; a physical replug restored everything with no lasting
     damage. Mechanism unconfirmed — plausibly hub-level disruption from the
     brownout storm — but the practical point stands: **this primitive can
     take down the control channel paniolo needs for HID.**

3. **CH340 `RTS#` → net `HIDRESET` → CH9329 reset/config pin** (p3). A
   wedged HID chip should be recoverable in software — the natural watchdog
   action for a CH9329-based `hid` backend. **Still untested**: RTS sat
   asserted throughout the 2026-08-18 session and was never toggled. Note the
   kernel asserts RTS at open, and the shipped backend works, so asserted-RTS
   is evidently not a *held* reset; whether a transition triggers one is
   unknown. (A third line, `DATAFLIP`, runs to the CH9329 `DEF`/`UP` pins —
   also untested. All four CH340 input lines read `0` in every sample taken,
   so if `DATAFLIP` is a status input on the CH340 side it never moved.)

4. **USB descriptors in a writable AT24C16 EEPROM** hanging off the MS2109's
   config I2C (p2, with a WP line). **Confirmed**: ms-tools reports an
   `EEPROM` region of exactly 2048 bytes (16 Kbit = AT24C16). Two independent
   dumps were byte-identical (`sha256 9b46336d…`). Layout: magic `a5 5a` at
   0x00; two length-prefixed ASCII string slots at 0x10 (`0c` + "Openterface")
   and 0x20 (`0d` + "OpenterfaceA"), 0xff-padded; 8051 code from 0x30.

   **Correction to the OTF-5 premise:** the unit reports a USB serial of
   `????????`, but that is **not** an unprogrammed EEPROM field. It is a
   fully-formed USB string descriptor synthesized into XDATA at `0xC676`
   (`12 03` followed by 8× `3f 00`), immediately preceding the device
   descriptor at `0xC688` — which cross-checks exactly against sysfs
   (`bDeviceClass ef`, `bMaxPacketSize0 40`, idVendor `534d`, idProduct
   `2109`, bcdDevice `2100`). Persistent serial stamping therefore requires
   patching the firmware that populates that region, not writing a data field
   — which puts OTF-5 behind the same wall as OTF-3 (finding 7).

5. **"Extension Pins" (datasheet ⑥, "for developer use")** expose a spare
   downstream port of *each* hub plus power (p1 `PROBE_USB`). The
   target-side one is a permanent slot for a gadget the target sees
   natively — e.g. a microcontroller mass-storage gadget (true virtual
   media) or a tap for a protocol analyzer.

6. **HDMI-embedded audio** is forwarded as a USB audio device (datasheet).
   **Confirmed**: the MS2109 exposes interfaces 2 and 3 as Audio class bound to
   `snd-usb-audio`, alongside the two Video interfaces and the vendor HID
   config interface (interface 4).

7. **The MS2109 config interface is reachable, but not patchable.**
   [BertoldVdb/ms-tools](https://github.com/BertoldVdb/ms-tools) (MIT) speaks
   the chip's factory HID interface via hidraw feature reports. Build it with
   `-tags puregohid` to avoid the cgo/hidapi dependency; in that mode
   `list-dev` is unsupported and `--raw-path=/dev/hidrawN` is mandatory. It
   reads fine — `list-regions` returns `RAM 65536`, `IRAM 256`, `EEPROM 2048`,
   `USERCONFIG 48 @ RAM.CBD0`, `USERRAM 8192 @ RAM.C000`, and region reads
   work with `--no-patch`.

   **But GPIO access requires uploading code into the running 8051, and that
   fails on this unit.** `gpio-get` under `--no-patch` returns "not supported
   in this mode"; with patching enabled it reports `Could not patch code`.
   Verbose logging shows the chip detected, a 14-byte patch allocated, and the
   writes genuinely landing (`ROMOut`/`ROMIn` readbacks match) — but the
   handshake byte at `0xCBD4` never changes from `0b`, so the injected code
   never executes. Expected, given the EEPROM carries vendor 8051 code rather
   than a stock dongle image. **OTF-3 has no path through ms-tools as it
   stands.** The remaining clean-room option is to capture the vendor
   application's USB traffic while toggling its software switch and derive the
   command from observed bytes.

## Testing TODO (OTF-1)

Done 2026-08-18:

- [x] Identify the unit's PCB revision — **v1.9**.
- [x] `TIOCMSET` DTR: polarity, disconnect/re-enumerate, hold time. Asserted =
      disconnected. Holds ≥500 ms reliably produce a replug (floor below
      500 ms not probed; low value given finding 2's instability).
- [x] Serial-open side effects — **confirmed**: opening the tty replugs the
      A-port device, closing restores it.
- [x] Dump the AT24C16 before any write; archive per-unit backups.
- [x] Does the switch override, or gate, the GPIO? **Neither** — see finding 1.

Still open:

- [ ] `TIOCMSET` RTS: confirm it resets the CH9329 (chip re-announces on
      serial; keyboard re-enumerates target-side) and that it does NOT
      disturb the A-port or video.
- [ ] Map `DATAFLIP` (DSR/CTS side): status input from the CH9329, or a
      default-restore strap? Document.
- [ ] Find the MS2109 GPIO write that drives the mux `Sel` — blocked on
      finding 7; needs a USB capture of the vendor app, or vendor register
      documentation.
- [ ] Measure mux-flip behavior once a flip is possible: does the previously
      attached side see a clean disconnect? Settle time?
- [ ] Video-disturbance check during mux flips — **not yet meaningful**: the
      current target outputs a black HDMI signal, so before/after frame
      comparison proves nothing. Needs real content on screen.
- [ ] Re-run hot-plug characterization with the unit attached to a
      non-virtualized host, to remove the passthrough layer entirely.
- [ ] Root-cause the failed-enumeration storm in finding 2 before OTF-4 ships.

## Integration sketch

- The CH9329 `hid` backend (the `ch9329/` helper crate,
  [clean-room spec](ch9329-spec.md)) has shipped; add RTS hardware reset as
  its recovery verb (OTF-2) — after confirming the reset actually works, and
  routing it through the existing session rather than a second `open()`.
- `usb attach-host` / `usb attach-target` (OTF-3) is **blocked** on a way to
  drive the MS2109 GPIO.
- `usb replug [--hold-ms]` (OTF-4) works mechanically but is **not
  production-ready**: degraded link speed, re-enumeration bursts, and
  hub-level instability that can take the CH340 with it.
- EEPROM serial-stamping utility (OTF-5) — blocked with OTF-3.
- Extension-pins gadget slot exploration (OTF-6, deferred).
