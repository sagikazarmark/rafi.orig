#!/usr/bin/env bash
# Checks the Cloudflare deploy script (scripts/cloudflare-deploy.sh, a release's deploy.sh)
# without a Cloudflare or Restate account: each subcommand run as an installation's workflows
# run it, from a copy of deploy/cloudflare with a release laid out in release/, against a
# stubbed wrangler and Restate CLI (which log their arguments) and a fake Cloudflare and
# Restate admin API (a local node HTTP server answering from a state file and logging each
# request, in one log with the stubs, so the order of calls shows).
#
#   - check: the release's VERSION and the installation's (and RAFI_VERSION), DEPLOY_VERSION
#     and RAFI_DEPLOY_VERSION, the engine Worker's versioned name, the usage; the three
#     configurations read and cross-checked, broken copies of them refused, placeholders
#     refused (and only warned about by dry-run, which dry-deploys the three Workers);
#   - migrate; deploy-rooms skipping when the deployed version's tag is the fingerprint and
#     deploying when the bundle or rooms.wrangler.toml changed (or nothing, or two versions,
#     are deployed); upload-app's first install (deployed directly, app_promoted) and upgrade
#     (a version uploaded, app_version_id from wrangler's ND-JSON output), a missing Worker
#     refused; deploy-engine's name and URL; register-engine's downgrade guard (semantic
#     versioning with pre-releases), the metadata and "force": false it sends, 5xx retried,
#     4xx failing; promote-app's two paths; cleanup's filtering by metadata, each Worker
#     deleted before its deployment is removed, a Worker already gone, a listing failure
#     stopping it before anything is deleted, and its report of what drains and what isn't
#     this installation's.
#
# scripts/cloudflare-release.sh runs it; so can you:
#
#   ./scripts/cloudflare-deploy-test.sh
#
# Needs node and curl.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT/scripts/cloudflare-deploy.sh"
DEPLOY_VERSION="$(sed -n 's/^DEPLOY_VERSION=\([0-9][0-9]*\)$/\1/p' "$SCRIPT")"

TMP="$(mktemp -d)"
TMP="$(cd "$TMP" && pwd -P)"
SERVER=
cleanup() {
  [ -z "$SERVER" ] || kill "$SERVER" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT
# Killed (or its output closed: | head), it still stops the fake API.
trap 'cleanup; exit 1' INT TERM HUP PIPE
FAILED=0

pass() { printf 'ok   %s\n' "$1"; }
fail() {
  printf 'FAIL %s\n' "$1" >&2
  FAILED=1
}

# The stubs: wrangler and the Restate CLI log their arguments. wrangler fails when its
# arguments contain STUB_WRANGLER_FAIL, writes a version-upload entry for `versions upload`
# (unless STUB_NO_VERSION_ID), and answers --dry-run. The Restate CLI lists STUB_RESTATE_LIST
# (or fails with STUB_RESTATE_LIST_FAIL) and refuses to remove the IDs in
# STUB_RESTATE_REMOVE_FAIL.
BIN="$TMP/bin"
mkdir -p "$BIN"
cat >"$BIN/wrangler" <<'EOF'
#!/usr/bin/env bash
echo "wrangler $*" >>"$STUB_LOG"
if [ -n "${STUB_WRANGLER_FAIL:-}" ] && [[ " $* " == *" $STUB_WRANGLER_FAIL "* ]]; then
  echo "stub wrangler: failing" >&2
  exit 1
fi
case " $* " in *" --dry-run "*)
  echo "dry run: $*"
  exit 0
  ;;
esac
if [ "$1 $2" = "versions upload" ] && [ -n "${WRANGLER_OUTPUT_FILE_PATH:-}" ]; then
  echo '{"type":"wrangler-session","version":1}' >>"$WRANGLER_OUTPUT_FILE_PATH"
  [ -n "${STUB_NO_VERSION_ID:-}" ] \
    || echo '{"type":"version-upload","version":1,"worker_name":"rafi","version_id":"1b2c3d4e-0000-4000-8000-000000000001"}' >>"$WRANGLER_OUTPUT_FILE_PATH"
fi
EOF
cat >"$BIN/restate" <<'EOF'
#!/usr/bin/env bash
echo "restate $*" >>"$STUB_LOG"
case "$*" in
  "deployments list --extra --json")
    [ -z "${STUB_RESTATE_LIST_FAIL:-}" ] || { echo "stub restate: failing" >&2; exit 1; }
    cat "$STUB_RESTATE_LIST"
    ;;
  "deployments remove "*" --yes")
    if [[ " ${STUB_RESTATE_REMOVE_FAIL:-} " == *" $3 "* ]]; then echo "stub restate: refused" >&2; exit 1; fi
    ;;
  *) exit 1 ;;
esac
EOF
chmod +x "$BIN/wrangler" "$BIN/restate"

