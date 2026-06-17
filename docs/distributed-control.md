# Distributed control: one lab, one file

> **Status: Phases 0–5 implemented** (#20 for 0–3, #22 for 4–5; 2026-06-01).
> Shipped: the SSH transport, the one-file lab model (`--lab` / `PANIOLO_LAB`),
> transparent re-exec of one-shot commands on a target's host, a tunnelled
> `console` for a remote target, remote `setup --host`, and discovery-assisted
> `configure`. Channels of one target may live on different control hosts —
> each command routes per-channel; only the composite `console` still requires
> its channels co-located. Still design-only: cross-host composite commands,
> `console --detach`, and multi-user locking (see the
> [implementation plan](https://github.com/curtisgalloway/paniolo/blob/main/docs/distributed-control-plan.md) for the phasing). Compare
> [related work: paniolo vs. labgrid](ci-integration/related-work.md),
> whose distributed model directly informs this.

## The problem

Today paniolo assumes the machine you run it on is the machine wired to the
target. When your dev machine isn't the control host — the common case — you
SSH into the control host by hand and run `paniolo …` there (the remote-control
pattern in the root [README](https://github.com/curtisgalloway/paniolo/blob/main/README.md)). That works, but it leaks the host
boundary into every workflow: you manage SSH sessions yourself, and anything
that serves a port (the dashboard, `serial watch`, `video preview`) means
hand-rolling `ssh -L` port forwards. The friction is acute for the console —
making the live dashboard reachable from your laptop is just enough manual SSH
plumbing to discourage using it.

The goal: **abstract away host location.** From your dev machine you say
`paniolo console fortune` and it transparently does the right thing on whichever
control host `fortune` is wired to — including, eventually, a target whose
hardware is spread across more than one control host.

## The core decision: one lab, one file

A **lab** is described by a single config file that lives in a git repo. You
point paniolo at it (a `--lab` flag or `PANIOLO_LAB` env var, defaulting to a
conventional path). That file declares every **host** in the lab and every
**target**, and which host each piece of a target's hardware lives on. One lab,
one file — deliberately simple; multi-file / multi-lab composition can come
later if it ever earns its keep.

The file is the contract. It is plain, reviewable config under version control,
edited by a human (optionally with an agent's help — see
[Configuration workflow](#configuration-workflow)); paniolo reads it but is not
the authority over it. This keeps paniolo's existing grain, where target config
is reviewable TOML rather than daemon-managed state.

## Design principles

These fell out of the discussion and constrain everything below:

1. **The dev machine is the hub.** It is the only node guaranteed to reach every
   control host (a star topology). Control hosts cannot be assumed to reach each
   other — they may sit on isolated lab segments with no mutual SSH trust. So the
   data plane must **rendezvous at the dev machine**, never between control hosts.
2. **Config is centralized and reviewed; runtime state lives next to the
   hardware.** The lab file (config) lives in one place a human reviews. Only
   *runtime* state — daemons, capture logs, advisory locks, discovery files —
   lives on the control host, because it must be co-located with the hardware it
   describes. This sharpens paniolo's old "state lives next to the hardware" rule,
   which was only ever true because everything ran on one host.
3. **Control hosts are stateless executors.** They hold no durable target config.
   paniolo ships the relevant slice of config to a host at command time. A control
   host is therefore *disposable*: re-image it, re-run `paniolo setup` on it, and
   it resumes its role from the lab file with nothing to restore.
4. **SSH is the transport.** It already solves auth, encryption, and identity, and
   the key infrastructure exists. A custom agent/RPC server would either reinvent
   that or tunnel over SSH anyway. labgrid uses SSH for its data plane for exactly
   this reason.
5. **Don't preclude multi-host targets.** A single logical target may span control
   hosts (serial on one, HDMI capture on another, power on a third). The schema is
   designed for this from day one even though the first implementation handles only
   the same-host case.

## The config model

Host binding lives on **each resource**, not on the target as a whole — because a
target can span hosts. This mirrors labgrid's Resource model (a Resource is
passive access-info bound to a specific exporter). A target-level `host` sets the
default; each resource inherits it unless it overrides. The default-of-the-default
is `local` (the dev machine itself), so a lab with one local host and one target
reproduces today's single-host behavior exactly.

```toml
# mylab.toml — checked into a git repo; PANIOLO_LAB points here.

[hosts.bench1]
ssh = "curtisg@bench1.local"      # ssh destination — how OTHER machines reach it; the only required field
# hostname = "bench1.local"       # this box's FQDN; set it so bench1 recognizes ITSELF when the
#                                   shared lab file is run on bench1 (matched against `hostname -f`)
# identity = "~/.ssh/id_lab"      # optional key; set it to avoid agent key-spray (below)
# control_path = "~/.ssh/cm-%h"   # optional ControlMaster socket (see Transport)
# paniolo_cmd = "/Users/me/.local/bin/paniolo"  # if paniolo isn't on the host's ssh PATH

[hosts.bench2]
ssh = "curtisg@bench2.local"
# hostname = "bench2.local"

# A normal single-host target. Everything inherits host = bench1.
[targets.fortune]
host = "bench1"                   # default host for this target's resources

[targets.fortune.netboot]
interface = "enx00e04c08d9a0"
host_ip   = "192.168.99.1"
tftp_root = "/home/curtisg/tftp/fortune"

[[targets.fortune.serial]]
name   = "console"
device = "/dev/serial/by-id/usb-FTDI_FT232R_USB_UART_BG00W7NY-if00-port0"
baud   = 115200

[targets.fortune.power]
cycle_cmd = "/home/curtisg/src/rpi5-bringup/scripts/power-cycle.sh"

# --- the future case: one target spanning two control hosts ---
[targets.fortune.video]
host   = "bench2"                 # HDMI capture is on a different host
device = "0x8300000534d2109"      # USB Video — stable, port-derived id
```

- `host = "local"` (or unset, on a single-host lab) means the dev machine — i.e.
  today's behavior, no SSH involved.
- **One shared lab file, run from any machine.** Give each host a `hostname` (its
  FQDN). At runtime each box compares its own `hostname -f` against every host's
  `hostname`; the match is treated as **local** (channels run directly there) and
  every other host is **remote** (dispatched over SSH). So the same git-tracked
  file works whether you run it on the Mac, on `bench1`, or on `bench2` — each
  recognizes itself. `ssh` stays the *reach* path (it may be an `~/.ssh/config`
  alias); `hostname` is the *self-recognition* key. Without a `hostname`, only
  `ssh = "local"` / `host = "local"` counts as local, so the file is
  single-driver (run it anywhere else and a host self-dispatches over SSH).
  `paniolo host list` prints the detected FQDN and marks the matching host.
- With no `--lab`/`PANIOLO_LAB`, paniolo reads the default lab at
  `~/.config/paniolo/lab.toml`; if none exists it errors and points at
  `paniolo init`. (The legacy Python CLI's per-target
  `~/.config/paniolo/targets/*.toml` files are not read by the Rust CLI.)

## Transport (the "Fork B" model)

The transport splits by command type, and leans entirely on SSH and the fact
that paniolo's subsystem daemons (`serialcap`, `hdmicap`) are *already* network
services speaking HTTP/WebSocket on a discovery port.

**One-shot control commands** — `power-cycle`, `netboot start/stop`,
`video shot`, `serial log`, `serial send`, config reads — **re-exec over SSH.**
For a resource on `bench1`, paniolo runs the same command on `bench1` and
forwards stdin/stdout/stderr and the exit code. The far-side paniolo is
unchanged, so this reuses 100% of existing logic; runtime state (logs, locks,
discovery) naturally stays on the control host where it belongs.

**Streaming / port-serving commands** — the dashboard, `serial watch`,
`video preview` — **SSH-tunnel to the existing daemon.** paniolo re-execs the
daemon start remotely (idempotent), reads the remote discovery port over SSH,
opens an `ssh -L` forward to it, and points the local client/browser at the
forwarded local port. No new protocol and no always-on server: the daemon's
existing HTTP/WS *is* the API.

**Latency** is handled with one **ControlMaster** connection per host, shared by
every re-exec and every `-L` forward, so only the first command per host pays the
SSH handshake. The host's `control_path` in the lab file names the master socket.

**Interactive `serial connect`** (tio) needs **no tunnel** — `ssh -t bench1
paniolo serial connect fortune` runs tio straight over SSH's own PTY. The tunnel
machinery is only for the browser dashboard, not the terminal CLI.

**Two operational notes** (learned while implementing this):

- **`paniolo` must be reachable on the host.** Re-exec runs `paniolo …` over a
  *non-interactive* ssh, whose PATH often omits `~/.local/bin`. If bare `paniolo`
  doesn't resolve there, set the host's `paniolo_cmd` to an absolute path.
- **Set `identity` to avoid ssh-agent key-spray.** An agent offering many keys
  (e.g. 1Password) can trip the host's `MaxAuthTries` *before* the right key on
  the first connect. A per-host `identity` makes paniolo pass
  `-i <key> -o IdentitiesOnly=yes`, offering exactly one. (This is the user's ssh
  setup, not something paniolo can fix for them — but the lab field is the lever.)

### The dashboard, and why multi-host rules out a reverse-proxy

The dashboard is the one place two subsystems interlock: hdmicap serves the page
but reaches serialcap by an **absolute URL** (`ws://<host>:8724/stream`), with a
`?serialws=` override (see [architecture §7](architecture.md)). So the browser
makes a *second* connection, to serialcap, possibly on a different port and a
different host.

The clean answer uses the override and the hub principle together: forward each
daemon's port to the dev machine, then open the dashboard at
`http://127.0.0.1:<local-hdmi>/?serialws=ws://127.0.0.1:<local-serial>/stream`.
The `?serialws=` knob — which already exists — stitches the two together and does
not care that the daemons are on different hosts, only that both resolve as
forwarded local ports. This needs **zero changes to hdmicap or serialcap.**

We explicitly considered, and rejected, making hdmicap **reverse-proxy** serialcap
to collapse the dashboard to one origin/one forward. It would require hdmicap on
one host to connect to serialcap on another — exactly the cross-host path
principle 1 says we cannot assume. The forward-each-daemon-to-the-dev-machine
model is the only one that always works, and it generalizes to multi-host targets
for free.

### Why not a long-running agent daemon (labgrid's exporter)

A per-host paniolo agent with its own RPC API (labgrid's exporter / "Option B" in
`AGENTS.md`) would give cleaner streaming multiplexing and a natural home for
multi-user locking. We chose against it for now because it directly trades away
paniolo's stated identity — *zero-infrastructure, no coordinator/exporter/client
to stand up* ([related-work](ci-integration/related-work.md)). SSH-tunnelling the
daemons that already exist gets local-feeling console with no always-on server and
no new auth surface. The agent remains a *someday* option, gated on whether
multi-user/board-farm scale ever becomes a goal — at which point paniolo would be
choosing to become a different kind of tool, deliberately.

## Console lifecycle

`paniolo console <target>` is **foreground-blocking** by default: it opens the
forward(s), launches the browser, and blocks holding the tunnel until you Ctrl-C,
then tears down. This feels exactly like a local dashboard and needs **no
persistent local runtime state** — the forwards die with the process. It assumes a
human is present, which is consistent with the fact that physical setup already
requires interactive access to the control host.

A non-blocking `--detach` mode (set up the forward, print the URL, return; reap on
`console --down` or idle timeout) is a plausible later addition for agent use, but
it introduces a *local tunnel registry* — transient runtime state on the dev
machine — so it is deferred until something actually needs the live console
without a terminal held open. (Most agent workflows use `video shot`/`read`, not
the live stream.)

## Configuration workflow

Discovery **assists** authoring; it does not replace it. Control hosts can
enumerate their hardware (serial devices, USB-Ethernet interfaces, HDMI capture
devices) to scaffold config, but the authoritative lab file is always written and
approved by a human.

The flow is two-phase — **propose, then approve**:

1. `paniolo configure fortune --serial bench1 --video bench2` runs discovery on
   the named hosts over SSH and merges their inventories into a **proposed** target
   definition.
2. paniolo shows the **diff** against the current lab file and writes nothing
   authoritative.
3. The human reviews and approves; the change lands as a git commit to the lab
   repo.

An agent can drive step 1 and prepare the diff, but it can only *stage* a
proposal — it never silently mutates the authoritative config. Because the lab
file is in git, every change is a reviewable, revertible commit. Reconfiguration
is the same flow against the existing file.

## What's deferred

Designed-for but **not** in the first implementation:

- **Multi-host targets.** The schema (per-resource `host`) and the transport
  (dev-machine rendezvous) both support it; the first cut handles same-host
  targets only and rejects cross-host targets with a clear error.
- **`console --detach`** and the local tunnel registry it requires.
- **Multi-user / locking / reservations** (labgrid's coordinator-enforced
  *places*). Single-user is assumed.
- **A long-running agent daemon / RPC API.** Only if scale demands it.
- **Multi-file / multi-lab composition.** One lab, one file, for now.

## Relationship to labgrid

This design is, knowingly, a smaller-footprint rediscovery of labgrid's
distributed shape: per-resource host binding ≈ labgrid Resources bound to
exporters; SSH data plane ≈ labgrid's client→exporter-over-SSH data plane;
discovery-assisted config ≈ a coordinator-as-registry, minus the always-on
server. The deliberate divergence is **no coordinator and no exporter daemon** —
a single git-tracked lab file plus SSH, preserving paniolo's zero-infrastructure,
agent-in-the-loop niche. See [related work](ci-integration/related-work.md) for
the full comparison.

## Open questions (all since resolved)

- *Exact spelling of "point paniolo at the lab"* → `--lab` flag, then
  `PANIOLO_LAB`, then the default `~/.config/paniolo/lab.toml`. The legacy
  `~/.config/paniolo/targets/*.toml` files were dropped rather than composed —
  the Rust CLI never reads them.
- *How the per-command config slice travels* → a temp file copied over SSH,
  re-invoking with `--lab <path>` (`cli/src/dispatch.rs`).
- *How `paniolo setup` is invoked per remote host* → `setup --host bench1`
  re-execs over SSH, as predicted.
