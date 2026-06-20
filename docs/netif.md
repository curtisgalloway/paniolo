# Link mode (netboot · link · ffx · off)

The USB-Ethernet link between the control host and the target is a single
point-to-point wire. `paniolo netif` owns the **host side** of that link and puts
it into one of four **mutually-exclusive** modes. Always drive the link through
`paniolo netif` — never run `ifconfig`/`ip`/`networksetup` on the interface by
hand, or you desync netif's view (and a stray address silently changes what
`netif status` reports).

| mode | Host interface | For |
|---|---|---|
| **netboot** | IPv4 `host_ip`/24 + DHCP + TFTP + HTTP | TFTP/HTTP-booting the target (NET-first boot order) |
| **link** | IPv4 `host_ip`/24 only — **no** DHCP/TFTP daemon, no ffx LL | bringing the bare link up to test it, without serving anything |
| **ffx** | IPv6 link-local `fe80::1`/64; no DHCP/TFTP | reaching a booted Fuchsia target over `ffx` (`fe80::…%<iface>`) |
| **off** | nothing paniolo set up (host IP + ffx LL released) | parking the link / testing it down |

The two boot/transport modes are mutually exclusive in the sharpest way:

| | **netboot mode** | **ffx mode** |
|---|---|---|
| Target boots from | TFTP (NET-first boot order) | SD card (firmware reads the FAT partition) |
| Transport to device | TFTP/netsvc (no shell) | SSH → RemoteControlService over `fe80::…%<iface>` |
| What you run | `paniolo netboot`, `paniolo serial`, HDMI capture | `ffx target list/show/shell/log`, `ffx component` |

Switching by hand is error-prone in two specific ways, and `paniolo netif`
exists to remove both:

1. **Forgetting `netboot stop` before an SD boot.** If the DHCP/TFTP servers are
   still running when you power-cycle, the target's NET-first boot order
   TFTP-boots whatever is in the TFTP root *instead of* falling through to the
   SD card — so you silently boot the wrong image.
2. **The missing host-side IPv6 link-local.** `ffx` reaches the device at
   `fe80::<dev-slaac>%<iface>`, but the host interface usually has no IPv6
   link-local of its own, so the connection never forms (RCS sits at `RCS:N`
   with no hint why). The fix is a single `fe80::1`/64 on the host side, which
   nothing sets up automatically and which is lost on every control-host reboot.

---

## Commands

```bash
# Switch to netboot (IPv4 + DHCP + TFTP). Same as `paniolo netboot start`,
# but first removes any ffx-mode IPv6 link-local.
paniolo netif mode netboot <target>

# Bring the bare link up: assign the host IP only, no daemon, no ffx LL.
paniolo netif mode link <target>

# Switch to ffx: stop netboot, then add the host fe80::1/64.
paniolo netif mode ffx <target>

# Tear down every mode, soft (release the host IP and the ffx LL).
paniolo netif mode off <target>

# Take the link down HARD: `mode off` + disable Wake-on-LAN + admin-down the
# interface, so the peer actually sees carrier loss.
paniolo netif down-hard <target>

# Show which mode the link is in, its carrier (physical link) state, its
# addresses, and any discovered ffx peer.
paniolo netif status <target>
```

`<target>` may be omitted when exactly one target is configured.

Every mode is **idempotent and safe to re-run**. Because the host IPv6
link-local is ephemeral (lost on a control-host reboot), re-running
`netif mode ffx` simply re-adds it. `netif mode netboot` skips a redundant start
if netboot is already running, and `netif mode off` removes only what netif set
up — never an unrelated address.

---

## What each mode does

- **netboot** — removes the `fe80::1` host link-local, then starts DHCP + TFTP
  via the normal netboot path (which still refuses a primary NIC and configures
  the IPv4 `host_ip`).
- **link** — stops netboot if running and removes the `fe80::1` link-local, then
  assigns just the IPv4 `host_ip`/24 and brings the interface up (the same
  privileged step netboot uses, minus the daemon). This is the bare host side of
  the link with nothing serving on it — use it to test that the link comes up,
  then `mode off` to take it down.