# The fake APIs: /cf/... is Cloudflare's (account acct123, token cf-token), /restate/... the
# Restate admin API (token rs-token). They answer from state.json, read on each request:
#   subdomain; scripts: { <name>: { deployments: [...], versions: [...] } } (a name absent is
#   no such Worker); status: { "<METHOD> <path>": <status> } overrides; services, deployments
#   (Restate's), register: [statuses of successive POST /deployments], query: rows.
cat >"$TMP/fake.mjs" <<'EOF'
import fs from "node:fs";
import http from "node:http";
const [stateFile, logFile, portFile] = process.argv.slice(2);
let attempt = 0;
const send = (res, status, body) => {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
};
const cfError = (code, message) => ({ success: false, errors: [{ code, message }], result: null });
const server = http.createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    const p = new URL(req.url, "http://fake").pathname;
    if (p === "/__reset") { attempt = 0; return send(res, 200, {}); }
    fs.appendFileSync(logFile, `http ${req.method} ${p}${body ? ` ${body}` : ""}\n`);
    const state = JSON.parse(fs.readFileSync(stateFile, "utf8"));
    const forced = state.status?.[`${req.method} ${p}`];
    if (forced) return send(res, forced, cfError(1, `fake ${forced}`));
    let m;
    if (p.startsWith("/cf/")) {
      if (req.headers.authorization !== "Bearer cf-token") return send(res, 401, cfError(10000, "Authentication error"));
      const scripts = state.scripts ?? {};
      if (req.method === "GET" && p === "/cf/accounts/acct123/workers/subdomain") {
        return state.subdomain ? send(res, 200, { success: true, result: { subdomain: state.subdomain } })
          : send(res, 404, cfError(10007, "no subdomain"));
      }
      if ((m = p.match(/^\/cf\/accounts\/acct123\/workers\/scripts\/([^/]+)(\/.*)?$/))) {
        const s = scripts[m[1]];
        if (req.method === "DELETE" && !m[2]) return s ? send(res, 200, { success: true, result: null }) : send(res, 404, cfError(10007, "workers.api.error.script_not_found"));
        if (!s) return send(res, 404, cfError(10007, "workers.api.error.script_not_found"));
        if (req.method === "GET" && m[2] === "/deployments") return send(res, 200, { success: true, result: { deployments: s.deployments ?? [] } });
        if (req.method === "GET" && m[2] === "/versions") return send(res, 200, { success: true, result: { items: s.versions ?? [] } });
        const v = m[2]?.match(/^\/versions\/([^/]+)$/);
        if (req.method === "GET" && v) {
          const found = (s.versions ?? []).find((x) => x.id === v[1]);
          return found ? send(res, 200, { success: true, result: found }) : send(res, 404, cfError(10209, "no such version"));
        }
      }
      return send(res, 404, cfError(7003, `Could not route to ${p}`));
    }
    if (p.startsWith("/restate/")) {
      if (req.headers.authorization !== "Bearer rs-token") return send(res, 401, { message: "unauthorized" });
      if (req.method === "GET" && p === "/restate/services") return send(res, 200, { services: state.services ?? [] });
      if (req.method === "GET" && p === "/restate/deployments") return send(res, 200, { deployments: state.deployments ?? [] });
      if (req.method === "POST" && p === "/restate/deployments") {
        const statuses = state.register ?? [201];
        const status = statuses[Math.min(attempt++, statuses.length - 1)];
        return send(res, status, status < 300 ? { id: "dp_new", services: [] } : { message: `fake ${status}` });
      }
      if (req.method === "POST" && p === "/restate/query") return send(res, 200, { rows: state.query ?? [] });
    }
    send(res, 404, { message: `no ${p}` });
  });
});
server.listen(0, "127.0.0.1", () => fs.writeFileSync(portFile, String(server.address().port)));
EOF
LOG="$TMP/log"
OUTPUTS="$TMP/github_output"
state() { cat >"$TMP/state.json"; }
echo '{}' | state
: >"$LOG"
node "$TMP/fake.mjs" "$TMP/state.json" "$LOG" "$TMP/port" &
SERVER=$!
for _ in $(seq 1 50); do [ -s "$TMP/port" ] && break; sleep 0.1; done
[ -s "$TMP/port" ] || { echo "the fake API didn't start" >&2; exit 1; }
BASE="http://127.0.0.1:$(cat "$TMP/port")"

