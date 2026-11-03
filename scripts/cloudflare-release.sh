#!/usr/bin/env bash
# Builds a release of Rafi for Cloudflare and proves it deploys, the way an
# installation deploys it. First that deploy/cloudflare, which an
# installation copies at the release's tag, names this release (its VERSION),
# that its two workflows (.github/workflows/deploy.yaml and
# restate-cleanup.yaml) name one Rafi repository (RAFI_REPOSITORY), verify the
# attestation release.yaml makes and read the version file deploy.sh checks
# (VERSION.<CLOUDFLARE_ENV> when it exists, VERSION otherwise), and that
# release.yaml makes it. Then deploy.sh's own checks
# (scripts/cloudflare-deploy-test.sh: each subcommand against a stubbed
# wrangler and Restate CLI and a fake Cloudflare and Restate API). Then the
# release (`just cloudflare-tarball`: worker.tar.gz, its files at the top
# level: VERSION, app/, engine/build/, live/build/, migrations/, deploy.sh),
# its layout and each Worker's exports checked (RaffleRoom the live Worker's
# alone); then a copy of deploy/cloudflare with the tarball unpacked into its
# release/, as an installation's workflow unpacks it, runs
# `release/deploy.sh dry-run`: deploy.sh's checks of the release, VERSION and
# the three configurations as wrangler reads them (the template's
# placeholders only warn in a dry run), legal/ copied in, and `wrangler
# deploy --dry-run` of the live Worker, the engine Worker under its versioned
# name and the app Worker, whose names and bindings are checked here. Then
# one Wrangler environment the same way: a copy whose three configurations
# have an [env.eu] (repeating what environments don't inherit: the vars, the
# engine's identity keys among them, the bindings, the required secrets) and
# a VERSION.eu, dry-run with CLOUDFLARE_ENV=eu, the Workers the
# environment's (rafi-app-eu, rafi-live-eu, rafi-engine-eu-v<version>).
# .github/workflows/release.yaml runs this on a tag and attaches
# <out dir>/worker.tar.gz; scripts/workers-check.sh runs it too, so CI
# rehearses a release on every push; running it by hand does too.
#
#   ./scripts/cloudflare-release.sh <version> [<out dir>]    # default out dir: target/cloudflare
#   ./scripts/cloudflare-release.sh 0.2.0
#
# Needs what the release needs (just, dx, worker-build, GNU tar), plus node
# and wrangler: WRANGLER names another command, RAFI_WRANGLER_PKG its package
# (by default the package the wrangler on PATH runs, the devenv's); deploy.sh
# is given both, so nothing is installed.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${1:?usage: cloudflare-release.sh <version> [<out dir>]}"
OUT="${2:-$ROOT/target/cloudflare}"
case "$OUT" in /*) ;; *) OUT="$PWD/$OUT" ;; esac
WRANGLER="${WRANGLER:-wrangler}"
if [ -z "${RAFI_WRANGLER_PKG:-}" ]; then
  RAFI_WRANGLER_PKG="$(dirname "$(readlink -f "$(command -v wrangler)")")/../lib"
fi
export RAFI_WRANGLER_PKG
DASHED="v$(printf '%s' "$VERSION" | tr '.' '-' | tr '[:upper:]' '[:lower:]')"

step() { printf '\n== %s\n' "$*"; }
fail() {
  echo "$*" >&2
  exit 1
}

step "deploy/cloudflare deploys this release, with its workflows"
# An installation's repository is a copy of deploy/cloudflare taken at a tag:
# its VERSION names the release it deploys, and its workflows run that
# release's deploy.sh. Both workflows download worker.tar.gz of the version
# file deploy.sh checks from the repository they name and verify that
# release.yaml attested it.
template="$ROOT/deploy/cloudflare"
named="$(tr -d '[:space:]' <"$template/VERSION" 2>/dev/null || true)"
[ "$named" = "$VERSION" ] || fail "deploy/cloudflare/VERSION says '$named', not $VERSION: set it to $VERSION"
upstream=
for workflow in deploy.yaml restate-cleanup.yaml; do
  file="$template/.github/workflows/$workflow"
  [ -f "$file" ] || fail "no deploy/cloudflare/.github/workflows/$workflow"
  repository="$(sed -n 's/^  RAFI_REPOSITORY: *\([^ #]*\).*/\1/p' "$file")"
  [[ "$repository" =~ ^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$ ]] || fail "$workflow names no one RAFI_REPOSITORY: '$repository'"
  [ -z "$upstream" ] || [ "$repository" = "$upstream" ] || fail "the workflows name two Rafi repositories: $upstream and $repository"
  upstream="$repository"
  # shellcheck disable=SC2016 # the workflow's own shell variables, matched literally
  grep -qF -- '--signer-workflow "github.com/$RAFI_REPOSITORY/.github/workflows/release.yaml"' "$file" \
    || fail "$workflow doesn't verify the attestation's signer as release.yaml"
  grep -qx '  CLOUDFLARE_ENV: ""' "$file" || fail "$workflow has no top-level CLOUDFLARE_ENV, empty by default"
  # shellcheck disable=SC2016 # the workflow's own shell variables, matched literally
  grep -qF 'if [ -n "$CLOUDFLARE_ENV" ] && [ -f "VERSION.$CLOUDFLARE_ENV" ]; then' "$file" \
    || fail "$workflow doesn't read VERSION.<CLOUDFLARE_ENV> when it exists, as deploy.sh does"
  ! grep -qE 'RAFI_(DEPLOY_VERSION|INSTANCE)|deploy-rooms' "$file" \
    || fail "$workflow still has RAFI_DEPLOY_VERSION, RAFI_INSTANCE or deploy-rooms, which deploy.sh no longer knows"
  echo "ok   $workflow: releases of $repository attested by release.yaml, the version file deploy.sh's"
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
[ "$top" = "VERSION app deploy.sh engine live migrations " ] || fail "worker.tar.gz has at its top level: $top"
for file in VERSION deploy.sh app/build/index.js app/assets/_headers engine/build/index.js \
  live/build/index.js migrations/0001_init.sql; do
  grep -qx "$file" <<<"$listing" || fail "worker.tar.gz has no $file"
