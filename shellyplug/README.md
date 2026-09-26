<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
-->

# shellyplug

Power control for **Shelly Gen2+ smart plugs and relays** (Plus, Pro, Gen3,
Gen4) through each device's **local HTTP RPC API**. Turn an outlet on, off, or
power-cycle it, and read its state and power metering. No cloud account, Home
Assistant, or Matter controller needed. Pure Rust via
[ureq](https://crates.io/crates/ureq); works on **macOS and Linux**.

Each run is a stateless one-shot of `GET /rpc/<Method>` requests; mutating
commands follow the switch call with a status read-back to confirm. There is no
daemon to run or supervise.

- **Supported:** Gen2/3/4 devices (the JSON-RPC API: `Switch.Set`,
  `Switch.GetStatus`, `Shelly.GetDeviceInfo`). Original **Gen1** devices use a
  different REST API and are not supported.
- **Auth:** only devices with authentication **disabled** (`auth_en: false`,
  the factory default) are supported for now. An auth-enabled device answers
  HTTP 401, and the tool says so.

## Install

```bash
cargo install --git https://github.com/curtisgalloway/paniolo shellyplug
```

Needs a [Rust toolchain](https://rustup.rs). The binary lands in
`~/.cargo/bin/shellyplug`.

> shellyplug lives in the [paniolo](https://github.com/curtisgalloway/paniolo)
> repository (a bench-automation toolkit) but builds and runs on its own. The
> command above pulls only what this crate needs.

## Quick start

Find your plug's address. A Shelly advertises an mDNS (local-network name
lookup) name like `shellyplugusg4-<mac>.local`. Then:

```bash
shellyplug -d 10.0.0.5 status     # device info + switch state and power
shellyplug -d 10.0.0.5 state      # prints exactly "on" or "off"
shellyplug -d 10.0.0.5 on         # switch on, confirm by read-back
shellyplug -d 10.0.0.5 off        # switch off, confirm by read-back
shellyplug -d 10.0.0.5 cycle      # off → confirm → 3 s → on → confirm (--delay-ms to change)
```

## Addressing

- **`-d <host>`**: the device address. A bare IP or hostname (`10.0.0.5`,
  `shelly.local`), optionally with a scheme or port (`http://10.0.0.5:8080`).
  Use the `.local` mDNS name, or pin the IP with a DHCP reservation, so a lease
  change doesn't break a saved command.
- **`[id]`**: the switch component id, default `0`. Single-outlet plugs have
  only switch `0`; multi-channel devices (e.g. a Pro 4PM) use `0..N`:
  `shellyplug -d 10.0.0.5 on 2`.

## Commands

```
shellyplug -d <host> status [id]          device info + switch state and power metering
shellyplug -d <host> state  [id]          print exactly "on" or "off"
shellyplug -d <host> on|off [id]          switch + read-back confirm
shellyplug -d <host> cycle  [id]          off → confirm → delay → on → confirm  [--delay-ms 3000]
```

`on`/`off`/`cycle` read `Switch.GetStatus` back and exit non-zero on a
mismatch, so a silent failure surfaces as an error. `cycle` confirms the off
phase before the hold begins: a relay that ignored the off command aborts the
cycle instead of reporting a "cycle" that never removed power.

## macOS Local Network permission

**Symptom:** a freshly built `shellyplug` fails with **`No route to host`
(EHOSTUNREACH)**, while your browser and `curl` reach the same device. The tell
is that the binary reaches the public internet but not a LAN device.

**Fix:** grant the terminal app you run it from **Local Network** access under
System Settings → Privacy & Security → Local Network. The first LAN access
usually prompts.

**Why:** on macOS Sequoia and later, local-subnet access is granted **per
application**. Apple-signed system binaries are exempt; a new third-party
binary is not.

## License

Apache-2.0. See [LICENSE](LICENSE).