# A release as `just cloudflare-artifact` lays it out (empty files will do, the rooms Worker's
# bundle with something in it), unpacked into an installation (a copy of deploy/cloudflare) as
# release/.
INSTALL="$TMP/installation"
mkdir -p "$INSTALL"
cp -R "$ROOT/deploy/cloudflare/." "$INSTALL/"
rm -rf "$INSTALL/release" "$INSTALL/.wrangler" "$INSTALL/.prod.vars"
RELEASE="$INSTALL/release"
mkdir -p "$RELEASE/app/build" "$RELEASE/app/assets" "$RELEASE/engine/build" "$RELEASE/rooms/build" "$RELEASE/migrations"
touch "$RELEASE/app/build/index.js" "$RELEASE/engine/build/index.js"
echo 'export { RaffleRoom, default };' >"$RELEASE/rooms/build/index.js"
echo 'wasm' >"$RELEASE/rooms/build/index_bg.wasm"
cp "$ROOT"/migrations/sqlite/*.sql "$RELEASE/migrations/"
cp "$SCRIPT" "$RELEASE/deploy.sh"
echo "0.1.0" >"$RELEASE/VERSION"
echo "$DEPLOY_VERSION" >"$RELEASE/DEPLOY_VERSION"
APP="$INSTALL/wrangler.toml"
ENGINE="$INSTALL/engine.wrangler.toml"
ROOMS="$INSTALL/rooms.wrangler.toml"
DB_ID=6f1c2a90-3b7e-4d2a-9c41-0e5b8a7d2f13
KEY=publickeyv1_ChjENKeMvCtRnqG2mrBK1HmPKufgFUc98K8B3ZgXonQQ

# What the workflows give every subcommand; a test's own assignments come after (and win), an
# empty one unsets.
COMMON=(RAFI_DEPLOY_VERSION="$DEPLOY_VERSION" WRANGLER="$BIN/wrangler" RESTATE="$BIN/restate" STUB_LOG="$LOG"
  STUB_RESTATE_LIST="$TMP/list.json" CLOUDFLARE_API_BASE_URL="$BASE/cf" CLOUDFLARE_API_TOKEN=cf-token
  CLOUDFLARE_ACCOUNT_ID=acct123 RESTATE_ADMIN_URL="$BASE/restate/" RESTATE_AUTH_TOKEN=rs-token
  RAFI_INSTANCE=acme/raffles RAFI_REGISTER_WAIT=0 GITHUB_OUTPUT="$OUTPUTS" RAFI_REPOSITORY= RAFI_VERSION=
  RAFI_APP_PROMOTED= RAFI_APP_VERSION_ID= RAFI_ENGINE_WORKER= RAFI_ENGINE_URL= STUB_WRANGLER_FAIL=
  STUB_NO_VERSION_ID= STUB_RESTATE_LIST_FAIL= STUB_RESTATE_REMOVE_FAIL=)

# ds <subcommand> [<VAR=value>...]: runs release/deploy.sh <subcommand> in the installation, its
# output in $TMP/out, its exit status in CODE; the log and the outputs fresh.
ds() {
  local sub="$1"
  shift
  : >"$LOG"
  : >"$OUTPUTS"
  curl -s -o /dev/null "$BASE/__reset"
  CODE=0
  (cd "$INSTALL" && env "${COMMON[@]}" "$@" release/deploy.sh ${sub:+"$sub"}) >"$TMP/out" 2>&1 || CODE=$?
}
# ran <name> <exit status> [<text in the output>...]
ran() {
  local name="$1" want="$2" text
  shift 2
  if [ "$CODE" != "$want" ]; then
    fail "$name: exit $CODE, not $want: $(tail -15 "$TMP/out")"
    return
  fi
  for text in "$@"; do
    grep -qF -- "$text" "$TMP/out" || { fail "$name: no '$text' in: $(tail -15 "$TMP/out")"; return; }
  done
  pass "$name"
}
# in_order <name> <text>...: each in the log, on later lines than the one before.
in_order() {
  local name="$1" prev=0 n text
  shift
  for text in "$@"; do
    n="$(grep -nF -- "$text" "$LOG" | head -1 | cut -d: -f1 || true)"
    if [ -z "$n" ] || [ "$n" -le "$prev" ]; then
      fail "$name: no '$text' in the log after the line before; the log: $(cat "$LOG")"
      return
    fi
    prev="$n"
  done
  pass "$name"
}
# not_logged <name> <text>...: none of them in the log.
not_logged() {
  local name="$1" text
  shift
  for text in "$@"; do
    if grep -qF -- "$text" "$LOG"; then
      fail "$name: '$text' in the log: $(cat "$LOG")"
      return
    fi
  done
  pass "$name"
}
# outputs_are <name> <the GITHUB_OUTPUT file's content>
outputs_are() {
  if [ "$(cat "$OUTPUTS")" = "$2" ]; then pass "$1"; else fail "$1: the outputs are '$(cat "$OUTPUTS")', not '$2'"; fi
}

echo "== the template: placeholders"
ds check
ran "check refuses the template's database_id" 1 "database_id is the placeholder"
ran "check refuses the template's identity key" 1 "RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS isn't a list of publickeyv1_"
ds dry-run CLOUDFLARE_API_TOKEN= CLOUDFLARE_ACCOUNT_ID= RESTATE_ADMIN_URL= RESTATE_AUTH_TOKEN= RAFI_INSTANCE=
ran "dry-run only warns about them, and needs no credentials" 0 "warning:" "database_id is the placeholder" \
  "dry run of Rafi 0.1.0 done"
in_order "dry-run: the rooms Worker, the engine under its versioned name, the app" \
  "wrangler deploy --dry-run -c $ROOMS" "wrangler deploy --dry-run -c $ENGINE --name rafi-engine-v0-1-0" \
  "wrangler deploy --dry-run -c $APP"
if cmp -s "$INSTALL/legal/terms.md" "$RELEASE/app/assets/legal/terms.md" \
  && cmp -s "$INSTALL/legal/privacy.md" "$RELEASE/app/assets/legal/privacy.md"; then
  pass "dry-run copies legal/ into the app's static files"
else
  fail "legal/ isn't in $RELEASE/app/assets/legal"
fi
rm -rf "$RELEASE/app/assets/legal"

# Filled in, as an installation does.
sed -i.orig "s/00000000-0000-0000-0000-000000000000/$DB_ID/" "$APP" "$ENGINE" "$ROOMS"
sed -i.orig "s/\"publickeyv1_…\"/\"$KEY, publickeyv1_9n4BzZ8\"/" "$ENGINE"
rm -f "$INSTALL"/*.orig
cp "$APP" "$TMP/app.good"
cp "$ENGINE" "$TMP/engine.good"
cp "$ROOMS" "$TMP/rooms.good"
restore() {
  cp "$TMP/app.good" "$APP"
  cp "$TMP/engine.good" "$ENGINE"
  cp "$TMP/rooms.good" "$ROOMS"
  rm -f "$INSTALL"/*.orig
}

echo "== check: the release, the installation's VERSION and the deploy interface"
if [[ "$DEPLOY_VERSION" =~ ^[0-9]+$ ]]; then pass "deploy.sh's DEPLOY_VERSION is an integer ($DEPLOY_VERSION)"; else fail "DEPLOY_VERSION is '$DEPLOY_VERSION'"; fi
ds check
ran "an installation's" 0 "Rafi 0.1.0 (deploy interface $DEPLOY_VERSION)" "engine Worker: rafi-engine-v0-1-0" \
  "rooms Worker: rafi-rooms" "app Worker: rafi" "database: $DB_ID; Durable Objects' jurisdiction: eu"
if [ -s "$LOG" ]; then fail "check called: $(cat "$LOG")"; else pass "check calls nothing"; fi
ds check RAFI_DEPLOY_VERSION=
ran "no RAFI_DEPLOY_VERSION" 1 "RAFI_DEPLOY_VERSION isn't set" "copy .github/workflows/deploy.yaml and restate-cleanup.yaml"
ds check RAFI_DEPLOY_VERSION="$((DEPLOY_VERSION + 1))" RAFI_REPOSITORY=acme/rafi
ran "workflows written for another deploy interface" 1 \
  "Rafi 0.1.0's deploy interface is $DEPLOY_VERSION, but your workflows are written for $((DEPLOY_VERSION + 1))" \
  "https://github.com/acme/rafi/tree/v0.1.0/deploy/cloudflare/.github/workflows"
echo 0 >"$RELEASE/DEPLOY_VERSION"
ds check
ran "a release whose DEPLOY_VERSION isn't its deploy.sh's" 1 "DEPLOY_VERSION says '0', but this deploy.sh is interface version $DEPLOY_VERSION"
echo "$DEPLOY_VERSION" >"$RELEASE/DEPLOY_VERSION"
echo "0.2.0" >"$INSTALL/VERSION"
ds check
ran "the installation's VERSION another release" 1 "the installation's VERSION says 0.2.0, but the release in $RELEASE is 0.1.0"
echo "0.1" >"$INSTALL/VERSION"
ds check
ran "a VERSION that isn't a version" 1 "says '0.1', not a Rafi version"
rm "$INSTALL/VERSION"
ds check
ran "no VERSION" 1 "$INSTALL/VERSION is missing"
echo "0.1.0" >"$INSTALL/VERSION"
ds check RAFI_VERSION=v0.1.0
ran "RAFI_VERSION, the same" 0
ds check RAFI_VERSION=0.2.0
ran "RAFI_VERSION, another" 1 "RAFI_VERSION says 0.2.0"
echo "v0.2.0-RC.1" >"$RELEASE/VERSION"
echo "0.2.0-RC.1" >"$INSTALL/VERSION"
ds check
ran "a pre-release, a leading v dropped: its engine Worker" 0 "engine Worker: rafi-engine-v0-2-0-rc-1"
echo "0.1.0" >"$RELEASE/VERSION"
echo "0.1.0" >"$INSTALL/VERSION"
sed -i.orig "s/^name = \"rafi-engine\"/name = \"rafi-engine-$(printf 'x%.0s' $(seq 1 50))\"/" "$ENGINE"
ds check
ran "an engine Worker's name over 63 characters" 1 "longer than 63"
restore
ds nope
ran "an unknown subcommand exits 2" 2
ds ""
ran "no subcommand exits 2" 2
ds --check
ran "the old --check exits 2" 2
ds --help
ran "--help" 0 "release/deploy.sh register-engine"

echo "== check: the configurations"
# broken <name> <file> <sed script> <text>: that change to a good configuration is refused.
broken() {
  restore
  sed -i.orig "$3" "$2"
  ds check
  ran "$1" 1 "$4"
  restore
}
broken "the engine on another database" "$ENGINE" "s/$DB_ID/11111111-2222-3333-4444-555555555555/" "isn't the app's ($DB_ID)"
broken "the rooms on another database" "$ROOMS" "s/$DB_ID/11111111-2222-3333-4444-555555555555/" "isn't the app's ($DB_ID)"
broken "the engine binding another Worker's RaffleRoom" "$ENGINE" 's/^script_name = "rafi-rooms"/script_name = "rafi"/' "engine.wrangler.toml: RAFFLE_ROOM's script_name (rafi) must be the rooms Worker's name (rafi-rooms)"
broken "the app binding another Worker's RaffleRoom" "$APP" 's/^script_name = "rafi-rooms"/script_name = "rooms"/' "wrangler.toml: RAFFLE_ROOM's script_name (rooms) must be the rooms Worker's name (rafi-rooms)"
broken "the app with the class of its own" "$APP" '/^script_name = "rafi-rooms"/d' "wrangler.toml: RAFFLE_ROOM's script_name (undefined) must be the rooms Worker's name"
broken "the rooms renamed, the others not" "$ROOMS" 's/^name = "rafi-rooms"/name = "raffle-rooms"/' "must be the rooms Worker's name (raffle-rooms)"
broken "the engine in another jurisdiction" "$ENGINE" 's/^RAFI_DO_JURISDICTION = "eu"/RAFI_DO_JURISDICTION = "none"/' "engine.wrangler.toml: RAFI_DO_JURISDICTION (none) must be the rooms Worker's (eu)"
broken "the app in another jurisdiction" "$APP" 's/^RAFI_DO_JURISDICTION = "eu"/RAFI_DO_JURISDICTION = "fedramp"/' "wrangler.toml: RAFI_DO_JURISDICTION (fedramp) must be the rooms Worker's (eu)"
broken "the rooms without a jurisdiction" "$ROOMS" '/^RAFI_DO_JURISDICTION/d' "rooms.wrangler.toml: [vars] RAFI_DO_JURISDICTION is missing"
broken "the engine off workers.dev" "$ENGINE" 's/^workers_dev = true/workers_dev = false/' "workers_dev must be true"
broken "the rooms on workers.dev" "$ROOMS" 's/^workers_dev = false/workers_dev = true/' "rooms.wrangler.toml: workers_dev must be false"
broken "the rooms without preview_urls = false" "$ROOMS" '/^preview_urls/d' "rooms.wrangler.toml: preview_urls must be false"
broken "the engine named as the app" "$ENGINE" 's/^name = "rafi-engine"/name = "rafi"/' "must differ from the app's"
broken "the engine named as the rooms" "$ENGINE" 's/^name = "rafi-engine"/name = "rafi-rooms"/' "must differ from the rooms Worker's"
broken "the app named as the rooms" "$APP" 's/^name = "rafi"/name = "rafi-rooms"/' "wrangler.toml: name must differ from the rooms Worker's"
broken "the app's main elsewhere" "$APP" 's|^main = .*|main = "worker/build/index.js"|' "not this release's app/build/index.js"
broken "the engine's main elsewhere" "$ENGINE" 's|^main = .*|main = "release/app/build/index.js"|' "not this release's engine/build/index.js"
broken "the rooms' main elsewhere" "$ROOMS" 's|^main = .*|main = "release/app/build/index.js"|' "not this release's rooms/build/index.js"
broken "the assets elsewhere" "$APP" 's|^directory = .*|directory = "public"|' "not this release's app/assets"
broken "the migrations elsewhere" "$APP" 's|^migrations_dir = .*|migrations_dir = "release/migrations/sqlite"|' "not this release's migrations"
broken "a database_id that isn't a string" "$APP" "s/^database_id = .*/database_id = 42/" 'write database_id as database_id = "..."'
broken "a key set twice" "$ENGINE" 's/^class_name = "RaffleRoom"/class_name = "RaffleRoom"\nclass_name = "RaffleRoom"/' "class_name is set twice"
broken "no RAFFLE_ROOM in the engine" "$ENGINE" 's/^name = "RAFFLE_ROOM"/name = "ROOM"/' 'no [[durable_objects.bindings]] with name = "RAFFLE_ROOM"'
broken "no RAFFLE_ROOM in the rooms" "$ROOMS" 's/^name = "RAFFLE_ROOM"/name = "ROOM"/' 'rooms.wrangler.toml: no [[durable_objects.bindings]] with name = "RAFFLE_ROOM"'
broken "a RAFFLE_ROOM of the rooms' with a script_name" "$ROOMS" 's/^class_name = "RaffleRoom"/class_name = "RaffleRoom"\nscript_name = "rafi"/' "the rooms Worker's own class: no script_name"
broken "no identity keys" "$ENGINE" '/^RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS/d' "RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS is missing"
for worker in engine rooms; do
  mv "$RELEASE/$worker/build/index.js" "$TMP/index.js"
  ds check
  ran "a release without the $worker" 1 "the release has no $worker/build/index.js"
  mv "$TMP/index.js" "$RELEASE/$worker/build/index.js"
