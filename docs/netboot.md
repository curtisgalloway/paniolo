# Netboot

paniolo netboots a target by running a minimal DHCP + TFTP + HTTP server over a
direct USB-Ethernet link. No router, switch, or upstream DHCP server is involved.

It serves three kinds of client from one configuration, selecting the path from
the client's DHCP vendor class (option 60):

| Client | How it boots | Served over |
|---|---|---|
| Raspberry Pi 5 bootloader | no vendor class → legacy reply | TFTP |
| UEFI **PXE** client (e.g. EDK2 on an Indiedroid Nova) | `PXEClient` → bootfile + `PXEClient` echo | TFTP |
| UEFI **HTTP Boot** client | `HTTPClient` → `http://` URL + `HTTPClient` echo | HTTP |

For UEFI clients, HTTP Boot is the nicer transport (kernel TCP — fast,
loss-tolerant, robust under host load, and none of the macOS raw-frame machinery
the silent Pi bootloader needs) **where the firmware allows plain HTTP**. Many
EDK2 builds enforce HTTPS-only and reject our `http://` URL; on those, use **PXE**
(verified end-to-end on the Nova). See [UEFI clients](#uefi-clients-pxe-http-boot)
below.

---

## Hardware setup

1. Plug a USB-to-Ethernet adapter into your Mac.
2. Connect an Ethernet cable from the adapter directly to the target's Ethernet
   port (no switch needed — modern adapters handle MDI/MDIX automatically).
3. Find the macOS interface name:

```bash
networksetup -listallhardwareports
```

---

## Target configuration

Config lives in the lab file (see [config-redesign.md](https://github.com/curtisgalloway/paniolo/blob/main/notes/config-redesign.md));
the netboot link is a per-target `netboot` channel:

```bash
# Create the target, then configure its netboot channel
paniolo target add target-machine
paniolo netboot set -t target-machine \
    --interface en3 \
    --tftp-root ~/src/fuchsia/pxe/tftp-root

# List candidate USB-Ethernet interfaces (primary NIC excluded)
paniolo netboot devices

# Show all configured targets / a specific one
paniolo target show
paniolo target show target-machine

# Remove the netboot channel, or the whole target
paniolo netboot rm -t target-machine
paniolo target rm target-machine
```

netboot channel fields:

| Field | Default | Description |
|---|---|---|
| `--interface` | (required) | USB-Ethernet interface name (e.g. `en3`). Every netbootd listener is pinned to it — see [Interface pinning](#interface-pinning) |
| `--host-ip` | `192.168.99.1` | Static IP assigned to the interface; also the TFTP/HTTP server address and the router the client is told about. The client's lease is derived from it (same /24, last octet `100` — `192.168.99.100` by default) — see [Lease](#dhcp--tftp-behavior-notes) |
| `--tftp-root` | (none) | Directory whose contents are served over TFTP **and** HTTP |
| `--boot-file` | `kernel_2712.img` | Boot program (filename under the root, e.g. `grubaa64.efi`); served as a TFTP filename to PXE and wrapped in an `http://` URL for HTTP Boot |
| `--http-port` | `80` | HTTP server port; also embedded in the HTTP Boot URL (omitted from the URL when 80) |
| `--content-type` | `application/octet-stream` | `Content-Type` for HTTP responses (UEFI treats octet-stream as an EFI application) |
| `--host` | target default | Lab host the channel lives on |

Power-cycle and DTR control are configured on the `power` channel
(`paniolo power set …` — see [power.md](power.md)).

---

## Starting and stopping

```bash
paniolo netboot start [target-machine]
paniolo netboot stop  [target-machine]
```

`start` assigns the static `host_ip` to the interface, then launches paniolo's
own **DHCP + TFTP + HTTP server** — the single `netbootd` binary (Rust), serving
all three protocols from one background process. No external daemons (`dnsmasq`,
`tftp-now`) are required at runtime. `stop` sends SIGTERM (via `sudo kill`
when the daemon is root's), waits up to 3 s, SIGKILLs a holdout and waits 2 s
more, then clears the state file and restores the interface. It only signals
a pid whose command line still names `netbootd` — a recorded pid the kernel
has since recycled is just forgotten — and if the daemon survives even the
SIGKILL, `stop` fails and keeps the state file rather than report a stop that
did not happen.

**`start` confirms the daemon came up.** netbootd validates its configuration
and binds every listener *before* it serves anything, so a port already in use,
an interface it cannot pin, or a bad client IP shows up as an early exit.
`start` watches the new process for about two seconds and only then records it
as running; if it exits in that window, `start` fails with the last 20 lines of
its log (`netbootd exited with … during startup; last lines of
~/.local/share/paniolo/<name>/netboot.log: …`) and writes no state file, so
`netboot status` never reports a daemon that is not there.

**One netboot per interface.** `start` refuses to start a second target on an
interface where another target's netbootd is already alive (`netboot for
'<other>' is already running on <iface> … stop it first`). Two servers on one
link would only fight for the DHCP/TFTP ports; give each target its own adapter.

**Privileged ports (67/69, and 80 by default):** macOS 10.14+ allows binding
`0.0.0.0` on privileged ports without root, so on macOS the only step needing
sudo is assigning the static IP. On **Linux**, ports 67/69 (and 80) require
root, so `start` auto-prepends `sudo` when spawning `netbootd`, and interface
configuration (`ip addr add`) uses sudo as well. Configure **NOPASSWD sudo** on
the control host for unattended agent use. To avoid privileged-port binds for
HTTP entirely, set `--http-port` to an unprivileged high port (e.g. `8080`) — it
is embedded in the boot URL, so the UEFI client follows it.

**netbootd gives root back (Linux).** Root is only needed to bind the low ports
and pin the sockets. Once that is done — before the first packet is served —
netbootd drops to the user who ran `sudo` (from the `SUDO_UID`/`SUDO_GID` sudo
sets) with `setgroups`/`setgid`/`setuid`, and refuses to start if any of the
three fails. The log records `dropped privileges to uid N gid M`. The ARP pin
(`ip neigh`) and the interface-IP monitor (`ip addr add`) then run through
`sudo` *as that user*, which is why the passwordless-sudo requirement above is
not optional on Linux. Started as root directly (no `sudo`, so no `SUDO_UID`),
netbootd stays root and says so in its log. On macOS netbootd never had root
(the setuid helper below holds the only privilege), so nothing changes there.

**HTTP is optional; DHCP and TFTP are not.** If the HTTP port cannot be bound
(something else owns port 80, say), netbootd logs a warning naming the port and
keeps serving DHCP + TFTP — the Pi and UEFI PXE paths still work. HTTP Boot is
then unavailable: an `HTTPClient` DHCP request still gets its `http://` offer,
but the fetch fails. Free the port or set `--http-port` to an unused one. A DHCP
or TFTP socket that cannot be bound or pinned is fatal.

**Interface safety:** `start` **refuses** an interface that carries your system
default route (a primary NIC). netboot reconfigures the interface to the static
`host_ip`, which would break your real networking — the netboot link must be a
dedicated USB-Ethernet adapter.

### Interface pinning

netbootd answers every DHCP DISCOVER it hears with a lease, and hands out
whatever is in the TFTP root to anyone who asks — so it must only ever hear the
netboot link. Every listen socket (DHCP 67, TFTP 69, HTTP) is **pinned to the
netboot interface** before it is bound: `IP_BOUND_IF` on macOS,
`SO_BINDTODEVICE` on Linux. Requests arriving on any other interface — your
office LAN on the primary NIC — are never seen, and replies (including the
limited-broadcast DHCP offers) can only leave via the netboot link. A pin that
cannot be applied is fatal: netbootd refuses to start rather than serve
unpinned, and `netbootd`'s `--interface` flag is therefore required, not
optional. The sockets still bind the wildcard address so they keep working
through the link flaps where the interface IP is momentarily gone.

**TFTP reply sockets are pinned too, on both platforms.** Each TFTP transfer
answers from its own per-transfer socket (a fresh ephemeral port), not the
port-69 listen socket, so it needs its own pin. On macOS that is
`IP_BOUND_IF`, same as the listen sockets. On Linux it is the same
`SO_BINDTODEVICE`, even though the reply socket is created after netbootd has
already dropped root (see [netbootd gives root back](#starting-and-stopping)
above): `SO_BINDTODEVICE` has not needed `CAP_NET_RAW` since Linux 5.7, which
covers every kernel paniolo targets (Pi OS Trixie 6.x, Debian 12 6.1). On an
older kernel the pin fails with `EPERM`/`EACCES`; netbootd logs one `warn!`
naming the interface and falls back to the pre-fix behavior — bind the reply
socket to the interface IP alone and let the kernel route — rather than
failing the transfer. Either way the reply socket also binds to `host_ip`,
not the wildcard, so its source address is the one the client dialled.

Without this pin, two netboot links sharing the same `/24` (paniolo's default
`192.168.99.1` on both) send every OACK and DATA block out of whichever
interface currently owns the kernel route for that subnet, not the one the
RRQ arrived on — the client retransmits its RRQ forever and the log repeats
`no ACK for OACK` (issue #109). **To verify:** bring up two netboot-capable
links both at `192.168.99.1/24` — put one target in [`netif mode
link`](netif.md#testing-the-link-up-and-down) so it holds the IP without
running a daemon on it — then `paniolo netboot start` a target on the *other*
link and PXE-boot it. Before this fix the daemon log repeats `no ACK for
OACK`; after, it logs `completed <file>`.

**Just the link, no daemon.** `start`/`stop` bring the link up *and* run (or
stop) the DHCP/TFTP server together. To bring the **bare link** up or down on its
own — assign or release the host IP without serving anything, e.g. to test that
the link comes up and drops — use [`paniolo netif mode link`](netif.md) and
`paniolo netif mode off` instead. Note that "down" only releases the host IP; it
does not force the physical carrier down (a NIC with Wake-on-LAN enabled keeps
the link energized) — see [Link mode](netif.md#testing-the-link-up-and-down).

### The netbootd engine

`netbootd` is the single-binary DHCP + TFTP + HTTP server (Rust); it is the only
netboot engine. (It was originally ported from a pure-Python `_dhcp`/`_tftp`
subprocess pair, since removed.)

On macOS, netbootd's raw-frame send path (the Sequoia delivery workaround) needs
a `/dev/bpf` descriptor. Rather than run the daemon as root, `paniolo setup`
installs a tiny **setuid-root** helper, `netbootd-bpf-helper`, whose only job is
to open `/dev/bpf`, bind the interface, and hand the descriptor to the
unprivileged `netbootd`. It is the only paniolo component that runs as root. If
it is missing or not setuid, the rust engine logs a warning and falls back to
the kernel send path (which is unreliable on macOS 15+). Run `paniolo setup`
(one sudo) to install it.

What the helper will and will not do, since it is setuid-root and any local
user can execute it:

- **Only the installing user may invoke it.** It refuses every caller whose
  real uid is not the owner of the directory it was installed into (your
  private libexec dir, or the Homebrew keg) — root excepted. Another user gets
  `refused: caller uid N is not the installing user (uid M)` and no descriptor.
- **It refuses the default-route interface.** Asking it to bind the primary
  NIC (the one `route -n get default` names) yields `refused: <iface> carries
  the default route`; the netboot link must be a dedicated secondary adapter.
- **The descriptor is write-only, with a reject-all filter.** It is opened
  `O_WRONLY`, so `read(2)` on it fails, and a `BIOCSETF` program that accepts
  nothing is installed before it leaves the helper. The daemon can inject
  frames on the netboot interface and nothing else — it cannot capture.

`netbootd` reports the helper's exit status and message in its log when the
handoff fails (`BPF handoff failed (netbootd-bpf-helper exited with exit
status: 1: refused: …)`), so a refused or not-setuid helper is diagnosable
from the log alone.

---

## Status and logs

```bash
paniolo netboot status [target-machine]      # running? interface? uptime?
paniolo netboot logs   [target-machine]      # tail the combined DHCP + TFTP log
paniolo netboot logs -f [target-machine]     # follow
```

---

## Getting the TFTP root path

```bash
paniolo netboot tftp-root [target-machine]
```

Prints the bare TFTP root path, designed for shell substitution:

```bash
TFTP_ROOT=$(ssh control-mac "paniolo netboot tftp-root target-machine")
scp kernel_2712.img control-mac:"${TFTP_ROOT}/kernel_2712.img"
```

---

## Expected TFTP sequence for Raspberry Pi 5

When the Pi 5 EEPROM PXE client boots it walks this file request sequence.
The 404s are normal:

```
404  <serial>/<mac>/start.elf    ← Pi 5 doesn't need it; 404 expected
200  config.txt
200  bcm2712-rpi-5-b.dtb
200  kernel_2712.img              ← your boot shim or kernel
```

The TFTP root must contain at minimum `config.txt`, `bcm2712-rpi-5-b.dtb`,
and `kernel_2712.img`.

---

## UEFI clients (PXE / HTTP Boot)

UEFI firmware (e.g. Tianocore EDK2 on an Indiedroid Nova, RK3588S) can netboot
over IPv4 by **PXE** or **HTTP Boot**. `netbootd` serves both from the same
channel — it reads the client's DHCP vendor class (option 60) and replies in the
matching style. You only configure the boot program:

```bash
paniolo netboot set -t nova \
    --interface en7 \
    --tftp-root ~/nova/boot-root \
    --boot-file grubaa64.efi      # any UEFI NBP: grubaa64.efi, ipxe.efi, a UKI…
paniolo netboot start nova
```

**HTTP Boot.** A client whose option 60 begins `HTTPClient` (arch 19 = ARM64
UEFI HTTP) is answered with the required `HTTPClient` class echo and an
`http://<host_ip>[:<http_port>]/<boot_file>` URL in option 67, then served the
file over HTTP. In the EDK2 boot menu choose **HTTP Boot (IPv4)**. Where the
firmware allows plain HTTP it is the better transport — kernel TCP, fast, robust
under host load. `paniolo netboot logs -f nova` shows the `DISCOVER` (carrying
`HTTPClient:Arch:00019`), the offer, then `HEAD` + `GET /grubaa64.efi`.

> **Many EDK2 builds reject plain HTTP.** UEFI HTTP Boot ships with
> `PcdAllowHttpConnections=FALSE`, so the firmware demands `https://` and refuses
> netbootd's `http://` URL (it reports *"HTTPS only"*) — observed on the
> Indiedroid Nova, with no runtime toggle exposed. netbootd serves plain HTTP (no
> TLS), so on such firmware **use PXE** (below); HTTP Boot works only where the
> firmware permits plain HTTP.

**PXE (hardware-verified).** A client whose option 60 begins `PXEClient` (arch 11
= ARM64 UEFI) gets the TFTP reply, a `PXEClient` echo, **and DHCP option 43**
carrying `PXE_DISCOVERY_CONTROL=0x08` — which tells the client to boot the offered
`boot_file` directly over TFTP rather than hunting for a boot server (BINL).
Without option 43, strict EDK2 completes DHCP but then prints *"no valid offer
returned"*. Pick **UEFI PXEv4** in the boot menu; the log shows
`RRQ <boot_file> … completed`. This path is verified end-to-end on the Nova.

Because a UEFI client has a full IP/TCP/ARP stack (it answers ARP, unlike the
silent Pi bootloader), the HTTP transfer uses ordinary kernel TCP — **no
`/dev/bpf` raw-frame path, no setuid helper, no static ARP entry** — and behaves
identically on macOS and Linux.

> **Verified end-to-end via PXE/IPv4** on an Indiedroid Nova (RK3588S / EDK2),
> netbooting a UEFI Shell. **IPv6 and HTTPS are not supported** — netboot is IPv4
> + plain HTTP/TFTP over the private point-to-point link. See
> [`notes/uefi-http-boot-design.md`](https://github.com/curtisgalloway/paniolo/blob/main/notes/uefi-http-boot-design.md)
> for the design, the hardware findings, and the IPv6 future work.

---

## DHCP / TFTP behavior notes

**Lease.** The DHCP server hands the target one fixed lease, **derived from
`host_ip`**: the same /24, with the last octet replaced by `100` (or `101` when
the host itself is `.100`) — so the default `192.168.99.1` leases
`192.168.99.100`, and a host at `10.20.30.1` leases `10.20.30.100`. The lease
carries a `255.255.255.0` mask and `host_ip` as the router, matching the /24
`start` configures on the interface. `netbootd` itself accepts a `--client-ip`
override (it must be in the host's /24 and be neither the host nor the
network/broadcast address, or netbootd refuses to start); the lab file does not
yet expose it. The reply sets **both** `siaddr` (the BOOTP next-server) and
**DHCP option 66** (TFTP server name) to `host_ip`. The Pi 5 EEPROM reads option
66 preferentially, but setting both ensures compatibility with older EEPROM
firmware. Replies are broadcast to the **limited broadcast `255.255.255.255`**
(per RFC 2131), not the subnet-directed `.255` broadcast, and the DHCP socket is
pinned to the netboot interface so they still egress it. This matters for strict
clients: a UEFI IP4 stack sitting at `0.0.0.0` drops a packet addressed to a
subnet it has no address on, so it never sees a *directed*-broadcast offer — the
Pi firmware is lenient and accepts either, but EDK2 is not.

A DHCPREQUEST that asks for any address other than the lease (in option 50, or
by claiming it in `ciaddr`) is answered with a **DHCPNAK** rather than an ACK
carrying a different address, so a client holding a stale lease from elsewhere
goes back to DISCOVER; a REQUEST addressed to another server (option 54) is
ignored; and only Ethernet clients with a 6-byte hardware address are answered
at all.

**TFTP.** The TFTP server is **read-only** (RFC 1350) and negotiates
`blksize`/`tsize` options. Files are streamed from disk one block at a time
(never read whole), each retransmit attempt has a fixed one-second deadline so a
peer sending anything but the awaited ACK cannot keep a transfer alive past six
attempts, and a repeated RRQ from the same client port replaces the transfer in
flight rather than starting a parallel one. When replies go out as raw frames
(the macOS BPF path) the negotiated `blksize` is capped at 1468 bytes so every
DATA block fits one Ethernet frame. A symlink inside the TFTP root that points
outside it is refused like any other escape (TFTP `file not found`, HTTP 404):
only regular files whose real path is under the root are served.

**HTTP.** The HTTP server sends exactly the `Content-Length` it announced even
if the file changes underneath it (a file that grows is cut at the announced
length; one that shrinks drops the connection), closes HTTP/1.0 connections
after the response unless the client asks for keep-alive, cuts off a connection
that has not delivered a complete request within 10 s, and serves at most 64
connections at once. Request names are sanitized before they are logged.

Both servers log to the combined log at
`~/.local/share/paniolo/<name>/netboot.log`.

> **Switching to ffx-over-network?** With NET-first boot order, leaving netboot
> running means the next power-cycle TFTP-boots instead of falling through to
> the SD card. Use [`paniolo netif mode ffx`](netif.md) to stop netboot and
> ready the host IPv6 side in one atomic, idempotent step.

---

## Runtime paths

| Purpose | Path |
|---|---|
| Daemon state (netbootd PID, uptime) | `~/.local/share/paniolo/<name>/netboot.json` |
| Combined log | `~/.local/share/paniolo/<name>/netboot.log` |

---

## Known issue: TFTP responsiveness under host load

A heavily loaded control host can starve a TFTP server — it doesn't service
requests quickly enough and the client (e.g. the Pi 5 EEPROM) times out the
transfer. This was observed on the original Python TFTP server (since removed);
the stopgap there was to raise the server's scheduling priority (`renice` to a
negative nice value).

Future work for `netbootd`: make TFTP serving robust to host load by design
rather than relying on `renice` — e.g. run the
send path on a dedicated/elevated-priority thread, set socket priority, and keep
the per-request hot path allocation-free so latency stays bounded when the
machine is busy. (Tracked from a real starvation incident; netbootd is already
the default, so this is the right place to fix it permanently.)

Status 2026-06-04: netbootd carried a full real boot (Pi 5 firmware DHCP +
TFTP, 20 MB ZBI at ~3.9 MB/s) on an idle host; the deliberate under-load
re-test is still to be done.

For **UEFI** clients this is largely moot: prefer [HTTP
Boot](#uefi-clients-pxe-http-boot), which runs over kernel TCP with real flow
control and loss recovery, so it stays robust under host load without the
lock-step per-block ACKs that make TFTP fragile. The starvation concern applies
to TFTP clients (the Pi, and UEFI PXE) only.
