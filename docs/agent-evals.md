# Agent discoverability & usage evals

How well can a *naive* agent — one that has never used paniolo — go from a
plain-language goal ("boot this Pi over the network and show me the console") to
the correct paniolo commands, **without inventing commands or flailing**? This
doc specifies a reproducible eval suite that answers that question.

The suite is **no-hardware**: every scenario is either a *stated-commands*
exercise (the agent says what it would run) or a *config-only* exercise (the
agent really runs paniolo against a throwaway lab file). Both run on any machine
with the `paniolo` CLI installed; neither touches a bench target.

---

## 1. What is actually under test

Paniolo is designed to be **self-describing**: an agent that hears the word
"paniolo" is meant to reach for `paniolo --help` → `paniolo skill` →
`docs/`, and that chain should be enough to turn a goal into the right command.
The eval measures that surface, not the agent's prior knowledge.

Every scenario decomposes into two independently gradable layers:

- **Discovery** — did the agent find the right capability, via an authoritative
  source (`--help`, `paniolo skill`, the docs), *before* acting? Did it avoid
  fabricating commands like `paniolo adb reboot` or `paniolo flash`?
- **Usage** — having found the capability, did it form the correct command
  (subcommand, flags, the `-t`-vs-positional convention) and interpret the
  output?

A failure localizes the problem: weak Discovery means the help/skill/docs surface
isn't leading agents to the feature; weak Usage means the command surface itself
is confusing once found.

---

## 2. Experimental design

### 2.1 Discovery conditions (the primary independent variable)

Each scenario is run under three conditions that differ only in **what guidance
the harness pushes into the agent's context up front**. Crucially, in every
condition the bundled skills remain *pullable* via `paniolo skill` — that is the
design intent being tested (guidance available from the CLI, not pre-loaded by
the harness).

| Condition | What the agent starts with | Isolates |
|---|---|---|
| **Cold (named)** | `paniolo` on PATH; task names "paniolo"; the skill is **not** registered with the harness. The only guidance is what the agent pulls itself (`paniolo --help`, `paniolo skill`, `docs/`). | The self-describing surface, end to end. |
| **Warm (registered)** | The `paniolo` skill is **registered** so its one-line description is visible, but its body is loaded only if the agent invokes it. | The "decide to invoke the skill" step + usage. |
| **Preloaded** | The full `skills/paniolo/SKILL.md` is injected into context. Discovery is removed. | **Usage only** — the ceiling. |

The deltas between conditions are the real signal:

- **Preloaded success rate** = the *usage ceiling*. If an agent fails here, the
  command surface is at fault, not discovery.
- **Warm → Preloaded gap** = the cost of the "should I open the skill?" decision.
- **Cold → Warm gap** = the value of *registering* the skill at all.
- **Cold success rate** = how well the pure CLI self-description carries an agent
  with zero pushed guidance.

> **Nested discovery.** The `paniolo` skill points at sub-skills
> (`paniolo skill kvm-puppeting`, `paniolo skill usbhub`). In the **Warm** and
> **Preloaded** conditions, scenarios that need those (e.g. GUI puppeting) test
> whether the agent *follows the pointer* — a second discovery hop even when the
> top-level skill is in hand.

### 2.2 Execution tiers

| Tier | The agent… | Graded by | Hardware |
|---|---|---|---|
| **T0 — stated commands** | outputs the exact command sequence it would run, in order, and cites where it learned each | LLM-judge against a reference + trap list | none |
| **T1 — config-only** | really runs paniolo against an isolated temp lab file | scripted inspection of the resulting `lab.toml` + exit codes | none |

There is no T2 (hardware-in-the-loop) in this suite — runtime/hardware
capabilities (netboot start, serial watch/send, video, power-cycle, netif) are
covered at **T0** as stated commands. A separate hardware suite can promote
chosen T0 scenarios to live execution when the bench is wired up.

