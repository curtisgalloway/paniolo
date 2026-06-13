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
(verified end-to-end on the Nova). See [UEFI clients](#uefi-clients-pxe--http-boot)
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

Config lives in the lab file (see [config-redesign.md](https://github.com/curtisgalloway/paniolo/blob/main/docs/config-redesign.md));
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
| `--interface` | (required) | USB-Ethernet interface name (e.g. `en3`) |
| `--host-ip` | `192.168.99.1` | Static IP assigned to the interface; also the TFTP/HTTP server address |
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
`tftp-now`) are required at runtime. `stop` sends SIGTERM and clears the state
file.

**Privileged ports (67/69, and 80 by default):** macOS 10.14+ allows binding
`0.0.0.0` on privileged ports without root, so on macOS the only step needing
sudo is assigning the static IP. On **Linux**, ports 67/69 (and 80) require
root, so `start` auto-prepends `sudo` when spawning `netbootd`, and interface
configuration (`ip addr add`) uses sudo as well. Configure **NOPASSWD sudo** on
the control host for unattended agent use. To avoid privileged-port binds for
HTTP entirely, set `--http-port` to an unprivileged high port (e.g. `8080`) — it
is embedded in the boot URL, so the UEFI client follows it.

**Interface safety:** `start` **refuses** an interface that carries your system
default route (a primary NIC). netboot reconfigures the interface to the static
`host_ip`, which would break your real networking — the netboot link must be a
dedicated USB-Ethernet adapter.

### The netbootd engine

`netbootd` was ported from a pure-Python `_dhcp`/`_tftp` subprocess pair, which
survives only in the legacy Python CLI (`src/paniolo/`, being retired) — the
Rust `paniolo` always runs `netbootd`.

On macOS, netbootd's raw-frame send path (the Sequoia delivery workaround) needs
a `/dev/bpf` descriptor. Rather than run the daemon as root, `paniolo setup`
installs a tiny **setuid-root** helper, `netbootd-bpf-helper`, whose only job is
to open `/dev/bpf`, bind the interface, and hand the descriptor to the
unprivileged `netbootd`. It is the only paniolo component that runs as root. If
it is missing or not setuid, the rust engine logs a warning and falls back to
the kernel send path (which is unreliable on macOS 15+). Run `paniolo setup`
(one sudo) to install it.

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
> [`docs/uefi-http-boot-design.md`](https://github.com/curtisgalloway/paniolo/blob/main/docs/uefi-http-boot-design.md)
> for the design, the hardware findings, and the IPv6 future work.

---

## DHCP / TFTP behavior notes

The DHCP server hands the target a fixed lease and sets **both** `siaddr` (the
BOOTP next-server) and **DHCP option 66** (TFTP server name) to `host_ip`. The
Pi 5 EEPROM reads option 66 preferentially, but setting both ensures
compatibility with older EEPROM firmware. Replies are broadcast to the **limited
broadcast `255.255.255.255`** (per RFC 2131), not the subnet-directed `.255`
broadcast, and the DHCP socket is pinned to the netboot interface so they still
egress it. This matters for strict clients: a UEFI IP4 stack sitting at `0.0.0.0`
drops a packet addressed to a subnet it has no address on, so it never sees a
*directed*-broadcast offer — the Pi firmware is lenient and accepts either, but
EDK2 is not. The TFTP server is **read-only** (RFC 1350) and negotiates
`blksize`/`tsize` options. Both servers log to the combined log at
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

On a heavily loaded control host the **Python** legacy TFTP server has been
observed to starve — it doesn't service requests quickly enough and the
client (e.g. the Pi 5 EEPROM) times out the transfer. A stopgap is to raise the
server's scheduling priority (`renice` to a negative nice value).

Future work for the **Rust `netbootd`** default engine: make TFTP serving
robust to host load by design rather than relying on `renice` — e.g. run the
send path on a dedicated/elevated-priority thread, set socket priority, and keep
the per-request hot path allocation-free so latency stays bounded when the
machine is busy. (Tracked from a real starvation incident; netbootd is already
the default, so this is the right place to fix it permanently.)

Status 2026-06-04: netbootd carried a full real boot (Pi 5 firmware DHCP +
TFTP, 20 MB ZBI at ~3.9 MB/s) on an idle host; the deliberate under-load
re-test is still to be done.

For **UEFI** clients this is largely moot: prefer [HTTP
Boot](#uefi-clients-pxe--http-boot), which runs over kernel TCP with real flow
control and loss recovery, so it stays robust under host load without the
lock-step per-block ACKs that make TFTP fragile. The starvation concern applies
to TFTP clients (the Pi, and UEFI PXE) only.
