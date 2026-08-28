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

On Linux, `video devices` hides SoC-internal video nodes (e.g. a Raspberry
Pi's `pispbe-*` pipeline stages and HEVC decoder, which otherwise flood the
list) — only external capture devices are shown. `paniolo helper hdmicap
devices --all` lists everything, and an explicitly configured internal device
still resolves.

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
[config-redesign.md](https://github.com/curtisgalloway/paniolo/blob/main/docs/config-redesign.md)); `paniolo configure` proposes the
stable id (with the human name as a comment) when one non-built-in capture
device is present, and lists id alternatives when there are several.

---

## Starting and stopping the daemon

```bash
paniolo video watch [target-machine]   # start hdmicap daemon for a target
paniolo video watch --restart          # force-restart a running (stalled) daemon
paniolo video stop  [target-machine]   # stop it (on the target's host)
paniolo video show  [target-machine]   # show daemon URL and status
```

`watch` starts `hdmicap daemon` detached and polls for startup. The daemon URL
is printed — open it in a browser for the live preview.

After an upgrade or rebuild, a daemon still running the old binary is flagged
**stale** by `paniolo video show` and `paniolo daemons`; `watch` auto-restarts a
stale daemon (no `--restart` needed), or restart it explicitly with
`paniolo daemons restart hdmicap` (see [architecture](architecture.md)).

---

## Capturing frames

```bash
paniolo video shot [target-machine] -o out.png   # save a screenshot (PNG)
paniolo video shot [target-machine]              # PNG to stdout (default -o -)
paniolo video shot --stable -o out.png           # wait for a steady frame first
paniolo video shot --changed-since <hex-hash> --timeout 10000 -o out.png
                                                 # block until the frame differs
paniolo video preview [target-machine]           # print the live-dashboard URL (optional target, like `show`)
```

`shot` fetches a single PNG-encoded frame from the running daemon and prints
`signal=… hash=…` to stderr; feed that hash to a later `--changed-since` to
wait for the screen to change.

`-o <path>` always means the **invoking machine's** filesystem, including when
the target's video channel lives on a remote control host: the remote shot
streams over SSH and the PNG is written locally (a failed capture removes the
stub file). No copy-back step needed.

---

## OCR

```bash
paniolo video read [target-machine]            # OCR the current frame, text to stdout
paniolo video read --stable [--timeout <ms>]   # wait for a steady frame first
```

`read` wraps the running daemon's `GET /ocr` endpoint (also reachable directly
— `curl -s "$(paniolo video preview)/ocr"` — and via the OCR button on the
[web dashboard](dashboard.md)).

**Two things `signal` used to get wrong**, both fixed and both worth knowing
about if you read older captures. A *mostly black* screen — which is what every
firmware, bootloader and console screen is — was classified as no-signal,
because the sampling lattice was coarse enough to land on none of the text: a
Gigaboot screen with 1.35% of its pixels lit reported `no_signal` for minutes.
And a *stalled* capture kept reporting `stable`: the capture loop publishes only
on success, so the last frame stayed in place with its old label, and a machine
whose mains had been cut went on reporting `stable` on its pre-cut desktop. A
frame older than `STALE_AFTER` now reports `stale`, and `/snapshot`, `/ocr` and
`--stable` all refuse it.

The lesson for anything collecting frames: **treat `signal` as a hint, save
every frame, and de-duplicate by hash afterwards** rather than filtering on the
label as you go.

When the capture has **no video signal** (the
target's display is off or unplugged) `read` errors with `no video signal`
instead of returning empty text, so "display is off" and "screen is blank"
stay distinguishable. OCR is on-device on both platforms — no network, no
model download: Apple Vision's `VNRecognizeTextRequest` on macOS, Tesseract
on Linux.

`paniolo setup` installs the platform's helper into the private libexec dir:
on macOS it compiles `ocr/visionocr.swift` with `swiftc`
(`~/.local/libexec/paniolo/bin/visionocr`); on Linux it installs `linuxocr`,
a Python 3 script that shells out to Tesseract (`apt-get install
tesseract-ocr`; Pillow is optional, for its upscale/pad preprocessing). The
hdmicap daemon finds the helper there (or via `PANIOLO_VISIONOCR`, on both
platforms) and shells out to it per request.

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
| Video config | the target's `video` channel in the lab file (`~/.config/paniolo/lab.toml`) |
| hdmicap discovery | `/tmp/paniolo-<uid>/hdmicap/<target>/daemon.json` (`{pid, port}`) |
| hdmicap advisory lock | `/tmp/paniolo-<uid>/hdmicap/<target>/daemon.lock` |
| hdmicap stderr log | `/tmp/paniolo-<uid>/hdmicap/<target>/daemon.log` (truncated on each start; shown on start timeout) |

The hdmicap daemon is **per target** (the `<target>` segment), so multiple
targets capture concurrently on one host; the runtime base honors
`$PANIOLO_RUNTIME_BASE` (default `/tmp`).
