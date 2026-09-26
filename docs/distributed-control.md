# Distributed control: one lab, one file

> **Status: Phases 0–5 implemented** (2026-06-01).
> Shipped: the SSH transport, the one-file lab model (`--lab` / `PANIOLO_LAB`),
> transparent re-exec of one-shot commands on a target's host, a tunnelled
> `console` for a remote target, remote `setup --host`, and discovery-assisted
> `configure`. Channels of one target may live on different control hosts, and
> each command routes per channel. Not yet built: see
> [What's deferred](#whats-deferred) and the
> [implementation plan](https://github.com/curtisgalloway/paniolo/blob/main/notes/distributed-control-plan.md).
> Compare [related work: paniolo vs. labgrid](dev/ci-integration/related-work.md);
> labgrid is an open-source board-lab automation framework whose distributed
> model directly informs this.

From your dev machine you run `paniolo console fortune`, and paniolo does the
right thing on whichever control host `fortune` is wired to. You describe the
lab once, in one git-tracked file; paniolo reaches the hosts over SSH.

## The problem

Without this, paniolo assumes the machine you run it on is the machine wired to
the target. When your dev machine isn't the control host (the common case), you
SSH into the control host by hand and run `paniolo …` there (the remote-control
pattern in the root [README](https://github.com/curtisgalloway/paniolo/blob/main/README.md)).
That works, but every workflow then has to know where the host is. You manage
SSH sessions yourself, and anything that serves a port (the dashboard,
`serial watch`, `video preview`) needs a hand-rolled `ssh -L` port forward.
The console suffers most: making the live dashboard reachable from your laptop
takes just enough manual SSH plumbing to discourage using it.

The goal: **abstract away host location**, including, eventually, a target
whose hardware is spread across more than one control host.

## The core decision: one lab, one file

A **lab** is described by a single config file in a git repo. Point paniolo at
it with the `--lab` flag or the `PANIOLO_LAB` env var (defaulting to a
conventional path). The file declares every **host** in the lab, every
**target**, and which host each piece of a target's hardware lives on.
Multi-file / multi-lab composition can come later if it ever earns its keep.

The file is the contract: plain, reviewable config under version control,
edited by a human (optionally with an agent's help — see
[Configuration workflow](#configuration-workflow)). paniolo reads it but is not
the authority over it. This keeps paniolo's existing approach, where target
config is reviewable TOML rather than daemon-managed state.

**The control host runs the same paniolo.** A re-exec forwards your argv
verbatim (`dispatch::subcommand_args`). A flag this version accepts is sent as
typed, and a control host on an older release answers `unexpected argument`.
That is true of every flag paniolo has added, so keep the hosts on the release
the dev machine runs. Where a spelling is optional (the target as `-t` rather
than positionally), the positional form is the one every past release
understands.

## Design principles

These constrain everything below:

1. **The dev machine is the hub.** It is the only node guaranteed to reach every
   control host (a star topology). Control hosts may sit on isolated lab
   segments with no mutual SSH trust, so they cannot be assumed to reach each
   other. The data plane must therefore **rendezvous at the dev machine**, never
   between control hosts.
2. **Config is centralized and reviewed; runtime state lives next to the
   hardware.** The lab file (config) lives in one place a human reviews. Only
   *runtime* state (daemons, capture logs, advisory locks, discovery files)
   lives on the control host, because it must be co-located with the hardware it
   describes. This sharpens paniolo's old "state lives next to the hardware"
   rule, which was only ever true because everything ran on one host.
3. **Control hosts are stateless executors.** They hold no durable target config;
   paniolo ships the relevant slice of config to a host at command time. So a
   control host is *disposable*: re-image it, re-run `paniolo setup` on it, and
   it resumes its role from the lab file with nothing to restore. The re-image
   path is [standing up a control host](control-host.md).
4. **SSH is the transport.** It already solves auth, encryption, and identity,
   and the key infrastructure exists. A custom agent/RPC server would either
   reinvent that or tunnel over SSH anyway. labgrid uses SSH for its data plane
   for the same reason.
5. **Don't preclude multi-host targets.** A single logical target may span
   control hosts (serial on one, HDMI capture on another, power on a third). The
   schema supports this from day one, even though the first implementation
   handles only the same-host case.

## The config model

Host binding lives on **each resource**, not on the target as a whole, because
a target can span hosts. (This mirrors labgrid's Resource model, where a
Resource is passive access info bound to a specific exporter.) A target-level
`host` sets the default, and each resource inherits it unless it overrides.
The default of that default is `local` (the dev machine itself), so a lab with
one local host and one target behaves exactly like single-host paniolo.

```toml
# mylab.toml — checked into a git repo; PANIOLO_LAB points here.

[hosts.bench1]
ssh = "user@bench1.local"      # ssh destination — how OTHER machines reach it; the only required field
# hostname = "bench1.local"       # this box's FQDN; set it so bench1 recognizes ITSELF when the
#                                   shared lab file is run on bench1 (matched against `hostname -f`)
# identity = "~/.ssh/id_lab"      # optional key; set it to avoid agent key-spray (below)
# control_path = "~/.ssh/cm-%h"   # optional ControlMaster socket (see Transport)
# paniolo_cmd = "/Users/me/.local/bin/paniolo"  # if paniolo isn't on the host's ssh PATH

[hosts.bench2]
ssh = "user@bench2.local"
# hostname = "bench2.local"

# A normal single-host target. Everything inherits host = bench1.
[targets.fortune]
host = "bench1"                   # default host for this target's resources

[targets.fortune.netboot]
interface = "enx001122334455"
host_ip   = "192.168.99.1"
tftp_root = "~/tftp/fortune"

[[targets.fortune.serial]]
name   = "console"
device = "/dev/serial/by-id/usb-FTDI_FT232R_USB_UART_AA00BB11-if00-port0"
baud   = 115200

[targets.fortune.power]
cycle_cmd = "~/scripts/power-cycle.sh"

# --- the future case: one target spanning two control hosts ---
[targets.fortune.video]
host   = "bench2"                 # HDMI capture is on a different host
device = "0x8300000534d2109"      # USB Video — stable, port-derived id
```

- `host = "local"` (or unset, on a single-host lab) means the dev machine: no
  SSH involved.
- **One shared lab file, run from any machine.** Give each host a `hostname`
  (its FQDN). At runtime each box compares its own `hostname -f` against every
  host's `hostname`. The match is treated as **local** (channels run directly
  there); every other host is **remote** (dispatched over SSH). So the same
  git-tracked file works whether you run it on the Mac, on `bench1`, or on
  `bench2`: each recognizes itself.
  - `ssh` is the *reach* path (it may be an `~/.ssh/config` alias); `hostname`
    is the *self-recognition* key.
  - Without a `hostname`, only `ssh = "local"` / `host = "local"` counts as
    local, so the file is single-driver: run it anywhere else and a host
    self-dispatches over SSH.
  - `paniolo host list` prints the detected FQDN and marks the matching host.
- With no `--lab`/`PANIOLO_LAB`, paniolo reads the default lab at
  `~/.config/paniolo/lab.toml`. If none exists it errors and points at
  `paniolo init`. (Old per-target `~/.config/paniolo/targets/*.toml` files from
  the pre-lab-file layout are not read.)

## Transport (the "Fork B" model)

The transport splits by command type. It relies entirely on SSH, and on the
fact that paniolo's subsystem daemons (`serialcap`, `hdmicap`) are *already*
network services speaking HTTP/WebSocket on a discovery port.

| Command type | Examples | How it reaches the host |
|---|---|---|
| **One-shot control** | `power-cycle`, `netboot start/stop`, `video shot`, `serial log`, `serial send`, config reads | **re-exec over SSH** |
| **Streaming / port-serving** | the dashboard, `serial watch`, `video preview` | **SSH tunnel to the existing daemon** |
| **Interactive `serial connect`** (the tio terminal program) | `ssh -t bench1 paniolo serial connect fortune` | **no tunnel**: tio runs over SSH's own PTY |

**Re-exec.** For a resource on `bench1`, paniolo runs the same command on
`bench1` and forwards stdin/stdout/stderr and the exit code (an SSH failure
exits 4; see [Exit status and errors](errors.md)). The far-side paniolo is
unchanged, so this reuses 100% of existing logic, and runtime state (logs,
locks, discovery) stays on the control host where it belongs.

**Tunnel.** paniolo re-execs the daemon start remotely (idempotent), reads the
remote discovery port over SSH, opens an `ssh -L` forward to it, and points the
local client/browser at the forwarded local port. No new protocol and no
always-on server: the daemon's existing HTTP/WS *is* the API. The tunnel
machinery is only for the browser dashboard, not the terminal CLI.

**Latency.** Each host gets one SSH **ControlMaster** connection (a shared
master connection that later SSH commands reuse), shared by every re-exec and
every `-L` forward. Only the first command per host pays the SSH handshake. The
host's `control_path` in the lab file names the master socket.

**Two operational notes:**

- **`paniolo` must be reachable on the host.** Re-exec runs `paniolo …` over a
  *non-interactive* ssh, whose PATH often omits `~/.local/bin`. If bare
  `paniolo` doesn't resolve there, set the host's `paniolo_cmd` to an absolute
  path.
- **Set `identity` to avoid ssh-agent key-spray.** An agent offering many keys
  (e.g. 1Password) can trip the host's `MaxAuthTries` *before* the right key on
  the first connect. A per-host `identity` makes paniolo pass
  `-i <key> -o IdentitiesOnly=yes`, offering exactly one. This is the user's ssh
  setup, not something paniolo can fix for them, but the lab field is the lever.

### Env forwarding to a control host

A re-exec inherits the far side's own environment, not the caller's, like any
non-interactive `ssh host cmd`. That matters for one helper whose credential
paniolo deliberately keeps out of the lab file and every flag: AMT's
`AMT_PASSWORD` (see [power.md](power.md#credentials)). For `power-cycle nuc` to
work when `nuc`'s power channel lives on a remote host, `AMT_PASSWORD` has to
reach *that* host's `paniolo` without landing on its command line. A plain
`KEY=value` prefix would sit in `argv`, which `ps` shows to every local user on
the control host.

The fix is a small, fixed allowlist (`ssh::FORWARDED_ENV` — today, just
`AMT_PASSWORD`) and a **stdin prelude**. When a listed variable is set in the
dispatching process's own environment, the re-exec'd remote command is wrapped
as

```sh
sh -c 'IFS= read -r AMT_PASSWORD && export AMT_PASSWORD && exec "$@"' sh paniolo …
```

and the value is written to the child's stdin, one line per variable in a
fixed order, ahead of whatever the command's own stdin carries (a `serial send`
payload, say). The value is never in the command string. Only the *name* is, so
a process listing shows "this hook expects `AMT_PASSWORD`", never the secret.

Which dispatches forward it:

- **Every non-interactive dispatch does** (`run`, `run_passthrough`,
  `run_stdout_to` in `cli/src/ssh.rs`): `power-cycle`, `power on/off/state`,
  and any other re-exec or captured command.
- **`run_interactive` forwards nothing, by construction** (`serial connect`,
  `adb shell`, `setup --host`). Its stdin *is* the terminal you're typing into,
  so there is no stdin channel for a secret that your own keystrokes wouldn't
  also share. Those commands don't need `AMT_PASSWORD` anyway.

### The dashboard, and why multi-host rules out a reverse-proxy

The dashboard is the one place two subsystems interlock. hdmicap serves the page,
but the page reaches serialcap by an **absolute URL** (`ws://<host>:8724/stream`),
with a `?serialws=` override (see [architecture §7](dev/architecture.md)). So the
browser makes a *second* connection, to serialcap, possibly on a different port
and a different host.

The solution combines that override with the hub principle. Forward each
daemon's port to the dev machine, then open the dashboard at
`http://127.0.0.1:<local-hdmi>/?token=<hdmi-token>&serialws=ws://127.0.0.1:<local-serial>/stream?token=<serial-token>`
(the nested URL percent-encoded; the tokens come from each daemon's
`daemon.json`, read over the same SSH session). The existing `?serialws=` knob
stitches the two together. It does not care that the daemons are on different
hosts, only that both resolve as forwarded local ports. This needs **zero
changes to hdmicap or serialcap.**

That URL carries three daemons' bearer tokens in one line, so `paniolo console`
hands it to the browser and prints only `http://127.0.0.1:<local-hdmi>`. A
remote console is exactly the case where the terminal may be recorded or pasted.
`paniolo video preview` prints the openable video URL when you need it, and
`--open` skips printing it at all.

We considered and rejected making hdmicap **reverse-proxy** serialcap, to
collapse the dashboard to one origin and one forward. That would need hdmicap on
one host to connect to serialcap on another: exactly the cross-host path
principle 1 says we cannot assume. Forwarding each daemon to the dev machine is
the only model that always works, and it extends to multi-host targets for free.

### Why not a long-running agent daemon (labgrid's exporter)

A per-host paniolo agent with its own RPC API (labgrid's exporter, "Option B" in
`AGENTS.md`) would give cleaner streaming multiplexing and a natural home for
multi-user locking. We chose against it for now because it trades away
paniolo's stated identity: *zero-infrastructure, no coordinator/exporter/client
to stand up* ([related-work](dev/ci-integration/related-work.md)). Tunnelling
the daemons that already exist over SSH gives a local-feeling console with no
always-on server and no new auth surface. The agent remains a *someday* option,
if multi-user/board-farm scale ever becomes a goal. At that point paniolo would
be deliberately choosing to become a different kind of tool.

## Console lifecycle

`paniolo console <target>` is **foreground-blocking** by default. It opens the
forward(s), launches the browser, and holds the tunnel until you Ctrl-C, then
tears down. This feels exactly like a local dashboard and needs **no persistent
local runtime state**: the forwards die with the process. It assumes a human is
present, which fits: physical setup already requires interactive access to the
control host.

A non-blocking `--detach` mode (set up the forward, print the URL, return; reap
on `console --down` or idle timeout) is a plausible later addition for agent
use. But it needs a *local tunnel registry* (transient runtime state on the dev
machine), so it is deferred until something actually needs the live console
without a terminal held open. Most agent workflows use `video shot`/`read`, not
the live stream.

## Configuration workflow

Discovery **assists** authoring; it does not replace it. Control hosts can
enumerate their hardware (serial devices, USB-Ethernet interfaces, HDMI capture
devices) to scaffold config, but a human always writes and approves the
authoritative lab file.

The flow is two-phase — **propose, then approve**:

1. `paniolo configure fortune -H bench1` runs discovery on the named host over
   SSH and turns its inventory into a **proposed** `[targets.fortune]` block. It
   best-guesses the USB-Ethernet interface and serial device, and lists other
   candidates as comments.
2. paniolo **prints** the proposed block (with a reconcile-by-hand note if the
   target already exists) and writes nothing authoritative.
3. The human reviews it, pastes it into the lab file, and edits as needed; the
   change lands as a git commit to the lab repo.

An agent can drive step 1 and prepare the proposal, but it can only *stage* it.
It never silently changes the authoritative config. Because the lab file is in
git, every change is a reviewable, revertible commit. Reconfiguration is the
same flow against the existing file.

## What's deferred

Designed for, but **not** in the first implementation:

- **`console` on a cross-host target.** Per-channel host routing has shipped:
  a target's channels may live on different hosts, and each command routes to
  the host of the channel it touches. But the composite `console` still needs
  the **serial and video** channels it stitches together on one host. It
  rejects a target whose serial and video live apart with a clear error (other
  channels may sit anywhere).
- **`console --detach`** and the local tunnel registry it requires.
- **Multi-user / locking / reservations** (labgrid's coordinator-enforced
  *places*). Single-user is assumed.
- **A long-running agent daemon / RPC API.** Only if scale demands it.
- **Multi-file / multi-lab composition.** One lab, one file, for now.

## Relationship to labgrid

This design is knowingly a smaller-footprint rediscovery of labgrid's
distributed shape:

| paniolo | labgrid |
|---|---|
| per-resource host binding | Resources bound to exporters |
| SSH data plane | client→exporter-over-SSH data plane |
| discovery-assisted config | a coordinator-as-registry, minus the always-on server |

The deliberate divergence is **no coordinator and no exporter daemon**: a single
git-tracked lab file plus SSH, preserving paniolo's zero-infrastructure,
agent-in-the-loop niche. See [related work](dev/ci-integration/related-work.md)
for the full comparison.

## Open questions (all since resolved)

- *Exact spelling of "point paniolo at the lab"* → `--lab` flag, then
  `PANIOLO_LAB`, then the default `~/.config/paniolo/lab.toml`. The legacy
  `~/.config/paniolo/targets/*.toml` files were dropped rather than composed;
  the Rust CLI never reads them.
- *How the per-command config slice travels* → a temp file copied over SSH,
  re-invoking with `--lab <path>` (`cli/src/dispatch.rs`).
- *How `paniolo setup` is invoked per remote host* → `setup --host bench1`
  re-execs over SSH, as predicted.
