#!/usr/bin/env bash
# Builds a release of Rafi for Cloudflare and proves it deploys, the way an
# installation deploys it: the release as apps/worker packages it (`just
# release`: the artifact as worker.tar.gz, its files at the top level); then
# a copy of deploy/cloudflare runs its own `npm run build`, which downloads
# that tarball exactly as it downloads a GitHub release's (here from a
# file:// URL laid out like GitHub's, through RAFI_RELEASE_URL) and puts the
# documents in place, and `wrangler deploy --dry-run` there validates every
# binding. .github/workflows/release.yaml runs this on a tag and attaches
# <out dir>/worker.tar.gz; scripts/workers-check.sh runs it for the version
# deploy/cloudflare pins, so CI rehearses a release on every push; running it
# by hand does too.
#
#   ./scripts/cloudflare-release.sh <version> [<out dir>]    # default out dir: target/cloudflare
#   ./scripts/cloudflare-release.sh 0.2.0
#
# Needs what the release needs (just, dx, worker-build), plus node and npm
# (the template's scripts), curl and wrangler (WRANGLER names another
# command; the template's npm scripts find wrangler on PATH, so nothing is
# installed).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${1:?usage: cloudflare-release.sh <version> [<out dir>]}"
OUT="${2:-$ROOT/target/cloudflare}"
case "$OUT" in /*) ;; *) OUT="$PWD/$OUT" ;; esac
WRANGLER="${WRANGLER:-wrangler}"

step() { printf '\n== %s\n' "$*"; }

# The pinned version must be this one: that is what an installation fetches.
pinned="$(node -p "require('$ROOT/deploy/cloudflare/package.json').config.rafi_version")"
if [ "$pinned" != "$VERSION" ]; then
  echo "deploy/cloudflare/package.json says config.rafi_version is $pinned, but this release is $VERSION: bump it first" >&2
  exit 1
fi

# The Deploy to Cloudflare buttons deploy the template at this release's tag,
# so the template and the release it fetches agree; between releases they
# keep naming the last one.
for doc in deploy/cloudflare/README.md docs/operations/cloudflare.md; do
  if ! grep -q "tree/v$VERSION/deploy/cloudflare" "$ROOT/$doc"; then
    echo "$doc: its Deploy to Cloudflare button doesn't name tree/v$VERSION/deploy/cloudflare: point it at this release" >&2
    exit 1
  fi
done

step "the release: the artifact and worker.tar.gz"
just "$ROOT/apps/worker/" release "$VERSION" "$OUT"
if command -v sha256sum >/dev/null; then SHA256="sha256sum"; else SHA256="shasum -a 256"; fi
$SHA256 "$OUT/worker.tar.gz"

step "deploy/cloudflare builds and deploys it (dry run)"
# The tarball where GitHub serves a release's assets: <base>/v<version>/worker.tar.gz.
rm -rf "$OUT/download" "$OUT/installation"
mkdir -p "$OUT/download/v$VERSION" "$OUT/installation"
cp "$OUT/worker.tar.gz" "$OUT/download/v$VERSION/"
(cd "$ROOT/deploy/cloudflare" && tar -c --exclude ./worker --exclude ./node_modules --exclude ./.wrangler --exclude ./.dev.vars .) \
  | tar -x -C "$OUT/installation"
built="$(cd "$OUT/installation" && RAFI_VERSION='' RAFI_RELEASE_URL="file://$OUT/download" npm run build 2>&1)" \
  || { echo "$built" | tail -30 >&2; exit 1; }
shipped="$(cat "$OUT/installation/worker/VERSION")"
[ "$shipped" = "$VERSION" ] || { echo "the release unpacked says it is $shipped, not $VERSION" >&2; exit 1; }
for doc in terms.md privacy.md; do
  cmp -s "$ROOT/deploy/cloudflare/legal/$doc" "$OUT/installation/worker/worker-assets/legal/$doc" \
    || { echo "npm run build didn't put legal/$doc into worker/worker-assets/legal/" >&2; exit 1; }
done
echo "ok   npm run build unpacked Rafi $shipped into worker/, with legal/"
dry="$(cd "$OUT/installation" && $WRANGLER deploy --dry-run 2>&1)" \
  || { echo "$dry" | tail -30 >&2; exit 1; }
echo "$dry" | grep -q "env.ASSETS" || { echo "the dry deploy lists no assets binding:" >&2; echo "$dry" | tail -20 >&2; exit 1; }
echo "ok   deploy/cloudflare deploys worker.tar.gz"
rm -rf "$OUT/installation" "$OUT/download"

step "release $VERSION: $OUT/worker.tar.gz"
