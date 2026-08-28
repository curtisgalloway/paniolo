#!/usr/bin/env bash
# Sync crate sources to the Windows bench host (brik) for a build there.
# Usage: scripts/sync-brik.sh <crate> [crate...]
#
# Excludes target/ deliberately: the build dirs are gigabytes and the transfer
# stalls the SSH session long before it finishes. Only sources, manifests and
# lockfiles cross the wire; brik keeps its own target/.
set -euo pipefail
HOST="${BRIK_HOST:-brik.h.curtisg.xyz}"
DEST='C:/Users/curti/src/paniolo'
cd "$(dirname "$0")/.."
tar czf - --exclude=target --exclude=.git "$@" \
  | ssh -o BatchMode=yes -o ServerAliveInterval=20 "$HOST" "tar xzf - -C $DEST"