**T1-safe command set** (touch only the lab file or do read-only probes; safe to
execute anywhere): the lab-mutating verbs `target add/rm`, `serial add/set/rm`,
`netboot set/rm`, `power set/rm`, `video set/rm`, `hid set/rm`, `adb set/rm`,
`host add/set/rm`, plus `init`; and the read-only inspections `config show`,
`target/host/adb/video show`, `*/list`, `serial log/devices`, `netboot
tftp-root/status`, `netif status`, `discover`, `doctor`, `skill`, `configure`,
`helper` (bare list), `daemons` (bare list). Everything else (any `start`/
`watch`/`send`/`mode`/`on`/`off`/`cycle`/`console`/`setup`, or running a named
`helper`/`daemons stop`) is **T0-only**. The authoritative list is
`SAFE` in [`evals/graders/t1_config.py`](../evals/graders/t1_config.py), and
`run.py --check` asserts every key there is a real CLI group/subcommand.

### 2.3 The matrix

`scenarios × {Cold, Warm, Preloaded} × N trials`. Run **N ≥ 3** trials per cell
and report rates, not single pass/fail — agents are non-deterministic.

---

## 3. Harness setup

### 3.1 Run a *clean* agent (most important rule)

Results are worthless if the evaluating agent already knows paniolo from its
environment. Before every run:

- **No user-global instructions** — disable `~/.claude/CLAUDE.md` and any
  injected user memory (the project's own auto-memory names paniolo and its
  bench hosts).
- **Fresh project dir** — not the paniolo checkout (so the agent can't read
  `AGENTS.md`/`docs/` for free unless a scenario explicitly allows it).
- **No prior session history.**
- Note any **training-data leakage** risk: paniolo is on public GitHub, so a
  model may know it independent of the surface under test. The condition *deltas*
  remain valid even so; absolute Cold numbers should be read with this caveat.

The maintainer's own workstation is the *most* contaminated environment
possible — run evals in a clean sandbox/container or a stripped Agent SDK
session, never an interactive session on the bench machine. The `evals/` runner
does this with `--isolation home` (a sandbox `HOME` with no user `CLAUDE.md` or
memory). **macOS caveat:** if Claude Code authenticates via the Keychain, the
copyable `~/.claude/.credentials.json` may be stale and the clean-HOME agent
gets `401 Invalid authentication credentials`; supply a valid
`ANTHROPIC_API_KEY` (passed through) or mint a fresh token into the sandbox home
first. The default `--isolation light` keeps your real `HOME` (auth works) but
loads user memory — only a harness smoke test, not a valid Cold number.

### 3.2 Realizing each condition

- **Cold:** install only the `paniolo` binary; do **not** place
  `skills/paniolo/` in the harness's skills directory. `paniolo skill` still
  works (the binary ships the skills), which is exactly the path under test.
- **Warm:** register `skills/paniolo/SKILL.md` with the harness so its
  description is listed; leave `kvm-puppeting`/`usbhub` pullable-only.
- **Preloaded:** prepend the full `skills/paniolo/SKILL.md` to the system/context.

### 3.3 Lab-file isolation & safety (T1)

- Point every run at a throwaway lab: `PANIOLO_LAB=$SANDBOX/lab.toml`. Seed it
  per scenario (`paniolo init`, or a fixture file for scenarios that mutate an
  existing target).
- Wrap `paniolo` with a logging shim that records every invoked `argv`, so the
  grader can assert **no out-of-allowlist command ran** (a T1 scenario that
  shells `netboot start` should fail closed, not power-cycle a bench).
- `doctor`/`discover` are read-only but *do* enumerate the host's real hardware;
  that's fine (no mutation), just don't assert on host-specific output.

---

## 4. Scoring rubric

Score each dimension **0 / 1 / 2** unless noted. Dimensions marked *(hard-fail)*
force the whole scenario to 0 when failed.

| # | Dimension | 2 (good) | 0 (bad) |
|---|---|---|---|
| D1 | **Discovery** *(Cold/Warm only)* | Consulted an authoritative source (`--help`/`skill`/docs) fitting the goal before acting | Guessed; never consulted anything |
| D2 | **Right capability** | Chose the correct subsystem/command | Wrong subsystem entirely |
| D3 | **Command formation** | Correct subcommand, flags, and `-t`/positional convention | Malformed / wrong flags |
| D4 | **Invent-resistance** *(hard-fail)* | Used only real commands & flags | Ran or recommended a fabricated command/flag |
| D5 | **Gotcha-awareness** | Respected the scenario's documented trap | Walked into the trap |
| D6 | **Goal achievement** | T1: end-state matches; T0: the sequence would achieve the goal | Goal not met |
| D7 | **Efficiency** *(soft)* | Few/no wrong turns or redundant help reads | Extensive flailing |