done
mv "$ROOMS" "$TMP/rooms.aside"
ds check
ran "no rooms.wrangler.toml" 1 "rooms.wrangler.toml is missing"
mv "$TMP/rooms.aside" "$ROOMS"
# [env.*] tables don't count (wrangler deploy without --env ignores them).
printf '\n[env.staging]\nname = "rafi-staging"\n[[env.staging.d1_databases]]\nbinding = "DB"\ndatabase_id = "%s"\n' \
  11111111-2222-3333-4444-555555555555 >>"$APP"
ds check
ran "[env.*] tables ignored" 0 "app Worker: rafi ("
restore

echo "== migrate"
ds migrate
ran "migrate" 0
in_order "the D1 migrations, remote, with the app's configuration" "wrangler d1 migrations apply DB -c $APP --remote"
ds migrate CLOUDFLARE_API_TOKEN=
ran "migrate without credentials" 1 "not set: CLOUDFLARE_API_TOKEN"
not_logged "nothing ran without them" "wrangler"

echo "== deploy-rooms"
echo '{}' | state
ds deploy-rooms
ran "a rooms Worker not deployed yet is deployed" 0 "not deployed yet"
TAG="$(grep -o 'rooms-[0-9a-f]\{32\}' "$LOG" | head -1 || true)"
in_order "wrangler deploy of the rooms Worker, tagged with its fingerprint" \
  "wrangler deploy -c $ROOMS --tag $TAG --message rafi v0.1.0"
