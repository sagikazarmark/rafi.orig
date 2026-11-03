#!/usr/bin/env bash
# Checks the parts of the Cloudflare deploy script (scripts/cloudflare-deploy.sh, a release's
# deploy.sh) that need no Cloudflare or Restate account: the release's version and the engine
# Worker's name, reading and cross-checking the two configurations (deploy/cloudflare's, and
# broken copies of them), --check and the usage, which Restate deployments a prune would touch
# (from `restate deployments list --extra --json` as Restate's CLI 1.8.1 prints it), and the
# "is anything live?" preflight against a local D1 database (wrangler d1 execute --local, the
# remote command's twin): before the migrations, with nothing live, and with a raffle open.
# scripts/cloudflare-release.sh runs it; so can you:
#
#   ./scripts/cloudflare-deploy-test.sh
#
# Needs node, and wrangler for the preflight (WRANGLER names another command; without either,
# that part is skipped, saying so).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/cloudflare-deploy.sh
source "$ROOT/scripts/cloudflare-deploy.sh"

TMP="$(mktemp -d)"
TMP="$(cd "$TMP" && pwd -P)"
trap 'rm -rf "$TMP"' EXIT
# wrangler keeps a cache in its working directory: this run's.
cd "$TMP"
FAILED=0

pass() { printf 'ok   %s\n' "$1"; }
fail() {
  printf 'FAIL %s\n' "$1" >&2
  FAILED=1
}
# expect <name> <expected stdout> <command...>: succeeds, printing exactly that.
expect() {
  local name="$1" want="$2" got
  shift 2
  if got="$("$@" 2>"$TMP/stderr")" && [ "$got" = "$want" ]; then
    pass "$name"
  else
    fail "$name: wanted '$want', got '$got'; $(cat "$TMP/stderr")"
  fi
}
# refused <name> <text in stderr> <command...>: fails, saying that.
refused() {
  local name="$1" text="$2"
  shift 2
  if ("$@") >/dev/null 2>"$TMP/stderr"; then
    fail "$name: succeeded"
  elif grep -qF -- "$text" "$TMP/stderr"; then
    pass "$name"
  else
    fail "$name: no '$text' in: $(cat "$TMP/stderr")"
  fi
}

# A release as `just cloudflare-artifact` lays it out (empty files will do), unpacked into an
# installation (a copy of deploy/cloudflare) as release/.
INSTALL="$TMP/installation"
mkdir -p "$INSTALL"
cp -R "$ROOT/deploy/cloudflare/." "$INSTALL/"
rm -rf "$INSTALL/release" "$INSTALL/.wrangler" "$INSTALL/.dev.vars"
RELEASE="$INSTALL/release"
mkdir -p "$RELEASE/app/build" "$RELEASE/app/assets" "$RELEASE/engine/build" "$RELEASE/migrations"
touch "$RELEASE/app/build/index.js" "$RELEASE/engine/build/index.js"
cp -R "$ROOT/migrations/sqlite" "$RELEASE/migrations/sqlite"
cp "$ROOT/scripts/cloudflare-deploy.sh" "$RELEASE/deploy.sh"
echo "0.1.0" >"$RELEASE/VERSION"
APP="$INSTALL/wrangler.toml"
ENGINE="$INSTALL/engine.wrangler.toml"
DB_ID=6f1c2a90-3b7e-4d2a-9c41-0e5b8a7d2f13
KEY=publickeyv1_ChjENKeMvCtRnqG2mrBK1HmPKufgFUc98K8B3ZgXonQQ

echo "== the version and the engine Worker's name"
expect "a release's version" 0.1.0 release_version "$RELEASE"
echo "v0.2.0-rc.1" >"$TMP/VERSION"
expect "a pre-release, its v dropped" 0.2.0-rc.1 release_version "$TMP"
echo "0.1" >"$TMP/VERSION"
refused "a VERSION that isn't a version" "not a release version" release_version "$TMP"
rm "$TMP/VERSION"
refused "no VERSION" "is missing" release_version "$TMP"
expect "RAFI_VERSION, the same" 0.1.0 env RAFI_VERSION=v0.1.0 bash -c "source '$ROOT/scripts/cloudflare-deploy.sh'; release_version '$RELEASE'"
refused "RAFI_VERSION, another" "RAFI_VERSION says 0.2.0" env RAFI_VERSION=0.2.0 bash -c "source '$ROOT/scripts/cloudflare-deploy.sh'; release_version '$RELEASE'"
expect "the engine Worker's name" rafi-engine-0-1-0 engine_worker_name rafi-engine 0.1.0
expect "a pre-release's engine Worker" rafi-engine-0-2-0-rc-1 engine_worker_name rafi-engine 0.2.0-RC.1
refused "a name over 63 characters" "longer than 63" engine_worker_name "rafi-engine-$(printf 'x%.0s' $(seq 1 50))" 0.1.0

