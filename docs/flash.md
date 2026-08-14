# Flash: hands-free firmware flashing over serial

The `flash` channel reflashes a target through its **serial console**, with no
buttons, no mass-storage volume, and no new wiring:

```bash
paniolo flash write dabao apps.uf2 --cycle --boot
```

power-cycles the board into its bootloader, streams the UF2 image(s) over the
console, and boots the new firmware. The transfer goes *through* the running
serialcap daemon (the [send/expect primitive](serial.md#sendexpect-scripted-console-exchanges)),
so capture keeps running: the device's per-block acks and the transfer's
start/end markers land in the serial log.

One method exists today: **`bao1x-uf2`** — the Baochip-1x (bao1x SoC, e.g. the
dabao board) `boot1` bootloader REPL, which accepts base64-encoded UF2 blocks
over the console. The protocol reference is `bao1x-boot/uf2send.py` in
[betrusted-io/xous-core](https://github.com/betrusted-io/xous-core); board and
boot-chain background is that repo's `README-baochip.md`.

---

## Configuration

Flash is a per-target singleton channel that **rides one of the target's
serial interfaces** — it has no `host` of its own; the transfer always runs
where that interface's serialcap daemon runs (remote targets work through the
normal per-channel dispatch).

```bash
# The board's boot1 console: USB CDC at 1 Mbaud (macOS: /dev/cu.usbmodem*)
paniolo serial add console -t dabao --device /dev/cu.usbmodem1101 --baud 1000000

paniolo flash set -t dabao --method bao1x-uf2 --interface console
paniolo flash show dabao
paniolo flash rm -t dabao
```

`--interface` is optional when the target has exactly one serial interface.
`paniolo discover` recognizes a connected boot1 console (USB `1d50:6196`,
"Baochip-1x") and `paniolo configure` proposes the channel.

## One-time board prep (`bootwait`)

Out of the box the board auto-boots its OS, and entering boot1 needs the PROG
button. Make it stop at the boot1 REPL on every power-up instead, **once, by
hand**:

1. Enter boot1 manually (hold PROG while plugging in USB).
2. On its console run `bootwait enable`, then **confirm with `bootwait check`**
   — enable prints *nothing* on success.

After that the board is permanently power-cycle-into-REPL, which is what
`--cycle` relies on. (`bootwait` is stored in a one-way hardware counter, so
each toggle spends counter increments — fine for a dev board, not something to
automate.) With bootwait enabled, the only path from REPL to OS is boot1's
`boot` command — a plain `reset` lands back in the REPL, because the warm-boot
flag it checks is OS-managed and still clear during flashing. That's why
`--boot` sends `boot`, not `reset`.

## Flashing

```bash
# The common dev loop: reflash the app image and boot it
paniolo flash write dabao apps.uf2 --cycle --boot

# Multiple images, flashed in argument order (loader → xous → apps)
paniolo flash write dabao loader.uf2 xous.uf2 apps.uf2 --cycle --boot

# REPL already up (e.g. you just power-cycled): no --cycle needed
paniolo flash write dabao apps.uf2
```

- `--cycle` power-cycles via the target's **power channel** (prefers
  `cycle_cmd`, falls back to `off_cmd` → `on_cmd`), then probes with `echo`
  nonces until the REPL answers (up to 30 s — USB CDC re-enumeration takes a
  moment). A timeout names the likely cause: bootwait not enabled.
- `--boot` sends `boot` after the last file.
- The serialcap daemon is started automatically if it isn't running (a stale
  one is restarted — an old binary lacks the `/expect` endpoint).
- Every image is validated locally (512-byte multiple, per-block UF2 magics)
  **before** any power or REPL action.
- Any block that fails all its attempts fails the whole transfer (exit ≠ 0) —
  there is deliberately no "mostly worked" (uf2send.py exits 0 after up to 4
  failed blocks; paniolo does not copy that).

