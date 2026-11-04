#!/usr/bin/env bash
# Builds a release of Rafi for Cloudflare and proves it deploys, the way an
# installation deploys it. First that the test fixture,
# testdata/cloudflare-installation (an installation's configuration as the
# deploy template ships it: VERSION and wrangler.<app|engine|live>.toml),
# names this release, with the app on its workers.dev address by default
# (workers_dev = true, a custom domain's routes commented out), and that
# release.yaml attests the tarball, which installations' workflows verify.
# Then deploy.sh's own checks (scripts/cloudflare-deploy-test.sh: each
# subcommand against a stubbed wrangler and Restate CLI and a fake Cloudflare
# and Restate API). Then the release (`just cloudflare-tarball`:
# worker.tar.gz, its files at the top level: VERSION, app/, engine/build/,
# live/build/, migrations/, deploy.sh), its layout and each Worker's exports
# checked (RaffleRoom the live Worker's alone); then a copy of the fixture
# with the tarball unpacked into its release/, as an installation's workflow
# unpacks it, runs `release/deploy.sh dry-run` with the devenv's wrangler
# (WRANGLER; nothing installed, nothing asked of the network): the release's
# own VERSION read (deploy.sh reads no version file and looks for no
# placeholder), no legal documents copied (the fixture has none), and
# `wrangler deploy --dry-run` of the live Worker, the engine Worker and the
# app Worker, whose names (read from the dry runs' ND-JSON output, kept in
# RAFI_ARTIFACTS_DIR) and bindings are checked here: no RAFI_DO_JURISDICTION,
# the default. Then one Wrangler environment the same way: a copy whose three
# configurations have an [env.eu] (repeating what environments don't
# inherit: the vars, with RAFI_DO_JURISDICTION = "eu" and the engine's
# identity keys, the bindings, the required secrets; the app on a custom
# domain, workers_dev = false) and a VERSION.eu, dry-run with
# CLOUDFLARE_ENV=eu, the Workers the environment's (rafi-app-eu,
# rafi-live-eu, rafi-engine-eu-v<version>), each with the jurisdiction.
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

step "the installation names this release"
# The test fixture, an installation's configuration as the deploy template
# ships it: its VERSION names the release it deploys, its three
# configurations are what deploy.sh reads (-c wrangler.<Worker>.toml), and the
# app is on its workers.dev address by default (no zone, no routes; a custom
# domain is an installation's edit, its route commented out here).
fixture="$ROOT/testdata/cloudflare-installation"
named="$(tr -d '[:space:]' <"$fixture/VERSION" 2>/dev/null || true)"
[ "$named" = "$VERSION" ] || fail "testdata/cloudflare-installation/VERSION says '$named', not $VERSION: set it to $VERSION"
for key in app engine live; do
  [ -f "$fixture/wrangler.$key.toml" ] || fail "no testdata/cloudflare-installation/wrangler.$key.toml"
done
app_toml="$fixture/wrangler.app.toml"
if ! grep -qx 'workers_dev = true' "$app_toml" || ! grep -qx 'preview_urls = false' "$app_toml" \
  || grep -q '^routes' "$app_toml" \
  || ! grep -qx '# routes = \[{ pattern = "raffle\.example\.org", custom_domain = true }\]' "$app_toml"; then
  fail "testdata/cloudflare-installation/wrangler.app.toml isn't on workers.dev by default: workers_dev = true, preview_urls = false, the custom domain's routes commented out"
fi
echo "ok   testdata/cloudflare-installation: VERSION $VERSION, wrangler.{app,engine,live}.toml, the app on workers.dev"
# What installations' workflows verify of a release: release.yaml's attestation.
grep -q 'uses: actions/attest-build-provenance@' "$ROOT/.github/workflows/release.yaml" \
  || fail "release.yaml doesn't attest the tarball: the workflows would refuse every release"
echo "ok   release.yaml attests the tarball"

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