**Scenario pass** = no hard-fail **and** weighted score ≥ 0.75 of max (weight D4
heavily; D7 lightly). Tune the threshold during calibration (§7).

---

## 5. Scenario catalog

Goals are phrased the way a *user* would phrase them — never in paniolo's own
vocabulary. Each scenario lists the capability exercised, the reference answer,
the trap it probes, and the grader. **Core set** (highest trap signal, run these
first): C1, C3, C7, R1, R2, R3, R4, S11, M2.

### Config-only (T1 — scripted grader on `lab.toml`)

**C1 — Provision a Pi target end-to-end.**
*Goal:* "Set up a new target called `pico`: it netboots over USB-Ethernet
interface `en7` with TFTP root `~/tftp/pico`, has a serial console on
`/dev/cu.usbserial-XYZ` at 115200 baud, and is power-cycled by the script
`~/bin/cycle.sh`. Then show me the config."
*Reference:* `target add pico` → `netboot set -t pico --interface en7
--tftp-root ~/tftp/pico` → `serial add console -t pico --device
/dev/cu.usbserial-XYZ` → `power set -t pico --cycle-cmd ~/bin/cycle.sh` →
`config show`.
*Trap:* `target add` must precede channel sets; channel commands take `-t`, not a
positional.
*Grade:* parse `lab.toml`; assert the four channels exist with the right fields.

**C2 — Add a second named serial interface.** *(fixture: `pico` exists)*
*Goal:* "Add a BMC console to `pico` on `/dev/ttyUSB1` at 9600 baud, alongside the
main console."
*Reference:* `serial add bmc -t pico --device /dev/ttyUSB1 --baud 9600`.
*Trap:* named interfaces (`bmc` ≠ default `console`); don't clobber the existing one.
*Grade:* two `[[serial]]` blocks, names `console` and `bmc`, baud 9600 on `bmc`.

**C3 — Power-cycle through a Shelly plug.** *(invent-detector)*
*Goal:* "`pico` is plugged into a Shelly smart plug at `10.66.27.141`. Make
`paniolo power-cycle pico` actually cut and restore its power."
*Reference:* discover the `shellyplug` helper (`paniolo helper`, `paniolo skill
usbhub`/power docs) and wire it into the power hooks:
`power set -t pico --cycle-cmd "shellyplug -d 10.66.27.141 cycle" --state-cmd
"shellyplug -d 10.66.27.141 state"` (and on/off).
*Trap:* there is **no** `paniolo shelly` command — smart plugs are helpers behind
generic `power` hooks.
*Grade:* `power.cycle_cmd` references `shellyplug` with the right host; no
fabricated top-level command in the transcript.

**C4 — Inspect the lab.** *(fixture: 2 targets)*
*Goal:* "What targets are configured and what hardware does each one have?"
*Reference:* `paniolo config show` (or `target show <name>` per target).
*Trap:* none — baseline inspection-discovery.
*Grade:* read-only; judge confirms it reported the real channels (no scripted lab
mutation expected).

**C5 — Drop one channel, keep the target.** *(fixture: `pico` with `console`+`bmc`)*
*Goal:* "We're not using `pico`'s BMC console anymore — remove it but keep the
main console and everything else."
*Reference:* `serial rm bmc -t pico`.
*Trap:* not `target rm pico`; not removing `console`.
*Grade:* `bmc` gone, `console` and other channels intact.

**C6 — Author a remote control host.**
*Goal:* "There's a control Mac reachable at `curtisg@bench1.local`. Add it to the
lab and propose a target block for the Pi wired to it."
*Reference:* `host add bench1 --ssh curtisg@bench1.local` (T1, mutates lab), then
`configure <target> --host bench1` (T0 — note it **proposes/prints**, writes
nothing).
*Trap:* `configure` is a propose-only step; the human pastes the block. Don't
expect it to mutate the lab.
*Grade:* `[hosts.bench1]` with the right ssh dest; judge confirms the agent
understood `configure` writes nothing.

**C7 — Bind an Android target, then reboot it.** *(invent-detector)*
*Goal:* "Add my Pixel tablet (adb serial `ABC123`) as a target named `tablet`,
and make `power-cycle tablet` reboot it."
*Reference:* `adb set -t tablet --serial ABC123`, then
`power set -t tablet --cycle-cmd "adb -s ABC123 reboot"`.
*Trap:* there is **no** `paniolo adb reboot`; reboot wires through the power hook.
*Grade:* `adb` channel + `power.cycle_cmd` invoking `adb … reboot`; no fabricated
command.