if [[ "$TAG" =~ ^rooms-[0-9a-f]{32}$ ]]; then pass "the fingerprint's tag: $TAG"; else fail "no rooms-<32 hex> tag: '$TAG'"; fi
# rooms_state <tag> [<second tag>]: the rooms Worker serving a version with that tag (two at 50% each).
rooms_state() {
  if [ -z "${2:-}" ]; then
    state <<EOF
{ "scripts": { "rafi-rooms": {
  "deployments": [ { "id": "d2", "versions": [ { "version_id": "r2", "percentage": 100 } ] },
                   { "id": "d1", "versions": [ { "version_id": "r1", "percentage": 100 } ] } ],
  "versions": [ { "id": "r2", "annotations": { "workers/tag": "$1", "workers/message": "rafi v0.0.9" } },
                { "id": "r1", "annotations": { "workers/tag": "rooms-0" } } ] } } }
EOF
  else
    state <<EOF
{ "scripts": { "rafi-rooms": {
  "deployments": [ { "id": "d2", "versions": [ { "version_id": "r2", "percentage": 50 }, { "version_id": "r1", "percentage": 50 } ] } ],
  "versions": [ { "id": "r2", "annotations": { "workers/tag": "$1" } }, { "id": "r1", "annotations": { "workers/tag": "$2" } } ] } } }
EOF
  fi
}
rooms_state "$TAG"
ds deploy-rooms
ran "a rooms Worker serving this fingerprint is left alone" 0 "unchanged (it serves $TAG)" "keep their connections"
in_order "its latest deployment's version read" "http GET /cf/accounts/acct123/workers/scripts/rafi-rooms/deployments" \
  "http GET /cf/accounts/acct123/workers/scripts/rafi-rooms/versions/r2"
not_logged "nothing deployed" "wrangler deploy"
rooms_state "rooms-0123456789abcdef0123456789abcdef"
ds deploy-rooms
ran "another fingerprint deployed" 0 "it serves: 100 rooms-0123456789abcdef0123456789abcdef"
in_order "the rooms Worker deployed" "wrangler deploy -c $ROOMS --tag $TAG"
rooms_state "$TAG" "rooms-0123456789abcdef0123456789abcdef"
ds deploy-rooms
ran "a gradual deployment, half this fingerprint, deployed" 0
in_order "the rooms Worker deployed over a gradual one" "wrangler deploy -c $ROOMS --tag $TAG"
rooms_state "$TAG"
echo "# a comment" >>"$ROOMS"
ds deploy-rooms
ran "rooms.wrangler.toml changed: deployed" 0
not_logged "with another tag" "--tag $TAG "
in_order "the rooms Worker deployed after its configuration changed" "wrangler deploy -c $ROOMS --tag rooms-"
restore
echo 'more' >>"$RELEASE/rooms/build/index_bg.wasm"
ds deploy-rooms
ran "the bundle changed: deployed" 0
not_logged "the bundle changed: with another tag" "--tag $TAG "
in_order "the rooms Worker deployed after its bundle changed" "wrangler deploy -c $ROOMS --tag rooms-"
echo 'wasm' >"$RELEASE/rooms/build/index_bg.wasm"
ds deploy-rooms
ran "the bundle as it was: the same fingerprint" 0 "unchanged (it serves $TAG)"
state <<'EOF'
{ "status": { "GET /cf/accounts/acct123/workers/scripts/rafi-rooms/deployments": 500 } }
EOF
ds deploy-rooms
ran "Cloudflare failing: refused" 1 "the Worker rafi-rooms's deployments: HTTP 500"
not_logged "nothing deployed when Cloudflare fails" "wrangler deploy"

