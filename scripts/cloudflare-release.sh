#!/usr/bin/env bash
# Builds a release of Rafi for Cloudflare and proves it deploys, the way an
# installation deploys it: the release as apps/worker packages it (`just
# release`: the artifact as rafi-cloudflare-<version>.tar.gz with its
# SHA256SUMS); then a copy of deploy/cloudflare fetches that tarball from a
# local server with its own `deploy.sh fetch`, exactly as it fetches a GitHub
# release (the checksum, the version), and `deploy.sh deploy --dry-run` puts
# the documents in place and validates every binding. .github/workflows/release.yaml
# runs this on a tag and attaches what is left in <out dir>; scripts/workers-check.sh
# runs it for the version deploy/cloudflare pins, so CI rehearses a release
# on every push; running it by hand does too.
#
#   ./scripts/cloudflare-release.sh <version> [<out dir>]    # default out dir: target/cloudflare
#   ./scripts/cloudflare-release.sh 0.2.0
#
# Needs what the release needs (just, dx, worker-build), plus python3 (the local server), curl and wrangler.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${1:?usage: cloudflare-release.sh <version> [<out dir>]}"
OUT="${2:-$ROOT/target/cloudflare}"
case "$OUT" in /*) ;; *) OUT="$PWD/$OUT" ;; esac
TARBALL="rafi-cloudflare-$VERSION.tar.gz"
PORT=8765
PIDS=()

cleanup() {
  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap cleanup EXIT

step() { printf '\n== %s\n' "$*"; }

# The pinned version must be this one: that is what an installation fetches.
pinned="$(sed -n 's/^RAFI_VERSION=//p' "$ROOT/deploy/cloudflare/deploy.sh")"
if [ "$pinned" != "$VERSION" ]; then
  echo "deploy/cloudflare/deploy.sh says RAFI_VERSION=$pinned, but this release is $VERSION: bump it first" >&2
  exit 1
fi

step "the release: the artifact, its tarball and checksums"
just "$ROOT/apps/worker/" release "$VERSION" "$OUT"
cat "$OUT/SHA256SUMS"

step "deploy/cloudflare fetches and deploys it (dry run)"
rm -rf "$OUT/installation"
mkdir -p "$OUT/installation"
(cd "$ROOT/deploy/cloudflare" && tar -c --exclude ./rafi --exclude ./node_modules --exclude ./.wrangler .) \
  | tar -x -C "$OUT/installation"
(cd "$OUT" && exec python3 -m http.server "$PORT" --bind 127.0.0.1) >/dev/null 2>&1 &
PIDS+=($!)
for _ in $(seq 1 30); do curl -s -m 2 -o /dev/null "http://127.0.0.1:$PORT/SHA256SUMS" && break; sleep 1; done
(cd "$OUT/installation" && RAFI_RELEASE_URL="http://127.0.0.1:$PORT" ./deploy.sh fetch)
dry="$(cd "$OUT/installation" && ./deploy.sh deploy --dry-run 2>&1)" \
  || { echo "$dry" | tail -30 >&2; exit 1; }
echo "$dry" | grep -q "env.ASSETS" || { echo "the dry deploy lists no assets binding:" >&2; echo "$dry" | tail -20 >&2; exit 1; }
echo "ok   deploy/cloudflare deploys $TARBALL"
rm -rf "$OUT/installation"

step "release $VERSION: $OUT/$TARBALL and $OUT/SHA256SUMS"