**C8 — Config-vs-reality check.** *(fixture: target pointing at absent devices)*
*Goal:* "Is `pico`'s configuration consistent with what's actually plugged into
this machine right now?"
*Reference:* `paniolo doctor` (read-only; reports each channel device present/absent).
*Trap:* `doctor`, not re-deriving by hand or running `discover` and eyeballing.
*Grade:* judge confirms it ran `doctor` and read the mismatch report it produced.

### Runtime / hardware capabilities (T0 — judge against reference)

**R1 — Netboot a kernel and read the console.**
*Goal:* "I built a new kernel at `out/kernel.img`. Get it onto the Pi, boot it
over the network, and show me the first 50 lines of boot console output."
*Reference:* `netboot tftp-root pico` (find dir) → copy `kernel.img` to
`kernel_2712.img` in it → `netboot start pico` → `serial watch pico` →
`serial log pico --tail 50`.
*Traps:* `serial log` needs a running `watch` daemon; `netboot start` refuses a
primary NIC; Pi 5 kernel filename is `kernel_2712.img`.

**R2 — Hand the link over to ffx.**
*Goal:* "I'm done with TFTP bring-up. Switch the Pi over so I can reach it with
`ffx`, and give me the address to add."
*Reference:* `netif mode ffx pico` → `power-cycle pico` → `netif status pico`
(read the ready-to-paste `ffx target add fe80::…%iface`).
*Traps:* netboot and ffx are mutually exclusive — `netif mode ffx` stops netboot
first so a power-cycle falls through to SD; the address comes from `netif status`,
not from scraping the serial log.

**R3 — Power-cycle and read the boot screen.**
*Goal:* "The Pi seems hung after my last change. Power-cycle it, wait for the boot
screen, and tell me what it says."
*Reference:* `power-cycle pico` → `video watch pico` → `video read pico --stable`
(or `video shot --stable` then read).
*Traps:* use `--stable` after a reboot; OCR is weak on tiny console fonts; **do
not change the target's console font** to improve OCR (the docs forbid it — other
agents rely on it).

**R4 — Type a command into the console.**
*Goal:* "Run `uname -a` on the Pi's serial console and show me the output."
*Reference:* `serial watch pico` → `serial send pico "uname -a"` →
`serial log pico --since <seq>`.
*Traps:* `serial send` goes through the *running* `watch` daemon (start it first);
`serial send` takes the target as a positional (two positionals = `<target>
<text>`); input only lands if the target's console actually reads the UART.

**R5 — Wait efficiently for the screen to change.**
*Goal:* "Boot the board and let me know the moment the boot screen actually comes
up — don't just sleep and guess."
*Reference:* `video shot` (note the `hash=` on stderr) → `video shot
--changed-since <hash> --timeout <ms>` to block until the frame differs.
*Trap:* discover the `--changed-since`/`--timeout` hash mechanism rather than
polling `video read` in a busy loop.

**R6 — Click through a BIOS menu.** *(nested-discovery)*
*Goal:* "On this UEFI board I can only see the screen over capture and type/click
through the emulated keyboard+mouse. Walk into BIOS setup and enable network
boot."
*Reference:* follow the pointer to `paniolo skill kvm-puppeting`; use the `video`
+ `hid` channels (`video read`/`shot` to see, `hid send … moveabs/click/key`),
applying the look-act-settle-verify loop.
*Trap:* this needs the *companion* skill; absolute-mouse `moveabs` in 0..32767
logical space, not pixel coords.

**R7 — A wedged daemon holds the port.**
*Goal:* "I can't open the serial console — something already has the port. Find
what and clear it."
*Reference:* `paniolo daemons` (lists daemons + stray libexec helpers) →
`paniolo daemons stop <name>` (or `--all`/`--force`).
*Trap:* serial ports are exclusive (one of `connect`/`watch`/external `tio`);
strays are surfaced by `daemons`, not `ps`-guessing.

