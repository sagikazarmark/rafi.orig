#!/usr/bin/env bash
# Assembles the Worker's static files (apps/worker/worker-assets, served by
# Workers Assets): the web app bundle (dx), the participant scripts and the
# installation's legal documents.
#
#   ./scripts/app-assets.sh                                    # the templates as documents
#   RAFI_DOCS_DIR=/path/to/your/legal ./scripts/app-assets.sh  # reviewed documents (wrangler dev as an installation)
#
# Nothing of an installation's is compiled into the web app: the server tells
# it how organizers sign in at runtime (/config.js), so one bundle serves
# every installation. The legal documents (terms.md, privacy.md) are files
# the Worker reads from legal/ among these: from RAFI_DOCS_DIR, the same
# variable the native server reads them from, or the repository's templates
# (deploy/cloudflare/legal), which carry the template marker, so a production
# Worker refuses them. A deployment doesn't run this: its documents go in
# through deploy/cloudflare (scripts/cloudflare-artifact.sh leaves legal/ out).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/apps/worker/worker-assets"
PUBLIC="$ROOT/target/dx/rafi-web/release/web/public"
DOCS="${RAFI_DOCS_DIR:-$ROOT/deploy/cloudflare/legal}"

[ $# -eq 0 ] || { echo "usage: RAFI_DOCS_DIR=<dir> app-assets.sh" >&2; exit 2; }
for doc in terms.md privacy.md; do
  [ -f "$DOCS/$doc" ] || { echo "app-assets: $DOCS/$doc is missing (RAFI_DOCS_DIR)" >&2; exit 1; }
done
if grep -q '\*\*Template\.\*\*' "$DOCS/terms.md" "$DOCS/privacy.md"; then
  echo "app-assets: legal documents from $DOCS: the templates (a production Worker refuses them)"
else
  echo "app-assets: legal documents from $DOCS"
fi

# dx bundle leaves earlier bundles (other hashes) in its output: start empty.
rm -rf "${PUBLIC:?}"
(cd "$ROOT/apps/web" && npm ci --no-fund --no-audit && dx bundle --platform web --release)
"$ROOT/scripts/participant.sh" "$OUT.participant.tmp"

rm -rf "${OUT:?}"
mkdir -p "$OUT/legal"
cp -R "$PUBLIC/." "$OUT/"
mv "$OUT.participant.tmp" "$OUT/participant"
cp "$DOCS/terms.md" "$DOCS/privacy.md" "$OUT/legal/"
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
