#!/usr/bin/env bash
# Deploys this installation of Rafi to Cloudflare Workers.
#
#   ./deploy.sh                      # fetch the release named below, then deploy
#   ./deploy.sh fetch                # only fetch it: into rafi/, checked against RAFI_SHA256 (or its SHA256SUMS)
#   ./deploy.sh deploy [--dry-run]   # only deploy rafi/: legal/ into place, the D1 migrations,
#                                    # wrangler deploy (--dry-run: validate, touch nothing)
#
# The release is Rafi's build (the Worker, the static files, the D1 migrations),
# the same for every installation. What is yours is this directory:
# wrangler.toml, legal/ and the secrets (.dev.vars.example). Upgrading is
# changing the two lines below: the version, and the tarball's SHA256 as the
# release page shows it (GitHub computes one for every asset at upload).
# Set, RAFI_SHA256 is what the download is checked against, so this file
# says which bytes the installation deploys and an asset that changed under
# its version is refused. Empty (as the template ships it: the template
# can't know the hash of a release not yet built), the download is checked
# against the release's own SHA256SUMS, which catches a corrupt download and
# nothing else. A build from source (the Rafi repository's `just
# cloudflare-artifact`, into rafi/) is deployed with `deploy` alone.
#
# Needs curl, tar, sha256sum (or shasum) and wrangler (WRANGLER names another
# command; npx fetches it when none is installed).

RAFI_VERSION=0.1.0
RAFI_SHA256=

set -euo pipefail
cd "$(dirname "$0")"

RELEASE="${RAFI_RELEASE_URL:-https://github.com/sagikazarmark/rafi/releases/download/v$RAFI_VERSION}"
TARBALL="rafi-cloudflare-$RAFI_VERSION.tar.gz"
if [ -z "${WRANGLER:-}" ]; then
  if command -v wrangler >/dev/null; then WRANGLER=wrangler; else WRANGLER="npx --yes wrangler@4"; fi
fi
if command -v sha256sum >/dev/null; then SHA256="sha256sum"; else SHA256="shasum -a 256"; fi

fetch() {
  echo "deploy: Rafi $RAFI_VERSION from $RELEASE"
  tmp="$(mktemp -d)"
  curl -fsSL -o "$tmp/$TARBALL" "$RELEASE/$TARBALL"
  # The release page shows the digest as sha256:<hex>; either form pins.
  pinned="${RAFI_SHA256#sha256:}"
  if [ -n "$pinned" ]; then
    # Against the pin above: the bytes this file names, whatever the
    # release serves today.
    got="$($SHA256 "$tmp/$TARBALL" | cut -d' ' -f1)"
    [ "$got" = "$pinned" ] || {
      echo "deploy: $TARBALL is sha256:$got, RAFI_SHA256 pins $pinned: not the release this file names (upgrading? set RAFI_SHA256 from the release page)" >&2
      exit 1
    }
  else
    # Against the checksums that travel with the release: this catches a
    # corrupt download, not a changed release. Pin RAFI_SHA256 for that.
    curl -fsSL -o "$tmp/SHA256SUMS" "$RELEASE/SHA256SUMS"
    line="$(grep " $TARBALL\$" "$tmp/SHA256SUMS")" || { echo "deploy: SHA256SUMS has no line for $TARBALL" >&2; exit 1; }
    (cd "$tmp" && echo "$line" | $SHA256 -c - >/dev/null)
  fi
  rm -rf rafi
  mkdir rafi
  tar -xzf "$tmp/$TARBALL" -C rafi --strip-components=1
  rm -rf "$tmp"
  shipped="$(cat rafi/VERSION)"
  [ "$shipped" = "$RAFI_VERSION" ] || { echo "deploy: the release says it is $shipped, not $RAFI_VERSION" >&2; exit 1; }
}

deploy() {
  [ -f rafi/build/index.js ] || { echo "deploy: rafi/ holds no release: ./deploy.sh fetch, or a build from source" >&2; exit 1; }
  mkdir -p rafi/worker-assets/legal
  for doc in terms.md privacy.md; do
    [ -f "legal/$doc" ] || { echo "deploy: legal/$doc is missing" >&2; exit 1; }
    cp "legal/$doc" rafi/worker-assets/legal/
  done
  if grep -q '\*\*Template\.\*\*' legal/terms.md legal/privacy.md; then
    echo "deploy: legal/ still holds the templates (they say **Template.**): the Worker refuses every request until reviewed documents replace them" >&2
  fi
  if [ "${1:-}" = "--dry-run" ]; then
    $WRANGLER deploy --dry-run
  else
    $WRANGLER d1 migrations apply DB --remote
    $WRANGLER deploy
  fi
}

case "${1:-}" in
  "") fetch; deploy ;;
  fetch) fetch ;;
  deploy) shift; deploy "$@" ;;
  *) echo "usage: deploy.sh [fetch | deploy [--dry-run]]" >&2; exit 2 ;;
esac