# installation <dir>: a copy of the fixture with the tarball unpacked into its release/.
installation() {
  rm -rf "$1"
  mkdir -p "$1/release"
  cp -R "$fixture/." "$1/"
  tar -xzf "$OUT/worker.tar.gz" -C "$1/release"
}
# dry_run <dir> [<VAR=value>...]: release/deploy.sh dry-run there with WRANGLER (nothing
# installed), its output in DRY, its JSON in <dir>/artifacts/dry-run.
dry_run() {
  local dir="$1"
  shift
  DRY="$(cd "$dir" && env WRANGLER="$WRANGLER" RAFI_ARTIFACTS_DIR="$dir/artifacts" \
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
  live="$(section "live Worker (wrangler.live.toml)")"
  app="$(section "app Worker (wrangler.app.toml)")"
  engine="$(section "engine Worker (wrangler.engine.toml)")"
  [ -n "$live" ] || { echo "$DRY" | tail -40 >&2; fail "no dry deploy of the live Worker"; }
  [ -n "$app" ] || { echo "$DRY" | tail -40 >&2; fail "no dry deploy of the app Worker"; }
  [ -n "$engine" ] || { echo "$DRY" | tail -40 >&2; fail "no dry deploy of the engine Worker"; }
  expect "the live Worker" "$live" '^env\.DB \(rafi\) ' '^env\.RAFFLE_ROOM \(RaffleRoom\) '
  ! grep -qE '^env\.(ASSETS|[A-Z_]+_LIMITER|RESTATE_IDENTITY_KEYS) ' <<<"$live" \
    || fail "the live Worker binds the app's or the engine's"
  ! grep -qE '^env\.RAFFLE_ROOM \(RaffleRoom, defined in ' <<<"$live" || fail "the live Worker binds another Worker's RaffleRoom"
  expect "the app Worker" "$app" '^env\.ASSETS ' '^env\.DB \(rafi\) ' "^env\\.RAFFLE_ROOM \\(RaffleRoom, defined in $live_name\\) " \
    '^env\.JOIN_PER_RAFFLE_LIMITER ' '^env\.JOIN_PER_EMAIL_LIMITER ' '^env\.CONNECT_LIMITER ' '^env\.ADD_LIMITER '
  expect "the engine Worker" "$engine" '^env\.DB \(rafi\) ' "^env\\.RAFFLE_ROOM \\(RaffleRoom, defined in $live_name\\) " \
    '^env\.RESTATE_IDENTITY_KEYS '
  ! grep -qE '^env\.(ASSETS|[A-Z_]+_LIMITER) ' <<<"$engine" || fail "the engine Worker binds the app's assets or limits"
  for key in live app engine; do
    case "$key" in live) got="$live" ;; app) got="$app" ;; *) got="$engine" ;; esac
    if [ -n "$jurisdiction" ]; then
      expect "the $key Worker" "$got" "^env\\.RAFI_DO_JURISDICTION \\(\"$jurisdiction\"\\) "
    elif grep -qE '^env\.RAFI_DO_JURISDICTION ' <<<"$got"; then
      fail "the $key Worker has a RAFI_DO_JURISDICTION, which the fixture leaves unset"
    fi
  done
  echo "ok   the live Worker $live_name: DB, RAFFLE_ROOM (its own)"
  echo "ok   the app Worker $app_name: ASSETS, DB, RAFFLE_ROOM ($live_name's), the four rate limiters"
  echo "ok   the engine Worker $engine_base: DB, RAFFLE_ROOM ($live_name's), RESTATE_IDENTITY_KEYS"
  echo "ok   RAFI_DO_JURISDICTION: ${jurisdiction:-none} in all three"
}

step "the installation deploys it (dry run)"
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
# The fixture has no legal documents (each is the operator's, and optional): none copied.
[ ! -e "$dir/release/app/assets/legal" ] || fail "deploy.sh put legal documents the installation hasn't into release/app/assets/legal/"
echo "ok   deploy.sh copied no legal documents, the installation having none"
grep -qF "Rafi $VERSION (the release's VERSION)" <<<"$DRY" || fail "deploy.sh didn't read the release's VERSION"
check_dry_run "$dir" rafi-app rafi-live rafi-engine ""
rm -rf "$dir"

