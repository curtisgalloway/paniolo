# Video capture

paniolo drives `hdmicap`, a Rust warm-stream daemon that keeps a USB HDMI
capture device open continuously and serves the current frame over HTTP. This
avoids the multi-second reopen latency you'd get by running ffmpeg per capture.

---

## Hardware

Any USB HDMI capture card that presents as a UVC device (V4L2 / AVFoundation).
Tested with the MS2109-based cards (e.g. generic "USB3.0 HDMI Capture" dongles).

Connect the target's HDMI output to the capture card, then the card to the Mac.

---

## Setup

```bash
# Detect available capture devices (each line ends with its stable id)
paniolo video devices

# Configure the target's video channel — prefer the stable id
paniolo video set -t target-machine --device "0x8300000534d2109"
```

The `--device` value may be:

- a **stable id** (preferred): the AVFoundation `uniqueID` on macOS, the
  `/dev/v4l/by-path/...` symlink on Linux. Both are derived from USB port
  topology — they survive reboots and enumeration-order shifts, distinguish
  two identical dongles, and change only if the dongle moves to a different
  physical port.
- a **name substring** (e.g. `"USB Video"`): convenient, but identical dongles
  share a name. A substring matching more than one device is an error that
  lists the candidates' ids — never a silent first-match guess.
- a **`/dev/video*` path** (Linux): accepted, but not stable across reboots.

The device lives on the target's `video` channel in the lab file (see
[config-redesign.md](config-redesign.md)); `paniolo configure` proposes the
stable id (with the human name as a comment) when one non-built-in capture
device is present, and lists id alternatives when there are several.

---

## Starting and stopping the daemon

```bash
paniolo video watch [target-machine]   # start hdmicap daemon for a target
paniolo video stop  [target-machine]   # stop it
paniolo video show  [target-machine]   # show daemon URL and status
```

`watch` starts `hdmicap daemon` detached and polls for startup. The daemon URL
is printed — open it in a browser for the live preview.

---

## Capturing frames

```bash
paniolo video shot [target-machine]           # save a screenshot to a temp file
paniolo video shot [target-machine] -o out.png  # save to a specific path
paniolo video preview [target-machine]        # open the live MJPEG stream in a browser
```

`shot` fetches a single PNG-encoded frame from the running daemon via
`hdmicap shot`.

---

## OCR (Apple Vision)

```bash
paniolo video read [target-machine]
```

Fetches the current frame and runs Apple Vision's `VNRecognizeTextRequest` on
it, printing the recognized text to stdout. On-device — no network, no model
download required.

`paniolo setup` compiles `ocr/visionocr.swift` into `~/.cargo/bin/visionocr`
with `swiftc`. The OCR tool is also accessible from the [web dashboard](dashboard.md)
via the OCR button.

**OCR tuning notes:**
- `.fast` recognition level is used (not `.accurate` — the latter misses small
  console text entirely; it's tuned for natural document text).
- The frame is 2×-upscaled and black-padded before recognition to improve
  accuracy on thin console fonts.
- `minimumTextHeight` is lowered from the default to catch small terminal text.

---

## Runtime paths

| Purpose | Path |
|---|---|
| Video config | `~/.config/paniolo/video.toml` |
| hdmicap discovery | `$TMPDIR/hdmicap/daemon.json` (`{pid, port}`) |
| hdmicap advisory lock | `$TMPDIR/hdmicap/daemon.lock` |
