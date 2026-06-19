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

# Reproduce the Linux GitHub CI jobs (.github/workflows/ci.yml) locally, so you
# can catch failures before pushing. Run it inside a Linux environment:
#
#   # On a Mac, via a Lima VM (Linux on macOS):
#   limactl shell <instance> -- bash -l <repo>/scripts/ci-local.sh
#
#   # Or directly on a Linux box / dev container:
#   bash scripts/ci-local.sh
#
# It installs the toolchain if missing (rustup + uv + apt build deps) and copies
# the working tree to a VM-local dir before building, so nothing is written to a
# shared host mount (a virtiofs/9p mount rejects setuptools' editable egg-info
# and would clobber a host cargo target/). The macOS-only CI job (hdmicap
# AVFoundation + visionocr) is not covered here — run it on the host.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SRC="$(cd "$SCRIPT_DIR/.." && pwd)"
DST="${PANIOLO_CI_DIR:-$HOME/.cache/paniolo-ci-src}"
export DEBIAN_FRONTEND=noninteractive

echo "### [setup] system deps"
sudo apt-get update -qq
sudo apt-get install -y -qq pkg-config libudev-dev build-essential \
  libclang-dev clang cmake nasm libturbojpeg0-dev curl ca-certificates rsync >/dev/null

if ! command -v cargo >/dev/null 2>&1; then
  echo "### [setup] rustup (stable, minimal + clippy + rustfmt)"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --component clippy --component rustfmt >/dev/null
fi
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

if ! command -v uv >/dev/null 2>&1; then
  echo "### [setup] uv"
  curl -LsSf https://astral.sh/uv/install.sh | sh >/dev/null 2>&1
fi
export PATH="$HOME/.local/bin:$PATH"

if ! command -v cargo >/dev/null 2>&1; then
  echo "FATAL: cargo still not on PATH after setup; aborting." >&2
  exit 2
fi
echo "### toolchain: $(cargo --version) | $(uv --version 2>/dev/null || echo 'uv missing')"

echo "### [setup] copy working tree to local disk ($DST)"
mkdir -p "$DST"
rsync -a --delete \
  --exclude 'target' --exclude '.venv' --exclude '*.egg-info' \
  --exclude '_site' --exclude 'site' --exclude '.git' \
  "$SRC/" "$DST/"

declare -A RES

# fmt + clippy (-D warnings) + a final build/test, mirroring each crate's CI job.
crate_job () {
  local name="$1" dir="$2" lastcmd="$3"
  echo
  echo "===== $name ====="
  (
    cd "$DST/$dir" || exit 90
    cargo fmt --check \
      && cargo clippy --all-targets -- -D warnings \
      && $lastcmd
  )
  RES["$name"]=$?
  echo "----- $name exit ${RES[$name]} -----"
}

crate_job "cli"       "cli"       "cargo test"
crate_job "serialcap" "serialcap" "cargo test"
crate_job "netbootd"  "netbootd"  "cargo test"
crate_job "hdmicap"   "hdmicap"   "cargo build"

echo
echo "===== python (pytest) ====="
(
  cd "$DST" \
    && uv sync --quiet \
    && uv run pytest -q
)
RES["pytest"]=$?
echo "----- pytest exit ${RES[pytest]} -----"

echo
echo "########## LOCAL CI SUMMARY ##########"
fail=0
for k in "cli" "serialcap" "netbootd" "hdmicap" "pytest"; do
  c="${RES[$k]:-NA}"
  if [ "$c" = "0" ]; then printf 'PASS       %s\n' "$k"; else printf 'FAIL(%s)  %s\n' "$c" "$k"; fail=1; fi
done
echo "######################################"
exit "$fail"