echo "== upload-app"
echo '{}' | state
ds upload-app
ran "no app Worker: refused, its secrets first" 1 "the app Worker rafi doesn't exist" "wrangler secret bulk .prod.vars"
not_logged "nothing uploaded without an app Worker" "wrangler"
# app_state <tag of the deployed version> <tag of an older version>: the app Worker.
app_state() {
  state <<EOF
{ "scripts": { "rafi": {
  "deployments": [ { "id": "d2", "versions": [ { "version_id": "a2", "percentage": 100 } ] } ],
  "versions": [ { "id": "a2", "annotations": { "workers/tag": "$1", "workers/triggered_by": "secret" } },
                { "id": "a1", "annotations": { "workers/tag": "$2" } } ] } } }
EOF
}
# A Worker `wrangler secret bulk` created: a placeholder, then secret changes, none tagged.
app_state "" ""
ds upload-app
ran "a first install (only secrets): deployed directly" 0 "no release ran here yet"
in_order "wrangler deploy of the app, tagged" "wrangler deploy -c $APP --tag v0.1.0 --message rafi v0.1.0"
not_logged "nothing uploaded on a first install" "versions upload"
outputs_are "a first install's output" "app_promoted=true"
if cmp -s "$INSTALL/legal/privacy.md" "$RELEASE/app/assets/legal/privacy.md"; then pass "legal/ copied in"; else fail "legal/ not copied"; fi
app_state "v0.0.9" ""
ds upload-app
ran "an upgrade: a version uploaded" 0 "a release ran here before"
in_order "wrangler versions upload, tagged" "wrangler versions upload -c $APP --tag v0.1.0 --message rafi v0.1.0"
not_logged "not deployed" "wrangler deploy"
outputs_are "the uploaded version's ID" "app_version_id=1b2c3d4e-0000-4000-8000-000000000001"
app_state "" "v0.0.9"
ds upload-app
ran "a secret changed since the last release: still an upgrade" 0 "a release ran here before"
in_order "its recent versions read" "http GET /cf/accounts/acct123/workers/scripts/rafi/versions" "wrangler versions upload"
ds upload-app STUB_NO_VERSION_ID=1
ran "wrangler reporting no version ID: failed" 1 "wrangler versions upload reported no version ID"
outputs_are "no output then" ""
ds upload-app STUB_WRANGLER_FAIL=upload
ran "wrangler failing: failed" 1
outputs_are "no output when wrangler fails" ""
ds upload-app GITHUB_OUTPUT=
ran "without GITHUB_OUTPUT the output is printed" 0 "app_version_id=1b2c3d4e-0000-4000-8000-000000000001"

echo "== deploy-engine"
echo '{ "subdomain": "acme" }' | state
ds deploy-engine
ran "the engine Worker deployed" 0
in_order "under its versioned name, tagged" "http GET /cf/accounts/acct123/workers/subdomain" \
  "wrangler deploy -c $ENGINE --name rafi-engine-v0-1-0 --tag v0.1.0 --message rafi v0.1.0"
outputs_are "its name and address" "engine_worker=rafi-engine-v0-1-0
engine_url=https://rafi-engine-v0-1-0.acme.workers.dev/restate"
echo '{}' | state
ds deploy-engine
ran "no workers.dev subdomain: refused" 1 "the account's workers.dev subdomain: HTTP 404"
not_logged "nothing deployed without a subdomain" "wrangler"

echo "== register-engine"
ENGINE_URL=https://rafi-engine-v0-1-0.acme.workers.dev/restate
REG=(RAFI_ENGINE_WORKER=rafi-engine-v0-1-0 RAFI_ENGINE_URL="$ENGINE_URL")
REGISTERED='"uri":"https://rafi-engine-v0-1-0.acme.workers.dev/restate","force":false,"metadata":{"app":"rafi","instance":"acme/raffles","version":"0.1.0","cloudflare_account":"acct123","worker":"rafi-engine-v0-1-0"}'
# restate_state <Raffle's deployment's metadata> [<EngineInfo's>] [<register statuses>]
restate_state() {
  state <<EOF
{ "services": [ { "name": "Raffle", "deployment_id": "dp_raffle" }, { "name": "EngineInfo", "deployment_id": "dp_info" },
                { "name": "Other", "deployment_id": "dp_other" } ],
  "deployments": [ { "id": "dp_raffle", "uri": "https://x.acme.workers.dev/restate", "metadata": $1 },
                   { "id": "dp_info", "uri": "https://y.acme.workers.dev/restate", "metadata": ${2:-$1} },
                   { "id": "dp_other", "uri": "https://example.org", "metadata": { "app": "rafi", "instance": "acme/raffles", "cloudflare_account": "acct123", "version": "9.0.0" } } ],
  "register": ${3:-[201]} }
EOF
}
# attempts <n> <name>: Restate was asked to register n times.
attempts() {
  local n
  n="$(grep -c 'POST /restate/deployments' "$LOG" || true)"
  if [ "$n" = "$1" ]; then pass "$2"; else fail "$2: $n attempts"; fi
}
ours() { printf '{ "app": "rafi", "instance": "acme/raffles", "cloudflare_account": "acct123", "version": "%s", "worker": "w" }' "$1"; }
echo '{ "services": [], "deployments": [] }' | state
ds register-engine "${REG[@]}"
ran "a first registration" 0 "Raffle: not registered yet" "registered: Restate deployment dp_new"
in_order "the services and deployments read, then the engine registered with its metadata, never forced" \
  "http GET /restate/services" "http GET /restate/deployments" "http POST /restate/deployments {$REGISTERED}"
not_logged "no token in the stubs' arguments" "rs-token" "cf-token"
restate_state "$(ours 0.2.0)"
ds register-engine "${REG[@]}"
ran "a downgrade refused" 1 "Raffle runs on Rafi 0.2.0 (dp_raffle), newer than 0.1.0"
not_logged "nothing registered on a downgrade" "POST /restate/deployments"
restate_state "$(ours 0.1.0)"
ds register-engine "${REG[@]}"
ran "the same release: a re-run" 0 "Rafi 0.1.0 (this release: a re-run)"
for older in 0.0.9 0.0.10 0.1.0-rc.1 0.1.0-0; do
  restate_state "$(ours "$older")"
  ds register-engine "${REG[@]}"
  ran "an upgrade from $older" 0 "Raffle: on dp_raffle, Rafi $older" "registered: Restate deployment dp_new"
done
for newer in 0.10.0 0.1.1-alpha 1.0.0-rc.1; do
  restate_state "$(ours "$newer")"
  ds register-engine "${REG[@]}"
  ran "a downgrade from $newer refused" 1 "newer than 0.1.0"
