# Video capture

paniolo captures the target's screen through `hdmicap`, a Rust daemon that
keeps a USB HDMI capture device open and serves the current frame over HTTP.
Keeping the stream warm avoids the multi-second reopen delay of running ffmpeg
for each capture.

---

## Hardware

Any USB HDMI capture card that presents as a UVC device (USB Video Class, the
standard webcam protocol; driven by V4L2 on Linux and AVFoundation on macOS).
Tested with MS2109-based cards (e.g. generic "USB3.0 HDMI Capture" dongles).

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

| Form | Notes |
|---|---|
| **Stable id** (preferred) | The AVFoundation `uniqueID` on macOS, the `/dev/v4l/by-path/...` symlink on Linux. Both come from USB port topology: they survive reboots and enumeration-order changes, tell two identical dongles apart, and change only if the dongle moves to a different physical port. |
| **Name substring** (e.g. `"USB Video"`) | Convenient, but identical dongles share a name. A substring that matches more than one device is an error listing the candidates' ids, never a silent first-match guess. |
| **`/dev/video*` path** (Linux) | Accepted, but not stable across reboots. |

On Linux, `video devices` hides SoC-internal video nodes (e.g. a Raspberry
Pi's `pispbe-*` pipeline stages and HEVC decoder, which would flood the list)
and shows only external capture devices. `paniolo helper hdmicap devices --all`
lists everything, and an internal device you configure explicitly still
resolves.

The device is stored on the target's `video` channel in the lab file (see
[config-redesign.md](https://github.com/curtisgalloway/paniolo/blob/main/notes/config-redesign.md)).
When one non-built-in capture device is present, `paniolo configure` proposes
its stable id (with the human name as a comment); when there are several, it
lists the id alternatives.

---

## Starting and stopping the daemon

```bash
paniolo video watch [target-machine]   # start hdmicap daemon for a target
paniolo video watch --restart          # force-restart a running (stalled) daemon
paniolo video stop  [target-machine]   # stop it (on the target's host)
paniolo video show  [target-machine]   # show daemon address and status
```

`watch` starts `hdmicap daemon` detached, waits for it to come up, and prints
its address. To open the dashboard, run `paniolo video preview`: its URL
carries the daemon's `?token=`, and the daemon answers nothing without it.

### The token and the dashboard URL

**Every request to the daemon needs its token.** hdmicap makes a fresh one on
each start and publishes it as `token` in its discovery file (see
[Runtime paths](#runtime-paths)), readable only by the operator's uid.
paniolo's own commands (`shot`, `read`, `console`, …) send it automatically. By
hand, send it as `Authorization: Bearer <token>` or `?token=<token>`. The
daemon also requires a loopback `Host` and `Origin`, so a web page in your
browser cannot reach it. A daemon started by a paniolo older than the token
has none; `paniolo daemons restart --stale` replaces it.

**`show` and `console` print the address without the token, on purpose.** The
token is a live bearer credential, and their output lands where credentials
should not: terminal scrollback, `script`/`asciinema` recordings, CI logs,
terminal output pasted into issues, and agent transcripts (agents run
`video show` constantly). They print `http://127.0.0.1:<port>`, which
identifies the daemon and is useless on its own.

**`video preview` is the only command that prints the openable URL**, and the
only place in the CLI that builds one. There is deliberately no shared helper,
so no other command can print it by accident. `video watch` prints the
token-free address and points you at `preview`.

**If `console` cannot launch a browser** (a headless control host, a
container, no `xdg-open`), it does not just give up. An address you cannot
open would strand you, and on the remote path the SSH tunnels die with the
command, so there is no second chance. Instead it writes the full URL to a
`0600` `dashboard-url.txt` in the target's runtime dir and prints the **path**.
The token stays out of the terminal but is one `cat` away:

```bash
xdg-open "$(cat /tmp/paniolo-1000/hdmicap/target-machine/dashboard-url.txt)"
```

### Stopping safely

`video stop` (and `hdmicap stop`) shuts the daemon down through its
authenticated `POST /stop`, never by signaling the PID in the discovery file.
A record left behind by a crash can name a PID the kernel has since given to
an unrelated process. A daemon too old to have the endpoint must be stopped
with `paniolo daemons stop hdmicap`, which checks the process identity first.

### Stale and untracked daemons

**Stale.** After an upgrade or rebuild, a daemon still running the old binary
shows as **stale** in `paniolo video show` and `paniolo daemons`. `watch`
restarts a stale daemon automatically (no `--restart` needed), or run
`paniolo daemons restart hdmicap` (see [architecture](dev/architecture.md)).

**Untracked.** An *untracked* daemon is one that outlived its discovery file,
which is paniolo's only record of a running daemon. On Linux the file sits in
`/tmp`, which systemd cleans by age. Debian's stock policy is
`q /tmp 1777 root root 10d`, so a daemon that runs for ten days with no
command against it loses its file. Nothing tells the daemon. It keeps running
and keeps the capture device, while `video show` reports the channel stopped
and `video watch` starts a replacement that dies on the advisory lock the
orphan still holds (`another hdmicap daemon is already running`).

paniolo handles this:

- `video show` reports `running, untracked (pid N)` instead of `stopped`.
- `paniolo daemons` lists it under **Untracked daemons**.
- `video watch` and `video stop` reap it (`SIGTERM`, then `SIGKILL`), so you
  don't need `ps` and `kill`. Its port and token were lost with the file, so
  there is no way to talk to it; a signal is the only handle left.

The serial channel works the same way (`paniolo serial show` / `watch` /
`stop`); see [serial.md](serial.md).

**Prevention.** The `.deb` ships `/usr/lib/tmpfiles.d/paniolo.conf`
(`x /tmp/paniolo-*`), so this does not happen on a packaged install. **A
control host installed with `make install` should add that one-line drop-in
itself**, or its daemons will go untracked every ten days:

```bash
echo 'x /tmp/paniolo-*' | sudo tee /usr/lib/tmpfiles.d/paniolo.conf
```

### Stall recovery and format choice

**A stalled capture usually recovers on its own.** A watchdog in the capture
thread notices when no new frame has arrived for a while (12s after opening
the device, or a further 4s without progress after that) and reopens the
device in place, publishing `no_device` meanwhile. No restart is needed, and
this is why `/snapshot`/`/status` briefly show `no_device` instead of a frozen
frame during a stall. The daemon gives up and exits only if the device keeps
stalling right after every reopen (8 in a row with no healthy frame between).
Then `paniolo video watch` (or `daemons restart --stale`) brings it back.

**The capture format is chosen by what streams, not by what negotiates.** On
Linux the daemon tries formats highest resolution first and accepts one only
after it actually delivers a frame (within 2 s). A mode can allocate buffers
and then fail once streaming starts; the classic case is uncompressed 1080p
over USB 2.0. A rejected format is logged to the daemon's stderr log
(`allocated buffers but produced no frame`). If *nothing* delivers a frame,
which is normal when the target is off and the device sends nothing, the best
format that allocated is opened anyway, so the daemon is ready when a signal
arrives.

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

`shot` fetches one PNG frame from the running daemon and prints
`signal=… hash=…` to stderr. Pass that hash to a later `--changed-since` to
wait for the screen to change.

- `--stable` waits for any steady frame.
- `--changed-since` waits for any frame that differs from the hash.
- **Both together** (`GET /snapshot?wait=stable&changed_since=<hash>`) wait
  for a frame that is *both* stable *and* different: the next steady screen
  that is not the one you already have.

`-o <path>` always writes to the **invoking machine's** filesystem, even when
the target's video channel is on a remote control host. The remote shot
streams over SSH and the PNG is written locally (a failed capture removes the
stub file). No copy-back step is needed.

`--stable`/`--changed-since` wait by polling the daemon's internal frame
channel, not by re-hitting the endpoint. `GET /snapshot` returns **503** in
three cases worth telling apart when scripting:

| Response | Meaning |
|---|---|
| `x-signal: stale` | The last frame is too old to describe the screen now (capture stopped delivering, though the daemon process is up). |
| `x-signal: no_device` | No capture device is open. |
| **No `x-signal` header, body `capture thread gone`** | The daemon's capture thread has exited and is not coming back. This daemon needs `paniolo video watch --restart`, not another `shot`. |

PNG encoding (and, on Linux, the MJPEG decode feeding it) is real CPU work.
`/snapshot` and `/ocr` share a small concurrency limit for it, so a burst of
clicks queues briefly instead of piling up unbounded work (see [OCR](#ocr)).

### `preview`

`preview` prints the URL **with** the token, because a browser can receive it
no other way. Treat that output as a credential: paste it into a browser, not
into a log. `--open` hands the URL to the default browser and prints only the
token-free address, which is safer when anything is recording the terminal.
If no browser can be launched, it writes the URL to the same `0600` file
`console` uses and prints that path.

`--open` is refused when the target's video channel is on another host: the
command would re-exec there and open a browser on the bench machine, not on
yours. Use `paniolo console <target>`, which forwards the ports and opens a
browser locally, or plain `preview` and open the URL through your own tunnel.

`GET /preview` is the MJPEG (a stream of JPEG frames) behind
`paniolo video preview` and the [dashboard](dashboard.md). Its behavior:

- **Shared encode limit.** Its JPEG-encode fallback (macOS/Windows NV12
  frames) uses the same concurrency limit as `/snapshot` and `/ocr`. Linux
  serves the device's raw MJPEG bytes directly and never takes this path.
- **Coalescing.** Every open preview connection watching the same frame
  reuses one encode, so a few browser tabs left open cannot starve `/snapshot`
  or `/ocr` of CPU.
- **Stale frames are replaced.** Every multipart part carries an `X-Signal`
  header naming the effective signal behind it. When that signal is anything
  but `stable`/`mode_switching` (usually `stale`, the same staleness
  `/snapshot` and `/ocr` refuse, but also `no_signal`/`no_device`), the stream
  stops sending the frame and sends a placeholder instead: a dark gray field
  with a red diagonal X, once per transition, at the last known resolution.
  Without it, the `<img>` in a browser tab left open would freeze on the last
  real frame forever, which looks exactly like a live, unchanging screen.
- **A bad frame is tried once.** If the fallback JPEG encode *fails* for a
  live frame (a malformed pixel buffer whose length doesn't match its
  dimensions), `/preview` does not retry it on every 67 ms tick. It records the
  frame, logs one `warn!` with the reason, and skips it until a new, encodable
  frame arrives. So one bad frame stuck in the channel can't spin the encoder
  or drain the shared limit ~15 times a second.

---

## OCR

OCR (optical character recognition) reads the text on the captured screen.

```bash
paniolo video read [target-machine]            # OCR the current frame, text to stdout
paniolo video read --stable [--timeout <ms>]   # wait for a steady frame first
```

OCR runs on the machine itself, with no network and no model download:
Apple Vision's `VNRecognizeTextRequest` on macOS, Tesseract on Linux.

When the capture has **no video signal** (the target's display is off or
unplugged), `read` fails with `no video signal` instead of returning empty
text, so "display is off" and "screen is blank" stay distinguishable.

`read` wraps the daemon's `GET /ocr` endpoint. The OCR button on the
[web dashboard](dashboard.md) uses it too, or call it directly with the token
from the discovery file:

```bash
d=/tmp/paniolo-$(id -u)/hdmicap/target-machine/daemon.json
curl -s -H "Authorization: Bearer $(jq -r .token "$d")" \
    "http://127.0.0.1:$(jq -r .port "$d")/ocr"
```

### GUI screens on Linux

**GUI screens on Linux** get more accurate OCR from a different engine than
console/firmware screens do. See
[dev/ocr.md](dev/ocr.md#linux-needs-two-engines-the-other-platforms-need-one)
for the accuracy numbers and why. Set it per target:

```bash
paniolo video set -t target-machine --device "0x8300000534d2109" --ocr-mode gui
paniolo video set -t target-machine --ocr-mode text   # back to the platform default
```

- `--ocr-mode` is `text` (the platform default) or `gui`. Unset means `text`.
- It only matters on Linux. macOS and Windows use their one native engine
  regardless.
- It works on a target whose video channel is on a remote control host: the
  field travels with the channel when paniolo re-execs there, like `--device`.
- `paniolo setup` builds the ~317 MB `rapidocr` venv only when some target in
  the active lab has `--ocr-mode gui` set (see dev/ocr.md for why it's
  opt-in).

### Limits

**OCR is bounded, so a wedged or slow helper can't hang the daemon.**
`GET /ocr` gives the `visionocr`/`linuxocr`/`winocr` subprocess 30 seconds.
After that the daemon kills it and answers **504**. PNG encoding and the OCR
subprocess share a small concurrency limit (2 at a time), so repeated
OCR/snapshot clicks from the dashboard queue briefly instead of spawning
unbounded helper processes or CPU work. A burst of clicks is slower, not
runaway.

### Treat `signal` as a hint

The lesson for anything collecting frames: **treat `signal` as a hint, save
every frame, and de-duplicate by hash afterwards** instead of filtering on the
label as you go.

If you read older captures, two `signal` bugs (both fixed) may show up in them:

- A *mostly black* screen, which is what every firmware, bootloader and
  console screen is, was classified as no-signal because the sampling lattice
  was too coarse to hit any text. A Gigaboot screen with 1.35% of its pixels
  lit reported `no_signal` for minutes.
- A *stalled* capture kept reporting `stable`. The capture loop publishes only
  on success, so the last frame stayed in place with its old label; a machine
  whose mains had been cut kept reporting `stable` on its pre-cut desktop.

Now a frame older than `STALE_AFTER` reports `stale`, and `/snapshot`, `/ocr`,
`--stable`, and `/preview` all refuse to treat it as live. (`/preview` swaps in
the placeholder image described above instead of refusing outright, since a
stream can't return an error mid-part.)

### Helper installation

`paniolo setup` installs the platform's helper into the private libexec dir:

- **macOS:** compiles `ocr/visionocr.swift` with `swiftc`
  (`~/.local/libexec/paniolo/bin/visionocr`).
- **Linux:** installs `linuxocr`, a Python 3 script that shells out to
  Tesseract (`apt-get install tesseract-ocr`; Pillow is optional, for its
  upscale/pad preprocessing).

The hdmicap daemon finds the helper there (or via `PANIOLO_VISIONOCR`, on both
platforms) and shells out to it per request.

**OCR tuning notes:**
- `.fast` recognition level is used, not `.accurate`: the latter is tuned for
  natural document text and misses small console text entirely.
- The frame is 2×-upscaled and black-padded before recognition to improve
  accuracy on thin console fonts.
- `minimumTextHeight` is lowered from the default to catch small terminal text.

---

## Runtime paths

| Purpose | Path |
|---|---|
| Video config | the target's `video` channel in the lab file (`~/.config/paniolo/lab.toml`) |
| hdmicap discovery | `/tmp/paniolo-<uid>/hdmicap/<target>/daemon.json` (`{pid, port, token}`; owner-only, it holds the token) |
| hdmicap advisory lock | `/tmp/paniolo-<uid>/hdmicap/<target>/daemon.lock` |
| hdmicap stderr log | `/tmp/paniolo-<uid>/hdmicap/<target>/daemon.log` (truncated on each start; shown on start timeout) |

The hdmicap daemon is **per target** (the `<target>` segment), so several
targets can capture at once on one host. The runtime base honors
`$PANIOLO_RUNTIME_BASE` (default `/tmp`). Nothing rewrites these files after
the daemon starts, which is why a `/tmp` sweep can remove them (see
[Stale and untracked daemons](#stale-and-untracked-daemons)).
