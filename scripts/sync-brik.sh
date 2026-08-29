#!/usr/bin/env bash
# Sync crate sources to the Windows bench host (brik) for a build there.
# Usage: scripts/sync-brik.sh <crate> [crate...]
#
# Excludes target/ deliberately: the build dirs are gigabytes and the transfer
# stalls the SSH session long before it finishes. Only sources, manifests and
# lockfiles cross the wire; brik keeps its own target/.
#
# BRIK_SSH_AGENT points ssh at a session-scoped agent (see the homelab-ssh
# skill) so a locked 1Password desktop app cannot interrupt a long build.
set -euo pipefail
HOST="${BRIK_HOST:-brik.h.curtisg.xyz}"
DEST='C:/Users/curti/src/paniolo'
SSH_OPTS=(-o BatchMode=yes -o ServerAliveInterval=20)
[ -n "${BRIK_SSH_AGENT:-}" ] && SSH_OPTS+=(-o "IdentityAgent=${BRIK_SSH_AGENT}")
cd "$(dirname "$0")/.."
tar czf - --exclude=target --exclude=.git "$@" \
  | ssh "${SSH_OPTS[@]}" "$HOST" "tar xzf - -C $DEST"
