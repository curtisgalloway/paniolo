# Netboot

paniolo netboots a target over a direct USB-Ethernet cable. It runs its own
small DHCP server (hands the target an address), TFTP server (a simple
file-transfer protocol that boot ROMs speak), and HTTP server. No router,
switch, or upstream DHCP server is involved.

One configuration serves three kinds of client. netbootd picks the reply from
the client's DHCP vendor class (option 60):

| Client | How it boots | Served over |
|---|---|---|
| Raspberry Pi 5 bootloader | no vendor class → legacy reply | TFTP |
| UEFI **PXE** client (e.g. EDK2 on an Indiedroid Nova) | `PXEClient` → bootfile + `PXEClient` echo | TFTP |
| UEFI **HTTP Boot** client | `HTTPClient` → `http://` URL + `HTTPClient` echo | HTTP |

PXE is the standard firmware netboot (DHCP, then TFTP); HTTP Boot is the UEFI
variant that fetches the boot file over HTTP. For UEFI clients, HTTP Boot is
the better transport **where the firmware allows plain HTTP**: kernel TCP is
fast, loss-tolerant, robust under host load, and needs none of the macOS
raw-frame machinery the silent Pi bootloader needs. Many EDK2 builds allow only
HTTPS and reject our `http://` URL; on those, use **PXE** (verified end-to-end
on the Nova). See [UEFI clients](#uefi-clients-pxe-http-boot).

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

Config lives in the lab file (see [config-redesign.md](https://github.com/curtisgalloway/paniolo/blob/main/notes/config-redesign.md)).
The netboot link is a per-target `netboot` channel:

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
| `--host-ip` | `192.168.99.1` | Static IP assigned to the interface; also the TFTP/HTTP server address and the router the client is told about. The client's lease is derived from it (same /24, last octet `100` — `192.168.99.100` by default) — see [Lease](#dhcp-tftp-behavior-notes). **The default is for the first link on a host only**: every further link needs its own /24 — see [One subnet per link](#one-subnet-per-link) |
| `--tftp-root` | (none) | Directory whose contents are served over TFTP **and** HTTP |
| `--boot-file` | `kernel_2712.img` | Boot program (filename under the root, e.g. `grubaa64.efi`); served as a TFTP filename to PXE and wrapped in an `http://` URL for HTTP Boot |
| `--http-port` | `80` | HTTP server port; also embedded in the HTTP Boot URL (omitted from the URL when 80). `0` binds an OS-assigned ephemeral port — the URL always carries the port actually bound, never a literal `0` |
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

**Naming the target.** `[target-machine]` may be omitted when exactly one
target is configured. Every runtime verb (`start`, `stop`, `status`, `logs`,
`tftp-root`) also accepts it as `-t/--target`, so `paniolo netboot stop -t
target-machine` and `paniolo netboot stop target-machine` are the same command.
Giving both at once is refused. The config verbs (`set`, `rm`) still require
`-t` and take no positional.

**`start`** assigns the static `host_ip` to the interface, then launches
`netbootd`: paniolo's own single Rust binary that serves DHCP, TFTP, and HTTP
from one background process. No external daemons (`dnsmasq`, `tftp-now`) are
needed at runtime.

**`start` confirms the daemon came up.** netbootd validates its configuration
and binds every listener *before* serving anything, so a port already in use,
an interface it cannot pin, or a bad client IP shows up as an early exit.
`start` watches the new process for about two seconds before recording it as
running. If it exits in that window, `start` fails with the last 20 lines of
its log (`netbootd exited with … during startup; last lines of
~/.local/share/paniolo/<name>/netboot.log: …`) and writes no state file, so
`netboot status` never reports a daemon that is not there.

**`stop`** sends SIGTERM (via `sudo kill` when the daemon is root's), waits up
to 3 s, SIGKILLs a holdout and waits 2 s more, then clears the state file and
restores the interface. It only signals a pid whose command line still names
`netbootd`; a recorded pid the kernel has since reused is just forgotten. If
the daemon survives even SIGKILL, `stop` fails and keeps the state file rather
than report a stop that did not happen.

**One netboot per interface.** `start` refuses to start a second target on an
interface where another target's netbootd is already alive (`netboot for
'<other>' is already running on <iface> … stop it first`). Two servers on one
link would only fight over the DHCP/TFTP ports; give each target its own
adapter.

### One subnet per link

Several targets can netboot from one control host at once, one adapter each,
**as long as every link is in its own /24.**

**Why:** netbootd's own sockets are pinned to their interface, so two daemons
on two adapters coexist even at the same address. But everything else that
dials a target address (ssh into the booted target, the AMT helper, ffx,
fastboot) uses the routing table, and a kernel with two interfaces in
`192.168.99.0/24` sends that traffic out whichever it listed first. The
symptom is a target that is reachable only some of the time, or only after a
hand-added host route.

**The rule:** the default `192.168.99.1` is for the **first** netboot link on a
host. Every further link on that host sets `--host-ip` in an unused /24
(`192.168.100.1`, `192.168.101.1`, …). The client lease follows the host IP
into that /24.

paniolo enforces this in four places:

- **`netboot set` refuses** a link whose /24 another target's link already
  uses on a different interface of the same host, including a link left at the
  default: `target 'pi4' netboot: host_ip 192.168.99.1 (the default, since
  none is set) puts eth4 in 192.168.99.0/24, which target 'optiplex' already
  uses on eth3 of host 'bench1'`. A lab file that already has a clash still
  loads (so `netboot stop` keeps working); `paniolo doctor` reports it as
  `CONFLICT`.
- **`netboot start` and `netif mode link` refuse** to assign the address when
  any *other* interface on the host holds that /24 right now, whether paniolo
  put it there (a target left in `mode link`) or someone did by hand:
  `refusing to put 'eth4' in 192.168.99.0/24: 'eth3' already holds
  192.168.99.1 on this host`. Release the other link (`paniolo netif mode off
  <target>`) or give this one its own subnet.
- **`paniolo doctor` reports a link running somewhere else** as `MISMATCH`:
  the interface holds IPv4 addresses and the effective host IP is not among
  them. The lab file only says where the link *should* be, and netbootd
  re-applies that address only while it runs. So a stale `netif mode link`
  from an older config, an address set by hand, or an edit made while the
  daemon was down can leave the link elsewhere indefinitely, silently. Two
  cases are deliberately **not** a mismatch:
    - an interface with no IPv4 at all (`netif mode off`): the link is down,
      not misplaced;
    - one holding the configured address *alongside* another: it serves where
      it should, and the extra address is a routing question this check does
      not judge.

    The message gives the remedy: `paniolo netif mode link <target>`, or
    `netboot start <target>`. Either assigns the configured address and drops
    the others.
- **`target show` and `netboot status` always print the host IP**, marked
  `(default)` when the field is unset, so the address a link runs at is never
  hidden.

Two targets that share one adapter (a bench slot re-cabled between boards) may
keep the same address. The clash is between *interfaces*, and the
per-interface daemon check above already stops them running together.

> **macOS control hosts run one netboot at a time.** `IP_BOUND_IF` (the macOS
> pin) does not split the bind namespace the way Linux's `SO_BINDTODEVICE`
> does, so a second netbootd's `bind` of port 67 fails with `EADDRINUSE`
> even on a different adapter and subnet. `SO_REUSEPORT` would lift that, but
> it would also let two daemons on the *same* interface share the port
> silently, which netbootd deliberately refuses today. Not fixed; the subnet
> rule still applies there so the links stay routable.

### Privileges and sudo

**Privileged ports (67/69, and 80 by default).** macOS 10.14+ lets any user
bind `0.0.0.0` on privileged ports, so on macOS only assigning the static IP
needs sudo. On **Linux**, ports 67/69 (and 80) need root: `start` prepends
`sudo` when spawning `netbootd`, and interface setup (`ip addr add`) uses sudo
too. Configure **NOPASSWD sudo** on the control host for unattended agent use.
To avoid a privileged bind for HTTP, set `--http-port` to a high port (e.g.
`8080`); it is embedded in the boot URL, so the UEFI client follows it.

**netbootd gives root back (Linux).** Root is only needed to bind the low ports
and pin the sockets. Once that is done, before the first packet is served,
netbootd drops to the user who ran `sudo` (from the `SUDO_UID`/`SUDO_GID` sudo
sets) with `setgroups`/`setgid`/`setuid`, and refuses to start if any of the
three fails. The log records `dropped privileges to uid N gid M`. The ARP pin
(`ip neigh`) and the interface-IP monitor (`ip addr add`) then run through
`sudo` *as that user*, which is why passwordless sudo is not optional on
Linux. Started as root directly (no `sudo`, so no `SUDO_UID`), netbootd stays
root and says so in its log. On macOS netbootd never has root (the setuid
helper below holds the only privilege), so nothing changes there.

**HTTP is optional; DHCP and TFTP are not.** If the HTTP port cannot be bound
(say something else owns port 80), netbootd logs a warning naming the port and
keeps serving DHCP + TFTP, so the Pi and UEFI PXE paths still work. HTTP Boot
is then unavailable: an `HTTPClient` DHCP request gets no offer at all (with a
rate-limited warning in the log) rather than an `http://` URL pointing at a
missing server. EDK2's `HttpBootDxe` would reject a non-HTTP reply anyway,
since it requires the `HTTPClient` class echo before accepting an offer. Free
the port or set `--http-port` to an unused one. A DHCP or TFTP socket that
cannot be bound or pinned is fatal.

**Interface safety.** `start` **refuses** an interface that carries your system
default route (a primary NIC). netboot reconfigures the interface to the
static `host_ip`, which would break your real networking, so the netboot link
must be a dedicated USB-Ethernet adapter.

### Interface pinning

netbootd answers every DHCP DISCOVER it hears with a lease, and hands whatever
is in the TFTP root to anyone who asks, so it must only ever hear the netboot
link. Every listen socket (DHCP 67, TFTP 69, HTTP) is **pinned to the netboot
interface** before it is bound: `IP_BOUND_IF` on macOS, `SO_BINDTODEVICE` on
Linux. Requests arriving on any other interface (your office LAN on the
primary NIC) are never seen, and replies, including the limited-broadcast DHCP
offers, can only leave via the netboot link.

A pin that cannot be applied is fatal: netbootd refuses to start rather than
serve unpinned, which is why `netbootd`'s `--interface` flag is required. The
sockets still bind the wildcard address, so they keep working through link
flaps when the interface IP is briefly gone.

**TFTP reply sockets are pinned too, on both platforms.** Each TFTP transfer
answers from its own socket on a fresh ephemeral port, not the port-69 listen
socket, so it needs its own pin: `IP_BOUND_IF` on macOS, `SO_BINDTODEVICE` on
Linux. On Linux the reply socket is created after netbootd has dropped root
(see [netbootd gives root back](#privileges-and-sudo) above); that works
because `SO_BINDTODEVICE` has not needed `CAP_NET_RAW` since Linux 5.7, which
covers every kernel paniolo targets (Pi OS Trixie 6.x, Debian 12 6.1). On an
older kernel the pin fails with `EPERM`/`EACCES`; netbootd logs one `warn!`
naming the interface and falls back to binding the reply socket to the
interface IP alone and letting the kernel route, rather than failing the
transfer. Either way the reply socket binds to `host_ip`, not the wildcard, so
its source address is the one the client dialed.

Without this pin, two netboot links in the same `/24` (paniolo's default
`192.168.99.1` on both) send every OACK and DATA block out of whichever
interface owns the kernel route for that subnet, not the one the RRQ arrived
on. The client retransmits its RRQ forever and the log repeats `no ACK for
OACK`. **To verify:** bring up two netboot-capable links both at
`192.168.99.1/24` — put one target in [`netif mode
link`](netif.md#testing-the-link-up-and-down) so it holds the IP without
running a daemon — then `paniolo netboot start` a target on the *other* link
and PXE-boot it. Without the pin the daemon log repeats `no ACK for OACK`;
with it, the log shows `completed <file>`.

**Just the link, no daemon.** `start`/`stop` bring the link up *and* run (or
stop) the DHCP/TFTP server together. To bring the **bare link** up or down on
its own — assign or release the host IP without serving anything, e.g. to test
that the link comes up and drops — use [`paniolo netif mode link`](netif.md)
and `paniolo netif mode off`. "Down" only releases the host IP; it does not
force the physical carrier down (a NIC with Wake-on-LAN enabled keeps the link
energized). See [Link mode](netif.md#testing-the-link-up-and-down).

### The netbootd engine

`netbootd` is the only netboot engine: a single Rust binary serving DHCP,
TFTP, and HTTP.

On macOS, netbootd's raw-frame send path (the workaround for packet delivery
on Sequoia) needs a `/dev/bpf` descriptor. BPF is the kernel interface for
sending and capturing raw Ethernet frames. Rather than run the daemon as root,
`paniolo setup` installs a tiny **setuid-root** helper, `netbootd-bpf-helper`,
whose only job is to open `/dev/bpf`, bind the interface, and hand the
descriptor to the unprivileged `netbootd`. It is the only paniolo component
that runs as root. If it is missing or not setuid, netbootd logs a warning and
falls back to the kernel send path, which is unreliable on macOS 15+. Run
`paniolo setup` (one sudo) to install it.

Because the helper is setuid-root and any local user can run it, it is
restricted:

- **Only the installing user may invoke it.** It refuses any caller whose real
  uid is not the owner of the directory it was installed into (your private
  libexec dir, or the Homebrew keg); root is excepted. Another user gets
  `refused: caller uid N is not the installing user (uid M)` and no
  descriptor.
- **It refuses the default-route interface.** Asking it to bind the primary
  NIC (the one `route -n get default` names) yields `refused: <iface> carries
  the default route`; the netboot link must be a dedicated secondary adapter.
- **The descriptor is write-only, with a reject-all filter.** It is opened
  `O_WRONLY`, so `read(2)` on it fails, and a `BIOCSETF` program that accepts
  nothing is installed before it leaves the helper. The daemon can inject
  frames on the netboot interface and nothing else; it cannot capture.

When the handoff fails, `netbootd` logs the helper's exit status and message
(`BPF handoff failed (netbootd-bpf-helper exited with exit status: 1:
refused: …)`), so a refused or not-setuid helper is diagnosable from the log
alone.

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

Prints the bare TFTP root path, for shell substitution:

```bash
TFTP_ROOT=$(ssh control-mac "paniolo netboot tftp-root target-machine")
scp kernel_2712.img control-mac:"${TFTP_ROOT}/kernel_2712.img"
```

---

## Expected TFTP sequence for Raspberry Pi 5

The Pi 5 EEPROM PXE client requests files in this order. The 404s are normal:

```
404  <serial>/<mac>/start.elf    ← Pi 5 doesn't need it; 404 expected
200  config.txt
200  bcm2712-rpi-5-b.dtb
200  kernel_2712.img              ← your boot shim or kernel
```

The TFTP root must contain at least `config.txt`, `bcm2712-rpi-5-b.dtb`, and
`kernel_2712.img`.

---

## UEFI clients (PXE / HTTP Boot)

UEFI firmware (e.g. Tianocore EDK2 on an Indiedroid Nova, RK3588S) can netboot
over IPv4 by **PXE** or **HTTP Boot**. `netbootd` serves both from the same
channel, replying in the style that matches the client's DHCP vendor class
(option 60). You only configure the boot program (NBP, the network boot
program the firmware downloads and runs):

```bash
paniolo netboot set -t nova \
    --interface en7 \
    --tftp-root ~/nova/boot-root \
    --boot-file grubaa64.efi      # any UEFI NBP: grubaa64.efi, ipxe.efi, a UKI…
paniolo netboot start nova
```

**PXE (hardware-verified).** Pick **UEFI PXEv4** in the boot menu. A client
whose option 60 begins `PXEClient` (arch 11 = ARM64 UEFI) gets the TFTP reply,
a `PXEClient` echo, **and DHCP option 43** carrying
`PXE_DISCOVERY_CONTROL=0x08`. That tells the client to boot the offered
`boot_file` directly over TFTP instead of hunting for a boot server (BINL).
Without option 43, strict EDK2 completes DHCP but then prints *"no valid offer
returned"*. The log shows `RRQ <boot_file> … completed`.

**HTTP Boot.** Pick **HTTP Boot (IPv4)** in the EDK2 boot menu. A client whose
option 60 begins `HTTPClient` (arch 19 = ARM64 UEFI HTTP) gets the required
`HTTPClient` class echo and an `http://<host_ip>[:<http_port>]/<boot_file>` URL
in option 67, then the file over HTTP. `paniolo netboot logs -f nova` shows the
`DISCOVER` (carrying `HTTPClient:Arch:00019`), the offer, then `HEAD` +
`GET /grubaa64.efi`.

> **Many EDK2 builds reject plain HTTP.** UEFI HTTP Boot ships with
> `PcdAllowHttpConnections=FALSE`, so the firmware demands `https://` and refuses
> netbootd's `http://` URL (it reports *"HTTPS only"*). This was seen on the
> Indiedroid Nova, which exposes no runtime toggle. netbootd serves plain HTTP
> (no TLS), so on such firmware **use PXE**.

A UEFI client has a full IP/TCP/ARP stack (it answers ARP, unlike the silent Pi
bootloader), so the HTTP transfer uses ordinary kernel TCP — **no `/dev/bpf`
raw-frame path, no setuid helper, no static ARP entry** — and behaves the same
on macOS and Linux.

> **Verified end-to-end via PXE/IPv4** on an Indiedroid Nova (RK3588S / EDK2),
> netbooting a UEFI Shell. **IPv6 and HTTPS are not supported**: netboot is IPv4
> + plain HTTP/TFTP over the private point-to-point link. See
> [`notes/uefi-http-boot-design.md`](https://github.com/curtisgalloway/paniolo/blob/main/notes/uefi-http-boot-design.md)
> for the design, the hardware findings, and the IPv6 future work.

---

## DHCP / TFTP behavior notes

**Lease.** The DHCP server gives the target one fixed lease, **derived from
`host_ip`**: the same /24, with the last octet replaced by `100` (or `101` when
the host itself is `.100`). So the default `192.168.99.1` leases
`192.168.99.100`, and a host at `10.20.30.1` leases `10.20.30.100`.

- The lease carries a `255.255.255.0` mask and `host_ip` as the router,
  matching the /24 `start` configures on the interface.
- `netbootd` itself accepts a `--client-ip` override. It must be in the host's
  /24 and be neither the host nor the network/broadcast address, or netbootd
  refuses to start. The lab file does not expose it yet.
- The reply sets **both** `siaddr` (the BOOTP next-server) and **DHCP option
  66** (TFTP server name) to `host_ip`. The Pi 5 EEPROM prefers option 66;
  setting both keeps older EEPROM firmware working.
- Replies go to the **limited broadcast `255.255.255.255`** (per RFC 2131),
  not the subnet-directed `.255` broadcast, and the pinned DHCP socket still
  sends them out the netboot interface. This matters for strict clients: a
  UEFI IP4 stack at `0.0.0.0` drops a packet addressed to a subnet it has no
  address on, so it never sees a *directed*-broadcast offer. The Pi firmware
  accepts either; EDK2 does not.
- A DHCPREQUEST asking for any address other than the lease (in option 50, or
  by claiming it in `ciaddr`) gets a **DHCPNAK** rather than an ACK with a
  different address, so a client holding a stale lease from elsewhere goes
  back to DISCOVER.
- A REQUEST addressed to another server (option 54) is ignored.
- Only Ethernet clients with a 6-byte hardware address are answered at all.

**Single client.** The lease is the *address* contract; netbootd also enforces
an *identity* one, by hardware address (MAC). The first DISCOVER, or the first
REQUEST netbootd actually answers, locks that MAC in as the active client for
as long as the process runs. A DISCOVER or REQUEST from a **different** MAC —
a second device plugged into the same netboot link — gets no reply at all (no
OFFER, ACK, or NAK), just a rate-limited warning in the log (`netboot logs`).
Retransmissions from the active MAC are unaffected.

Only a message netbootd answers can take the lock. A DHCPINFORM, DHCPDECLINE
or DHCPRELEASE, or a REQUEST addressed to another server, changes nothing
whoever sends it, so a chatty neighbor cannot claim the session with a packet
that would never have been served. There is no lease timer and no way to
release the lock short of restarting the daemon. Since `netboot start`/`stop`
already restarts `netbootd` per boot session, switching which device netboots
on a link means stopping and starting netboot again, just as switching TFTP
roots or boot files does.

**TFTP.**

- **Read-only** (RFC 1350); negotiates `blksize`/`tsize` options.
- **One source only.** It answers requests only from the one IP DHCP leases on
  this link; a request from any other source address is ignored, like a second
  DHCP client's MAC above.
- **At most `MAX_TRANSFERS` (4) transfers open at once.** A request that
  arrives when every slot is taken is dropped, not queued. A slot frees once
  its transfer completes, errors, or runs out of retransmit attempts.
- **Streaming and timeouts.** Files are streamed from disk one block at a time
  (never read whole). Each retransmit attempt has a fixed one-second deadline,
  so a peer sending anything but the awaited ACK cannot keep a transfer alive
  past six attempts.
- **Repeated RRQ.** A repeated RRQ from the same client port replaces the
  transfer in flight rather than starting a parallel one (it still counts as
  one of the `MAX_TRANSFERS` slots).
- **Block size on macOS BPF.** When replies go out as raw frames (the macOS
  BPF path), the negotiated `blksize` is capped at 1468 bytes so every DATA
  block fits one Ethernet frame.
- **No escapes.** Only regular files whose real path is under the root are
  served. A symlink inside the TFTP root that points outside it is refused like
  any other escape (TFTP `file not found`, HTTP 404).

**HTTP.** The HTTP server:

- sends exactly the `Content-Length` it announced even if the file changes
  underneath it (a file that grows is cut at the announced length; one that
  shrinks drops the connection);
- closes HTTP/1.0 connections after the response unless the client asks for
  keep-alive;
- cuts off a connection that has not delivered a complete request within 10 s;
- serves at most 64 connections at once;
- sanitizes request names before logging them.

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

A heavily loaded control host can starve a TFTP server: it does not answer
requests fast enough and the client (e.g. the Pi 5 EEPROM) times out the
transfer. This was seen in a real starvation incident with the old Python TFTP
server (since removed), where
the stopgap was raising the server's scheduling priority (`renice` to a
negative nice value).

Future work for `netbootd`: make TFTP serving robust to host load by design
rather than relying on `renice` — e.g. run the send path on a dedicated or
higher-priority thread, set socket priority, and keep the per-request hot path
allocation-free so latency stays bounded when the machine is busy.

Status 2026-06-04: netbootd carried a full real boot (Pi 5 firmware DHCP +
TFTP, 20 MB ZBI at ~3.9 MB/s) on an idle host; the deliberate under-load
re-test is still to be done.

The concern applies only to TFTP clients (the Pi, and UEFI PXE). UEFI clients
whose firmware allows plain HTTP can use [HTTP
Boot](#uefi-clients-pxe-http-boot) instead: kernel TCP has real flow control
and loss recovery, without the lock-step per-block ACKs that make TFTP
fragile.