**R8 — Is the target powered on right now?**
*Goal:* "Before I start, tell me whether `pico` is powered on — don't change it."
*Reference:* `paniolo power-state pico` (reads the configured `state_cmd` or the
serial sense line).
*Trap:* this is a *read* — not `power on`/`power-cycle`; it can only answer when a
`state_cmd` or a `power_sense_signal` is configured.

### Serial — the operating workhorse (s1–s11): T0 + one T1; executable on Linux

Driving the serial console is most of how an agent uses paniolo, so it gets a
dedicated, detailed cluster. `s1` is config (T1, scripted); `s2`–`s11` are
*operating* scenarios, stated-command/judge by default, with the `[loopback]`
ones runnable for real on Linux (see below).

| ID | Goal | What it tests | Key traps |
|---|---|---|---|
| **s1** | Configure `dut` with a main `console` (CTS sense) + a `bmc` @9600, with DTR enabled on `console` | dual named interfaces + DTR opt-in | `--sense` → `power_sense_signal`; DTR is opt-in via `serial … --power-button`, not `--sense` |
| **s2** | "Start capturing the console, show the last 30 boot lines." | the capture+read basics | `watch` before `log`; sole interface → no `-i`; `log` reads disk |
| **s3** | "Poll for *new* lines after the ones you've seen — don't re-dump." | the canonical agent poll loop | note the stable `seq`, then `log --since <seq>`; `*` marks the unterminated current line |
| **s4** | "Run `cat /proc/version` on the console and show its output." | send + capture response | `send` needs the running `watch` daemon; positional target; CR appended by default; input lands only if the console reads the UART |
| **s5** | "Interrupt a slow polled U-Boot autoboot with one keypress." | slow-console input | `--no-newline` (bare key); `--pace-ms` for a polled UART with no flow control |
| **s6** | "Read the last 20 lines of the BMC console specifically." | multi-interface selection | one `watch` owns all interfaces; `-i` required with >1 (omitting errors + lists names) |
| **s7** | "Re-read lines #840–#870 as machine-readable JSON." | range + parse | `--from/--to` seq range; `--json`; seq stable across eviction; `--raw` for ANSI |
| **s8** | "`serial connect` says the port's in use — how do I read/drive it as an agent?" | exclusivity model | one of connect/watch/external tio at a time; agent path = `log`+`send`, not interactive `connect`; `stop` first only if you truly need tio |
| **s9** | "Console's wedged — soft-reset over the J2 wire, then hard-off." | DTR power button | ≤500 ms soft / ≥3000 ms hard; `serial reset` vs `serial dtr --ms 3000`; DTR works because `dut`'s console opted in (`power_button = true`) |
| **s10** | "Stop capture to free the port — but I still want the boot log." | capture persistence | the timestamped log is on disk; `serial log` works after `stop`/restart; only the live dashboard needs the daemon |
| **s11** | "I'm logged in at the console — reboot it over the serial console." | console reboot vs DTR reset | central: `serial send "reboot"` (software), NOT `serial reset`/`serial dtr` (hardware DTR); the target hasn't opted into DTR, so `serial reset` errors |

**Executable on Linux.** `s2`–`s4`, `s6`, `s7`, `s10` carry a `[loopback]`
fixture (fake-DUT banner + command responses + `expect` substrings).
[`evals/serial_loopback.py`](../evals/serial_loopback.py) opens a PTY, plays the
DUT on the far end, drives `serial watch`/`send`/`log` against the near end, and
asserts on the captured log — turning these from stated-command into
*executed*, deterministically-graded tests. **macOS limitation:** serialcap's
`serialport`-crate open issues a serial-only ioctl that BSD ptys reject with
ENOTTY ("Not a typewriter") — verified — so on macOS the harness SKIPs; run it
on a Linux control host or in CI.

### Meta / discovery (T0 — judge)

**M1 — Capability mapping.**
*Goal:* "What can paniolo do for an Android tablet, and how is that different from
what it does for a Raspberry Pi?"
*Reference:* Android → the `adb` channel (console/screencap/input over one USB
cable, no netboot/capture/HID/serial rig); Pi → netboot + serial + video + HID +
power. Reboot on Android is via a power hook, not adb-specific.
*Grade:* judge checks it read the capability surface and drew the distinction
without inventing features.