done
# engine/ and live/ are their build/ alone; migrations/ the D1 migrations' files alone.
! grep -vE '^(engine|live)/(build/.*)?$' <<<"$(grep -E '^(engine|live)/' <<<"$listing")" \
  || fail "worker.tar.gz has more than build/ in engine/ or live/"
! grep -vE '^migrations/([0-9]{4}_[a-z0-9_]+\.sql)?$' <<<"$(grep '^migrations/' <<<"$listing")" \
  || fail "worker.tar.gz has more than the D1 migrations' .sql files in migrations/"
! grep -q '^app/assets/legal' <<<"$listing" || fail "worker.tar.gz has app/assets/legal/: an installation's"
echo "ok   worker.tar.gz: VERSION, app/, engine/build/, live/build/, migrations/ and deploy.sh"

# installation <dir>: a copy of deploy/cloudflare with the tarball unpacked into its release/.
installation() {
  rm -rf "$1"
  mkdir -p "$1/release"
  (cd "$ROOT/deploy/cloudflare" && tar -c --exclude ./release --exclude ./node_modules --exclude ./.wrangler \
    --exclude ./.prod.vars --exclude ./.rafi-tools .) | tar -x -C "$1"
  tar -xzf "$OUT/worker.tar.gz" -C "$1/release"
}
# dry_run <dir> [<VAR=value>...]: release/deploy.sh dry-run there, its output in DRY.
dry_run() {
  local dir="$1"
  shift
  DRY="$(cd "$dir" && env WRANGLER="$WRANGLER" RAFI_TOOLS_DIR="$dir/.rafi-tools" RAFI_VERSION="$VERSION" \
    "$@" release/deploy.sh dry-run 2>&1)" \
    || { echo "$DRY" | tail -40 >&2; fail "release/deploy.sh dry-run failed in $dir"; }
  [ ! -e "$dir/.rafi-tools" ] || fail "deploy.sh installed tools although WRANGLER and RAFI_WRANGLER_PKG were given"
}
# Each Worker's part of a dry run's output, by deploy.sh's step lines: from its own to the next.
section() {
  echo "$DRY" | awk -v head="==> $1: wrangler deploy --dry-run" '
    index($0, "==> ") == 1 { inside = ($0 == head) }
    inside'
}
# expect <what> <output> <binding line patterns...>
expect() {
  local what="$1" output="$2" pattern
  shift 2
  for pattern in "$@"; do
    grep -qE "$pattern" <<<"$output" || { echo "$output" | tail -30 >&2; fail "$what: no binding matching '$pattern'"; }
  done
}
# check_dry_run <app> <live> <engine>: the three Workers dry-deployed under those names, each
# with its bindings, the app and the engine bound to that live Worker's class.
check_dry_run() {
  local app_name="$1" live_name="$2" engine_name="$3" app live engine
  live="$(section "live Worker $live_name")"
  app="$(section "app Worker $app_name")"
  engine="$(section "engine Worker $engine_name")"
  [ -n "$live" ] || { echo "$DRY" | tail -40 >&2; fail "no dry deploy of the live Worker $live_name"; }
  [ -n "$app" ] || { echo "$DRY" | tail -40 >&2; fail "no dry deploy of the app Worker $app_name"; }
  [ -n "$engine" ] || { echo "$DRY" | tail -40 >&2; fail "no dry deploy of the engine Worker $engine_name (a versioned name)"; }
  expect "the live Worker" "$live" '^env\.DB \(rafi\) ' '^env\.RAFFLE_ROOM \(RaffleRoom\) ' '^env\.RAFI_DO_JURISDICTION \("eu"\) '
  ! grep -qE '^env\.(ASSETS|[A-Z_]+_LIMITER|RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS) ' <<<"$live" \
    || fail "the live Worker binds the app's or the engine's"
  ! grep -qE '^env\.RAFFLE_ROOM \(RaffleRoom, defined in ' <<<"$live" || fail "the live Worker binds another Worker's RaffleRoom"
  expect "the app Worker" "$app" '^env\.ASSETS ' '^env\.DB \(rafi\) ' "^env\\.RAFFLE_ROOM \\(RaffleRoom, defined in $live_name\\) " \
    '^env\.JOIN_PER_RAFFLE_LIMITER ' '^env\.JOIN_PER_EMAIL_LIMITER ' '^env\.CONNECT_LIMITER ' '^env\.ADD_LIMITER ' \
    '^env\.RAFI_DO_JURISDICTION \("eu"\) '
  expect "the engine Worker" "$engine" '^env\.DB \(rafi\) ' "^env\\.RAFFLE_ROOM \\(RaffleRoom, defined in $live_name\\) " \
    '^env\.RAFI_DO_JURISDICTION \("eu"\) ' '^env\.RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS '
  ! grep -qE '^env\.(ASSETS|[A-Z_]+_LIMITER) ' <<<"$engine" || fail "the engine Worker binds the app's assets or limits"
  echo "ok   the live Worker $live_name: DB, RAFFLE_ROOM (its own), RAFI_DO_JURISDICTION"
  echo "ok   the app Worker $app_name: ASSETS, DB, RAFFLE_ROOM ($live_name's), the four rate limiters, RAFI_DO_JURISDICTION"
  echo "ok   the engine Worker $engine_name: DB, RAFFLE_ROOM ($live_name's), RAFI_DO_JURISDICTION, RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS"
}

