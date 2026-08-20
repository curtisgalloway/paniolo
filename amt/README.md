<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
-->

# amt

Power control for **Intel AMT (vPro) machines** over WS-Management — turn the
host on, off, or power-cycle it, and read its true power state back, over the
network with no smart plug. Pure Rust via
[ureq](https://crates.io/crates/ureq); works on **macOS and Linux**.

Commands talk to the machine's **Management Engine** (ME), which runs on
standby power: it answers with the host on, off, or bare-metal with no OS
installed. That makes `state` a genuine power *sensor* — something an
outlet-side smart plug cannot provide at all.

- **Protocol:** WS-Management (SOAP over HTTP) on port **16992**, acting on
  `CIM_PowerManagementService.RequestPowerStateChange` and reading back
  `CIM_AssociatedPowerManagementService.PowerState`.
- **Auth:** HTTP **Digest** (MD5, `qop=auth`), implemented in the helper —
  AMT 11+ advertises Digest-only and rejects plaintext, which is why e.g.
  Debian's `amtterm` cannot talk to modern AMT.
- **Not supported:** TLS-provisioned AMT (port 16993). The helper speaks the
  plain WS-Man port only and says so clearly if pointed at an `https://`
  address.

## Credentials

The Digest password comes **only** from the `AMT_PASSWORD` environment
variable — never from a flag or a config file, so it cannot leak into a lab
file, shell history, or `ps` output. Inject it at call time; with 1Password:

```bash
op run --env-file .env -- bash -c 'amt state -d 192.168.99.50'
```

Single quotes matter: the parent shell must not expand `$AMT_PASSWORD` before
the wrapper sets it. The username (default `admin`) is not secret and lives in
the hook string via `-u`.

## Usage

```bash
amt -d <host> status                # firmware identity + power state detail
amt -d <host> state                 # prints exactly "on" or "off"
amt -d <host> on                    # power on, confirm by read-back
amt -d <host> off                   # power off (hard, not a graceful shutdown)
amt -d <host> cycle [--delay-ms 3000]   # off → delay → on → confirm
```

`state` prints `on` only when the host is running (PowerState 2); sleep,
hibernate, and soft-off all print `off`. `off` is the CIM "Off - Soft"
unconditional power-off — equivalent to holding the power button, not an OS
shutdown. `cycle` is built as off → delay → on (rather than the fixed CIM
power-cycle state) so the off-hold duration is controllable and matches the
other paniolo power helpers' `--delay-ms` semantics; all three mutating
commands confirm by read-back and exit non-zero if the machine did not comply.

## paniolo integration

Installed by `make install` / `paniolo setup` into the private libexec dir;
run by hand via `paniolo helper amt …`. Wire the four generic power hooks:

```bash
paniolo power set -t <target> \
    --cycle-cmd "amt cycle -d <host> --delay-ms 5000" \
    --on-cmd    "amt on -d <host>" \
    --off-cmd   "amt off -d <host>" \
    --state-cmd "amt state -d <host>"
```

See `docs/power.md` in the paniolo repository for the full recipe, including
how to provide `AMT_PASSWORD` to `paniolo power …` invocations.