echo "== the configurations"
expect "the template, placeholders and all (warned about)" \
  "app_name=rafi
app_assets=$RELEASE/app/assets
database_id=00000000-0000-0000-0000-000000000000
engine_base=rafi-engine
jurisdiction=eu" read_configs "$APP" "$ENGINE" "$RELEASE" 0
if grep -q "database_id is the placeholder" "$TMP/stderr" && grep -q "isn't a list of publickeyv1_" "$TMP/stderr"; then
  pass "the template's placeholders are warned about"
else
  fail "no warnings about the placeholders: $(cat "$TMP/stderr")"
fi
refused "the template's placeholders, for a deploy" "database_id is the placeholder" read_configs "$APP" "$ENGINE" "$RELEASE" 1
refused "the template's identity key, for a deploy" "RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS isn't a list" read_configs "$APP" "$ENGINE" "$RELEASE" 1

# Filled in, as an installation does.
sed -i.orig "s/00000000-0000-0000-0000-000000000000/$DB_ID/" "$APP" "$ENGINE"
sed -i.orig "s/\"publickeyv1_…\"/\"$KEY, publickeyv1_9n4BzZ8\"/" "$ENGINE"
rm -f "$INSTALL"/*.orig
cp "$APP" "$TMP/app.good"
cp "$ENGINE" "$TMP/engine.good"
expect "an installation's" \
  "app_name=rafi
app_assets=$RELEASE/app/assets
database_id=$DB_ID
engine_base=rafi-engine
jurisdiction=eu" read_configs "$APP" "$ENGINE" "$RELEASE" 1

# broken <name> <file> <sed script> <text>: that change to a good configuration is refused.
broken() {
  cp "$TMP/app.good" "$APP"
  cp "$TMP/engine.good" "$ENGINE"
  sed -i.orig "$3" "$2"
  refused "$1" "$4" read_configs "$APP" "$ENGINE" "$RELEASE" 0
  cp "$TMP/app.good" "$APP"
  cp "$TMP/engine.good" "$ENGINE"
  rm -f "$INSTALL"/*.orig
}
broken "the engine on another database" "$ENGINE" "s/$DB_ID/11111111-2222-3333-4444-555555555555/" "isn't the app's ($DB_ID)"
broken "the engine binding another Worker's RaffleRoom" "$ENGINE" 's/^script_name = "rafi"/script_name = "raffles"/' "must be the app Worker's name (rafi)"
broken "the app renamed, the engine not" "$APP" 's/^name = "rafi"/name = "raffles"/' "must be the app Worker's name (raffles)"
broken "the engine in another jurisdiction" "$ENGINE" 's/^RAFI_DO_JURISDICTION = "eu"/RAFI_DO_JURISDICTION = "none"/' "must be the app's (eu)"
broken "the engine off workers.dev" "$ENGINE" 's/^workers_dev = true/workers_dev = false/' "workers_dev must be true"
broken "the engine named as the app" "$ENGINE" 's/^name = "rafi-engine"/name = "rafi"/' "must differ from the app's"
broken "the app's main elsewhere" "$APP" 's|^main = .*|main = "worker/build/index.js"|' "not this release's app/build/index.js"
broken "the engine's main elsewhere" "$ENGINE" 's|^main = .*|main = "release/app/build/index.js"|' "not this release's engine/build/index.js"
broken "the assets elsewhere" "$APP" 's|^directory = .*|directory = "public"|' "not this release's app/assets"
broken "the migrations elsewhere" "$APP" 's|^migrations_dir = .*|migrations_dir = "migrations"|' "not this release's migrations/sqlite"
broken "a database_id that isn't a string" "$APP" "s/^database_id = .*/database_id = 42/" 'write database_id as database_id = "..."'
broken "a key set twice" "$ENGINE" 's/^class_name = "RaffleRoom"/class_name = "RaffleRoom"\nclass_name = "RaffleRoom"/' "class_name is set twice"
broken "no RAFFLE_ROOM in the engine" "$ENGINE" 's/^name = "RAFFLE_ROOM"/name = "ROOM"/' 'no [[durable_objects.bindings]] with name = "RAFFLE_ROOM"'
broken "a RAFFLE_ROOM of the app's with a script_name" "$APP" 's/^class_name = "RaffleRoom"/class_name = "RaffleRoom"\nscript_name = "rafi"/' "the app's own class: no script_name"
broken "no identity keys" "$ENGINE" '/^RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS/d' "RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS is missing"
mv "$RELEASE/engine/build/index.js" "$TMP/index.js"
refused "a release without the engine" "the release has no engine/build/index.js" read_configs "$APP" "$ENGINE" "$RELEASE" 0
mv "$TMP/index.js" "$RELEASE/engine/build/index.js"
# [env.*] tables don't count (wrangler deploy without --env ignores them).
printf '\n[env.staging]\nname = "rafi-staging"\n[[env.staging.d1_databases]]\nbinding = "DB"\ndatabase_id = "%s"\n' \
  11111111-2222-3333-4444-555555555555 >>"$APP"
