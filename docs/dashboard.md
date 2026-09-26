# Combined dashboard

One web page to watch and drive a target: HDMI video on top, a serial
terminal below. hdmicap (video daemon) serves the page; the terminal connects
to serialcap (serial daemon) over WebSocket.

---

## Starting the dashboard

```bash
paniolo console                  # open in the default browser
```

`paniolo console` starts any daemon not already running (hdmicap, serialcap,
and the hid daemon if the target has a `hid` channel), then opens the page.
`paniolo video watch` / `paniolo serial watch` start them individually.

**Use the URL the browser was handed**, not a bare `http://127.0.0.1:<port>/`:
it carries each daemon's token (hdmicap's as `?token=`, the others inside
`?serialws=`/`?hidws=`). `console` prints only the bare address, to keep tokens
out of pasted output. If no browser launches, it writes the full URL to a
`0600` `dashboard-url.txt` in the target's runtime dir and prints that path.

The page shows one terminal pane per serialcap interface. To pin one
interface (single-pane mode):

```bash
paniolo console -i bmc
```

---

## Features

**Live video:** MJPEG from the capture card.

**Serial terminal:** xterm.js, vendored, so it works on an isolated lab
network. The `?interface=<name>` URL parameter (or `console -i <name>`) pins
one interface.

**Layout toggle:** switches the terminal between bottom (default, 40 vh) and
right-panel (380 px) layouts; saved in `localStorage`.

**OCR button:** calls `GET /ocr` on hdmicap, which OCRs the current frame
(`visionocr` on macOS, `linuxocr` on Linux). Needs the OCR helper
(`paniolo setup`).

**Capture input (KVM):** with a `hid` channel, **⌨ Capture input** sends your
keyboard and absolute mouse to the target; it reads **⌨ Capturing** while
active. Click again or leave the window to release. Your cursor stays visible
(no pointer lock), and CLI `paniolo hid send` input intermixes. See
[HID injection › KVM mode](hid.md#kvm-mode-type-and-click-from-the-web-console).

**Power:** with a `power` channel, a live **toggle switch**
(`Power [switch] ON/OFF`) and a **⟳ Cycle** button, each confirming before it
acts. Loading the page never powers the target (`GET /power` only reads).

---

## URL parameters

| Parameter | Effect |
|---|---|
| `?token=<token>` | hdmicap's own token; the page puts it on every request it makes back to hdmicap |
| `?serialws=<url>` | Connect the terminal to this serialcap WebSocket URL, serialcap's `?token=` inside (percent-encoded; what `paniolo console` passes) |
| `?serial=<port>` | Connect to serialcap on this local port; no token, so only a daemon started without one accepts it |
| `?serial=none` | No terminal pane (a target with no serial channel) |
| `?interface=<name>` | Preselect a named serial interface |
| `?hidws=<url>` | Enable KVM input via this hid WebSocket URL, hid's `?token=` inside (what `paniolo console` passes) |
| `?hid=<port>` | Enable KVM input via the hid daemon on this local port (no token, as for `?serial=`) |

`serialws`/`hidws` (and URLs built from `serial`/`hid`) must be loopback
(`127.0.0.1`, `localhost` or `[::1]`); the page refuses anything else. It also
refuses to render in a frame (`Content-Security-Policy:
frame-ancestors 'none'`).

---

## Connecting the daemons

`paniolo console` reads each daemon's discovery file (port and token) and
passes `?serialws=ws://127.0.0.1:<port>/stream?token=…` and
`?hidws=…/hid?token=…`; over a remote tunnel, `<port>` is the tunnel's local
end.

The `?serial=` / `?hid=` port forms carry no token, so they work only against a
daemon started by an older paniolo. With no parameters, the page falls back to
`ws://<host>:8724/stream` (the standalone `serialcap --port` default).
