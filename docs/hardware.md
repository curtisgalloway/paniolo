# Tested hardware

The bench hardware paniolo has been verified with, grouped by subsystem. Links go to the
exact items purchased (Amazon listings are just for reference and don't imply an
endorsement). Equivalents that meet the same contract — UVC for video, FTDI for
serial/DTR, CC2652 for Zigbee — should generally be expected to work; each subsystem
guide states the actual compatibility requirement.

Note that this is not any kind of guarantee that the hardware or implementation is
bug-free. Use at your own risk; your mileage may vary. Testing is best-effort.

## Power control

See the [power guide](power.md).

| Device | Role |
|---|---|
| [Sonoff Zigbee 3.0 USB Dongle Plus (ZBDongle-P, CC2652P)](https://www.amazon.com/dp/B09KXTCMSC) | Zigbee coordinator for the `zigplug` helper. |
| [ThirdReality Zigbee Smart Plug (15 A, energy monitoring)](https://www.amazon.com/dp/B0BPY5D1KC) | Switched mains outlet for target power, driven by `zigplug` through the generic power hooks. |
| [AINOPE USB 3.0 extension cable (6.6 ft)](https://www.amazon.com/dp/B07RQRMGKB) | Distances the Zigbee dongle from USB 3 devices. RF noise from USB 3 hardware (especially video capture) can break Zigbee network formation and joining; an extension cable is the fix. |
| Cambrionix programmable USB hub | Per-port USB power switching via the [`cambrionix` helper](power.md#cambrionix-hub-control) (control UART, 115200 8N1). |

## Serial console

See the [serial guide](serial.md).

| Device | Role |
|---|---|
| [DSD TECH SH-U09C5 USB-to-TTL cable (FTDI, 1.8/2.5/3.3/5 V selectable)](https://www.amazon.com/dp/B07WX2DSVB) | TTL UART to a bare GPIO header. Being FTDI, its DTR line can also drive the Pi 5 J2 power button — see [DTR power control](power.md#dtr-power-control-ftdi-j2-wiring). |
| [Waveshare Industrial USB-to-TTL (D), FT232RNL](https://www.amazon.com/dp/B0CX5C5KR4) | Pi 5 debug (UART) connector — ships with both an SH1.0 3-pin plug and a separate 4-pin header. |

## HID injection

See the [HID guide](hid.md) and [`hidrig/README.md`](../hidrig/README.md).

| Device | Role |
|---|---|
| Adafruit KB2040 | Reference HID injector (CircuitPython firmware in [`hidrig/`](../hidrig/README.md)); any CircuitPython-capable RP2040 board with free UART pins works with minor pin edits. |
| 3.3 V USB-serial adapter (FTDI) | Control UART to the KB2040 — e.g. the DSD TECH cable above. The `hidrig` daemon applies its macOS low-latency fix when opening FTDI adapters, so prefer FTDI here. |

## Video capture

See the [video guide](video.md).

| Device | Role |
|---|---|
| [Generic 4K HDMI capture dongle (MS2109-class, UVC)](https://www.amazon.com/dp/B09FLN63B3) | Target HDMI out → `hdmicap` warm stream + OCR. Any UVC capture card works; MS2109-class dongles are the tested baseline. |
| [IPEVO V4K 8 MP USB document camera (UVC)](https://www.amazon.com/dp/B079DLTG9F) | UVC camera source used to verify `hdmicap` against non-capture-card devices; also handy for watching the physical bench. |

## Netboot link

See the [netboot](netboot.md) and [link mode](netif.md) guides.

| Device | Role |
|---|---|
| [TP-Link UE330 — 3-port USB 3.0 hub + Gigabit Ethernet](https://www.amazon.com/dp/B01N9M32TA) | The direct host↔target Ethernet link, with spare USB ports for the rest of the rig. |
| [Anker USB-C to Gigabit Ethernet adapter](https://www.amazon.com/dp/B08CK9X9Z8) | Dedicated USB-C network adapter for the direct link. |

______________________________________________________________________

*When you verify paniolo against new hardware, add it here under the matching subsystem
and note anything an equivalent device must provide.*
