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
| `--power-cycle-cmd` | (none) | Shell command run by `paniolo power-cycle` |
| `--power-serial` | (none) | Serial interface name used for DTR power control |

---

## Starting and stopping

```bash
paniolo netboot start [target-machine]
paniolo netboot stop  [target-machine]
```

`start` assigns the static `host_ip` to the interface, then launches paniolo's
own **pure-Python DHCP and TFTP servers** as background subprocesses
(`python -m paniolo._dhcp` and `python -m paniolo._tftp`). No external daemons
(`dnsmasq`, `tftp-now`) are required at runtime. `stop` sends SIGTERM to both
and clears the state file.

**Privileged ports (67/69):** macOS 10.14+ allows binding `0.0.0.0` on
privileged ports without root, so on macOS the only step needing sudo is
assigning the static IP. On **Linux**, ports 67/69 require root, so `start`
auto-prepends `sudo` when spawning the two servers, and interface configuration
(`ip addr add`) uses sudo as well. Configure **NOPASSWD sudo** on the control
host for unattended agent use.

**Interface safety:** `start` **refuses** an interface that carries your system
default route (a primary NIC). netboot reconfigures the interface to the static
`host_ip`, which would break your real networking — the netboot link must be a
dedicated USB-Ethernet adapter.

### Experimental rust engine

```bash
paniolo netboot start --engine rust [target-machine]
```

`--engine rust` launches a single `netbootd` binary (a Rust port of the two
Python servers) instead of the `_dhcp`/`_tftp` subprocesses. The default stays
`python`; the rust engine is **opt-in for validation** before any reconciliation.
`stop`/`status`/`logs` follow whichever engine `start` recorded.

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

## DHCP / TFTP behavior notes

The DHCP server hands the target a fixed lease and sets **both** `siaddr` (the
BOOTP next-server) and **DHCP option 66** (TFTP server name) to `host_ip`. The
Pi 5 EEPROM reads option 66 preferentially, but setting both ensures
compatibility with older EEPROM firmware. The TFTP server is **read-only**
(RFC 1350) and negotiates `blksize`/`tsize` options. Both servers log to the
combined log at `~/.local/share/paniolo/<name>/netboot.log`.

---

## Runtime paths

| Purpose | Path |
|---|---|
| Daemon state (DHCP/TFTP PIDs, uptime) | `~/.local/share/paniolo/<name>/netboot.json` |
| Combined log | `~/.local/share/paniolo/<name>/netboot.log` |