step "the installation deploys it as a Wrangler environment (CLOUDFLARE_ENV=eu, dry run)"
# One repository, several installations: an [env.eu] in each configuration,
# repeating what environments don't inherit (vars, bindings, required
# secrets; the rate limits with namespaces of their own), RAFFLE_ROOM bound
# to the environment's live Worker, RAFI_DO_JURISDICTION = "eu" in all three
# (INSTALL.md's "Keeping data in the EU"), and the environment's own version
# file, which the workflows download the release by and deploy.sh doesn't
# read (the top level's VERSION names another release here: deploy.sh
# deploys the release it is part of). Its app is on a custom domain
# (INSTALL.md's step 4, option B: workers_dev = false and its own route,
# written out as INSTALL.md's [env.eu] does, which wrangler's dry run
# validates), so the two rehearsals dry-run both addresses: the top level's
# workers.dev, the fixture's default, and this one's domain.
dir="$OUT/installation-eu"
installation "$dir"
secrets="$(sed -n '/^\[secrets\]$/,/^\]$/p' "$dir/wrangler.app.toml" | sed '1d')"
[ -n "$secrets" ] || fail "wrangler.app.toml has no [secrets] required list"
keys="$(sed -n 's/^RESTATE_IDENTITY_KEYS = \(".*"\)$/\1/p' "$dir/wrangler.engine.toml")"
[ -n "$keys" ] || fail "wrangler.engine.toml has no RESTATE_IDENTITY_KEYS"
database="$(sed -n 's/^database_id = \(".*"\)$/\1/p' "$dir/wrangler.app.toml")"
{
  printf '\n[env.eu]\n'
  printf 'workers_dev = false\nroutes = [{ pattern = "raffle.example.eu", custom_domain = true }]\n'
  printf '[[env.eu.d1_databases]]\nbinding = "DB"\ndatabase_name = "rafi"\ndatabase_id = %s\nmigrations_dir = "release/migrations"\n' "$database"
  printf '[[env.eu.durable_objects.bindings]]\nname = "RAFFLE_ROOM"\nclass_name = "RaffleRoom"\nscript_name = "rafi-live-eu"\n'
  n=2000
  for limit in JOIN_PER_RAFFLE_LIMITER JOIN_PER_EMAIL_LIMITER CONNECT_LIMITER ADD_LIMITER; do
    n=$((n + 1))
    printf '[[env.eu.ratelimits]]\nname = "%s"\nnamespace_id = "%s"\nsimple = { limit = 10, period = 60 }\n' "$limit" "$n"
  done
  printf '[env.eu.vars]\nRAFI_DO_JURISDICTION = "eu"\n'
  printf '[env.eu.secrets]\n%s\n' "$secrets"
} >>"$dir/wrangler.app.toml"
{
  printf '\n[env.eu]\n'
  printf '[[env.eu.d1_databases]]\nbinding = "DB"\ndatabase_name = "rafi"\ndatabase_id = %s\n' "$database"
  printf '[[env.eu.durable_objects.bindings]]\nname = "RAFFLE_ROOM"\nclass_name = "RaffleRoom"\nscript_name = "rafi-live-eu"\n'
  printf '[env.eu.vars]\nRAFI_DO_JURISDICTION = "eu"\nRESTATE_IDENTITY_KEYS = %s\n' "$keys"
} >>"$dir/wrangler.engine.toml"
{
  printf '\n[env.eu]\n'
  printf '[[env.eu.d1_databases]]\nbinding = "DB"\ndatabase_name = "rafi"\ndatabase_id = %s\n' "$database"
  printf '[[env.eu.durable_objects.bindings]]\nname = "RAFFLE_ROOM"\nclass_name = "RaffleRoom"\n'
  printf '[env.eu.vars]\nRAFI_DO_JURISDICTION = "eu"\n'
} >>"$dir/wrangler.live.toml"
# The environment's version file is this release; the top level's another.
echo "$VERSION" >"$dir/VERSION.eu"
echo "0.0.0" >"$dir/VERSION"
dry_run "$dir" CLOUDFLARE_ENV=eu
grep -qF "Rafi $VERSION (the release's VERSION)" <<<"$DRY" || fail "deploy.sh didn't deploy the release's own VERSION for CLOUDFLARE_ENV=eu"
grep -qF "environment: eu ([env.eu] of each configuration)" <<<"$DRY" || fail "deploy.sh didn't read the environment eu"
echo "ok   deploy.sh deployed the release's VERSION in [env.eu]"
check_dry_run "$dir" rafi-app-eu rafi-live-eu rafi-engine-eu eu
rm -rf "$dir"

step "release $VERSION: $OUT/worker.tar.gz"
