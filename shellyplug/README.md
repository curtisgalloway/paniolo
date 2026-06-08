<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
-->

# shellyplug

Power control for **Shelly Gen2+ smart plugs and relays** (Plus, Pro, Gen3,
Gen4) over each device's **local HTTP RPC API** — turn an outlet on, off, or
power-cycle it, and read its state and power metering. No cloud account, no
Home Assistant, no Matter controller. Pure Rust via
[ureq](https://crates.io/crates/ureq); works on **macOS and Linux**.

Each invocation is a single stateless HTTP request (`GET /rpc/<Method>`), so
the tool is a plain one-shot — there is no daemon to run or supervise.

- **Supported:** Gen2/3/4 devices (the JSON-RPC API: `Switch.Set`,
  `Switch.GetStatus`, `Shelly.GetDeviceInfo`). The original **Gen1** devices
  use a different REST API and are not supported.
- **Auth:** only devices with authentication **disabled** (`auth_en: false`,
  the factory default) are supported for now. An auth-enabled device answers
  HTTP 401, and the tool says so plainly.

## Install

```bash
cargo install --git https://github.com/curtisgalloway/paniolo shellyplug
```

Needs a [Rust toolchain](https://rustup.rs). The binary lands in
`~/.cargo/bin/shellyplug`.

> shellyplug lives in the [paniolo](https://github.com/curtisgalloway/paniolo)
> repository (a bench-automation toolkit) but builds and runs entirely on its
> own — the command above pulls only what this crate needs.

## Quick start

Find your plug's address (a Shelly advertises an mDNS name like
`shellyplugusg4-<mac>.local`), then:

```bash
shellyplug -d 10.0.0.5 status     # device info + switch state and power
shellyplug -d 10.0.0.5 state      # prints exactly "on" or "off"
shellyplug -d 10.0.0.5 on         # switch on, confirm by read-back
shellyplug -d 10.0.0.5 off        # switch off, confirm by read-back
shellyplug -d 10.0.0.5 cycle      # off → 3 s → on → confirm (--delay-ms to change)
```

## Addressing

- **`-d <host>`** — the device address: a bare IP or hostname (`10.0.0.5`,
  `shelly.local`), optionally with a scheme or port (`http://10.0.0.5:8080`).
  Pin the IP with a DHCP reservation or use the `.local` mDNS name so a lease
  change doesn't break a saved command.
- **`[id]`** — the switch component id, default `0`. Single-outlet plugs only
  have switch `0`; multi-channel devices (e.g. a Pro 4PM) use `0..N`:
  `shellyplug -d 10.0.0.5 on 2`.

## Commands

```
shellyplug -d <host> status [id]          device info + switch state and power metering
shellyplug -d <host> state  [id]          print exactly "on" or "off"
shellyplug -d <host> on|off [id]          switch + read-back confirm
shellyplug -d <host> cycle  [id]          off → delay → on → confirm  [--delay-ms 3000]
```

`on`/`off`/`cycle` confirm the result by reading `Switch.GetStatus` back and
exit non-zero on mismatch, so a command that silently failed surfaces as an
error rather than a wrong assumption.

## macOS Local Network permission

On macOS (Sequoia and later), access to your local subnet is gated **per
application**. A freshly built `shellyplug` may fail with **`No route to host`
(EHOSTUNREACH)** even though your browser and `curl` reach the same device —
Apple-signed system binaries are exempt, a new third-party binary is not. Grant
the terminal app you run it from **Local Network** access under System Settings
→ Privacy & Security → Local Network (the first LAN access usually prompts).
The binary reaching the public internet but not a LAN device is the tell.

## License

Apache-2.0. See [LICENSE](LICENSE).
