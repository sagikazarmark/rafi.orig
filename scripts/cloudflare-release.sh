#!/usr/bin/env bash
# Builds a release of Rafi for Cloudflare and proves it deploys, the way an
# installation deploys it. First deploy.sh's own checks
# (scripts/cloudflare-deploy-test.sh: the configurations, the version, the
# engine Worker's name, which deployments a prune touches, the "is anything
# live?" preflight on a local D1). Then the release (`just
# cloudflare-tarball`: worker.tar.gz, its files at the top level: VERSION,
# app/, engine/, migrations/sqlite/, deploy.sh), its layout checked; then a
# copy of deploy/cloudflare with the tarball unpacked into its release/, as an
# installation's workflow unpacks it, runs `release/deploy.sh --dry-run`:
# deploy.sh's checks of the release and the two configurations (the template's
# placeholders only warn in a dry run), legal/ copied in, and `wrangler deploy
# --dry-run` of the app Worker and of the engine Worker under its versioned
# name, whose bindings are checked here. .github/workflows/release.yaml runs
# this on a tag and attaches <out dir>/worker.tar.gz; scripts/workers-check.sh
# runs it too, so CI rehearses a release on every push; running it by hand
# does too.
#
#   ./scripts/cloudflare-release.sh <version> [<out dir>]    # default out dir: target/cloudflare
#   ./scripts/cloudflare-release.sh 0.2.0
#
# Needs what the release needs (just, dx, worker-build, GNU tar), plus node
# and wrangler (WRANGLER names another command; deploy.sh is given the same,
# so nothing is installed).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${1:?usage: cloudflare-release.sh <version> [<out dir>]}"
OUT="${2:-$ROOT/target/cloudflare}"
case "$OUT" in /*) ;; *) OUT="$PWD/$OUT" ;; esac
WRANGLER="${WRANGLER:-wrangler}"
ENGINE_NAME="rafi-engine-$(printf '%s' "$VERSION" | tr '.' '-' | tr '[:upper:]' '[:lower:]')"

step() { printf '\n== %s\n' "$*"; }
fail() {
  echo "$*" >&2
  exit 1
}

step "deploy.sh's own checks"
WRANGLER="$WRANGLER" "$ROOT/scripts/cloudflare-deploy-test.sh"

step "the release: worker.tar.gz"
just "$ROOT/" cloudflare-tarball "$VERSION" "$OUT"
if command -v sha256sum >/dev/null; then SHA256="sha256sum"; else SHA256="shasum -a 256"; fi
$SHA256 "$OUT/worker.tar.gz"
# What an installation's release/ is: these at the top level, nothing else,
# and none of an installation's documents.
listing="$(tar -tzf "$OUT/worker.tar.gz")"
top="$(echo "$listing" | cut -d/ -f1 | sort -u | tr '\n' ' ')"
[ "$top" = "VERSION app deploy.sh engine migrations " ] || fail "worker.tar.gz has at its top level: $top"
for file in VERSION deploy.sh app/build/index.js app/assets/_headers engine/build/index.js migrations/sqlite/0001_init.sql; do
  grep -qx "$file" <<<"$listing" || fail "worker.tar.gz has no $file"
done
! grep -q '^app/assets/legal' <<<"$listing" || fail "worker.tar.gz has app/assets/legal/: an installation's"
echo "ok   worker.tar.gz: VERSION, app/, engine/, migrations/sqlite/ and deploy.sh"

step "deploy/cloudflare deploys it (dry run)"
installation="$OUT/installation"
rm -rf "$installation"
mkdir -p "$installation/release"
(cd "$ROOT/deploy/cloudflare" && tar -c --exclude ./release --exclude ./node_modules --exclude ./.wrangler --exclude ./.dev.vars .) \
  | tar -x -C "$installation"
tar -xzf "$OUT/worker.tar.gz" -C "$installation/release"
shipped="$(cat "$installation/release/VERSION")"
[ "$shipped" = "$VERSION" ] || fail "the release unpacked says it is $shipped, not $VERSION"
[ -x "$installation/release/deploy.sh" ] || fail "release/deploy.sh isn't executable"
dry="$(cd "$installation" && WRANGLER="$WRANGLER" RAFI_VERSION="$VERSION" release/deploy.sh --dry-run 2>&1)" \
  || { echo "$dry" | tail -40 >&2; fail "release/deploy.sh --dry-run failed"; }
for doc in terms.md privacy.md; do
  cmp -s "$ROOT/deploy/cloudflare/legal/$doc" "$installation/release/app/assets/legal/$doc" \
    || fail "deploy.sh didn't put legal/$doc into release/app/assets/legal/"
done
echo "ok   deploy.sh copied legal/ into release/app/assets/legal/"
# Each Worker's part of the output, by deploy.sh's step lines.
app="$(echo "$dry" | sed -n '/^==> app Worker rafi: wrangler deploy --dry-run/,/^==> engine Worker/p')"
engine="$(echo "$dry" | sed -n "/^==> engine Worker $ENGINE_NAME: wrangler deploy --dry-run/,/^==> dry run of Rafi $VERSION done/p")"
[ -n "$app" ] || { echo "$dry" | tail -40 >&2; fail "no dry deploy of the app Worker rafi"; }
[ -n "$engine" ] || { echo "$dry" | tail -40 >&2; fail "no dry deploy of the engine Worker $ENGINE_NAME (a versioned name)"; }
# expect <what> <output> <binding line patterns...>
expect() {
  local what="$1" output="$2" pattern
  shift 2
  for pattern in "$@"; do
    grep -qE "$pattern" <<<"$output" || { echo "$output" | tail -30 >&2; fail "$what: no binding matching '$pattern'"; }
  done
}
expect "the app Worker" "$app" '^env\.ASSETS ' '^env\.DB \(rafi\) ' '^env\.RAFFLE_ROOM \(RaffleRoom\) ' \
  '^env\.JOIN_PER_RAFFLE_LIMITER ' '^env\.JOIN_PER_EMAIL_LIMITER ' '^env\.CONNECT_LIMITER ' '^env\.ADD_LIMITER ' \
  '^env\.RAFI_DO_JURISDICTION \("eu"\) '
expect "the engine Worker" "$engine" '^env\.DB \(rafi\) ' '^env\.RAFFLE_ROOM \(RaffleRoom, defined in rafi\) ' \
  '^env\.RAFI_DO_JURISDICTION \("eu"\) ' '^env\.RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS '
! grep -qE '^env\.(ASSETS|[A-Z_]+_LIMITER) ' <<<"$engine" || fail "the engine Worker binds the app's assets or limits"
echo "ok   the app Worker rafi: ASSETS, DB, RAFFLE_ROOM, the four rate limiters, RAFI_DO_JURISDICTION"
echo "ok   the engine Worker $ENGINE_NAME: DB, RAFFLE_ROOM (rafi's), RAFI_DO_JURISDICTION, RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS"
rm -rf "$installation"

step "release $VERSION: $OUT/worker.tar.gz"
