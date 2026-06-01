# Link mode (netboot ↔ ffx)

The USB-Ethernet link between the control host and the target is a single
point-to-point wire that serves two **mutually-exclusive** purposes during
bring-up:

| | **netboot mode** | **ffx mode** |
|---|---|---|
| Host interface | IPv4 `host_ip`/24 + DHCP + TFTP | IPv6 link-local `fe80::1`/64; no DHCP/TFTP |
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
paniolo netif mode netboot <target> --engine rust   # use the rust netbootd

# Switch to ffx: stop netboot, then add the host fe80::1/64.
paniolo netif mode ffx <target>

# Tear down both modes.
paniolo netif mode off <target>

# Show which mode the link is in, plus its addresses and any discovered peer.
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
- **ffx** — stops netboot first (so the next power-cycle boots from SD), then
  enables IPv6 on the interface and adds `fe80::1`/64. On Linux this is
  `sysctl net/ipv6/conf/<iface>/disable_ipv6=0` + `ip -6 addr add`; on macOS it
  is an `ifconfig … inet6 … alias`. The privileged steps reuse the same `sudo`
  path netboot already uses.
- **off** — stops netboot, removes the `fe80::1` link-local, and clears a
  lingering `host_ip`/24.

---

## Status and finding the device

`paniolo netif status` **probes** the active mode rather than storing it, so it
is correct even after a control-host reboot clears interface state:

- netboot daemons running → `netboot`
- else the `fe80::1` host link-local present → `ffx`
- else → `off`

In ffx mode it also reads the interface's IPv6 neighbour table (`ip -6 neigh`,
Linux) and prints a ready-to-paste command for any discovered peer, e.g.:

```
mode    ffx
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
