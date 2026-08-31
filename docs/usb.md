# Switchable USB media

Some KVM-over-USB devices carry a USB mux: one physical device — a microSD
card, or whatever is plugged into a switchable USB-A port — routed to either
the control host or the target, but **never both at once**. paniolo drives that
mux through a generic per-target **usb channel**.

The point of it is hands-free *physical* media. Write an image to the card on
your control host, hand it to the target, and the target sees an ordinary USB
mass-storage device — visible to firmware and boot menus, which streamed
virtual media generally is not. No hands on the bench, no stick to swap.

---

## Supported hardware

| Device | Switched thing | Status |
|---|---|---|
| **Openterface KVM-Go** | onboard microSD reader | ✅ works, hardware-verified |
| **Openterface Mini-KVM** | switchable USB-A port | ⚠️ mechanism known, no helper support yet |

The KVM-Go's mux is driven by its CH32V208 over the same serial port the
`ch9329` helper already uses for keyboard and mouse, so a device you have
already wired up as a `hid` channel needs no new hardware — just a second
channel pointing at the same helper.

The Mini-KVM's mux is reachable a different way (a register write over the
capture chip's HID configuration interface, not the serial port). That path is
documented in
[the mux spec](https://github.com/curtisgalloway/paniolo/blob/main/notes/openterface-usb-mux-spec.md)
but no shipped helper implements it yet, so `paniolo usb` cannot drive a
Mini-KVM today.

## Wiring it up

```bash
paniolo usb set -t <target> --cmd "ch9329 -d /dev/cu.usbmodemXXXXX"
```

On a KVM-Go this is the same device path as the target's `hid` channel. Setting
both is normal and correct:

```bash
paniolo hid set -t pi5 --cmd "ch9329 -d /dev/cu.usbmodem51201"
paniolo usb set -t pi5 --cmd "ch9329 -d /dev/cu.usbmodem51201"
```

## Using it

```bash
paniolo usb state          -t pi5    # -> host | target
paniolo usb attach-host    -t pi5    # route the card to the control host
paniolo usb attach-target  -t pi5    # hand it to the target
```

A typical hands-free media hand-off:

```bash
paniolo usb attach-host -t pi5
# ... wait for the block device, write your image to it, unmount ...
paniolo usb attach-target -t pi5
paniolo power cycle -t pi5
```

---

## Four things that will bite you

**Unmount before you switch.** Switching physically detaches the device from
the side that currently has it. If a filesystem is still mounted there, that is
a surprise removal — the same as yanking a stick out mid-write. paniolo cannot
see mount state on either side, so it will not stop you. On macOS, plain
`diskutil unmount` frequently fails because Spotlight has the freshly written
volume open; use `diskutil unmount force`.

**Success means the mux moved, not that the media is ready.** `attach-host` and
`attach-target` return once the device confirms the new mux position. The USB
mass-storage device on the receiving side still has to enumerate, which takes a
few seconds. Wait for the block device to appear — do not assume it is there
because the command exited zero.

**Never assume the position persisted.** The mux resets to the host side
whenever the KVM loses power, and on hardware with a physical switch button
someone can move it by hand. Ask with `paniolo usb state` rather than
remembering what you last set.

**Not every device has a mux.** On hardware that does not implement switching,
the underlying protocol has no "unsupported" error — the device simply does not
answer. That surfaces as a timeout, and the helper says so explicitly rather
than leaving you to guess whether the device is broken.

---

## The helper contract

Like the [power hooks](power.md) and the [hid channel](hid.md), paniolo does not
talk to mux hardware itself. It runs the configured command with a verb
appended:

| paniolo command | what it runs |
|---|---|
| `paniolo usb attach-host` | `<cmd> usb host` |
| `paniolo usb attach-target` | `<cmd> usb target` |
| `paniolo usb state` | `<cmd> usb state` |

A helper implementing this contract must:

- exit non-zero on failure, and print the resulting side (`host` or `target`) on
  stdout;
- **verify** a switch rather than trusting it. On the KVM-Go the device's reply
  reports the resulting position rather than a success code, so a unit that
  ignored the request still answers with a well-formed frame stating the old
  position. The helper compares the two.

Unlike `paniolo hid send`, arguments are **not** passed through — the vocabulary
is fixed at three verbs. That keeps the surface a constrained or remote control
host is asked to expose as small as possible.
