#!/usr/bin/env bash
# Builds what a Cloudflare deployment of Rafi is made of, into one directory:
#
#   build/              the Worker (worker-build --release): index.js and its wasm
#   worker-assets/      the web app and the participant scripts (scripts/app-assets.sh)
#   migrations/sqlite/  the D1 migrations
#   VERSION             which Rafi this is
#
# Nothing of an installation's is in it: its legal documents go into
# worker-assets/legal/ and its values into secrets at deploy time, which is
# deploy/cloudflare's job (its wrangler.toml points into this layout as
# rafi/). A release's tarball is this directory, built by
# .github/workflows/release.yaml; building from source is running this and
# pointing deploy/cloudflare at the result (docs/operations/cloudflare.md).
#
#   ./scripts/cloudflare-artifact.sh [<out dir>]    # default: target/cloudflare/rafi
#   RAFI_VERSION=0.2.0 ./scripts/cloudflare-artifact.sh   # what VERSION says (default: git describe)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/target/cloudflare/rafi}"
case "$OUT" in /*) ;; *) OUT="$PWD/$OUT" ;; esac
VERSION="${RAFI_VERSION:-$(git -C "$ROOT" describe --tags --always --dirty 2>/dev/null || echo unknown)}"

echo "cloudflare-artifact: Rafi $VERSION -> $OUT"
"$ROOT/scripts/app-assets.sh"
(cd "$ROOT/apps/worker" && worker-build --release)

rm -rf "${OUT:?}"
mkdir -p "$OUT/migrations"
cp -R "$ROOT/apps/worker/build" "$OUT/build"
rm -f "$OUT/build/.gitignore"
cp -R "$ROOT/apps/worker/worker-assets" "$OUT/worker-assets"
# The documents are the installation's, not Rafi's.
rm -rf "$OUT/worker-assets/legal"
cp -R "$ROOT/migrations/sqlite" "$OUT/migrations/sqlite"
echo "$VERSION" >"$OUT/VERSION"
echo "cloudflare-artifact: $(du -sh "$OUT" | cut -f1) in $OUT"
