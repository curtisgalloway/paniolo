# Video capture

paniolo captures the target's screen through `hdmicap`, a daemon that keeps a
USB HDMI capture device open and serves the current frame over HTTP, so there
is no per-capture reopen delay.

---

## Hardware

Any USB HDMI capture card that presents as a UVC device (standard webcam
protocol). Tested with MS2109-based cards (e.g. generic "USB3.0 HDMI Capture"
dongles). Connect the target's HDMI output to the card, and the card to the
control host.

---

## Setup

```bash
# Detect available capture devices (each line ends with its stable id)
paniolo video devices

# Configure the target's video channel — prefer the stable id
paniolo video set -t target-machine --device "0x8300000534d2109"
```

The `--device` value may be:

| Form | Notes |
|---|---|
| **Stable id** (preferred) | The AVFoundation `uniqueID` on macOS, the `/dev/v4l/by-path/...` symlink on Linux. Survives reboots, tells identical dongles apart, and changes only if the dongle moves to another USB port. |
| **Name substring** (e.g. `"USB Video"`) | A substring matching more than one device is an error listing the candidates' ids. |
| **`/dev/video*` path** (Linux) | Accepted, but not stable across reboots. |

On Linux, `video devices` hides SoC-internal video nodes (e.g. a Raspberry
Pi's `pispbe-*` stages). `paniolo helper hdmicap devices --all` lists
everything, and an internal device you configure explicitly still resolves.

The device is stored on the target's `video` channel in the lab file (see
[config-redesign.md](https://github.com/curtisgalloway/paniolo/blob/main/notes/config-redesign.md)).
`paniolo configure` proposes the stable id when it finds one capture device,
and lists the ids when it finds several.

---

## Starting and stopping the daemon

```bash
paniolo video watch [target-machine]   # start hdmicap daemon for a target
paniolo video watch --restart          # force-restart a running (stalled) daemon
paniolo video stop  [target-machine]   # stop it (on the target's host)
paniolo video show  [target-machine]   # show daemon address and status
```

`watch` starts `hdmicap daemon` detached and prints its address. To open the
dashboard, run `paniolo video preview`.

### The token and the dashboard URL

**Every request to the daemon needs its token**, from `token` in the
discovery file ([Runtime paths](#runtime-paths)), new on each start.
paniolo's commands (`shot`, `read`, `console`, …) send it automatically. By
hand, send `Authorization: Bearer <token>` or `?token=<token>`. The daemon
also requires a loopback `Host` and `Origin`. A daemon from a paniolo older
than the token has none; `paniolo daemons restart --stale` replaces it.

**`video preview` is the only command that prints the URL with the token.**
`show`, `console` and `watch` print `http://127.0.0.1:<port>` only, to keep
the credential out of logs and transcripts.

**If `console` cannot launch a browser** (headless host, no `xdg-open`), it
writes the full URL to a `0600` `dashboard-url.txt` in the target's runtime
dir and prints the **path**:

```bash
xdg-open "$(cat /tmp/paniolo-1000/hdmicap/target-machine/dashboard-url.txt)"
```

### Stopping safely

`video stop` (and `hdmicap stop`) uses the authenticated `POST /stop` and
never signals the discovery-file PID, which a crash could leave pointing at an
unrelated process. Stop a daemon too old for the endpoint with
`paniolo daemons stop hdmicap`, which checks process identity first.

### Stale and untracked daemons

**Stale.** A daemon running an old binary after an upgrade shows as **stale**
in `paniolo video show` and `paniolo daemons`. `watch` restarts it
automatically (no `--restart` needed), or run
`paniolo daemons restart hdmicap` (see [architecture](dev/architecture.md)).

**Untracked.** A daemon whose discovery file was deleted keeps running and
holding the device. On Linux, systemd's `/tmp` cleanup
(`q /tmp 1777 root root 10d` on Debian) removes the file after ten idle days.
A new `video watch` then dies on the advisory lock
(`another hdmicap daemon is already running`).

- `video show` reports `running, untracked (pid N)` instead of `stopped`.
- `paniolo daemons` lists it under **Untracked daemons**.
- `video watch` and `video stop` reap it (`SIGTERM`, then `SIGKILL`).

The serial channel works the same way (`paniolo serial show` / `watch` /
`stop`); see [serial.md](serial.md).

**Prevention.** The `.deb` ships `/usr/lib/tmpfiles.d/paniolo.conf`
(`x /tmp/paniolo-*`). **A control host installed with `make install` must add
it itself**, or its daemons go untracked every ten days:

```bash
echo 'x /tmp/paniolo-*' | sudo tee /usr/lib/tmpfiles.d/paniolo.conf
```

### Stall recovery and format choice

**A stalled capture recovers on its own.** A watchdog reopens the device when
no frame arrives (12s after opening, or 4s without progress after that),
and `/snapshot`/`/status` show `no_device` meanwhile. The daemon exits only after 8 consecutive
stalls with no healthy frame between; `paniolo video watch` (or
`daemons restart --stale`) brings it back.

**The capture format is chosen by what streams.** On Linux the daemon tries
formats highest resolution first and accepts one only if it delivers a frame
within 2 s (uncompressed 1080p over USB 2.0 typically fails). A rejected
format is logged as `allocated buffers but produced no frame`. If nothing
delivers (e.g. the target is off), the best format that allocated is used.

---

## Capturing frames

```bash
paniolo video shot [target-machine] -o out.png   # save a screenshot (PNG)
paniolo video shot [target-machine]              # PNG to stdout (default -o -)
paniolo video shot --stable -o out.png           # wait for a steady frame first
paniolo video shot --changed-since <hex-hash> --timeout 10000 -o out.png
                                                 # block until the frame differs
paniolo video preview [target-machine]           # print the live-dashboard URL (optional target, like `show`)
paniolo video preview --open                     # open it in a browser instead of printing it
```

### `shot`

`shot` prints `signal=… hash=…` to stderr. Pass the hash to a later
`--changed-since` to wait for the screen to change.

- `--stable` waits for any steady frame.
- `--changed-since` waits for any frame that differs from the hash.
- **Both together** (`GET /snapshot?wait=stable&changed_since=<hash>`) wait
  for the next steady screen that differs.

`-o <path>` always writes on the **invoking machine**, even when the video
channel is on a remote control host. A failed capture removes the stub file.

`GET /snapshot` returns **503** in three cases:

| Response | Meaning |
|---|---|
| `x-signal: stale` | The last frame is too old to describe the screen now (capture stopped delivering, though the daemon process is up). |
| `x-signal: no_device` | No capture device is open. |
| **No `x-signal` header, body `capture thread gone`** | The daemon's capture thread has exited and is not coming back. This daemon needs `paniolo video watch --restart`, not another `shot`. |

### `preview`

`preview` prints the URL **with** the token. Treat it as a credential.
`--open` opens the default browser and prints only the token-free address. If
no browser launches, it writes the URL to the same `0600` file `console` uses.

`--open` is refused when the video channel is on another host. Use
`paniolo console <target>`, which forwards ports and opens a local browser.

`GET /preview` is the MJPEG stream behind `paniolo video preview` and the
[dashboard](dashboard.md):

- Open preview connections share one encode per frame.
- Each part carries an `X-Signal` header. When the signal is not
  `stable`/`mode_switching` (e.g. `stale`, `no_signal`, `no_device`), the
  stream shows a placeholder (dark gray with a red X) instead of a frozen
  frame.
- A frame whose JPEG encode fails logs one `warn!` and is skipped until a new
  frame arrives.

---

## OCR

```bash
paniolo video read [target-machine]            # OCR the current frame, text to stdout
paniolo video read --stable [--timeout <ms>]   # wait for a steady frame first
```

OCR runs locally: Apple Vision's `VNRecognizeTextRequest` on macOS, Tesseract
on Linux. With **no video signal**, `read` fails with `no video signal`
instead of returning empty text.

`read` wraps the daemon's `GET /ocr`, which the [dashboard](dashboard.md)'s
OCR button also uses:

```bash
d=/tmp/paniolo-$(id -u)/hdmicap/target-machine/daemon.json
curl -s -H "Authorization: Bearer $(jq -r .token "$d")" \
    "http://127.0.0.1:$(jq -r .port "$d")/ocr"
```

### GUI screens on Linux

GUI screens on Linux read more accurately with a different engine (see
[dev/ocr.md](dev/ocr.md#linux-needs-two-engines-the-other-platforms-need-one)).
Set it per target:

```bash
paniolo video set -t target-machine --device "0x8300000534d2109" --ocr-mode gui
paniolo video set -t target-machine --ocr-mode text   # back to the platform default
```

- `--ocr-mode` is `text` (default) or `gui`. It matters only on Linux.
- It works when the video channel is on a remote control host.
- `paniolo setup` builds the ~317 MB `rapidocr` venv only when a target in the
  active lab has `--ocr-mode gui`.

### Limits

`GET /ocr` gives the `visionocr`/`linuxocr`/`winocr` subprocess 30 seconds,
then kills it and answers **504**. `/snapshot`, `/ocr` and `/preview`'s encode
fallback share a concurrency limit (2 at a time), so bursts of clicks queue.

### Treat `signal` as a hint

When collecting frames, **save every frame and de-duplicate by hash
afterwards** instead of filtering on `signal`. A frame older than
`STALE_AFTER` reports `stale`, and `/snapshot`, `/ocr`, `--stable`, and
`/preview` refuse to treat it as live. Captures from older versions may show
mostly-black firmware screens as `no_signal` and stalled captures as
`stable`.

### Helper installation

`paniolo setup` installs the platform's helper into the private libexec dir:

- **macOS:** compiles `ocr/visionocr.swift` with `swiftc`
  (`~/.local/libexec/paniolo/bin/visionocr`).
- **Linux:** installs `linuxocr`, which shells out to Tesseract
  (`apt-get install tesseract-ocr`; Pillow is optional, for preprocessing).

The daemon finds the helper there, or via `PANIOLO_VISIONOCR`.

**OCR tuning:** `.fast` recognition (not `.accurate`, which misses small
console text), 2× upscale with black padding, and a lowered
`minimumTextHeight`.

---

## Runtime paths

| Purpose | Path |
|---|---|
| Video config | the target's `video` channel in the lab file (`~/.config/paniolo/lab.toml`) |
| hdmicap discovery | `/tmp/paniolo-<uid>/hdmicap/<target>/daemon.json` (`{pid, port, token}`; owner-only, it holds the token) |
| hdmicap advisory lock | `/tmp/paniolo-<uid>/hdmicap/<target>/daemon.lock` |
| hdmicap stderr log | `/tmp/paniolo-<uid>/hdmicap/<target>/daemon.log` (truncated on each start; shown on start timeout) |

The daemon is **per target**, so several targets can capture at once. The
runtime base honors `$PANIOLO_RUNTIME_BASE` (default `/tmp`). A `/tmp` sweep
can remove these files (see
[Stale and untracked daemons](#stale-and-untracked-daemons)).