Expect roughly minutes for a full 4 MiB image (~512-byte blocks, one ack per
block at 1 Mbaud); the common apps.uf2 dev loop is well under a minute.
Progress prints every ~10%.

### What the transfer does (bao1x-uf2)

1. `localecho off` twice (paced per byte — boot1's echo processing lags and
   the first command after connect is sometimes garbled), then a settle delay;
   restored to `on` afterwards even on failure.
2. `echo <nonce>` probe — proves the REPL is pumping. Nonces are unique per
   attempt because queued console writes can burst out when the port reopens.
3. **`has-crc` variant probe.** A second protocol variant exists behind boot1's
   `uf2-spim` feature (external-memory boards, e.g. baosec): 2-arg `uf2`
   (block + CRC-32) plus `uf2_flush`. `has-crc` prints `true` only there. On
   `true`, paniolo aborts with a precise error instead of letting the mismatch
   surface as thousands of per-block retry timeouts. (Expected on dabao: the
   plain 1-arg variant.)
4. Per block: `uf2 <base64(512-byte block)>`, then wait (0.5 s) for the ack
   alternation — `Wrote <N> to 0x<ADDR>` or a named error (`Invalid write
   address` / `Corrupt base64` / `CRC error` / `Command not recognized`), so
   protocol errors fail fast with a diagnosis. The `Wrote` fields are
   **validated against the block's payload size and target address** — that,
   not the regex, is what makes a late ack landing in the next block's window
   self-correcting. 3 attempts per block; the transfer aborts after 5 failed
   blocks.

> **The ack means "address accepted", not "write verified".** boot1 prints
> `Wrote …` even when the ReRAM write errors (the error goes only to the debug
> UART), and normal builds have no readback. Post-flash verification is: does
> it boot. Corollary: normal boot1 can write loader/xous/apps images but **not
> boot1 itself** (that needs the alt-boot1 procedure in `README-baochip.md` —
> out of paniolo's scope).

### Remote targets

`flash write` against a target whose serial interface lives on another control
host ships the UF2 file(s) to that host (over the same SSH transport as the
lab slice), re-executes there, and cleans up. `--cycle` composes across hosts:
when the power channel lives on a *different* host than the serial interface,
the power step runs first from the dev machine, then the transfer is
dispatched.

---

## Troubleshooting

- **"boot1 REPL did not answer"** — the board probably booted its OS
  ([`bootwait enable`](#one-time-board-prep-bootwait) not done), or the
  console device/baud is wrong (boot1's CDC console is 1 Mbaud). Early boot
  banners print *before* USB CDC is up and only reach the hardware UART
  (PB14/PB13), so never expect a banner on the CDC console — the `echo` probe
  is the readiness signal.
- **Don't run uf2send.py against a watched port.** The serial port is
  exclusive: flashing through paniolo needs the daemon (which owns the port);
  uf2send.py needs the port to itself. To cross-check whether a failure is
  protocol or paniolo plumbing, `paniolo serial stop <target>` first, then run
  uf2send.py directly.
- **`serial dtr` / the dashboard power button return 409 during a transfer** —
  by design: a DTR pulse drops and reopens the port, which would kill the
  flash mid-write.
- The transfer is legible in `paniolo serial log`: yellow `flash … start/done`
  markers bracket the device's `Wrote …` lines (~8.8k of them for a full
  4 MiB image — well inside the default 50k-line retention).

## Hardware verification status

Implemented against the boot1 sources (`bao1x-boot/boot1/src/repl.rs`) and
uf2send.py; the checklist still to confirm on a real dabao before this doc is
considered verified:

- `reset` returns to the REPL and `boot` starts the OS, with bootwait enabled.
- The shipped dabao boot1 runs the plain (1-arg) `uf2` variant (`has-crc`).
- Cold-boot-to-REPL timing (CDC enumeration + first `echo` answer) fits the
  30 s `--cycle` window.
- Real transfer timing for apps.uf2.
- `localecho off` through the daemon's paced write path behaves like
  uf2send.py's per-character flush.
