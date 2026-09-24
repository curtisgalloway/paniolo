<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# CLI error contract — Implementation Plan

Design: [design](design.md), approved 2026-09-24 (D1–D4 as recommended) at
commit `d1d2d25`.
Project checks (from `cli/`): `cargo fmt --check`, `cargo clippy --all-targets
-- -D warnings`, `cargo test`; from the repo root `python3.12 evals/run.py
--check`. Building `cli` on Linux needs `pkg-config` and `libudev-dev`.

## Conventions

- Working branch: `error-contract` (from `origin/main`).
- Checkpoint commit prefix: `cli: error contract M<n> — <title>`.
- Design gate: approved 2026-09-24, D1–D4 accepted.
- User overrides: none.
- Review method: the `review-swarm` skill for every milestone; fallback a
  fresh-context reviewer subagent. Naming it here authorizes it.
- Review order: tests, review and fixes come before the checkpoint commit,
  which is the last step.
- Evidence: `docs/dev/error-contract/evidence/M<n>.md`.
- Commit identity: the repo's noreply address (see `git log`). Public repo:
  no private hostnames, addresses or home paths anywhere.
- Integration tests live in `cli/tests/error_contract.rs` (proposed) and run
  `env!("CARGO_BIN_EXE_paniolo")` against a temp lab; no new dependencies.

## Status

| ID | Outcome | Dependencies | Status |
|----|---------|--------------|--------|
| M1 | Error type, `main()` mapping, JSON object, not_configured/internal | — | pending |
| M2 | Child boundary: hooks, dispatch, passthroughs | M1 | pending |
| M3 | daemon_down, timeout, unreachable device; `serial log --require-live` | M1 | pending |
| M4 | Docs, `--help`, skill, evals; final verification incl. R6 on hardware | M2, M3 | pending |
| M5 | Release 0.5.0 through `RELEASE-TRAIN.md` | M4, user's push go-ahead | pending |

## Design coverage

| Requirement | Milestones | Verification |
|---|---|---|
| R1 exit code by kind | M1, M2, M3 | integration test per kind |
| R2 JSON object | M1 (M2, M3 add fields) | same tests parse the last stderr line |
| R3 dispatch keeps the kind | M2 | stub `ssh`/`sftp` on `PATH` |
| R4 no leaked child codes | M2 | hook and passthrough tests; grep for `exit(status.code` |
| R5 documented | M4 | rendered `--help`, docs build, evals check |
| R6 no silent failure | M1 (local), M4 (control host) | test + hardware repro |
| R7 CI gates | every milestone | the three cargo commands |

## M1 — Error type and the `main()` mapping

**Outcome:** `paniolo --lab lab.toml power-cycle nosuch` exits **3**; with
`PANIOLO_JSON_ERRORS=1` or `--json-errors`, the last stderr line is
`{"error":{"kind":"not_configured","code":3,…,"target":"nosuch",…}}`. Any
unclassified error exits **109**. Clap errors still exit 2. `doctor` still
exits 1 for problems found.
**Design coverage:** R1 (usage, not_configured, internal), R2, R6 (local), R7.
**In scope:** `cli/src/error.rs`; `main()`; the global flag (D1); the target,
channel, interface lookups; `LabError` → 3; "no lab configured" → 3.
**Out of scope:** child exit codes (M2), daemon/timeout kinds (M3), docs (M4).

### Implementation steps

1. Add `cli/src/error.rs`: `Kind`, `exit_code()`, `as_str()`, `PanioloError`
   with builder setters, `Display` = message, and `to_json()` (serde_json is
   already a dependency — confirm).
2. Global `--json-errors` flag on `Cli` (`global = true`) that sets
   `PANIOLO_JSON_ERRORS=1` for this process and children; add
   `PANIOLO_JSON_ERRORS` to `ssh::FORWARDED_ENV`.
3. `main()`: classify the chain (`PanioloError`, then `LabError`, else
   `internal`), print `{e:#}`, then the JSON line if enabled, exit with code.
4. `require_target` / `require_channel` helpers returning `PanioloError`;
   replace the `not found in lab` / `has no … channel` / `has no serial
   interface` sites and `maybe_dispatch`'s lookup. Keep message text identical.
5. `load_for_read`'s "No lab configured" → `not_configured`.
6. Tests: unknown target (power-cycle, serial log), target without hid
   channel, missing serial interface, missing lab file, clap error (2),
   an internal error (109), JSON present only when requested.

**Acceptance:** each case above has the stated code and JSON fields; without
the flag/env stderr is unchanged from 0.4.1 text; all CI gates pass.
**Review focus:** every replaced lookup keeps its message; no path prints two
JSON objects; `--json-errors` does not collide with subcommand `--json`.
**Sizing:** one new module, one mechanical sweep (~25 sites), one test file.
Split point: land steps 1–3 with one lookup, then the sweep.
**Status:** pending.

## M2 — The child boundary

**Outcome:** a `cycle_cmd` of `exit 7` → rc 101, `child_exit: 7`; a missing
hook binary → 3; a dispatch whose ssh exits 255 → 4 with `host` set; a remote
rc 3 plus JSON arrives unchanged with exactly one object; a passthrough child
killed by SIGTERM → 143, never 255.
**Design coverage:** R1 (unreachable, helper_failed), R3, R4.
**Dependencies:** M1.
**Steps:** a `child_exit` helper module (proposed in `error.rs`) with
`hook_result(status, label)`, `dispatch_result(code, host)`,
`passthrough_code(status)`; convert every `exit(status.code…)` and
`exit(code)` site; fix `unwrap_or(-1)` in `ssh.rs`; stub `ssh`/`sftp` scripts
on `PATH` in tests.
**Acceptance:** `rg 'exit\(status\.code'` finds only passthrough helpers; the
outcome cases pass as tests.
**Review focus:** a dispatched failure must not gain a second local JSON
object; passthrough semantics (D3) unchanged except signal deaths.
**Status:** pending.

## M3 — Daemons, timeouts, device nodes, `serial log`

**Outcome:** `serial send` with serialcap stopped → 100 (`channel: serial`);
same for hdmicap/hid/netbootd checks; the forwarded-port and other waits →
22; an absent configured device node → 4; `serial log` with serialcap
stopped prints a warning and exits 0, and with `--require-live` exits 100.
**Design coverage:** R1 (daemon_down, timeout, unreachable), D4.
**Dependencies:** M1. **First step:** inventory the daemon-down, timeout and
device-node sites (`rg` over `cli/src`) and list them in the evidence file.
**Status:** pending.

## M4 — Documentation and final verification

**Outcome:** `paniolo --help` has an exit-status section with only emitted
codes; `docs/` (a user-facing section, likely `docs/README.md` or a new page
in nav), `skills/paniolo/SKILL.md` and `AGENTS.md` describe the contract;
eval scenarios asserting rc 1 are updated; the consumer's cases pass against
a control host running this build, and R6 is reproduced there and fixed or
shown fixed.
**Design coverage:** R5, R6, and a full-design check of R1–R4.
**Dependencies:** M2, M3; a control host for the hardware check.
**Status:** pending.

## M5 — Release

Run `RELEASE-TRAIN.md` for 0.5.0 (breaking: errors no longer exit 1). Push,
tag and publish only on the user's explicit go-ahead and the public-repo
schedule. After `apt upgrade` on a control host: `paniolo daemons restart
--stale`. Report the version to the user for the consumer.
**Status:** pending.

## Backlog

- The remaining unclassified `anyhow!` sites (exit 109) — classify when a
  consumer needs one.

## Next session

Start M1 on branch `error-contract`.
