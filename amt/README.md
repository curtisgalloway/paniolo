<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
-->

# amt

Power control for **Intel AMT (vPro) machines** over the network, with no smart
plug. AMT (Active Management Technology) is Intel's out-of-band management
firmware. `amt` turns the host on, off, or power-cycles it, and reads its true
power state back. Pure Rust via [ureq](https://crates.io/crates/ureq); works on
**macOS and Linux**.

Commands talk to the machine's **Management Engine** (ME), a separate
controller that runs on standby power. It answers whether the host is on, off,
or bare metal with no OS installed. That makes `state` a real power *sensor*,
which an outlet-side smart plug cannot provide.

- **Protocol:** WS-Management (SOAP over HTTP) on port **16992**. It calls
  `CIM_PowerManagementService.RequestPowerStateChange` and reads back
  `CIM_AssociatedPowerManagementService.PowerState`.
- **Auth:** HTTP **Digest** (MD5, `qop=auth`), implemented in the helper.
  AMT 11+ accepts only Digest and rejects plaintext, which is why tools such as
  Debian's `amtterm` cannot talk to modern AMT.
- **Not supported:** TLS-provisioned AMT (port 16993). The helper speaks only
  the plain WS-Man port, and says so clearly if given an `https://` address.

## Credentials

The Digest password comes **only** from the `AMT_PASSWORD` environment
variable, never from a flag or a config file. That keeps it out of lab files,
shell history, and `ps` output. Inject it at call time; with 1Password:

```bash
op run --env-file .env -- bash -c 'amt state -d 192.168.99.50'
```

The single quotes matter: the parent shell must not expand `$AMT_PASSWORD`
before the wrapper sets it. The username (default `admin`) is not secret; pass
it in the hook string with `-u`.

## Usage

```bash
amt -d <host> status                # firmware identity + power state detail
amt -d <host> state                 # prints exactly "on" or "off"
amt -d <host> on                    # power on, confirm by read-back
amt -d <host> off                   # power off (hard, not a graceful shutdown)
amt -d <host> cycle [--delay-ms 3000]   # off → confirm → delay → on → confirm
```

- **`<host>`** is a hostname, IPv4 address, or bracketed IPv6 literal
  (`[fe80::1]`), optionally with a `:port` (default 16992). Anything else
  URL-shaped is rejected.
- **`state`** prints `on` only when the host is running (PowerState 2). Sleep,
  hibernate, and soft-off all print `off`. Any other reported state is an error,
  not a guess.
- **`off`** is the CIM "Off - Soft" unconditional power-off: like holding the
  power button, not an OS shutdown.
- **`cycle`** runs off → confirm → delay → on → confirm instead of using the
  fixed CIM power-cycle state. That makes the off time controllable and matches
  the `--delay-ms` meaning of the other paniolo power helpers. It powers off any
  host not already soft-off, so a sleeping or hibernating machine cold-boots
  instead of resuming.
- **`on`, `off`, `cycle`** confirm by read-back and exit non-zero if the
  machine did not comply.

## paniolo integration

`make install` / `paniolo setup` installs `amt` into paniolo's private libexec
dir. Run it by hand with `paniolo helper amt …`. Wire the four generic power
hooks:

```bash
paniolo power set -t <target> \
    --cycle-cmd "amt cycle -d <host> --delay-ms 5000" \
    --on-cmd    "amt on -d <host>" \
    --off-cmd   "amt off -d <host>" \
    --state-cmd "amt state -d <host>"
```

See `docs/power.md` in the paniolo repository for the full recipe, including
how to provide `AMT_PASSWORD` to `paniolo power …` invocations.
