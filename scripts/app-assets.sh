#!/usr/bin/env bash
# Assembles the app Worker's static files (apps/worker/worker-assets, served by
# Workers Assets): the web app bundle (dx) and the participant scripts.
#
#   ./scripts/app-assets.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/apps/worker/worker-assets"
PUBLIC="$ROOT/target/dx/rafi-web/release/web/public"

# dx bundle leaves earlier bundles (other hashes) in its output: start empty.
rm -rf "${PUBLIC:?}"
(cd "$ROOT/apps/web" && npm ci --no-fund --no-audit && dx bundle --platform web --release)
"$ROOT/scripts/participant.sh" "$OUT.participant.tmp"

rm -rf "${OUT:?}"
mkdir -p "$OUT"
cp -R "$PUBLIC/." "$OUT/"
mv "$OUT.participant.tmp" "$OUT/participant"
echo "assets: $(du -sh "$OUT" | cut -f1) in $OUT"
