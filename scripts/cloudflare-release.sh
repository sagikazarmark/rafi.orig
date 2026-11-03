#!/usr/bin/env bash
# Builds a release of Rafi for Cloudflare and proves it deploys, the way an
# installation deploys it. First that deploy/cloudflare, which an
# installation copies at the release's tag, names this release (its VERSION),
# that its two workflows (.github/workflows/deploy.yaml and
# restate-cleanup.yaml) are written for deploy.sh's interface (their
# RAFI_DEPLOY_VERSION, deploy.sh's DEPLOY_VERSION), name one Rafi repository
# (RAFI_REPOSITORY) and verify the attestation release.yaml makes, and that
# release.yaml makes it. Then deploy.sh's own checks
# (scripts/cloudflare-deploy-test.sh: each subcommand against a stubbed
# wrangler and Restate CLI and a fake Cloudflare and Restate API). Then the
# release (`just cloudflare-tarball`: worker.tar.gz, its files at the top level:
# DEPLOY_VERSION, VERSION, app/, engine/build/, rooms/build/, migrations/,
# deploy.sh), its layout and each Worker's exports checked (RaffleRoom the
# rooms Worker's alone); then a copy of deploy/cloudflare with the tarball
# unpacked into its release/, as an installation's workflow unpacks it, runs
# `release/deploy.sh dry-run` with the workflows' RAFI_DEPLOY_VERSION:
# deploy.sh's checks of the release, VERSION, the deploy interface and the
# three configurations (the template's placeholders only warn in a dry run),
# legal/ copied in, and `wrangler deploy --dry-run` of the rooms Worker, the
# engine Worker under its versioned name and the app Worker, whose bindings
# are checked here. .github/workflows/release.yaml runs
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
ENGINE_NAME="rafi-engine-v$(printf '%s' "$VERSION" | tr '.' '-' | tr '[:upper:]' '[:lower:]')"

step() { printf '\n== %s\n' "$*"; }
fail() {
  echo "$*" >&2
  exit 1
}