step "deploy/cloudflare deploys it (dry run)"
dir="$OUT/installation"
installation "$dir"
shipped="$(cat "$dir/release/VERSION")"
[ "$shipped" = "$VERSION" ] || fail "the release unpacked says it is $shipped, not $VERSION"
# Each Worker's module exports (worker-build's index.js ends in one export
# list): the RaffleRoom class is the live Worker's alone, since a Worker
# exporting it would host a second class of that name.
exports() {
  grep -o 'export *{[^}]*}' "$dir/release/$1/build/index.js" | tail -1 \
    | sed 's/^export *{//; s/}$//' | tr ',' '\n' | sed 's/.* as //; s/^ *//; s/ *$//' | LC_ALL=C sort | paste -sd ' ' -
}
for worker in app engine live; do
  list="$(exports "$worker")"
  grep -qw default <<<"$list" || fail "the $worker Worker exports no default handler: $list"
  if [ "$worker" = live ]; then
    grep -qw RaffleRoom <<<"$list" || fail "the live Worker doesn't export RaffleRoom: $list"
  else
    ! grep -qw RaffleRoom <<<"$list" || fail "the $worker Worker exports RaffleRoom, the live Worker's class: $list"
  fi
  echo "ok   the $worker Worker exports: $list"
done
[ -x "$dir/release/deploy.sh" ] || fail "release/deploy.sh isn't executable"
dry_run "$dir"
for doc in terms.md privacy.md; do
  cmp -s "$ROOT/deploy/cloudflare/legal/$doc" "$dir/release/app/assets/legal/$doc" \
    || fail "deploy.sh didn't put legal/$doc into release/app/assets/legal/"
