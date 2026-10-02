<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# An MCP server for paniolo

> **Status: plan — nothing built.** Converged 2026-10-02. The first step is a
> measurement (step 1 below), and its result decides how much of step 3 is
> built up front.

## Goal

Let an agent drive a target through MCP tools instead of shelling out to
`paniolo` over Bash and SSH. The CLI remains the one implementation; the MCP
server is a thin front end over the same command handlers.

## Why

1. **Images in the tool result.** The KVM loop in
   `skills/kvm-puppeting/SKILL.md` is: `video shot -o screen.png`, open the
   file, copy the `hash=` off stderr for the next `--changed-since`. A
   `video_shot` tool returns the image and the hash in one result. This is
   the most-repeated loop an agent runs, so it gains the most.
2. **Agents without a shell.** Harnesses with no Bash or SSH (desktop chat
   clients, other agent frameworks) can drive a target.
3. **Typed calls.** Schemas replace flag-guessing. Error kinds already exist
   (`docs/errors.md`, `--json-errors`) and map directly onto tool errors.

## Decisions

| Question | Decision | Why |
|---|---|---|
| Transport | **stdio**, spawned by the harness | No new listening port. Every daemon today is loopback-only and token-authenticated (#196); a network MCP endpoint would be the first non-loopback listener able to cut power and inject keys. |
| Where it runs | **The dev machine** (`paniolo mcp`) | The dev machine is the hub ([distributed-control.md](../docs/distributed-control.md)). One target's channels can live on several control hosts; only the hub reaches all of them. |
| Remote placement | `ssh <host> paniolo mcp` is a **fallback only** | When the harness runs on a machine with no paniolo install or lab file. The harness owns that pipe and generally will not relaunch a dead stdio server, so a drop loses every tool until restart. |
| Reaching control hosts | **One persistent worker per host**, opened by `paniolo mcp` on first use (step 3) | Removes per-call setup cost; see *Cost of per-call dispatch*. |
| Tool surface | **Runtime verbs only** | Configuration (`set`/`add`/`rm`) stays CLI-only, matching the rule that config commands never act on an implied target. |
| Implementation | Tools call the existing handlers | No second implementation to drift. Investigate generating schemas from the clap tree so docs/help/skill/MCP stay one source. |
| Server state | **None of its own** | The daemons already hold the real state (capture, serial logs, held keys). Restarting `paniolo mcp` loses nothing. |

### First tool set

`target_list`, `video_shot` (image + hash), `video_read`, `serial_log`,
`serial_send`, `hid_send`, `power_state`, `power_cycle`, `power_on`,
`power_off`.

## Cost of per-call dispatch

Today a command whose channel is on another host does three SSH operations
(`cli/src/dispatch.rs`), each over the shared ControlMaster
(`cli/src/ssh.rs`), so there's no new TCP connection or authentication:

1. `sftp_put` — ship a one-target lab file.
2. `ssh … paniolo …` — start a fresh `paniolo` on the host, which parses that
   file and runs the command.
3. `sftp_rm` — delete the file.

Three round trips plus a process start per action. An agent loop of
`shot → moveabs → click → shot` pays that four times: 12 round trips where a
persistent worker needs 4.

## Persistent worker design

```
harness ──stdio──▶ paniolo mcp (dev machine)
                     ├── ssh bench1 paniolo worker   ◀─ one long-lived pipe
                     └── ssh bench2 paniolo worker   ◀─ opened on first use
```

- `paniolo mcp` opens `paniolo worker` on a control host the first time a
  tool needs that host, and keeps it.
- The lab slice is sent once, when the worker starts.
- Each call is one JSON line down the pipe and one JSON line back: one round
  trip, no new process. Binary results (PNG) go base64 in the JSON, or as a
  length-prefixed frame if that turns out to matter.
- The worker runs the same handlers the CLI does, taking requests from stdin
  instead of argv. No new port, no new auth.
- A multi-host target routes each channel's call to that channel's host.
- The CLI keeps per-call dispatch. Nothing changes for a person at a shell.

### Reconnect

Because `paniolo mcp` owns the `ssh` child, it can restart it.

**Detecting the drop:**

- A clean drop (link down, host reboot): `ssh` exits, the pipe reads EOF,
  restart the worker.
- A silent drop (laptop sleep, Wi-Fi change) leaves TCP half-open and a read
  waits forever. Fix with `ServerAliveInterval=5` + `ServerAliveCountMax=3`
  (ssh exits within ~15 s), and a per-request deadline that kills the worker
  on expiry. `ssh.rs` sets neither ServerAlive option today — this is step 2,
  and it helps the CLI too.

**Retrying the call in flight:** if the pipe dies after a request was sent but
before the reply arrived, the outcome is unknown.

| Call | On a drop |
|---|---|
| `video_shot`, `video_read`, `serial_log`, `power_state`, `target_list` | Reconnect and retry automatically. Reads change nothing. |
| `hid_send`, `serial_send`, `power_cycle`, `power_on`, `power_off` | Reconnect but **do not resend**. Return an "outcome unknown: connection lost; the action may or may not have run" error; the agent re-reads state and decides. |

This is the rule `ch9329/src/uart.rs` already follows: retry idempotent
queries only. Request-ID deduplication doesn't help, because the remote worker
usually dies with its SSH session and so forgets which IDs it ran.

## Steps

Each step is independently useful and has a stop point.

### 1. Measure per-call dispatch cost

Run on the dev machine (the one holding the lab file). Pick a target `T`
whose video channel is on a different control host `H`. `video show` is a
dispatched read that does almost no work on the far side, so its time is
mostly dispatch overhead.

```bash
T=<target>; H=<control host>

# A. Full per-call dispatch: sftp put + ssh exec + sftp rm.
for i in 1 2 3 4 5 6; do /usr/bin/time -p paniolo video show "$T" >/dev/null; done 2>&1 | grep real

# B. One bare round trip over the warm ControlMaster (what a worker call costs, minus handler time).
#    Reuse paniolo's master socket so this matches what dispatch sees.
for i in 1 2 3 4 5 6; do /usr/bin/time -p ssh "$H" true; done 2>&1 | grep real

# C. The command run directly on the control host (handler + process start, no dispatch).
ssh "$H" "for i in 1 2 3 4 5 6; do /usr/bin/time -p paniolo video show $T >/dev/null; done 2>&1 | grep real"
```

Drop the first run of each (it may open the ControlMaster). `hyperfine` gives
cleaner numbers if it is installed.

Then the same for the loop that matters, with a real screenshot:

```bash
for i in 1 2 3 4 5 6; do /usr/bin/time -p paniolo video shot "$T" -o /tmp/s.png; done 2>&1 | grep real
ls -l /tmp/s.png   # payload size per shot
```

Record: median of A, B, C; shot time; PNG size; whether `H` is on the LAN or
across a WAN/VPN.

**Decision:**

- **A − B ≥ ~100 ms** → build the persistent worker into the spike (step 3).
- **A − B is a few tens of ms** → spike with per-call dispatch; an agent's own
  turn takes seconds. Add the worker when the KVM loop shows it is needed. The
  tool interface is the same either way.
- **Shot time dominated by PNG transfer** → neither transport choice fixes
  that; look at JPEG or downscaled returns for `video_shot` instead.

### 2. SSH keepalive (independent, small)

Add `ServerAliveInterval=5` and `ServerAliveCountMax=3` to the options in
`cli/src/ssh.rs`. Two lines. Helps every remote CLI call today, and the worker
depends on it later.

### 3. Spike `paniolo mcp`

- New module `cli/src/mcp.rs`, one clap variant `Command::Mcp`.
- Two tools only: `target_list` and `video_shot` (image + hash).
- Per-call dispatch or worker, per step 1's decision.
- Needs a dependency decision: an MCP crate (e.g. `rmcp`) versus a hand-rolled
  JSON-RPC over stdio. AGENTS.md requires discussion before a new dependency.
- Try it: point Claude Code at `paniolo mcp`, run one look-act-verify cycle
  against a real target.

**Stop point:** if image-in-the-result is not clearly better than
shot-then-read, stop here.

### 4. Fill out the tool set

The remaining tools from *First tool set*, with the retry rules from
*Reconnect* if the worker exists.

### 5. Measure against the skill

Add an **MCP** condition beside Cold / Warm / Preloaded in
`docs/dev/agent-evals.md` and run the KVM scenarios. Expand the surface only
if MCP beats the registered skill.

### 6. Ship it

Per the AGENTS.md PR checklist: `docs/` page (and `mkdocs.yml` nav),
`--help` text, `skills/paniolo/SKILL.md`, the AGENTS.md module layout, and a
note in `README.md`'s capabilities table.

## Open questions

- Generate tool schemas from clap, or hand-write the ten? Generation keeps one
  source of truth; hand-writing gives better tool descriptions.
- Does `serial_log` want MCP resources or notifications for a live tail, or is
  polling with a line range enough? Client support for notifications is uneven,
  so start with polling.
- Should `video_shot` return PNG, JPEG, or a downscaled image by default? Image
  size costs agent context as well as transfer time.