expect "[env.*] tables ignored" "rafi" bash -c "source '$ROOT/scripts/cloudflare-deploy.sh'; read_configs '$APP' '$ENGINE' '$RELEASE' 1 | sed -n 's/^app_name=//p'"
cp "$TMP/app.good" "$APP"

echo "== deploy.sh --check, and its usage"
if out="$(cd "$INSTALL" && release/deploy.sh --check 2>&1)" && grep -q "engine Worker: rafi-engine-0-1-0" <<<"$out"; then
  pass "--check names the engine Worker rafi-engine-0-1-0"
else
  fail "--check: $out"
fi
code=0
(cd "$INSTALL" && release/deploy.sh --nope >/dev/null 2>&1) || code=$?
if [ "$code" = 2 ]; then pass "an unknown option exits 2"; else fail "an unknown option exited $code"; fi
code=0
(cd "$INSTALL" && env -u CLOUDFLARE_API_TOKEN -u CLOUDFLARE_ACCOUNT_ID -u RESTATE_ADMIN_URL -u RESTATE_AUTH_TOKEN \
  WRANGLER=false RESTATE=false release/deploy.sh >"$TMP/out" 2>&1) || code=$?
if [ "$code" = 1 ] && grep -q "not set: CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID RESTATE_ADMIN_URL RESTATE_AUTH_TOKEN" "$TMP/out"; then
  pass "a deploy without credentials stops before anything remote"
else
  fail "a deploy without credentials (exit $code): $(cat "$TMP/out")"
fi
if cmp -s "$INSTALL/legal/terms.md" "$RELEASE/app/assets/legal/terms.md" \
  && cmp -s "$INSTALL/legal/privacy.md" "$RELEASE/app/assets/legal/privacy.md"; then
  pass "legal/ copied into the app's static files"
else
  fail "legal/ isn't in $RELEASE/app/assets/legal"
fi