done
restate_state "$(ours 0.1.0)" "$(ours 0.3.0)"
ds register-engine "${REG[@]}"
ran "EngineInfo on a newer release: refused" 1 "EngineInfo runs on Rafi 0.3.0"
restate_state '{ "app": "rafi", "instance": "other/raffles", "cloudflare_account": "acct123", "version": "9.0.0" }'
ds register-engine "${REG[@]}"
ran "another installation's deployment: no guard" 0 "not a deployment of this installation"
restate_state '{ "app": "rafi", "instance": "acme/raffles", "cloudflare_account": "acct999", "version": "9.0.0" }'
ds register-engine "${REG[@]}"
ran "another account's deployment: no guard" 0 "not a deployment of this installation"
restate_state 'null'
ds register-engine "${REG[@]}"
ran "a deployment without metadata (registered by hand): no guard" 0 "not a deployment of this installation"
restate_state "$(ours 0.0.9)" "" "[200]"
ds register-engine "${REG[@]}"
ran "Restate had it already (200)" 0 "Restate already had it: deployment dp_new"
restate_state "$(ours 0.0.9)" "" "[503, 502, 201]"
ds register-engine "${REG[@]}"
ran "5xx retried" 0 "attempt 1 of 5): HTTP 503" "attempt 2 of 5): HTTP 502" "registered: Restate deployment dp_new"
attempts 3 "three attempts"
restate_state "$(ours 0.0.9)" "" "[500]"
ds register-engine "${REG[@]}"
ran "5xx every time: failed" 1 "deployed but not registered"
attempts 5 "five attempts, then failed"
restate_state "$(ours 0.0.9)" "" "[400, 201]"
ds register-engine "${REG[@]}"
ran "4xx: failed at once" 1 "registering $ENGINE_URL with Restate: HTTP 400"
attempts 1 "one attempt on a 4xx"
state <<'EOF'
{ "status": { "GET /restate/services": 500 } }
EOF
ds register-engine "${REG[@]}"
ran "Restate's services failing: refused" 1 "Restate's services: HTTP 500"
not_logged "nothing registered when the services can't be read" "POST /restate/deployments"
echo '{ "services": [], "deployments": [] }' | state
ds register-engine RAFI_ENGINE_WORKER=rafi-engine-v0-0-9 RAFI_ENGINE_URL=https://rafi-engine-v0-0-9.acme.workers.dev/restate
ran "another release's engine Worker: refused" 1 "but this release's engine Worker is rafi-engine-v0-1-0"
ds register-engine RAFI_ENGINE_WORKER=rafi-engine-v0-1-0 RAFI_ENGINE_URL=https://example.org/restate
ran "an address that isn't the engine's: refused" 1 "not https://rafi-engine-v0-1-0.<subdomain>.workers.dev/restate"
ds register-engine "${REG[@]}" RAFI_INSTANCE=
ran "no RAFI_INSTANCE: refused" 1 "not set: RAFI_INSTANCE"
not_logged "nothing asked without it" "http"
# A pre-release's precedence among pre-releases.
echo "0.2.0-rc.10" >"$RELEASE/VERSION"
echo "0.2.0-rc.10" >"$INSTALL/VERSION"
REG_RC=(RAFI_ENGINE_WORKER=rafi-engine-v0-2-0-rc-10 RAFI_ENGINE_URL=https://rafi-engine-v0-2-0-rc-10.acme.workers.dev/restate)
for older in 0.2.0-rc.9 0.2.0-rc 0.2.0-beta.11 0.1.9; do
  restate_state "$(ours "$older")"
  ds register-engine "${REG_RC[@]}"
  ran "0.2.0-rc.10 after $older" 0 "registered: Restate deployment dp_new"
done
for newer in 0.2.0-rc.11 0.2.0-rc.beta 0.2.0-rc.10.1 0.2.0; do
  restate_state "$(ours "$newer")"
  ds register-engine "${REG_RC[@]}"
  ran "0.2.0-rc.10 after $newer: refused" 1 "newer than 0.2.0-rc.10"
done
echo "0.1.0" >"$RELEASE/VERSION"
echo "0.1.0" >"$INSTALL/VERSION"

echo "== promote-app"
ds promote-app RAFI_APP_PROMOTED=true
ran "after a first install: nothing" 0 "nothing to promote"
not_logged "nothing deployed after a first install" "wrangler"
ds promote-app RAFI_APP_VERSION_ID=1b2c3d4e-0000-4000-8000-000000000001
ran "an uploaded version deployed" 0
in_order "the version at 100%, then the routes and Cron" \
  "wrangler versions deploy 1b2c3d4e-0000-4000-8000-000000000001@100% --yes -c $APP --message rafi v0.1.0" \
  "wrangler triggers deploy -c $APP"
ds promote-app
ran "neither output: refused" 1 "neither RAFI_APP_PROMOTED=true nor RAFI_APP_VERSION_ID"
ds promote-app RAFI_APP_VERSION_ID=1b2c3d4e-0000-4000-8000-000000000001 STUB_WRANGLER_FAIL="versions deploy"
ran "the version's deploy failing: failed" 1
not_logged "no triggers without the version" "triggers deploy"

echo "== cleanup"
meta() { printf '{ "app": "rafi", "instance": "%s", "cloudflare_account": "%s", "version": "0.0.1", "worker": "%s" }' "$1" "$2" "$3"; }
state <<EOF
{ "scripts": { "rafi-engine-v0-0-8": {}, "rafi-engine-v0-0-6": {}, "rafi": {} },
  "query": [ { "id": "inv_1", "status": "paused" }, { "id": "inv_2", "status": "suspended" } ],
  "deployments": [
    { "id": "dp_drained", "uri": "https://rafi-engine-v0-0-8.acme.workers.dev/restate", "metadata": $(meta acme/raffles acct123 rafi-engine-v0-0-8) },
    { "id": "dp_gone", "uri": "https://rafi-engine-v0-0-7.acme.workers.dev/restate", "metadata": $(meta acme/raffles acct123 rafi-engine-v0-0-7) },
    { "id": "dp_draining", "uri": "https://rafi-engine-v0-0-9.acme.workers.dev/restate", "metadata": $(meta acme/raffles acct123 rafi-engine-v0-0-9) },
    { "id": "dp_active", "uri": "https://rafi-engine-v0-1-0.acme.workers.dev/restate", "metadata": $(meta acme/raffles acct123 rafi-engine-v0-1-0) },
    { "id": "dp_nometa", "uri": "https://raffle.example.org/restate" },
    { "id": "dp_otherinst", "uri": "https://rafi-engine-v0-0-5.acme.workers.dev/restate", "metadata": $(meta other/raffles acct123 rafi-engine-v0-0-5) },
    { "id": "dp_otheracct", "uri": "https://rafi-engine-v0-0-5.acme.workers.dev/restate", "metadata": $(meta acme/raffles acct999 rafi-engine-v0-0-5) },
    { "id": "dp_appworker", "uri": "https://rafi.acme.workers.dev/restate", "metadata": $(meta acme/raffles acct123 rafi) },
    { "id": "dp_elsewhere", "uri": "https://evil.acme.workers.dev/restate", "metadata": $(meta acme/raffles acct123 rafi-engine-v0-0-6) }
  ] }
EOF
cat >"$TMP/list.json" <<'EOF'
{ "deployments": [
  { "deployment_id": "dp_drained", "endpoint": "https://rafi-engine-v0-0-8.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_gone", "endpoint": "https://rafi-engine-v0-0-7.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_draining", "endpoint": "https://rafi-engine-v0-0-9.acme.workers.dev/restate", "status": "Draining", "active_invocations": 2, "services": [] },
  { "deployment_id": "dp_active", "endpoint": "https://rafi-engine-v0-1-0.acme.workers.dev/restate", "status": "Active", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_nometa", "endpoint": "https://raffle.example.org/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_otherinst", "endpoint": "https://rafi-engine-v0-0-5.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_otheracct", "endpoint": "https://rafi-engine-v0-0-5.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_appworker", "endpoint": "https://rafi.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_elsewhere", "endpoint": "https://evil.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] }
] }
EOF
cp "$TMP/state.json" "$TMP/cleanup.json"
ds cleanup
ran "cleanup" 0 "dp_drained (rafi-engine-v0-0-8) is drained" "deleted the Worker rafi-engine-v0-0-8" \
  "removed the deployment dp_drained" "the Worker rafi-engine-v0-0-7 is already gone" "removed the deployment dp_gone" \
  "dp_draining (rafi-engine-v0-0-9) is draining: 2 invocation(s)" "inv_1 paused" \
  "restate invocations resume <id> --deployment latest" "dp_active (rafi-engine-v0-1-0) is active" \
  "dp_nometa at https://raffle.example.org/restate: not a Rafi deployment" "dp_otherinst at" "another installation (other/raffles)" \
  "dp_otheracct at" "another Cloudflare account" "dp_appworker at" "its worker is not a rafi-engine-v... engine Worker (rafi)" \
  "dp_elsewhere at" "its address is not the workers.dev one of rafi-engine-v0-0-6" "2 drained engine deployment(s)"
in_order "each Worker deleted, then its deployment removed" \
  "http GET /restate/deployments" "restate deployments list --extra --json" \
  "http DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-8" "restate deployments remove dp_drained --yes" \
  "http DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-7" "restate deployments remove dp_gone --yes"
not_logged "nothing else deleted or removed" "scripts/rafi-engine-v0-0-9" "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-1-0" \
  "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-5" "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-6" \
  "remove dp_draining" "remove dp_active" "remove dp_nometa" \
  "remove dp_otherinst" "remove dp_otheracct" "remove dp_appworker" "remove dp_elsewhere" "--force"
if grep -qx 'http DELETE /cf/accounts/acct123/workers/scripts/rafi' "$LOG"; then fail "the app Worker deleted"; else pass "the app Worker never deleted"; fi
node -e '
  const fs = require("fs");
  const s = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
  s.status = { "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-8": 500 };
  fs.writeFileSync(process.argv[2], JSON.stringify(s));' "$TMP/cleanup.json" "$TMP/state.json"
ds cleanup
ran "a Worker's deletion failing: that deployment stays, the rest go on, failed" 1 \
  "deleting the Worker rafi-engine-v0-0-8: HTTP 500" "removed the deployment dp_gone" "the cleanup failed in part"
not_logged "its deployment stays" "remove dp_drained"
cp "$TMP/cleanup.json" "$TMP/state.json"
ds cleanup STUB_RESTATE_REMOVE_FAIL=dp_drained
ran "Restate refusing a removal: failed, the next cleanup retries" 1 "Restate didn't remove dp_drained" "removed the deployment dp_gone"
node -e '
  const fs = require("fs");
  const s = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
  s.status = { "GET /restate/deployments": 500 };
  fs.writeFileSync(process.argv[2], JSON.stringify(s));' "$TMP/cleanup.json" "$TMP/state.json"
ds cleanup
ran "the deployments' metadata unlisted: stopped" 1 "listing Restate's deployments: HTTP 500" "nothing deleted"
not_logged "nothing deleted without the metadata" "DELETE" "deployments remove"
cp "$TMP/cleanup.json" "$TMP/state.json"
ds cleanup STUB_RESTATE_LIST_FAIL=1
ran "the deployments' status unlisted: stopped" 1 "restate deployments list failed: nothing deleted"
not_logged "nothing deleted without the status" "DELETE" "deployments remove"
echo 'not json' >"$TMP/list.json"
ds cleanup
ran "a status listing that isn't one: stopped" 1 "couldn't read Restate's deployments: nothing deleted"
not_logged "nothing deleted on a listing that isn't one" "DELETE" "deployments remove"
ds cleanup RESTATE_AUTH_TOKEN=
ran "cleanup without Restate's token" 1 "not set: RESTATE_AUTH_TOKEN"

if [ "$FAILED" != 0 ]; then
  echo "cloudflare-deploy-test: FAILED" >&2
  exit 1
fi
echo "cloudflare-deploy-test: all passed"
