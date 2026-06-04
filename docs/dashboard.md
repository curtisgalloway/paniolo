# Combined dashboard

hdmicap's web UI serves a two-pane page: the HDMI video stream on top and an
xterm.js terminal below. The terminal connects over WebSocket to serialcap,
so the two daemons stay decoupled — hdmicap only references serialcap by URL.

---

## Starting the dashboard

```bash
paniolo video watch [target-machine]    # start hdmicap
paniolo serial watch [target-machine]   # start serialcap
paniolo console                  # open in the default browser
```

`paniolo console` verifies that both daemons are running and then opens the
dashboard. The page fetches the serialcap interface list and builds one
terminal pane per interface, displayed side by side in the serial panel (or
stacked in right-panel layout). With a single interface the panel looks the
same as before, with connection status in the top bar.

To open the dashboard pinned to a specific interface (single-pane mode):

```bash
paniolo console -i bmc
```

If a daemon isn't running, `console` prints which one is missing and the
command to start it.

---

## Features

**Live video** — MJPEG stream from the capture card, auto-refreshing.

**Serial terminal** — full xterm.js terminal connected to serialcap via
WebSocket. Keystrokes go to the serial port; output appears in the terminal.
xterm.js is vendored (not CDN) so the dashboard works on an isolated lab
network.

**Interface selector** — when serialcap is running multiple named interfaces,
a dropdown appears in the status bar. Selecting one reconnects the terminal
to that interface. The `?interface=<name>` URL parameter preselects one.

**Layout toggle** — a button in the status bar switches the terminal between
bottom (default, 40 vh) and right-panel (380 px fixed, video fills remaining
width) layouts. The choice persists in `localStorage`.

**OCR button** — triggers `GET /ocr` on the hdmicap daemon, which OCRs the
current frame via Apple Vision and displays the result. Requires
`visionocr` to be installed (`paniolo setup`).

**Capture input (KVM)** — when the target has a `hid` channel, a **⌨ Capture
input** button appears over the video. Engaging it forwards your keyboard and
mouse to the target as USB HID events (the mouse is absolute — the target
cursor follows where you point in the video). **Right-Ctrl** releases capture.
The page streams commands to the hid daemon over a WebSocket, and `paniolo hid
send` injections from the CLI intermix with them. See
[HID injection › KVM mode](hid.md#kvm-mode--type-and-click-from-the-web-console).

---

## URL parameters

| Parameter | Effect |
|---|---|
| `?serial=<port>` | Connect terminal to serialcap on a non-default port |
| `?serialws=<url>` | Connect terminal to an explicit WebSocket URL |
| `?interface=<name>` | Preselect a named serial interface |
| `?hid=<port>` | Enable KVM input via the hid daemon on a local port |
| `?hidws=<url>` | Enable KVM input via an explicit hid WebSocket URL |

---

## Connecting the daemons

By default hdmicap connects the terminal to `ws://<host>:8724/stream` (the
default serialcap port). The `?serial=` and `?serialws=` parameters let you
point it at a different port or host if serialcap is running elsewhere.
`paniolo console` supplies the right value automatically: the local path
passes serialcap's OS-assigned port as `?serial=PORT`, and the remote/tunnel
path passes an explicit `?serialws=` URL.
