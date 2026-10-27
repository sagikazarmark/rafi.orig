#!/usr/bin/env bash
# Assembles the Worker's static files (apps/worker/worker-assets, served by
# Workers Assets): the web app bundle (dx) and the participant scripts.
#
#   CLERK_PUBLISHABLE_KEY=pk_live_… ./scripts/app-assets.sh    # a deployment
#   ./scripts/app-assets.sh --dev                              # dev auth
#
# Clerk's publishable key is compiled into the web app: without one the admin
# has no sign-in at all (dev auth). That is what local development wants and
# what a deployment must never ship by accident (every admin call would be
# 401 with no sign-in button), so an empty key is refused unless --dev says
# so. The mode built is printed.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/apps/worker/worker-assets"
PUBLIC="$ROOT/target/dx/rafi-web/release/web/public"

DEV=0
case "${1:-}" in
  --dev) DEV=1 ;;
  "") ;;
  *) echo "usage: app-assets.sh [--dev]" >&2; exit 2 ;;
esac
if [ "$DEV" = 1 ]; then
  export CLERK_PUBLISHABLE_KEY=""
  echo "app-assets: dev auth (no Clerk key compiled in): for local development only"
elif [ -z "${CLERK_PUBLISHABLE_KEY:-}" ]; then
  echo "app-assets: CLERK_PUBLISHABLE_KEY is not set: the admin would ship without sign-in." >&2
  echo "  Set it (a deployment), or pass --dev (local development with RAFI_DEV_AUTH)." >&2
  exit 1
else
  echo "app-assets: Clerk sign-in with ${CLERK_PUBLISHABLE_KEY:0:12}…"
fi

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
