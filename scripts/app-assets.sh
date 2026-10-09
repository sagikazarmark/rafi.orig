#!/usr/bin/env bash
# Assembles the app Worker's static files (crates/app/worker-assets, served by
# Workers Assets): the web app bundle (dx) and the participant scripts.
#
#   ./scripts/app-assets.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/crates/app/worker-assets"

"$ROOT/scripts/tailwind.sh"
(cd "$ROOT/crates/app" && dx bundle --platform web --release)
"$ROOT/scripts/participant.sh" "$OUT.participant.tmp"

rm -rf "${OUT:?}"
mkdir -p "$OUT"
cp -R "$ROOT/target/dx/rafi/release/web/public/." "$OUT/"
mv "$OUT.participant.tmp" "$OUT/participant"
echo "assets: $(du -sh "$OUT" | cut -f1) in $OUT"
