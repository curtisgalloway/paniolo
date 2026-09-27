<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
-->

# amt

Power control for **Intel AMT (vPro) machines** over the network, with no smart
plug. `amt` turns the host on, off, or power-cycles it, and reads its true
power state back. Pure Rust via [ureq](https://crates.io/crates/ureq); works on
**macOS and Linux**.

Commands talk to the **Management Engine** (ME), which runs on standby power, so
`state` is a real power sensor, even on bare metal with no OS.

- **Protocol:** WS-Management (SOAP over HTTP) on port **16992**. It calls
  `CIM_PowerManagementService.RequestPowerStateChange` and reads back
  `CIM_AssociatedPowerManagementService.PowerState`.
- **Auth:** HTTP **Digest** (MD5, `qop=auth`). AMT 11+ rejects plaintext, so
  tools such as Debian's `amtterm` cannot talk to it.
- **Not supported:** TLS-provisioned AMT (port 16993). Given an `https://`
  address, the helper says so.

## Credentials

The Digest password is never a flag's value or a config-file entry. It comes
from the first of these that is set:

1. the `AMT_PASSWORD` environment variable;
2. `--password-file <path>` (one trailing newline dropped);
3. `--password-command <cmd>`, run by `sh -c` (`cmd /C` on Windows), whose
   stdout is the password; killed after 30 s.

```bash
amt -d 192.168.99.50 --password-command 'op read op://lab/amt/password' state
amt -d 192.168.99.50 --password-file ~/.config/paniolo/amt state
op run --env-file .env -- bash -c 'amt state -d 192.168.99.50'
```

In the `op run` form keep the single quotes, or the parent shell expands
`$AMT_PASSWORD` before the wrapper sets it. When no source works, `amt` exits
3 (not configured). Pass the username (default `admin`) with `-u`. `kvm enable`
takes its RFB password the same way: `AMT_RFB_PASSWORD`,
`--rfb-password-file`, or `--rfb-password-command`.

## Usage

```bash
amt -d <host> status                # firmware identity + power state detail
amt -d <host> state                 # prints exactly "on" or "off"
amt -d <host> on                    # power on, confirm by read-back
amt -d <host> off                   # power off (hard, not a graceful shutdown)
amt -d <host> cycle [--delay-ms 3000]   # off → confirm → delay → on → confirm
```

- **`<host>`**: a hostname, IPv4 address, or bracketed IPv6 literal
  (`[fe80::1]`), optionally with a `:port` (default 16992). Anything else is rejected.
- **`state`** prints `on` only when running (PowerState 2). Sleep, hibernate
  and soft-off print `off`; any other state is an error.
- **`off`** is the CIM "Off - Soft" unconditional power-off, like holding the
  power button.
- **`cycle`** uses the same `--delay-ms` as the other paniolo power helpers. A
  sleeping or hibernating machine cold-boots instead of resuming.
- **`on`, `off`, `cycle`** exit non-zero if the read-back shows the machine did
  not comply.

## paniolo integration

`make install` / `paniolo setup` installs `amt` into paniolo's private libexec
dir; run it by hand with `paniolo helper amt …`. Wire the four power hooks:

```bash
paniolo power set -t <target> \
    --cycle-cmd "amt cycle -d <host> --delay-ms 5000" \
    --on-cmd    "amt on -d <host>" \
    --off-cmd   "amt off -d <host>" \
    --state-cmd "amt state -d <host>"
```

Add `--password-file` or `--password-command` to each hook, or run
`paniolo power …` with `AMT_PASSWORD` set. See `docs/power.md` for the full
recipe, with systemd, 1Password and macOS Keychain examples.
