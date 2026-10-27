#!/usr/bin/env bash
# Assembles the Worker's static files (apps/worker/worker-assets, served by
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
# Cloudflare compresses at the edge; the precompressed variants are for the
# native server and would only be served as files here.
rm -f "$OUT"/participant/*.gz "$OUT"/participant/*.br

# Workers Assets' headers (https://developers.cloudflare.com/workers/static-assets/headers/),
# the same ones rafi-server puts on what it answers itself (security_headers
# in crates/server/src/lib.rs; rules here don't apply to the Worker's own
# responses). Every file: no sniffing, the origin only as referrer. The web
# app's shell may be framed only as the projector (/s/) and the verification
# page (/v/): later rules add to earlier ones, and the first two rules below
# deny framing everywhere, so the two frameable paths can't be matched by
# them; they are listed as the exception instead. The participant scripts
# keep fixed names, so phones must revalidate them: Assets does that by
# default (max-age=0, must-revalidate, with an ETag); this says so.
cat >"$OUT/_headers" <<'HEADERS'
/*
  X-Content-Type-Options: nosniff
  Referrer-Policy: strict-origin-when-cross-origin
  Content-Security-Policy: frame-ancestors 'none'
  X-Frame-Options: DENY

/s/*
  ! Content-Security-Policy
  ! X-Frame-Options

/v/*
  ! Content-Security-Policy
  ! X-Frame-Options

/participant/*
  Cache-Control: no-cache
HEADERS
echo "assets: $(du -sh "$OUT" | cut -f1) in $OUT"