echo "== pruning: which deployments"
SUB=acme
cat >"$TMP/list.json" <<EOF
{
  "deployments": [
    { "deployment_id": "dp_current", "endpoint": "https://rafi-engine-0-3-0.$SUB.workers.dev/restate", "transport": "HTTP/2.0", "status": "Active", "active_invocations": 0, "services": [] },
    { "deployment_id": "dp_draining", "endpoint": "https://rafi-engine-0-2-0-rc-1.$SUB.workers.dev/restate", "status": "Draining", "active_invocations": 2, "services": [] },
    { "deployment_id": "dp_drained", "endpoint": "https://rafi-engine-0-1-0.$SUB.workers.dev/restate/", "status": "Drained", "active_invocations": 0, "services": [] },
    { "deployment_id": "dp_active", "endpoint": "https://rafi-engine-0-0-9.$SUB.workers.dev/restate", "status": "Active", "active_invocations": 0, "services": [] },
    { "deployment_id": "dp_combined", "endpoint": "https://raffle.example.org/restate", "status": "Drained", "active_invocations": 0, "services": [] },
    { "deployment_id": "dp_othersub", "endpoint": "https://rafi-engine-0-1-0.other.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
    { "deployment_id": "dp_otherbase", "endpoint": "https://rafi-engine-staging-0-1-0.$SUB.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
    { "deployment_id": "dp_app", "endpoint": "https://rafi.$SUB.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
    { "deployment_id": "dp_http", "endpoint": "http://rafi-engine-0-0-8.$SUB.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
    { "deployment_id": "dp_path", "endpoint": "https://rafi-engine-0-0-7.$SUB.workers.dev/restate/v2", "status": "Drained", "active_invocations": 0, "services": [] }
  ]
}
EOF
expect "only this installation's older engine Workers" \
  "$(printf 'Draining\tdp_draining\trafi-engine-0-2-0-rc-1\t2\nDrained\tdp_drained\trafi-engine-0-1-0\t0\nActive\tdp_active\trafi-engine-0-0-9\t0')" \
  bash -c "source '$ROOT/scripts/cloudflare-deploy.sh'; prune_plan rafi-engine $SUB https://rafi-engine-0-3-0.$SUB.workers.dev/restate <'$TMP/list.json'"
expect "nothing but the current one" "" \
  bash -c "source '$ROOT/scripts/cloudflare-deploy.sh'; echo '{\"deployments\":[{\"deployment_id\":\"dp_a\",\"endpoint\":\"https://rafi-engine-0-1-0.$SUB.workers.dev/restate\",\"status\":\"Active\"}]}' | prune_plan rafi-engine $SUB https://rafi-engine-0-1-0.$SUB.workers.dev/restate"
refused "a list that isn't one" "no deployments in the list" bash -c "source '$ROOT/scripts/cloudflare-deploy.sh'; echo '{\"error\":\"x\"}' | prune_plan rafi-engine $SUB https://x"

echo "== is anything live? (local D1)"
WRANGLER="${WRANGLER:-wrangler}"
if command -v "${WRANGLER%% *}" >/dev/null; then
  export WRANGLER
  state="$TMP/d1"
  where=(--local --persist-to "$state")
  expect "before the migrations: nothing live" "" live_raffles "$APP" "${where[@]}"
  run_wrangler d1 migrations apply DB -c "$APP" "${where[@]}" >"$TMP/migrate.log" 2>&1 \
    || { cat "$TMP/migrate.log" >&2; fail "the migrations didn't apply locally"; }
  expect "no raffles: nothing live" "" live_raffles "$APP" "${where[@]}"
  insert() {
    run_wrangler d1 execute DB -c "$APP" "${where[@]}" --command "INSERT INTO raffles
      (id, owner_id, slug, join_code, title, event_date, event_ends_at, retention_days, state, created_at, updated_at)
      VALUES ('$1', 'user_1', '$1', 'code-$1', 'Raffle $1', '$3', 0, 30, '$2', 0, 0)" >"$TMP/insert.log" 2>&1 \
      || { cat "$TMP/insert.log" >&2; fail "inserting a raffle"; }
  }
  insert draft draft 2020-01-01
  insert old frozen 2020-01-01
  expect "a draft and an old frozen raffle: nothing live" "" live_raffles "$APP" "${where[@]}"
  insert open open 2020-01-02
  expect "an open raffle is live" 'open  open  2020-01-02  "Raffle open"  owner user_1' live_raffles "$APP" "${where[@]}"
  refused "the preflight stops a deploy" "raffles are live" preflight "$APP" "${where[@]}"
  expect "unless RAFI_DEPLOY_WHILE_LIVE=1" "" bash -c "source '$ROOT/scripts/cloudflare-deploy.sh'; RAFI_DEPLOY_WHILE_LIVE=1 preflight '$APP' ${where[*]} >/dev/null"
  sed -i.orig 's/^binding = "DB"/binding = "DATABASE"/' "$APP"
  refused "a failing query isn't nothing live" "couldn't ask the database" live_raffles "$APP" "${where[@]}"
  cp "$TMP/app.good" "$APP"
  rm -f "$INSTALL"/*.orig
else
  echo "skipped: no wrangler (set WRANGLER)"
fi

if [ "$FAILED" != 0 ]; then
  echo "cloudflare-deploy-test: FAILED" >&2
  exit 1
fi
echo "cloudflare-deploy-test: all passed"
