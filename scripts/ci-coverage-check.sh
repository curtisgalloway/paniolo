#!/usr/bin/env bash
# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# Enforce the project rule: every Rust crate in the repo has a CI job.
#
# A crate is "covered" when its directory appears as a `working-directory:`
# value in .github/workflows/ci.yml -- so a new crate that nobody wired into
# CI fails the build instead of silently going untested for months. It also
# checks scripts/ci-local.sh, which must mirror the Linux jobs so maintainers
# can reproduce them before pushing.
#
# It also enforces that every helper crate (every crate except cli) ships:
# it must appear in the HELPERS list of .github/workflows/release.yml (the
# .deb/tarball contents) and in HELPER_CRATES of cli/src/setup.rs (source
# installs). A helper missing from either builds green in CI and then silently
# never reaches users -- v0.1.13 shipped without the amt helper this way.
#
# Run it anywhere: bash scripts/ci-local.sh needs a Linux box, this needs
# nothing but a shell.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CI="$ROOT/.github/workflows/ci.yml"
LOCAL="$ROOT/scripts/ci-local.sh"
MAKEFILE="$ROOT/Makefile"
RELEASE="$ROOT/.github/workflows/release.yml"
SETUP="$ROOT/cli/src/setup.rs"

# Crates intentionally exempt from a CI job, one per line, with the reason
# stated here. Keep this empty unless there is a real platform blocker.
EXEMPT=""

if [ ! -f "$CI" ]; then
  echo "FATAL: $CI not found" >&2
  exit 2
fi

covered="$(grep -oE 'working-directory:[[:space:]]*[A-Za-z0-9_-]+' "$CI" \
  | awk '{print $NF}' | sort -u)"
local_covered=""
if [ -f "$LOCAL" ]; then
  local_covered="$(grep -oE '^crate_job[[:space:]]+"[A-Za-z0-9_-]+"' "$LOCAL" \
    | tr -d '"' | awk '{print $NF}' | sort -u)"
fi
make_covered=""
if [ -f "$MAKEFILE" ]; then
  make_covered="$(grep -E '^CRATES[[:space:]]*=' "$MAKEFILE" \
    | sed 's/^CRATES[[:space:]]*=//' | tr ' ' '\n' | grep -v '^$' | sort -u)"
fi
release_helpers=""
if [ -f "$RELEASE" ]; then
  release_helpers="$(grep -E '^[[:space:]]*HELPERS:' "$RELEASE" \
    | sed 's/^[[:space:]]*HELPERS:[[:space:]]*//' | tr ' ' '\n' | grep -v '^$' | sort -u)"
fi
setup_helpers=""
if [ -f "$SETUP" ]; then
  setup_helpers="$(sed -n '/HELPER_CRATES/,/];/p' "$SETUP" \
    | grep -oE '"[A-Za-z0-9_-]+"' | tr -d '"' | sort -u)"
fi

missing=""
missing_local=""
missing_make=""
missing_release=""
missing_setup=""
for manifest in "$ROOT"/*/Cargo.toml; do
  [ -f "$manifest" ] || continue
  crate="$(basename "$(dirname "$manifest")")"
  if printf '%s\n' "$EXEMPT" | grep -qx "$crate"; then
    echo "  skip  $crate (exempt)"
    continue
  fi
  if printf '%s\n' "$covered" | grep -qx "$crate"; then
    echo "  ok    $crate"
  else
    echo "  MISS  $crate"
    missing="$missing $crate"
  fi
  if [ -n "$local_covered" ] && ! printf '%s\n' "$local_covered" | grep -qx "$crate"; then
    missing_local="$missing_local $crate"
  fi
  if [ -n "$make_covered" ] && ! printf '%s\n' "$make_covered" | grep -qx "$crate"; then
    missing_make="$missing_make $crate"
  fi
  # Every crate except cli is a helper and must ship in packages + source installs.
  if [ "$crate" != "cli" ]; then
    if [ -n "$release_helpers" ] && ! printf '%s\n' "$release_helpers" | grep -qx "$crate"; then
      missing_release="$missing_release $crate"
    fi
    if [ -n "$setup_helpers" ] && ! printf '%s\n' "$setup_helpers" | grep -qx "$crate"; then
      missing_setup="$missing_setup $crate"
    fi
  fi
done

rc=0
if [ -n "$missing" ]; then
  echo >&2
  echo "FAIL: no CI job covers:$missing" >&2
  echo "Add a job to .github/workflows/ci.yml with 'working-directory: <crate>'" >&2
  echo "(copy an existing crate job), or add the crate to EXEMPT here with a reason." >&2
  rc=1
fi
if [ -n "$missing_local" ]; then
  echo >&2
  echo "FAIL: scripts/ci-local.sh does not mirror:$missing_local" >&2
  echo "Add a matching 'crate_job \"<crate>\" \"<crate>\" \"cargo test\"' line so the" >&2
  echo "Linux jobs stay reproducible before pushing." >&2
  rc=1
fi
if [ -n "$missing_make" ]; then
  echo >&2
  echo "FAIL: Makefile CRATES omits:$missing_make" >&2
  echo "Add it so 'make install' / 'make test' build and test the crate." >&2
  rc=1
fi
if [ -n "$missing_release" ]; then
  echo >&2
  echo "FAIL: release packages would omit:$missing_release" >&2
  echo "Add it to the HELPERS list in .github/workflows/release.yml (and the" >&2
  echo "rust-cache workspaces block) so the .deb and tarball ship the helper." >&2
  rc=1
fi
if [ -n "$missing_setup" ]; then
  echo >&2
  echo "FAIL: cli/src/setup.rs HELPER_CRATES omits:$missing_setup" >&2
  echo "Add it so 'paniolo setup' installs the helper from a source clone." >&2
  rc=1
fi
[ "$rc" = "0" ] && echo "All crates covered by CI, release packaging, and setup."
exit "$rc"
