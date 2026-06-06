# Recipe: adding a power-control helper for new hardware

How to add paniolo support for a new power-switching device — a PDU, a relay
board, a smart plug family, a USB-PD hub, a BMC, anything that can turn a
target's power on and off. Two shipped helpers serve as exemplars:
[`cambrionix/`](https://github.com/curtisgalloway/paniolo/tree/main/cambrionix) (Rust, Cambrionix USB hub control UART) and
[`zigplug/`](https://github.com/curtisgalloway/paniolo/tree/main/zigplug) (Python, Zigbee smart plugs via a CC2652 coordinator
dongle).

**The design principle** (from [power.md](power.md)): device-specific control
logic never goes in the core crates. It lives in a standalone helper binary,
and paniolo drives it through four generic shell-command hooks on the target's
power channel. Adding hardware support means writing a helper and wiring it in
— no `cli/` changes beyond (optionally) the install step.

---

## 1. The hook contract

Paniolo runs each hook with `sh -c <cmd>` (`cli/src/main.rs`,
`run_power_hook`). The contract per hook:

| Hook | Run by | Contract |
|---|---|---|
| `on_cmd` | `paniolo power on` | exit 0 = success; non-zero exit code is propagated |
| `off_cmd` | `paniolo power off` | same |
| `cycle_cmd` | `paniolo power-cycle` | same; the hook owns the *full* sequence (off, delay, on, confirm) — paniolo adds no timing of its own |
| `state_cmd` | `paniolo power-state` | the **first whitespace-delimited token of stdout** must be `on` or `off` (case-insensitive); anything else, or a non-zero exit, is an error. Takes precedence over serial sense-line state when configured |

Environment the helper must tolerate:

- **`sh -c`, no shell profile.** The command string is evaluated by `sh` with
  the paniolo process's PATH **plus the private libexec dir
  (`~/.local/libexec/paniolo/bin`) prepended** — so helpers installed by
  `paniolo setup` resolve by bare name without living on the user's PATH.
  Absolute paths also work. `paniolo doctor` probes both forms: `test -e` for
  absolute paths, `command -v` under the same libexec-then-PATH resolution
  for bare names.
- **Runs on the channel's control host.** Power commands re-exec over SSH on
  the host that owns the power channel (`paniolo power set --host <labhost>`).
  Install the helper on *that* host, not (only) where you type.
- **State and temp data go where paniolo says.** Every invocation carries two
  env vars, both pointing at directories that already exist:
  `PANIOLO_STATE_DIR` (`~/.config/paniolo/helpers/<name>/`) for durable state
  (databases, pairing records) and `PANIOLO_RUNTIME_DIR`
  (`/tmp/paniolo-<uid>/<name>/`) for discovery files, locks, and logs (wiped
  on reboot). `<name>` is the hook command's program basename (`zigplug …` →
  `zigplug`); channel daemons get the channel name instead (hidrig → `hid`).
  Prefer the env vars, fall back to the same literal paths when run
  standalone — and **never** write unnamespaced files into
  `~/.config/paniolo/` itself (that's where the lab file lives).
- **One-shot, stateless, exclusive.** Each invocation opens the device, acts,
  exits. If the transport is an exclusive-open serial port, two concurrent
  invocations will collide — keep any long-lived helper modes (pairing
  windows, monitors) off the hook paths.
- **stdout/stderr pass through** (except `state_cmd`, whose stdout is
  captured and parsed). Print something useful on success; print errors to
  stderr and exit non-zero on failure.

## 2. Helper CLI conventions

Mirror the existing helpers so hooks read uniformly across hardware:

```
<helper> -d <device> on <id>                  # switch on; confirm if the hw can report
<helper> -d <device> off <id>                 # switch off; confirm
<helper> -d <device> state <id>               # print exactly "on" or "off"
<helper> -d <device> cycle <id> [--delay-ms 3000]   # off → delay → on → confirm
<helper> -d <device> state                    # (optional) human-readable table of all ids
```

- `-d/--device` is the transport (serial port path, IP, hub address); `<id>`
  selects the outlet/port/plug (hub port number, IEEE address, outlet index).
  Both live in the hook string in the lab file, so the helper itself stays
  configuration-free.
- **Confirm by read-back wherever the hardware can report state.** `on`/`off`
  should verify the result and exit non-zero on mismatch (`zigplug` reads the
  OnOff attribute back; `cambrionix` re-reads the port table after `cycle`).
  A power-cycle that silently failed costs a whole debugging session.
- `cycle` defaults to a 3000 ms off-hold (both exemplars) — long enough for
  target PSU caps to drain.
- Device lifecycle commands beyond the contract are fine (`zigplug form` /
  `permit` / `list` / `remove`); keep them out of the four hook strings.

## 3. Implementation skeleton

Pick the language by ecosystem fit — Rust if the device speaks a simple
serial/HTTP protocol, Python if the driver library is Python (as with
zigpy-znp). Anything goes as long as it installs an executable into the
libexec dir (`~/.local/libexec/paniolo/bin`) — helpers stay off the user's
PATH; run one by hand with `paniolo helper <name> …`.

Whatever the language, read `PANIOLO_STATE_DIR`/`PANIOLO_RUNTIME_DIR` for
any state or temp paths (see the hook-contract section; zigplug's
`default_db_path()` and `runtime_dir()` are the reference implementations,
including lazy migration from a pre-API path).

**Rust helper (the `cambrionix` pattern):**

1. `cargo new <helper> --bin` at the repo root; Apache 2.0 headers; `clap`
   (derive) + `anyhow` + whatever transport crate (`serialport`, `ureq`).
2. `main.rs` = CLI surface + command logic; `proto.rs` = transport/protocol.
3. Add the crate name to `CRATES` in [`Makefile`](https://github.com/curtisgalloway/paniolo/blob/main/Makefile) **and** to
   `HELPER_CRATES` in `cli/src/setup.rs` so `make install` / `paniolo setup`
   build and install it into the libexec dir (`cargo install --root`).

**Python helper (the `zigplug` pattern):**

1. New top-level dir with its own `pyproject.toml` (uv project, **not** part
   of the root legacy package): `[tool.uv] package = true`,
   `[project.scripts] <helper> = "<pkg>._cli:app"`, src layout, typer CLI.
2. Wrap async device libraries with one `asyncio.run()` per subcommand;
   map library exceptions to clean one-line errors (a traceback in hook
   output reads as paniolo breakage).
3. Add an install block to `cli/src/setup.rs` following zigplug's: probe for
   `uv`, run `uv tool install --force <repo>/<helper>` with
   `UV_TOOL_BIN_DIR` pointed at the libexec dir (the shim lands there, the
   venv stays in uv's tool dir), skip with a note when uv is missing.
   Mention it in the Makefile header comment.

Either way: **install the helper before testing hooks** (`paniolo setup`, or
`cargo install --path <helper> --root ~/.local/libexec/paniolo` for a
one-off). Paniolo runs installed binaries, not repo checkouts.

## 4. Hardware verification ladder

Climb in this order — each rung isolates a layer, and the destructive test
comes last:

1. **Identify the device node first.** `ioreg -p IOUSB -w0` (macOS) /
   `lsusb` + `/dev/serial/by-id/` (Linux). Don't guess from `/dev` listings:
   some USB-serial chips (e.g. CP2102N) carry no serial number, so macOS
   names them by USB topology (`/dev/cu.usbserial-8310` ↔ location
   `08310000`) — the name changes if the dongle moves ports.
2. **Helper one-shots directly** (`paniolo helper <name> …` — helpers are
   not on PATH): any device-lifecycle setup (e.g. `paniolo helper zigplug
   form` + `permit`), then `state <id>`, `on`, `off`, `cycle`, confirming
   physically (relay click, LED, multimeter).
3. **`paniolo power-state <target>`** — read-only, proves the hook string,
   `sh -c` environment, and the `on`/`off` token contract.
4. **`paniolo power on/off <target>`** — switching through the full stack.
5. **`paniolo power-cycle <target>`** — last, it reboots the target.
6. `paniolo doctor` — confirms which hooks are configured and probes that
   each hook's program exists (absolute paths via `test -e`, bare names via
   `command -v` under libexec-then-PATH).

## 5. Wiring into a target

```bash
paniolo power set -t <target> \
    --cycle-cmd "<helper> -d <device> cycle <id>" \
    --on-cmd    "<helper> -d <device> on <id>" \
    --off-cmd   "<helper> -d <device> off <id>" \
    --state-cmd "<helper> -d <device> state <id>" \
    [--host <labhost>]        # the control host that owns the hardware
```

All four hooks are optional and independent — wire what the hardware
supports. Secrets (API tokens) come from the environment at call time, never
hardcoded in the hook string (see the Home Assistant example in
[power.md](power.md)).

## 6. Docs + PR checklist

A helper PR touches more than the helper directory:

- [ ] `docs/power.md` — a usage section: install, one-time setup, commands,
      hook-wiring example, hardware gotchas you hit
- [ ] `AGENTS.md` — directory-layout entry + the power bullet in
      "Current capabilities"
- [ ] `README.md` — both helper lists (the power row in the subsystem table
      and the `make install` paragraph) + the manual install command block
- [ ] `Makefile` — `CRATES` (Rust) or the header comment (other)
- [ ] `docs/README.md` — the Power row in the subsystem guide table
- [ ] Apache 2.0 headers on all new source files

## 7. Field notes (earned the hard way)

- **2.4 GHz radios hate USB 3.** zigplug's network formation failed
  reproducibly — zigpy-znp's literal "too much RF interference" error — with
  the coordinator dongle on a hub next to a USB video-capture device.
  Channel changes and NVRAM resets did nothing; a USB 2.0 extension cable
  fixed it instantly. If a radio-based helper misbehaves near capture
  hardware, move the dongle before debugging software.
- **Verify the library API against the installed version**, not memory or
  old examples — device libraries (zigpy et al.) break their APIs across
  majors.
- **One-shot for stateless transports; a daemon for stateful ones.** When
  each invocation is a self-contained request/response over a dumb transport
  (the `cambrionix` UART), one-shot is right: a few seconds of per-invocation
  cost removes a service to install, supervise, and debug. But a transport
  with *session state* needs a persistent owner: zigplug's one-shot first cut
  was unreliable by construction — every serial open toggled the CC2652's
  auto-bootloader lines and reset the radio (occasionally *into* the
  bootloader, hanging the client), and two concurrent hooks interleaving on
  one ZNP session wedged the coordinator for hours and cost it its NVRAM.
  The fix is the zigplug pattern: an auto-spawned daemon owns the port and
  serializes operations with hard timeouts, while the CLI proxies
  transparently — hook strings stay one-shot-shaped either way (cf. `hidrig
  serve` for the same pattern on the KVM path).
- **Make `state` cheap and honest.** It's the hook agents poll; never cache
  on the helper side, and fail loudly rather than report a guess.