step "deploy/cloudflare deploys this release, with its workflows"
# An installation's repository is a copy of deploy/cloudflare taken at a tag:
# its VERSION names the release it deploys, and its workflows run that
# release's deploy.sh, which refuses workflows written for another deploy
# interface (RAFI_DEPLOY_VERSION). Both workflows download worker.tar.gz from
# the repository they name and verify that release.yaml attested it.
template="$ROOT/deploy/cloudflare"
named="$(tr -d '[:space:]' <"$template/VERSION" 2>/dev/null || true)"
[ "$named" = "$VERSION" ] || fail "deploy/cloudflare/VERSION says '$named', not $VERSION: set it to $VERSION"
interface="$(sed -n 's/^DEPLOY_VERSION=\([0-9][0-9]*\)$/\1/p' "$ROOT/scripts/cloudflare-deploy.sh")"
[[ "$interface" =~ ^[0-9]+$ ]] || fail "scripts/cloudflare-deploy.sh has no DEPLOY_VERSION=<integer> line"
upstream=
for workflow in deploy.yaml restate-cleanup.yaml; do
  file="$template/.github/workflows/$workflow"
  [ -f "$file" ] || fail "no deploy/cloudflare/.github/workflows/$workflow"
  wanted="$(sed -n 's/^  RAFI_DEPLOY_VERSION: *"\{0,1\}\([^" #]*\)"\{0,1\}.*$/\1/p' "$file")"
  [ "$wanted" = "$interface" ] \
    || fail "$workflow says RAFI_DEPLOY_VERSION '${wanted//$'\n'/, }', but deploy.sh's DEPLOY_VERSION is $interface: update the workflow for the new interface, then its RAFI_DEPLOY_VERSION"
  repository="$(sed -n 's/^  RAFI_REPOSITORY: *\([^ #]*\).*/\1/p' "$file")"
  [[ "$repository" =~ ^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$ ]] || fail "$workflow names no one RAFI_REPOSITORY: '$repository'"
  [ -z "$upstream" ] || [ "$repository" = "$upstream" ] || fail "the workflows name two Rafi repositories: $upstream and $repository"
  upstream="$repository"
  # shellcheck disable=SC2016 # the workflow's own shell variable, matched literally
  grep -qF -- '--signer-workflow "github.com/$RAFI_REPOSITORY/.github/workflows/release.yaml"' "$file" \
    || fail "$workflow doesn't verify the attestation's signer as release.yaml"
  echo "ok   $workflow: deploy interface $interface, releases of $repository attested by release.yaml"
done
grep -q 'uses: actions/attest-build-provenance@' "$ROOT/.github/workflows/release.yaml" \
  || fail "release.yaml doesn't attest the tarball: the workflows would refuse every release"
echo "ok   deploy/cloudflare/VERSION is $VERSION"

step "deploy.sh's own checks"
"$ROOT/scripts/cloudflare-deploy-test.sh"

step "the release: worker.tar.gz"
just "$ROOT/" cloudflare-tarball "$VERSION" "$OUT"
if command -v sha256sum >/dev/null; then SHA256="sha256sum"; else SHA256="shasum -a 256"; fi
$SHA256 "$OUT/worker.tar.gz"
# What an installation's release/ is: these at the top level, nothing else,
# and none of an installation's documents.
listing="$(tar -tzf "$OUT/worker.tar.gz")"
top="$(echo "$listing" | cut -d/ -f1 | LC_ALL=C sort -u | tr '\n' ' ')"
[ "$top" = "DEPLOY_VERSION VERSION app deploy.sh engine migrations rooms " ] || fail "worker.tar.gz has at its top level: $top"
for file in DEPLOY_VERSION VERSION deploy.sh app/build/index.js app/assets/_headers engine/build/index.js \
  rooms/build/index.js migrations/0001_init.sql; do
  grep -qx "$file" <<<"$listing" || fail "worker.tar.gz has no $file"
done
# engine/ and rooms/ are their build/ alone; migrations/ the D1 migrations' files alone.
! grep -vE '^(engine|rooms)/(build/.*)?$' <<<"$(grep -E '^(engine|rooms)/' <<<"$listing")" \
  || fail "worker.tar.gz has more than build/ in engine/ or rooms/"
! grep -vE '^migrations/([0-9]{4}_[a-z0-9_]+\.sql)?$' <<<"$(grep '^migrations/' <<<"$listing")" \
  || fail "worker.tar.gz has more than the D1 migrations' .sql files in migrations/"
! grep -q '^app/assets/legal' <<<"$listing" || fail "worker.tar.gz has app/assets/legal/: an installation's"
echo "ok   worker.tar.gz: DEPLOY_VERSION, VERSION, app/, engine/build/, rooms/build/, migrations/ and deploy.sh"

step "deploy/cloudflare deploys it (dry run)"
installation="$OUT/installation"
rm -rf "$installation"
mkdir -p "$installation/release"
(cd "$ROOT/deploy/cloudflare" && tar -c --exclude ./release --exclude ./node_modules --exclude ./.wrangler --exclude ./.prod.vars .) \
  | tar -x -C "$installation"
tar -xzf "$OUT/worker.tar.gz" -C "$installation/release"
shipped="$(cat "$installation/release/VERSION")"
[ "$shipped" = "$VERSION" ] || fail "the release unpacked says it is $shipped, not $VERSION"
[ "$(cat "$installation/release/DEPLOY_VERSION")" = "$interface" ] \
  || fail "the release's DEPLOY_VERSION is '$(cat "$installation/release/DEPLOY_VERSION")', not deploy.sh's $interface"
echo "ok   DEPLOY_VERSION $interface, deploy.sh's"
# Each Worker's module exports (worker-build's index.js ends in one export
# list): the RaffleRoom class is the rooms Worker's alone, since a Worker
# exporting it would host a second class of that name.
exports() {
  grep -o 'export *{[^}]*}' "$installation/release/$1/build/index.js" | tail -1 \
    | sed 's/^export *{//; s/}$//' | tr ',' '\n' | sed 's/.* as //; s/^ *//; s/ *$//' | LC_ALL=C sort | paste -sd ' ' -
}
for worker in app engine rooms; do
  list="$(exports "$worker")"
  grep -qw default <<<"$list" || fail "the $worker Worker exports no default handler: $list"
  if [ "$worker" = rooms ]; then
    grep -qw RaffleRoom <<<"$list" || fail "the rooms Worker doesn't export RaffleRoom: $list"
  else
    ! grep -qw RaffleRoom <<<"$list" || fail "the $worker Worker exports RaffleRoom, the rooms Worker's class: $list"
  fi
  echo "ok   the $worker Worker exports: $list"
done
[ -x "$installation/release/deploy.sh" ] || fail "release/deploy.sh isn't executable"
dry="$(cd "$installation" && WRANGLER="$WRANGLER" RAFI_VERSION="$VERSION" RAFI_DEPLOY_VERSION="$interface" \
  release/deploy.sh dry-run 2>&1)" \
  || { echo "$dry" | tail -40 >&2; fail "release/deploy.sh dry-run failed"; }
for doc in terms.md privacy.md; do
  cmp -s "$ROOT/deploy/cloudflare/legal/$doc" "$installation/release/app/assets/legal/$doc" \
    || fail "deploy.sh didn't put legal/$doc into release/app/assets/legal/"
done
echo "ok   deploy.sh copied legal/ into release/app/assets/legal/"
# Each Worker's part of the output, by deploy.sh's step lines: from its own to the next.
section() {
  echo "$dry" | awk -v head="==> $1: wrangler deploy --dry-run" '
    index($0, "==> ") == 1 { inside = ($0 == head) }
    inside'
}
rooms="$(section "rooms Worker rafi-rooms")"
app="$(section "app Worker rafi")"
engine="$(section "engine Worker $ENGINE_NAME")"
[ -n "$rooms" ] || { echo "$dry" | tail -40 >&2; fail "no dry deploy of the rooms Worker rafi-rooms"; }
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
expect "the rooms Worker" "$rooms" '^env\.DB \(rafi\) ' '^env\.RAFFLE_ROOM \(RaffleRoom\) ' '^env\.RAFI_DO_JURISDICTION \("eu"\) '
! grep -qE '^env\.(ASSETS|[A-Z_]+_LIMITER|RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS) ' <<<"$rooms" \
  || fail "the rooms Worker binds the app's or the engine's"
! grep -qE '^env\.RAFFLE_ROOM \(RaffleRoom, defined in ' <<<"$rooms" || fail "the rooms Worker binds another Worker's RaffleRoom"
expect "the app Worker" "$app" '^env\.ASSETS ' '^env\.DB \(rafi\) ' '^env\.RAFFLE_ROOM \(RaffleRoom, defined in rafi-rooms\) ' \
  '^env\.JOIN_PER_RAFFLE_LIMITER ' '^env\.JOIN_PER_EMAIL_LIMITER ' '^env\.CONNECT_LIMITER ' '^env\.ADD_LIMITER ' \
  '^env\.RAFI_DO_JURISDICTION \("eu"\) '
expect "the engine Worker" "$engine" '^env\.DB \(rafi\) ' '^env\.RAFFLE_ROOM \(RaffleRoom, defined in rafi-rooms\) ' \
  '^env\.RAFI_DO_JURISDICTION \("eu"\) ' '^env\.RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS '
! grep -qE '^env\.(ASSETS|[A-Z_]+_LIMITER) ' <<<"$engine" || fail "the engine Worker binds the app's assets or limits"
echo "ok   the rooms Worker rafi-rooms: DB, RAFFLE_ROOM (its own), RAFI_DO_JURISDICTION"
echo "ok   the app Worker rafi: ASSETS, DB, RAFFLE_ROOM (rafi-rooms'), the four rate limiters, RAFI_DO_JURISDICTION"
echo "ok   the engine Worker $ENGINE_NAME: DB, RAFFLE_ROOM (rafi-rooms'), RAFI_DO_JURISDICTION, RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS"
rm -rf "$installation"

step "release $VERSION: $OUT/worker.tar.gz"
