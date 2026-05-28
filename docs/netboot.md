# Netboot

paniolo netboots a target by running a minimal DHCP + TFTP server over a
direct USB-Ethernet link. No router, switch, or upstream DHCP server is involved.

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

```bash
# Create or update a target
paniolo target set target-machine \
    --interface en3 \
    --tftp-root ~/src/fuchsia/pxe/tftp-root

# Show all configured targets
paniolo target show

# Show a specific target
paniolo target show target-machine

# Remove a target
paniolo target clear target-machine
```

Target config fields:

| Field | Default | Description |
|---|---|---|
| `--interface` | (required) | USB-Ethernet interface name (e.g. `en3`) |
| `--host-ip` | `192.168.99.1` | Static IP assigned to the interface; also the TFTP server address |
| `--tftp-root` | (none) | Directory whose contents are served over TFTP |
| `--ha-power-entity` | (none) | Home Assistant switch entity for power cycling |
| `--power-serial` | (none) | Serial interface name used for DTR power control |

---

## Starting and stopping

```bash
paniolo netboot start [target-machine]
paniolo netboot stop  [target-machine]
```

`start` assigns a static IP to the interface (`sudo ifconfig`), writes a
dnsmasq config, and launches dnsmasq + tftp-now as background daemons.
`stop` sends SIGTERM to both and clears the state file.

**No root for ports 67/69:** macOS 10.14+ allows binding to `0.0.0.0` on
privileged ports without root. paniolo binds to `0.0.0.0` and uses dnsmasq's
`--interface` flag for filtering. The only step requiring sudo is `ifconfig`
to assign the static IP — configure NOPASSWD sudo on the control Mac for
unattended agent use.

---

## Status and logs

```bash
paniolo netboot status [target-machine]      # running? interface? uptime?
paniolo netboot logs   [target-machine]      # tail the combined dnsmasq + tftp log
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

## dnsmasq configuration notes

paniolo sets both `siaddr` (BOOTP next-server, via `dhcp-boot`) and DHCP
option 66 (TFTP server name). The Pi 5 EEPROM reads option 66 preferentially,
but setting both ensures compatibility with older EEPROM firmware.

DNS is disabled (`port=0`). dnsmasq log output is redirected to the combined
log file at `~/.local/share/paniolo/<name>/netboot.log`.

---

## Runtime paths

| Purpose | Path |
|---|---|
| Generated dnsmasq config | `~/.local/share/paniolo/<name>/dnsmasq.conf` |
| Daemon state (PIDs, uptime) | `~/.local/share/paniolo/<name>/netboot.json` |
| Combined log | `~/.local/share/paniolo/<name>/netboot.log` |