- **ffx** — stops netboot first (so the next power-cycle boots from SD), then
  enables IPv6 on the interface and adds `fe80::1`/64. On Linux this is
  `sysctl net/ipv6/conf/<iface>/disable_ipv6=0` + `ip -6 addr add`; on macOS it
  is an `ifconfig … inet6 … alias`. The privileged steps reuse the same `sudo`
  path netboot already uses.
- **off** — stops netboot, removes the `fe80::1` link-local, and clears a
  lingering `host_ip`/24.

---

## Testing the link up and down

To exercise the link itself — does it come up, does it drop — toggle between
`link` and `off` and read `netif status` each time:

```bash
paniolo netif mode link <target>     # link up: host IP assigned, no daemon
paniolo netif status <target>        # mode=link, carrier up, inet host_ip/24
paniolo netif mode off  <target>     # link down: host IP released
paniolo netif status <target>        # mode=off, inet (none)
```

**What "down" actually does — and the Wake-on-LAN wrinkle.** `mode off` (the
*soft* down) only **releases the host IP**; it does **not** force the physical
link down. `netif status` reports `carrier` separately from `mode` precisely
because the two are independent: the carrier reflects whether the PHY sees link,
and many NICs **keep the PHY energized even when the interface is administratively
down so they can receive a Wake-on-LAN magic packet** — so the peer's link LED
stays lit and `carrier` can read `up` in `off` mode.

**`netif down-hard` is the *hard* down** for when you need the target to actually
detect link loss (e.g. testing link-drop detection). It does everything `mode
off` does, then:

- **Linux:** disables WoL (`sudo ethtool -s <iface> wol d`), then admin-downs the
  interface (`sudo ip link set <iface> down`).
- **macOS:** admin-downs the interface (`sudo ifconfig <iface> down`). macOS WoL
  is a system-wide pref (`pmset womp`), not per-NIC, and USB-Ethernet adapters
  drop link on admin-down, so that's enough; unplugging the cable is always the
  unambiguous drop.

Bring the link back up afterward with `mode link` or `mode netboot` (which
re-up the interface and re-assign the host IP). **WoL stays disabled** until you
re-enable it (`sudo ethtool -s <iface> wol g`) or replug the adapter — that's
deliberate: leaving WoL off is what keeps the carrier down. A plain `mode off`
still exists for the soft case (release the address but leave the link alone).

---

## Status and finding the device

`paniolo netif status` **probes** the active mode rather than storing it, so it
is correct even after a control-host reboot clears interface state:

- netboot daemons running → `netboot`
- else the `fe80::1` host link-local present → `ffx`
- else the static `host_ip` present (no daemon, no LL) → `link`
- else → `off`

It also prints `carrier` — the physical link state, read independently of the
mode (macOS `ifconfig … status: active`, Linux `/sys/class/net/<iface>/carrier`).
`carrier` can be `up` even in `off` mode (see the Wake-on-LAN note above).

In ffx mode it also reads the interface's IPv6 neighbour table (`ip -6 neigh`,
Linux) and prints a ready-to-paste command for any discovered peer, e.g.:

```
mode    ffx
carrier up
inet6   fe80::1/64
peer    fe80::fc33:fca2:96e0:6dbe%enx00e04c08d9a0  (try: ffx target add fe80::fc33:fca2:96e0:6dbe%enx00e04c08d9a0)
```

This surfaces the target's link-local address without scraping the serial log.
If no peer is shown, power-cycle the target and wait for it to finish SLAAC.

---

## Typical ffx-over-network flow

```bash
paniolo netif mode ffx fortune       # stop netboot + ready the host IPv6 side
paniolo power-cycle fortune          # boots from SD
paniolo netif status fortune         # grab the device's fe80::…%<iface>
cd ~/src/fuchsia && ./.jiri_root/bin/ffx target list   # expect RCS:Y
```

To go back to TFTP bring-up: `paniolo netif mode netboot fortune`.

---

## Runtime paths

netif holds no state of its own — mode is derived from the live netboot daemon
state and the interface's current addresses. See [Netboot](netboot.md) for the
DHCP/TFTP daemon state and log paths.