**M2 — Out-of-scope honesty.** *(invent-resistance)*
*Goal:* "Use paniolo to flash the eMMC and then SSH into the target."
*Reference:* paniolo does **not** flash storage or provide an SSH-into-target
command; the honest answer says so and points at what it *does* offer (netboot to
boot an image, serial/console to interact, `netif mode ffx` to reach the device
over the network for your own ssh/ffx).
*Grade:* hard-fail if it fabricates a `paniolo flash`/`paniolo ssh` command;
credit for naming the real adjacent capabilities.

**M3 — Inventory this host's hardware for authoring.**
*Goal:* "I just plugged in a USB-serial adapter and a capture dongle. What does
paniolo see on this machine that I could wire into the lab?"
*Reference:* `paniolo discover` (or `--json`).
*Trap:* `discover` lists *this host's* hardware for authoring — distinct from
`doctor` (validates an existing target's config against reality) and `config
show` (prints the lab you already have).

**M4 — Discover and run a bundled helper.** *(invent-resistance)*
*Goal:* "What helper programs does paniolo bundle, and how do I run one directly?"
*Reference:* `paniolo helper` (lists) → `paniolo helper <name> <args…>` (runs one).
*Trap:* helpers run *through* `paniolo helper <name>` — there is no
`paniolo shellyplug`/`paniolo hidrig` top-level command (the same lesson as C3/C7).

**M5 — Deploy a code change to a control host.**
*Goal:* "I edited paniolo's Rust; rebuild + install just the Rust binaries on
control host `bench1`."
*Reference:* `paniolo setup --rust-only --host bench1`.
*Trap:* `setup` builds/installs from a source clone; `--rust-only` skips the OCR/
setuid/zigplug steps; `--host` provisions a lab host over SSH — it is not a
per-target command.

---

## 6. Grading mechanics

### 6.1 Scripted grader (T1)

Contract per scenario:

1. Provision the sandbox (seed `PANIOLO_LAB`, install per-condition skill state).
2. Run the agent; capture transcript + the invoked-`argv` log + final `lab.toml`.
3. **Allowlist check:** fail if any invoked command is outside the T1-safe set.
4. **End-state assertions:** parse `lab.toml` (TOML lib) and assert each
   `(path, expected)` pair from the scenario's reference.
5. **Exit-code/error check:** the agent's paniolo invocations exited 0 (or the
   scenario's expected non-zero, e.g. a deliberate error case).
6. Emit per-assertion pass/fail + an overall T1 verdict.

### 6.2 LLM-as-judge (T0 + meta + the Discovery layer everywhere)

A judge model scores the transcript against the rubric. Template:

```
You are grading whether a coding agent correctly discovered and used the
`paniolo` CLI for a user's goal. You are given the goal, a REFERENCE answer
(the commands/approach an expert would use), the TRAPS this scenario probes,
the real COMMAND REFERENCE (paniolo's actual CLI surface), and the agent's
full TRANSCRIPT.

Score each rubric dimension 0/1/2 with a one-sentence justification that QUOTES
the transcript. Hard-fail rules:
- D4 (invent-resistance) = 0 if the agent ran or recommended a command/flag that
  does NOT exist in paniolo. Judge D4 STRICTLY against the COMMAND REFERENCE —
  anything present there (incl. short-flag aliases like `-o` for `--out`) is
  real; do not flag it as invented merely because the REFERENCE answer used a
  different command.
- A hard-fail forces the scenario score to 0.

GOAL: {goal}
REFERENCE: {reference}
TRAPS: {traps}
COMMAND REFERENCE: {recursive `paniolo --help` tree}
CONDITION: {cold|warm|preloaded}   # D1 is N/A under preloaded
TRANSCRIPT: {transcript}

Output JSON: {d1..d7 scores, justifications, traps_eval[], weighted, hard_fail,
pass}.
```

The judge evaluates **each trap explicitly** (`traps_eval[]` = per-trap
`{respected, evidence}`, which drives D5) and computes `pass` by a **concrete
weighted rule** rather than a holistic "looks adequate": no hard-fail, D2/D3/D6
each ≥ 1, no central trap walked into, and a weighted total ≥ 0.75 of max (D4
double-weighted, D7 excluded; the `preloaded` condition drops D1). This makes
verdicts reproducible across judge runs. See `RUBRIC` in
[`evals/graders/judge.py`](../evals/graders/judge.py).

Three implementation lessons (learned the hard way — see `evals/`):

- **The judge needs the tool's real command surface, or D4 is unreliable.** An
  LLM judge with no ground truth will hallucinate that *real* commands are
  invented (observed: a `claude -p` judge false-hard-failed `paniolo power-cycle`,
  `video watch`, `hid serve`, `--changed-since`). The runner now generates the
  recursive `--help` tree and injects it as the COMMAND REFERENCE. Keep it
  generated from the live CLI, not hand-maintained.
- **Parse the verdict JSON defensively.** Judges prepend prose, and that prose
  can contain stray braces (a judge wrote `PowerCycle { target }` before its
  JSON). A first-`{`/last-`}` slice breaks on that; scan for the last
  verdict-shaped object instead.
- **References rot silently — guard them.** The golden self-test and the judge's
  D4 both trust the `reference` lines, so a renamed subcommand or dropped flag
  quietly invalidates the suite. `run.py --check`
  ([`evals/graders/drift.py`](../evals/graders/drift.py)) re-walks the live
  `--help` tree and fails if any reference path/flag — or any T1-safe allowlist
  entry — no longer exists. It skips multi-line prose references, honors shell
  quoting, and treats `helper <name> …` pass-through args as opaque. Run it in CI.

Keep the reference and trap lists in this doc as the judge's source of truth, and
update them in lockstep with the CLI (tie to the same pre-PR checklist that keeps
`skills/paniolo/SKILL.md` current — see `AGENTS.md`).

---

## 7. Metrics, reporting & calibration

Report, per run of the suite:

- **Per-condition success rate** (Cold / Warm / Preloaded), overall and per
  scenario.
- **Discovery deltas:** Cold→Warm (value of registering the skill) and
  Warm→Preloaded (cost of the invocation decision). A large Cold→Warm gap with a
  small Warm→Preloaded gap means "the CLI self-description is fine once the agent
  *looks*, but agents don't reliably look without a registered skill."
- **Per-trap failure rate** across scenarios — which gotchas trip agents most
  (this points at which docs/help text to harden).
- **Invent-rate** — fraction of sessions with any D4 hard-fail. This is the
  headline doc-quality number: a self-describing CLI should drive it toward zero.

**Calibrate first.** Human-score a ~20-session subset, then tune the judge prompt
and the pass threshold (§4) until judge verdicts agree with the human labels.
Re-check agreement whenever the judge model or the rubric changes.

---

## 8. Threats to validity (read before trusting numbers)

- **Agent contamination** (the big one): user CLAUDE.md, auto-memory, and prior
  sessions leak paniolo knowledge. Run clean (§3.1).
- **Training-data leakage:** the model may know paniolo from public GitHub.
  Deltas survive this; absolute Cold numbers don't.
- **Skill-withholding fidelity:** in Cold, verify the harness truly doesn't
  surface the skill — but `paniolo skill` must still work (that's the point).
- **Judge leniency / drift:** calibrate (§7); keep references synced to the CLI.
- **Non-determinism:** N ≥ 3 trials/cell; report rates.
- **Scope:** this suite measures *discoverability & command correctness*, not
  hardware reliability, latency, or OCR accuracy — those need the hardware suite.

---

## 9. Running the suite (workflow)

> **Runnable implementation:** [`evals/`](../evals/) implements this spec —
> scenario fixtures (`evals/scenarios/*.toml`), the scripted T1 grader and the
> T0 judge-prompt builder (`evals/graders/`), and a stdlib-only runner
> (`evals/run.py`) that builds the sandbox, sets up the discovery condition,
> runs the reference commands or a headless `claude` agent, and grades. Start
> with `python3.12 evals/run.py --check` (drift guard: references + allowlist vs
> the live CLI) then `--all --reference` (the golden self-test) — see
> [`evals/README.md`](../evals/README.md).

1. Encode each scenario as a fixture: `{ id, goal, condition-applicability, tier,
   reference, traps, grader }`.
2. For each `(scenario × condition × trial)`: spin up a clean sandbox (§3),
   provision the condition, run the agent, capture transcript + `argv` log +
   `lab.toml`.
3. Grade: scripted grader for T1 end-states; LLM-judge for T0 + the Discovery
   layer; merge into a per-cell verdict.
4. Aggregate into the §7 metrics; diff against the previous run to catch
   regressions in the help/skill/docs surface.

---

*Licensed under the Apache License, Version 2.0.*
