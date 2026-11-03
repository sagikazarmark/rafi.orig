#!/usr/bin/env bash
# Builds a release of Rafi for Cloudflare and proves it deploys, the way an
# installation deploys it. First that deploy/cloudflare, which an
# installation copies at the release's tag, names this release (its VERSION),
# has the package.json its workflows install wrangler and the Restate CLI
# from, and that its two workflows (.github/workflows/deploy.yaml and
# restate-cleanup.yaml) name one Rafi repository (RAFI_REPOSITORY), verify the
# attestation release.yaml makes, read the version file deploy.sh checks
# (VERSION.<CLOUDFLARE_ENV> when it exists, VERSION otherwise), install the
# tools (npm ci or npm install) and keep deploy.sh's JSON (RAFI_ARTIFACTS_DIR,
# uploaded whatever happened), and that release.yaml makes it. Then deploy.sh's
# own checks (scripts/cloudflare-deploy-test.sh: each subcommand against a
# stubbed wrangler and Restate CLI and a fake Cloudflare and Restate API).
# Then the release (`just cloudflare-tarball`: worker.tar.gz, its files at the
# top level: VERSION, app/, engine/build/, live/build/, migrations/,
# deploy.sh), its layout and each Worker's exports checked (RaffleRoom the
# live Worker's alone); then a copy of deploy/cloudflare with the tarball
# unpacked into its release/, as an installation's workflow unpacks it, runs
# `release/deploy.sh dry-run` with the devenv's wrangler (WRANGLER; nothing
# installed, nothing asked of the network): deploy.sh's checks of the
# release, VERSION and the template's placeholders (only warned about in a
# dry run), legal/ copied in, and `wrangler deploy --dry-run` of the live
# Worker, the engine Worker and the app Worker, whose names (read from the dry
# runs' ND-JSON output, kept in RAFI_ARTIFACTS_DIR) and bindings are checked
# here: no RAFI_DO_JURISDICTION, the template's default. Then one Wrangler
# environment the same way: a copy whose three configurations have an
# [env.eu] (repeating what environments don't inherit: the vars, with
# RAFI_DO_JURISDICTION = "eu" and the engine's identity keys, the bindings,
# the required secrets) and a VERSION.eu, dry-run with CLOUDFLARE_ENV=eu, the
# Workers the environment's (rafi-app-eu, rafi-live-eu,
# rafi-engine-eu-v<version>), each with the jurisdiction.
# .github/workflows/release.yaml runs this on a tag and attaches
# <out dir>/worker.tar.gz; scripts/workers-check.sh runs it too, so CI
# rehearses a release on every push; running it by hand does too.
#
#   ./scripts/cloudflare-release.sh <version> [<out dir>]    # default out dir: target/cloudflare
#   ./scripts/cloudflare-release.sh 0.2.0
#
# Needs what the release needs (just, dx, worker-build, GNU tar), plus node,
# jq and wrangler (WRANGLER names another command; the devenv's by default).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${1:?usage: cloudflare-release.sh <version> [<out dir>]}"
OUT="${2:-$ROOT/target/cloudflare}"
case "$OUT" in /*) ;; *) OUT="$PWD/$OUT" ;; esac
WRANGLER="${WRANGLER:-wrangler}"
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
# release.yaml attested it; every job installs the tools from package.json
# (deploy.sh runs them with npx -y) and keeps deploy.sh's JSON as an artifact.
template="$ROOT/deploy/cloudflare"
named="$(tr -d '[:space:]' <"$template/VERSION" 2>/dev/null || true)"
[ "$named" = "$VERSION" ] || fail "deploy/cloudflare/VERSION says '$named', not $VERSION: set it to $VERSION"
jq -e '.private == true and (.devDependencies | keys) == ["@restatedev/restate", "wrangler"]' "$template/package.json" >/dev/null \
  || fail "deploy/cloudflare/package.json isn't private with wrangler and @restatedev/restate as its devDependencies"
grep -qx 'node_modules/' "$template/.gitignore" || fail "deploy/cloudflare/.gitignore doesn't ignore node_modules/"
echo "ok   deploy/cloudflare/package.json: wrangler and the Restate CLI"
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
  # Every job runs deploy.sh: each installs the tools and keeps its JSON, whatever happened.
  jobs="$(grep -c '^    runs-on: ' "$file")"
  # shellcheck disable=SC2016 # the workflow's own expressions, matched literally
  for line in 'if [ -f package-lock.json ]; then npm ci --no-audit --no-fund; else npm install --no-audit --no-fund; fi' \
    'RAFI_ARTIFACTS_DIR: ${{ github.workspace }}/rafi-artifacts' 'path: ${{ env.RAFI_ARTIFACTS_DIR }}'; do
    [ "$(grep -cF -- "$line" "$file")" = "$jobs" ] || fail "$workflow: not each of its $jobs jobs has: $line"
  done
  [ "$(grep -c '^        if: always()$' "$file")" = "$jobs" ] || fail "$workflow: not each job keeps its artifacts if: always()"
  echo "ok   $workflow: releases of $repository attested by release.yaml, the version file deploy.sh's; in each of its $jobs jobs the tools installed and deploy.sh's JSON kept"
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
    --exclude ./.prod.vars .) | tar -x -C "$1"
  tar -xzf "$OUT/worker.tar.gz" -C "$1/release"
}
# dry_run <dir> [<VAR=value>...]: release/deploy.sh dry-run there with WRANGLER (nothing
# installed), its output in DRY, its JSON in <dir>/artifacts/dry-run.
dry_run() {
  local dir="$1"
  shift
  DRY="$(cd "$dir" && env WRANGLER="$WRANGLER" RAFI_VERSION="$VERSION" RAFI_ARTIFACTS_DIR="$dir/artifacts" \
    "$@" release/deploy.sh dry-run 2>&1)" \
    || { echo "$DRY" | tail -40 >&2; fail "release/deploy.sh dry-run failed in $dir"; }
  [ ! -e "$dir/node_modules" ] || fail "something was installed in $dir although WRANGLER was given"
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
# check_dry_run <dir> <app> <live> <engine base> <jurisdiction, or empty>: the three Workers
# dry-deployed, their names those (from the dry runs' ND-JSON output, the engine's per release
# <engine base>-v<version>), each with its bindings, the app and the engine bound to that live
# Worker's class, RAFI_DO_JURISDICTION that jurisdiction in all three (none when empty).
check_dry_run() {
  local dir="$1" app_name="$2" live_name="$3" engine_base="$4" jurisdiction="$5" app live engine key want got
  local engine_name="$engine_base-$DASHED"
  for key in live engine app; do
    case "$key" in live) want="$live_name" ;; engine) want="$engine_base" ;; *) want="$app_name" ;; esac
    got="$(jq -r 'select(.type == "deploy") | .worker_name' "$dir/artifacts/dry-run/wrangler-dry-run-$key.ndjson" | tail -n 1)"
    [ "$got" = "$want" ] || fail "the $key Worker's dry run says it is '$got', not $want"
  done
  grep -qF "live Worker: $live_name (" <<<"$DRY" || fail "deploy.sh didn't name the live Worker $live_name"
  grep -qF "app Worker: $app_name (" <<<"$DRY" || fail "deploy.sh didn't name the app Worker $app_name"
  grep -qF "engine Worker: $engine_name (" <<<"$DRY" || fail "deploy.sh didn't name the engine Worker $engine_name"
  echo "ok   the dry runs name them $live_name, $engine_base (this release's: $engine_name) and $app_name"
  live="$(section "live Worker (live.wrangler.toml)")"
  app="$(section "app Worker (app.wrangler.toml)")"
  engine="$(section "engine Worker (engine.wrangler.toml)")"
  [ -n "$live" ] || { echo "$DRY" | tail -40 >&2; fail "no dry deploy of the live Worker"; }
  [ -n "$app" ] || { echo "$DRY" | tail -40 >&2; fail "no dry deploy of the app Worker"; }
  [ -n "$engine" ] || { echo "$DRY" | tail -40 >&2; fail "no dry deploy of the engine Worker"; }
  expect "the live Worker" "$live" '^env\.DB \(rafi\) ' '^env\.RAFFLE_ROOM \(RaffleRoom\) '
  ! grep -qE '^env\.(ASSETS|[A-Z_]+_LIMITER|RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS) ' <<<"$live" \
    || fail "the live Worker binds the app's or the engine's"
  ! grep -qE '^env\.RAFFLE_ROOM \(RaffleRoom, defined in ' <<<"$live" || fail "the live Worker binds another Worker's RaffleRoom"
  expect "the app Worker" "$app" '^env\.ASSETS ' '^env\.DB \(rafi\) ' "^env\\.RAFFLE_ROOM \\(RaffleRoom, defined in $live_name\\) " \
    '^env\.JOIN_PER_RAFFLE_LIMITER ' '^env\.JOIN_PER_EMAIL_LIMITER ' '^env\.CONNECT_LIMITER ' '^env\.ADD_LIMITER '
  expect "the engine Worker" "$engine" '^env\.DB \(rafi\) ' "^env\\.RAFFLE_ROOM \\(RaffleRoom, defined in $live_name\\) " \
    '^env\.RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS '
  ! grep -qE '^env\.(ASSETS|[A-Z_]+_LIMITER) ' <<<"$engine" || fail "the engine Worker binds the app's assets or limits"
  for key in live app engine; do
    case "$key" in live) got="$live" ;; app) got="$app" ;; *) got="$engine" ;; esac
    if [ -n "$jurisdiction" ]; then
      expect "the $key Worker" "$got" "^env\\.RAFI_DO_JURISDICTION \\(\"$jurisdiction\"\\) "
    elif grep -qE '^env\.RAFI_DO_JURISDICTION ' <<<"$got"; then
      fail "the $key Worker has a RAFI_DO_JURISDICTION, which the template leaves unset"
    fi
  done
  echo "ok   the live Worker $live_name: DB, RAFFLE_ROOM (its own)"
  echo "ok   the app Worker $app_name: ASSETS, DB, RAFFLE_ROOM ($live_name's), the four rate limiters"
  echo "ok   the engine Worker $engine_base: DB, RAFFLE_ROOM ($live_name's), RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS"
  echo "ok   RAFI_DO_JURISDICTION: ${jurisdiction:-none} in all three"
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
check_dry_run "$dir" rafi-app rafi-live rafi-engine ""
rm -rf "$dir"

step "deploy/cloudflare deploys it as a Wrangler environment (CLOUDFLARE_ENV=eu, dry run)"
# One repository, several installations: an [env.eu] in each configuration,
# repeating what environments don't inherit (vars, bindings, required
# secrets; the rate limits with namespaces of their own), RAFFLE_ROOM bound
# to the environment's live Worker, RAFI_DO_JURISDICTION = "eu" in all three
# (README.md's "Keeping data in the EU"), and the environment's own version
# file.
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
check_dry_run "$dir" rafi-app-eu rafi-live-eu rafi-engine-eu eu
rm -rf "$dir"

step "release $VERSION: $OUT/worker.tar.gz"
