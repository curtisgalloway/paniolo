# Combined dashboard

hdmicap's web UI serves a two-pane page: the HDMI video stream on top and an
xterm.js terminal below. The terminal connects over WebSocket to serialcap,
so the two daemons stay decoupled — hdmicap only references serialcap by URL.

---

## Starting the dashboard

```bash
paniolo console                  # open in the default browser
```

`paniolo console` starts any daemon that isn't already running — hdmicap,
serialcap, and the hid daemon when the target has a `hid` channel — then opens
the dashboard. The URL it opens (and prints) carries each daemon's token —
hdmicap's as `?token=`, the others' inside their `?serialws=`/`?hidws=` URLs —
so open that URL, not a bare `http://127.0.0.1:<port>/`. (`paniolo video watch` / `paniolo serial watch` still start them
individually.) The page fetches the serialcap interface list and builds one
terminal pane per interface, displayed side by side in the serial panel (or
stacked in right-panel layout). With a single interface the panel looks the
same as before, with connection status in the top bar.

To open the dashboard pinned to a specific interface (single-pane mode):

```bash
paniolo console -i bmc
```

---

## Features

**Live video** — MJPEG stream from the capture card, auto-refreshing.

**Serial terminal** — full xterm.js terminal connected to serialcap via
WebSocket. Keystrokes go to the serial port; output appears in the terminal.
xterm.js is vendored (not CDN) so the dashboard works on an isolated lab
network.

**Multiple interfaces** — when serialcap is running multiple named interfaces,
the page shows one terminal pane per interface side by side. The
`?interface=<name>` URL parameter (or `console -i <name>`) pins the page to a
single interface.

**Layout toggle** — a button in the status bar switches the terminal between
bottom (default, 40 vh) and right-panel (380 px fixed, video fills remaining
width) layouts. The choice persists in `localStorage`.

**OCR button** — triggers `GET /ocr` on the hdmicap daemon, which OCRs the
current frame on-device (Apple Vision via `visionocr` on macOS, Tesseract via
`linuxocr` on Linux) and displays the result. Requires the OCR helper to be
installed (`paniolo setup`).

**Capture input (KVM)** — when the target has a `hid` channel, the **⌨ Capture
input** button toggles KVM mode (it becomes **⌨ Capturing** while active; click
again to release). Once engaged, your keyboard and mouse drive the target as USB
HID events (the mouse is absolute — the target cursor follows where you point in
the video). Your own cursor stays visible as a crosshair (no pointer lock), so
there is a little feedback lag but you never lose your pointer; losing window
focus also releases. The page streams commands to the hid daemon over a
WebSocket, and `paniolo hid send` injections from the CLI intermix with them. See
[HID injection › KVM mode](hid.md#kvm-mode-type-and-click-from-the-web-console).

**Power** — when the target has a `power` channel, an on/off **toggle switch**
(`Power [switch] ON/OFF`, reflecting live state, polled every few seconds) and a
separate **⟳ Cycle** button appear in the overlay. Each asks for confirmation
before acting. Availability and state come from `GET /power`, which performs **no**
action — merely loading the dashboard never powers the target.

---

## URL parameters

| Parameter | Effect |
|---|---|
| `?token=<token>` | hdmicap's own token; the page puts it on every request it makes back to hdmicap |
| `?serialws=<url>` | Connect the terminal to this serialcap WebSocket URL, which carries serialcap's `?token=` inside (what `paniolo console` passes; percent-encoded) |
| `?serial=<port>` | Connect the terminal to serialcap on this local port — no token, so only a daemon started without one accepts it |
| `?interface=<name>` | Preselect a named serial interface |
| `?hidws=<url>` | Enable KVM input via this hid WebSocket URL, hid's `?token=` inside (what `paniolo console` passes) |
| `?hid=<port>` | Enable KVM input via the hid daemon on this local port (no token, as for `?serial=`) |

`serialws`/`hidws` (and the URLs built from `serial`/`hid`) must be loopback
— `127.0.0.1`, `localhost` or `[::1]`; the page refuses anything else with an
inline error and makes no connection, so a crafted link cannot point your
keystrokes, or the tokens in those URLs, somewhere else. The page also refuses
to render inside another page's frame (`Content-Security-Policy:
frame-ancestors 'none'`), so its power buttons cannot be clickjacked.

---

## Connecting the daemons

`paniolo console` supplies every connection automatically. It reads each
daemon's discovery file (port and token) and passes the page one complete
loopback WebSocket URL per daemon — `?serialws=ws://127.0.0.1:<port>/stream?token=…`
and `?hidws=…/hid?token=…` — on the local path as well as the remote/tunnel
path, where `<port>` is the tunnel's local end. hdmicap's own token rides as
`?token=`. The `?serial=` / `?hid=` port forms let you point a hand-opened page
at a daemon yourself, but they cannot carry a token, so they work only against
a daemon started by an older paniolo; without any of them the page falls back
to `ws://<host>:8724/stream` (the standalone `serialcap --port` default).
