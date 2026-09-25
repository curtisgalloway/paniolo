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
| M1 | Error type, `main()` mapping, JSON object, not_configured/internal | — | complete ([evidence](evidence/M1.md)) |
| M2 | Child boundary: hooks, dispatch, passthroughs | M1 | complete ([evidence](evidence/M2.md)) |
| M3 | daemon_down, timeout, unreachable device; `serial log --require-live` | M1 | complete ([evidence](evidence/M3.md)) |
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

**Outcome:** failures exit by kind (3 `not_configured`, 2 `usage`, 109
`internal`), with the JSON object on request. **Status:** complete —
[evidence](evidence/M1.md). **Open limitations:** R6 (silent power-cycle) did
not reproduce locally; its control-host repro is in M4. Review finding F5
(0.4.x remote rejects `--json-errors`) accepted as design D5.

## M2 — The child boundary

**Outcome:** hooks exit 101 (`child_exit` = their code) or 3 when missing;
an unreachable control host exits 4 with `host`; remote codes pass through
with one JSON object; passthroughs keep the child's code, 128+N on a signal.
**Status:** complete — [evidence](evidence/M2.md). **Open limitations:** hook
paths other than power are covered by construction, not by integration tests
(they need real helpers). Design D1 changed: hooks no longer inherit
`PANIOLO_JSON_ERRORS` (review N1, user decision).

## M3 — Daemons, timeouts, `serial log`

**Outcome:** a stopped or stale daemon exits 100; a daemon still starting
at its deadline, or a timed-out daemon request or port forward, exits 22; a
daemon that exited non-zero during startup or answered an error status exits
101; `serial log` warns when serialcap is stopped and `--require-live` makes
it exit 100. No exit 4 for device nodes (paniolo never probes them).
**Status:** complete — [evidence](evidence/M3.md). **Open limitations:** the
untracked-daemon case of `serial log` is covered by construction, not by a
test. A non-timeout transport error to a daemon (connection reset) is still
`internal`.

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

M3 complete (checkpoint commit `cli: error contract M3 — daemons, timeouts,
serial log`). Next: **M4 — documentation and final verification**, on branch
`error-contract`. Read the design (the code table is the contract to
document), the M4 section above, the three evidence files, and AGENTS.md's
"Before opening a PR" checklist (docs, `--help`, `skills/paniolo/SKILL.md`,
evals). The hardware part (R6 and the consumer's cases on a control host)
needs a control host running this build. Build with the rustup toolchain
(`~/.cargo/bin/cargo`).