done
echo "ok   deploy.sh copied legal/ into release/app/assets/legal/"
grep -qF "Rafi $VERSION (VERSION)" <<<"$DRY" || fail "deploy.sh didn't read VERSION"
check_dry_run rafi-app rafi-live "rafi-engine-$DASHED"
rm -rf "$dir"

step "deploy/cloudflare deploys it as a Wrangler environment (CLOUDFLARE_ENV=eu, dry run)"
# One repository, several installations: an [env.eu] in each configuration,
# repeating what environments don't inherit (vars, bindings, required
# secrets; the rate limits with namespaces of their own), RAFFLE_ROOM bound
# to the environment's live Worker, and the environment's own version file.
dir="$OUT/installation-eu"
installation "$dir"
secrets="$(sed -n '/^\[secrets\]$/,/^\]$/p' "$dir/app.wrangler.toml" | sed '1d')"
[ -n "$secrets" ] || fail "app.wrangler.toml has no [secrets] required list"
keys="$(sed -n 's/^RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS = \(".*"\)$/\1/p' "$dir/engine.wrangler.toml")"
[ -n "$keys" ] || fail "engine.wrangler.toml has no RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS"
database="$(sed -n 's/^database_id = \(".*"\)$/\1/p' "$dir/app.wrangler.toml")"
{
  printf '\n[env.eu]\n'
  printf '[[env.eu.d1_databases]]\nbinding = "DB"\ndatabase_name = "rafi"\ndatabase_id = %s\nmigrations_dir = "release/migrations"\n' "$database"
  printf '[[env.eu.durable_objects.bindings]]\nname = "RAFFLE_ROOM"\nclass_name = "RaffleRoom"\nscript_name = "rafi-live-eu"\n'
  n=2000
  for limit in JOIN_PER_RAFFLE_LIMITER JOIN_PER_EMAIL_LIMITER CONNECT_LIMITER ADD_LIMITER; do
    n=$((n + 1))
    printf '[[env.eu.ratelimits]]\nname = "%s"\nnamespace_id = "%s"\nsimple = { limit = 10, period = 60 }\n' "$limit" "$n"
  done
  printf '[env.eu.vars]\nRAFI_DO_JURISDICTION = "eu"\n'
  printf '[env.eu.secrets]\n%s\n' "$secrets"
} >>"$dir/app.wrangler.toml"
{
  printf '\n[env.eu]\n'
  printf '[[env.eu.d1_databases]]\nbinding = "DB"\ndatabase_name = "rafi"\ndatabase_id = %s\n' "$database"
  printf '[[env.eu.durable_objects.bindings]]\nname = "RAFFLE_ROOM"\nclass_name = "RaffleRoom"\nscript_name = "rafi-live-eu"\n'
  printf '[env.eu.vars]\nRAFI_DO_JURISDICTION = "eu"\nRAFI_RESTATE_ENDPOINT__IDENTITY_KEYS = %s\n' "$keys"
} >>"$dir/engine.wrangler.toml"
{
  printf '\n[env.eu]\n'
  printf '[[env.eu.d1_databases]]\nbinding = "DB"\ndatabase_name = "rafi"\ndatabase_id = %s\n' "$database"
  printf '[[env.eu.durable_objects.bindings]]\nname = "RAFFLE_ROOM"\nclass_name = "RaffleRoom"\n'
  printf '[env.eu.vars]\nRAFI_DO_JURISDICTION = "eu"\n'
} >>"$dir/live.wrangler.toml"
# The environment's version file is this release; the top level's another.
echo "$VERSION" >"$dir/VERSION.eu"
echo "0.0.0" >"$dir/VERSION"
dry_run "$dir" CLOUDFLARE_ENV=eu
grep -qF "Rafi $VERSION (VERSION.eu)" <<<"$DRY" || fail "deploy.sh didn't read VERSION.eu for CLOUDFLARE_ENV=eu"
grep -qF "environment: eu ([env.eu] of each configuration)" <<<"$DRY" || fail "deploy.sh didn't read the environment eu"
echo "ok   deploy.sh read VERSION.eu and [env.eu]"
check_dry_run rafi-app-eu rafi-live-eu "rafi-engine-eu-$DASHED"
rm -rf "$dir"

step "release $VERSION: $OUT/worker.tar.gz"
