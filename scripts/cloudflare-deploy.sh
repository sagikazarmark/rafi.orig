#!/usr/bin/env bash
# Deploys a release of Rafi to Cloudflare: the app Worker and the release's
# own raffle engine Worker, registered with Restate. Each release ships this
# file as its deploy.sh (the source is scripts/cloudflare-deploy.sh in the
# Rafi repository), so the deploy logic is the release's own, pinned to it and
# covered by its provenance attestation.
#
# Usage, from the root of an installation's repository (a copy of
# deploy/cloudflare: wrangler.toml, engine.wrangler.toml, legal/), with the
# release unpacked into release/ (the directory this script is in is the
# release; the two configurations point into it as release/...):
#
#   release/deploy.sh             deploy
#   release/deploy.sh --dry-run   check, then `wrangler deploy --dry-run` both Workers; touches nothing remote
#   release/deploy.sh --check     check the release and the two configurations only (no wrangler)
#
# Environment:
#   CLOUDFLARE_API_TOKEN, CLOUDFLARE_ACCOUNT_ID   the account (Workers Scripts and D1 edit); not for --dry-run or --check
#   RESTATE_ADMIN_URL      the Restate environment's admin API (https://<env>.env.<region>.restate.cloud:9070)
#   RESTATE_AUTH_TOKEN     a Restate API key with the Admin role
#   RAFI_VERSION           optional: the version this must be (the release's VERSION is refused otherwise)
#   RAFI_DEPLOY_WHILE_LIVE=1   deploy even though raffles are live (step 3)
#   RAFI_SECRETS_FILE      optional: a JSON or .env file of the app Worker's secrets, passed to its
#                          `wrangler deploy --secrets-file` (added to the ones it has; none removed)
#   WRANGLER               the wrangler command (default: npx --yes wrangler@4.148.0)
#   RESTATE                the Restate CLI command (default: npx --yes @restatedev/restate@1.8.1)
#
# Steps (each prints what it does):
#   1. The release and the configurations. VERSION is a version (x.y.z, optionally -pre.release),
#      RAFI_VERSION's if that is set. wrangler.toml (the app) and engine.wrangler.toml (the
#      engine) are read with a small line parser: the keys below must be written as
#      `key = "value"` (or 'value', or true/false) on one line, in the tables the template has
#      them in ([[d1_databases]], [[durable_objects.bindings]], [assets], [vars]); [env.*] tables
#      are ignored, as `wrangler deploy` without --env ignores them. They must agree: main, the
#      assets and the migrations point into this release; both bind DB to one database_id; the
#      engine binds RAFFLE_ROOM (class RaffleRoom) with script_name = the app's name; both say the
#      same RAFI_DO_JURISDICTION; the engine is on workers.dev and has
#      RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS. The placeholders (the zero database_id, an identity
#      key that isn't publickeyv1_...) fail a deploy, and only warn for --dry-run and --check.
#      The engine Worker's name is the engine configuration's name and the version, dots as
#      dashes: rafi-engine-0-1-0.
#   2. legal/terms.md and legal/privacy.md into the app's static files (legal/).
#   3. Is anything live? The runbook's query (docs/operations/runbook.md, "Before a deploy") on
#      the database: a deploy stops, listing the raffles, unless RAFI_DEPLOY_WHILE_LIVE=1. A
#      database without the tables yet (the first deploy) has nothing live.
#   4. The D1 migrations (`wrangler d1 migrations apply DB --remote`).
#   5. The two Workers, the engine first, so the app never calls a handler the registered
#      engine lacks; on the first install the app first, since the engine binds its RaffleRoom
#      class (the app is "installed" once Cloudflare reports its script with a Durable Object
#      migration tag). The engine Worker <name>-<version> on the account's workers.dev
#      subdomain: if Restate already has https://<it>.<subdomain>.workers.dev/restate, it is
#      left as it is (a registered engine is never redeployed: Restate keeps invocations on
#      the deployment they started on); otherwise it is deployed (again, if an earlier run
#      stopped before registering it: nothing can be pinned to it then) and registered (POST
#      /deployments with "force": false, retried while the new Worker comes up). A failed
#      registration fails the deploy, before the app is deployed (but on the first install).
#   6. Pruning: every Restate deployment of this installation's engine Workers (an https URL
#      <engine name>-<version>.<subdomain>.workers.dev/restate) but the current one that
#      Restate reports Drained is removed from Restate (`restate deployments remove`, which
#      itself refuses one that isn't drained; never forced), and then its Worker is deleted.
#      Those still draining are listed with the invocations pinned to them; a paused one stays
#      until a human resumes it on the latest deployment or kills it. Engine Workers Restate
#      doesn't know are listed, not deleted. A failure here warns; the deploy has succeeded.
#
# Exit status: 0 when deployed (pruning may have warned), 1 on a failure or a refusal (a check,
# live raffles, a failed registration), 2 on a usage error. Remote state on a failure: whatever
# steps finished before it (migrations apply forward and are kept; a registered engine stays
# registered); running it again continues from there.

# The node programs below are single-quoted: their ${...} are JavaScript's.
# shellcheck disable=SC2016
set -euo pipefail

WRANGLER_VERSION=4.148.0
RESTATE_CLI_VERSION=1.8.1
# Registering a Worker deployed a moment ago: Restate's discovery may reach it before workers.dev
# serves it. Attempts, and the seconds between them.
REGISTER_ATTEMPTS=6
REGISTER_WAIT=10
CLOUDFLARE_API=https://api.cloudflare.com/client/v4

# The runbook's "is anything live?" (docs/operations/runbook.md).
LIVE_QUERY="SELECT id, title, state, event_date, owner_id FROM raffles
  WHERE state IN ('open', 'closed', 'drawing')
     OR (state IN ('ready', 'frozen') AND event_date >= date('now', '-1 day'))
  ORDER BY event_date"

usage() {
  sed -n '/^# Usage/,/^#   release\/deploy.sh --check/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

step() { printf '\n==> %s\n' "$*"; }
say() { printf '    %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

run_wrangler() {
  local cmd
  read -r -a cmd <<<"${WRANGLER:-npx --yes wrangler@$WRANGLER_VERSION}"
  WRANGLER_SEND_METRICS=false command "${cmd[@]}" "$@"
}

run_restate() {
  local cmd
  read -r -a cmd <<<"${RESTATE:-npx --yes @restatedev/restate@$RESTATE_CLI_VERSION}"
  SCARF_ANALYTICS=false command "${cmd[@]}" "$@"
}

# The release's version, from <release>/VERSION: x.y.z with an optional pre-release (a leading v
# dropped), and RAFI_VERSION's if that is set.
release_version() {
  local file="$1/VERSION" version
  [ -f "$file" ] || die "$file is missing: is $1 an unpacked Rafi release?"
  version="$(tr -d '[:space:]' <"$file")"
  version="${version#v}"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] \
    || die "$file says '$version', not a release version (x.y.z or x.y.z-pre.release)"
  if [ -n "${RAFI_VERSION:-}" ] && [ "${RAFI_VERSION#v}" != "$version" ]; then
    die "the release in $1 is $version, but RAFI_VERSION says ${RAFI_VERSION#v}"
  fi
  printf '%s\n' "$version"
}

# The engine Worker of a release: <engine name>-<version, dots as dashes, lower case>. A Worker
# name is also its workers.dev host label: [a-z0-9-], at most 63 characters.
engine_worker_name() {
  local base="$1" version="$2" name
  name="$base-$(printf '%s' "$version" | tr '.' '-' | tr '[:upper:]' '[:lower:]')"
  [[ "$name" =~ ^[a-z0-9][a-z0-9-]*[a-z0-9]$ ]] || die "the engine Worker's name '$name' isn't a valid Worker name"
  [ "${#name}" -le 63 ] || die "the engine Worker's name '$name' is longer than 63 characters: shorten the engine's name"
  printf '%s\n' "$name"
}

# Reads and checks the two configurations: prints key=value lines (app_name, app_assets,
# database_id, engine_base, jurisdiction), warnings on stderr. strict=1 refuses placeholders;
# 0 warns about them.
read_configs() {
  node - "$1" "$2" "$3" "$4" <<'EOF'
const fs = require("fs");
const path = require("path");
const [appFile, engineFile, release, strict] = process.argv.slice(2);
const errors = [];
const warnings = [];

// The few keys read here, as wrangler.toml has them: `key = "value"` on one line, in [table]
// and [[array.of.tables]] sections. Anything else (numbers, arrays, inline tables, dotted keys)
// is skipped; a key read here written in another form is an error rather than skipped.
const READ = new Set(["name", "main", "workers_dev", "directory", "binding", "database_id",
  "migrations_dir", "class_name", "script_name", "RAFI_DO_JURISDICTION",
  "RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS"]);
function parse(file) {
  if (!fs.existsSync(file)) { errors.push(`${file} is missing`); return null; }
  const doc = { top: {}, tables: {}, arrays: {} };
  let current = doc.top;
  fs.readFileSync(file, "utf8").split(/\r?\n/).forEach((raw, i) => {
    const line = raw.trim();
    const where = `${path.basename(file)}:${i + 1}`;
    let m;
    if (line === "" || line.startsWith("#")) return;
    if ((m = line.match(/^\[\[\s*([A-Za-z0-9_.-]+)\s*\]\]\s*(#.*)?$/))) {
      current = {};
      (doc.arrays[m[1]] ??= []).push(current);
    } else if ((m = line.match(/^\[\s*([A-Za-z0-9_.-]+)\s*\]\s*(#.*)?$/))) {
      current = doc.tables[m[1]] ??= {};
    } else if ((m = line.match(/^([A-Za-z0-9_-]+)\s*=\s*(.*)$/)) && READ.has(m[1])) {
      const [, key, rest] = m;
      let value;
      let v;
      if ((v = rest.match(/^"((?:[^"\\]|\\.)*)"\s*(#.*)?$/))) value = JSON.parse(`"${v[1]}"`);
      else if ((v = rest.match(/^'([^']*)'\s*(#.*)?$/))) value = v[1];
      else if ((v = rest.match(/^(true|false)\s*(#.*)?$/))) value = v[1] === "true";
      else { errors.push(`${where}: write ${key} as ${key} = "..." on one line`); return; }
      if (key in current) { errors.push(`${where}: ${key} is set twice`); return; }
      current[key] = value;
    }
  });
  return doc;
}

const app = parse(appFile);
const engine = parse(engineFile);
if (!app || !engine) { console.error(errors.join("\n")); process.exit(1); }
const appName = path.basename(appFile);
const engineName = path.basename(engineFile);

// A path in a configuration, as wrangler resolves it: relative to the configuration's directory.
function inRelease(file, value, key, expected, kind) {
  const want = path.join(release, expected);
  if (typeof value !== "string") { errors.push(`${path.basename(file)}: ${key} is missing (${want})`); return null; }
  const got = path.resolve(path.dirname(file), value);
  const real = fs.existsSync(got) ? fs.realpathSync(got) : got;
  if (real !== want) {
    errors.push(`${path.basename(file)}: ${key} is ${value}, not this release's ${expected} (${want})`);
    return null;
  }
  const stat = fs.existsSync(want) ? fs.statSync(want) : null;
  if (!stat || (kind === "dir" ? !stat.isDirectory() : !stat.isFile())) {
    errors.push(`the release has no ${expected} (${want})`);
    return null;
  }
  return want;
}
function one(doc, array, file, binding, key = "binding") {
  const found = (doc.arrays[array] ?? []).filter((t) => t[key] === binding);
  if (found.length !== 1) {
    errors.push(`${path.basename(file)}: ${found.length ? "more than one" : "no"} [[${array}]] with ${key} = "${binding}"`);
    return {};
  }
  return found[0];
}
const workerName = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;

// The app.
if (!workerName.test(app.top.name ?? "")) errors.push(`${appName}: name is missing or not a Worker name (a-z, 0-9, -)`);
inRelease(appFile, app.top.main, "main", "app/build/index.js", "file");
const assets = inRelease(appFile, app.tables.assets?.directory, "[assets] directory", "app/assets", "dir");
const appDb = one(app, "d1_databases", appFile, "DB");
inRelease(appFile, appDb.migrations_dir, "[[d1_databases]] migrations_dir", "migrations/sqlite", "dir");
const appRoom = one(app, "durable_objects.bindings", appFile, "RAFFLE_ROOM", "name");
if (appRoom.class_name !== "RaffleRoom") errors.push(`${appName}: RAFFLE_ROOM's class_name must be "RaffleRoom"`);
if ("script_name" in appRoom) errors.push(`${appName}: RAFFLE_ROOM is the app's own class: no script_name`);
const jurisdiction = app.tables.vars?.RAFI_DO_JURISDICTION;
if (typeof jurisdiction !== "string" || jurisdiction === "") errors.push(`${appName}: [vars] RAFI_DO_JURISDICTION is missing`);

// The engine, and that it agrees with the app.
if (!workerName.test(engine.top.name ?? "")) errors.push(`${engineName}: name is missing or not a Worker name (a-z, 0-9, -)`);
else if (engine.top.name === app.top.name) errors.push(`${engineName}: name must differ from the app's (${app.top.name})`);
inRelease(engineFile, engine.top.main, "main", "engine/build/index.js", "file");
if (engine.top.workers_dev !== true) errors.push(`${engineName}: workers_dev must be true (Restate calls the engine at its workers.dev address)`);
const engineDb = one(engine, "d1_databases", engineFile, "DB");
if (engineDb.database_id !== appDb.database_id) {
  errors.push(`${engineName}: DB's database_id (${engineDb.database_id}) isn't the app's (${appDb.database_id}): the engine writes the app's database`);
}
const engineRoom = one(engine, "durable_objects.bindings", engineFile, "RAFFLE_ROOM", "name");
if (engineRoom.class_name !== "RaffleRoom") errors.push(`${engineName}: RAFFLE_ROOM's class_name must be "RaffleRoom"`);
if (engineRoom.script_name !== app.top.name) {
  errors.push(`${engineName}: RAFFLE_ROOM's script_name (${engineRoom.script_name}) must be the app Worker's name (${app.top.name})`);
}
const engineVars = engine.tables.vars ?? {};
if (engineVars.RAFI_DO_JURISDICTION !== jurisdiction) {
  errors.push(`${engineName}: RAFI_DO_JURISDICTION (${engineVars.RAFI_DO_JURISDICTION}) must be the app's (${jurisdiction})`);
}
const keys = engineVars.RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS;
if (typeof keys !== "string") errors.push(`${engineName}: [vars] RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS is missing`);

// The placeholders an installation replaces.
const placeholders = [];
if (typeof appDb.database_id !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(appDb.database_id)) {
  errors.push(`${appName}: DB's database_id is missing or not a D1 database ID`);
} else if (appDb.database_id === "00000000-0000-0000-0000-000000000000") {
  placeholders.push(`${appName}, ${engineName}: DB's database_id is the placeholder (wrangler d1 create)`);
}
if (typeof keys === "string" && !keys.split(",").every((k) => /^publickeyv1_[A-Za-z0-9]+$/.test(k.trim()))) {
  placeholders.push(`${engineName}: RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS isn't a list of publickeyv1_... keys (the Restate environment's)`);
}
(strict === "1" ? errors : warnings).push(...placeholders);

for (const w of warnings) console.error(`warning: ${w}`);
if (errors.length) { console.error(errors.map((e) => `error: ${e}`).join("\n")); process.exit(1); }
const out = { app_name: app.top.name, app_assets: assets, database_id: appDb.database_id,
  engine_base: engine.top.name, jurisdiction };
for (const [k, v] of Object.entries(out)) {
  if (/[\n\r]/.test(v)) { console.error(`error: ${k} has a line break`); process.exit(1); }
  console.log(`${k}=${v}`);
}
EOF
}

# Sets APP_NAME, APP_ASSETS, DATABASE_ID, ENGINE_BASE and JURISDICTION from read_configs.
load_configs() {
  local out key value
  out="$(read_configs "$@")" || die "the configurations don't fit this release (above)"
  while IFS='=' read -r key value; do
    case "$key" in
      app_name) APP_NAME="$value" ;;
      app_assets) APP_ASSETS="$value" ;;
      database_id) DATABASE_ID="$value" ;;
      engine_base) ENGINE_BASE="$value" ;;
      jurisdiction) JURISDICTION="$value" ;;
    esac
  done <<<"$out"
}

copy_legal() {
  local from="$1" to="$2/legal" doc
  for doc in terms.md privacy.md; do
    [ -f "$from/$doc" ] || die "$from/$doc is missing: the installation's legal documents"
  done
  mkdir -p "$to"
  cp "$from/terms.md" "$from/privacy.md" "$to/"
  say "copied $from/terms.md and privacy.md into $to"
}

require_env() {
  local name missing=()
  for name in CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID RESTATE_ADMIN_URL RESTATE_AUTH_TOKEN; do
    [ -n "${!name:-}" ] || missing+=("$name")
  done
  [ "${#missing[@]}" -eq 0 ] || die "not set: ${missing[*]} (--dry-run needs none of them)"
  for name in CLOUDFLARE_API_TOKEN RESTATE_AUTH_TOKEN; do
    if [[ "${!name}" == *'"'* || "${!name}" == *"\\"* || "${!name}" =~ [[:space:]] ]]; then
      die "$name has a quote, a backslash or whitespace in it"
    fi
  done
  RESTATE_ADMIN_URL="${RESTATE_ADMIN_URL%/}"
  export RESTATE_ADMIN_URL
}

# http <method> <url> <token> [<json body>]: prints the response body, a line break and the HTTP
# status (000 when there was no response). The token goes to curl on stdin, not in its arguments.
http() {
  local method="$1" url="$2" token="$3" data="${4:-}" args
  args=(-sS -X "$method" -K - -w '\n%{http_code}' -H 'accept: application/json' --max-time 60)
  [ -z "$data" ] || args+=(-H 'content-type: application/json' --data-binary "$data")
  printf 'header = "Authorization: Bearer %s"\n' "$token" | curl "${args[@]}" "$url" || printf '\n000'
}
cf_api() { http "$1" "$CLOUDFLARE_API$2" "$CLOUDFLARE_API_TOKEN" "${3:-}"; }
restate_api() { http "$1" "$RESTATE_ADMIN_URL$2" "$RESTATE_AUTH_TOKEN" "${3:-}"; }
status_of() { printf '%s\n' "${1##*$'\n'}"; }
body_of() { printf '%s\n' "${1%$'\n'*}"; }

# Cloudflare's errors in a response body, for a message.
cf_errors() {
  node -e '
    let d; try { d = JSON.parse(require("fs").readFileSync(0, "utf8")); } catch { process.exit(0); }
    process.stdout.write((d.errors ?? []).map((e) => `${e.code}: ${e.message}`).join("; "));'
}

# Prints the account's workers.dev subdomain.
cf_subdomain() {
  local resp sub
  resp="$(cf_api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/subdomain")"
  if [ "$(status_of "$resp")" = 200 ]; then
    sub="$(body_of "$resp" | node -e '
      const d = JSON.parse(require("fs").readFileSync(0, "utf8"));
      process.stdout.write(d.result?.subdomain ?? "");')"
    if [[ "$sub" =~ ^[a-z0-9][a-z0-9-]*$ ]]; then
      printf '%s\n' "$sub"
      return 0
    fi
  fi
  die "the account's workers.dev subdomain: HTTP $(status_of "$resp") $(body_of "$resp" | cf_errors). An account without one registers it at https://dash.cloudflare.com/$CLOUDFLARE_ACCOUNT_ID/workers/onboarding"
}

# Prints missing, present (a script, perhaps only secrets put before its first deploy) or
# installed (Cloudflare has applied its Durable Object migrations: its classes exist).
cf_worker_state() {
  local resp
  resp="$(cf_api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/services/$1")"
  case "$(status_of "$resp")" in
    200)
      body_of "$resp" | node -e '
        const d = JSON.parse(require("fs").readFileSync(0, "utf8"));
        console.log(d.result?.default_environment?.script?.migration_tag ? "installed" : "present");'
      ;;
    404)
      # 10007: no such Worker (wrangler's own test); anything else is a real failure.
      if body_of "$resp" | node -e '
        const d = JSON.parse(require("fs").readFileSync(0, "utf8"));
        process.exit((d.errors ?? []).some((e) => e.code === 10007 || e.code === 10090) ? 0 : 1);'; then
        echo missing
      else
        die "looking up the Worker $1: HTTP 404 $(body_of "$resp" | cf_errors)"
      fi
      ;;
    *) die "looking up the Worker $1: HTTP $(status_of "$resp") $(body_of "$resp" | cf_errors)" ;;
  esac
}

# Is anything live? Runs the runbook's query on the app's database (the location arguments:
# --remote, or --local --persist-to <dir> in the tests) and prints the live raffles, one per
# line. Fails when the query fails, but for a database without the tables yet (nothing live).
live_raffles() {
  local config="$1" out err
  shift
  out="$(mktemp)"
  err="$(mktemp)"
  if run_wrangler d1 execute DB -c "$config" "$@" --json --command "$LIVE_QUERY" >"$out" 2>"$err"; then
    node -e '
      const d = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
      for (const r of d[0]?.results ?? []) console.log(`${r.id}  ${r.state}  ${r.event_date}  ${JSON.stringify(r.title)}  owner ${r.owner_id}`);' "$out" \
      || { rm -f "$out" "$err"; die "couldn't read the live raffles from wrangler's output"; }
  elif grep -qi 'no such table: raffles' "$out" "$err"; then
    :
  else
    cat "$out" "$err" >&2
    rm -f "$out" "$err"
    die "couldn't ask the database whether anything is live (above)"
  fi
  rm -f "$out" "$err"
}

preflight() {
  local live
  live="$(live_raffles "$1" "${@:2}")" || exit 1
  if [ -z "$live" ]; then
    say "nothing live"
    return 0
  fi
  printf '%s\n' "$live" | sed 's/^/    live: /'
  if [ "${RAFI_DEPLOY_WHILE_LIVE:-}" = 1 ]; then
    warn "raffles are live (above); deploying anyway (RAFI_DEPLOY_WHILE_LIVE=1)"
  else
    die "raffles are live (above): deploy after them, or set RAFI_DEPLOY_WHILE_LIVE=1 (docs/operations/upgrades.md, the deploy freeze)"
  fi
}

# Prints the ID of the Restate deployment at <url>, if there is one.
restate_deployment_at() {
  local resp
  resp="$(restate_api GET /deployments)"
  [ "$(status_of "$resp")" = 200 ] || die "listing Restate's deployments: HTTP $(status_of "$resp") $(body_of "$resp")"
  body_of "$resp" | node -e '
    const d = JSON.parse(require("fs").readFileSync(0, "utf8"));
    const norm = (u) => String(u ?? "").replace(/\/+$/, "");
    const found = (d.deployments ?? []).find((x) => norm(x.uri) === norm(process.argv[1]));
    if (found) console.log(found.id);' "$1"
}

# Registers <url> with Restate (never forced), retrying while a fresh Worker comes up; sets
# REGISTERED_ID.
restate_register() {
  local url="$1" version="$2" body resp status attempt
  body="$(node -e 'process.stdout.write(JSON.stringify({ uri: process.argv[1], force: false, metadata: { rafi_version: process.argv[2] } }))' "$url" "$version")"
  for ((attempt = 1; attempt <= REGISTER_ATTEMPTS; attempt++)); do
    resp="$(restate_api POST /deployments "$body")"
    status="$(status_of "$resp")"
    case "$status" in
      200 | 201)
        REGISTERED_ID="$(body_of "$resp" | node -e 'process.stdout.write(JSON.parse(require("fs").readFileSync(0, "utf8")).id ?? "")')"
        [ -n "$REGISTERED_ID" ] || die "Restate registered $url but answered no deployment ID: $(body_of "$resp")"
        if [ "$status" = 201 ]; then say "registered with Restate: $REGISTERED_ID"; else say "Restate already had it: $REGISTERED_ID"; fi
        return 0
        ;;
      4??) die "registering $url with Restate: HTTP $status $(body_of "$resp")" ;;
    esac
    warn "registering $url with Restate (attempt $attempt of $REGISTER_ATTEMPTS): HTTP $status $(body_of "$resp")"
    [ "$attempt" -eq "$REGISTER_ATTEMPTS" ] || sleep "$REGISTER_WAIT"
  done
  die "Restate couldn't register $url: the engine Worker is deployed but not registered. Running this again retries."
}

# The engine Worker of this release: deployed unless Restate already has it, then registered.
deploy_engine() {
  local url="$1" registered state
  step "engine Worker $ENGINE_NAME: $url"
  registered="$(restate_deployment_at "$url")" || exit 1
  state="$(cf_worker_state "$ENGINE_NAME")" || exit 1
  if [ -n "$registered" ] && [ "$state" != missing ]; then
    say "deployed and registered with Restate ($registered): left as it is (an engine Worker is never redeployed once registered)"
    ENGINE_DEPLOYMENT_ID="$registered"
    return 0
  fi
  if [ -n "$registered" ]; then
    warn "Restate has $url ($registered) but the Worker $ENGINE_NAME is gone: deploying this release's engine there again"
  elif [ "$state" != missing ]; then
    say "the Worker exists but Restate doesn't know it (a deploy stopped before registering it): deploying it again"
  fi
  run_wrangler deploy -c "$ENGINE_CONFIG" --name "$ENGINE_NAME" --message "Rafi $VERSION"
  restate_register "$url" "$VERSION"
  ENGINE_DEPLOYMENT_ID="$REGISTERED_ID"
}

deploy_app() {
  local args=(deploy -c "$APP_CONFIG" --message "Rafi $VERSION")
  step "app Worker $APP_NAME"
  if [ -n "${RAFI_SECRETS_FILE:-}" ]; then
    [ -f "$RAFI_SECRETS_FILE" ] || die "RAFI_SECRETS_FILE: $RAFI_SECRETS_FILE is not a file"
    args+=(--secrets-file "$RAFI_SECRETS_FILE")
    say "with the secrets in \$RAFI_SECRETS_FILE"
  fi
  run_wrangler "${args[@]}"
}

# What to do with Restate's deployments: reads `restate deployments list --extra --json` on stdin
# and prints, for each deployment of one of this installation's engine Workers (https://<engine
# base>-<dashed version>.<subdomain>.workers.dev/restate) other than <current url>, one line:
# <status> <deployment id> <worker name> <active invocations>, tab separated.
prune_plan() {
  node -e '
    const [base, subdomain, current] = process.argv.slice(1);
    const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const engine = new RegExp(`^https://(${esc(base)}-[0-9]+-[0-9]+-[0-9]+(?:-[a-z0-9-]+)?)\\.${esc(subdomain)}\\.workers\\.dev/restate/?$`, "i");
    const norm = (u) => String(u ?? "").replace(/\/+$/, "").toLowerCase();
    const list = JSON.parse(require("fs").readFileSync(0, "utf8"));
    if (!Array.isArray(list.deployments)) throw new Error("no deployments in the list");
    for (const d of list.deployments) {
      const m = String(d.endpoint ?? "").match(engine);
      if (!m || norm(d.endpoint) === norm(current)) continue;
      if (!/^dp_[A-Za-z0-9]+$/.test(d.deployment_id ?? "")) throw new Error(`a deployment ID: ${d.deployment_id}`);
      console.log([d.status ?? "Unknown", d.deployment_id, m[1].toLowerCase(), d.active_invocations ?? "?"].join("\t"));
    }' "$1" "$2" "$3"
}

# The invocations pinned to a deployment and not completed, one per line: <id> <status>.
pinned_invocations() {
  local resp query
  query="SELECT id, status FROM sys_invocation WHERE pinned_deployment_id = '$1' AND status <> 'completed' ORDER BY id"
  resp="$(restate_api POST /query "$(node -e 'process.stdout.write(JSON.stringify({ query: process.argv[1] }))' "$query")")"
  [ "$(status_of "$resp")" = 200 ] || return 1
  body_of "$resp" | node -e '
    for (const r of JSON.parse(require("fs").readFileSync(0, "utf8")).rows ?? []) console.log(`${r.id} ${r.status}`);'
}

# Removes drained engine deployments from Restate, then their Workers; reports the rest. Warns,
# never fails: every step is checked here (errexit is off in a function called with ||).
prune() {
  local current="$1" list plan status id worker active pinned scripts known=""
  step "pruning drained engine Workers"
  if ! list="$(run_restate deployments list --extra --json)"; then
    warn "couldn't list Restate's deployments: nothing pruned"
    return 0
  fi
  if ! plan="$(printf '%s\n' "$list" | prune_plan "$ENGINE_BASE" "$SUBDOMAIN" "$current")"; then
    warn "couldn't read Restate's deployments: nothing pruned"
    return 0
  fi
  [ -n "$plan" ] || say "no older engine deployments"
  while IFS=$'\t' read -r status id worker active; do
    [ -n "$id" ] || continue
    known="$known $worker"
    case "$status" in
      Drained)
        say "$id ($worker) is drained: removing it from Restate, then deleting the Worker"
        if ! run_restate deployments remove "$id" --yes; then
          warn "Restate didn't remove $id ($worker): its Worker stays"
          continue
        fi
        if ! run_wrangler delete "$worker" --force -c "$ENGINE_CONFIG"; then
          warn "$id is removed from Restate, but deleting the Worker $worker failed: delete it by hand (wrangler delete $worker)"
        fi
        ;;
      Draining)
        say "$id ($worker) is draining: $active invocation(s) still pinned to it"
        if pinned="$(pinned_invocations "$id")"; then
          printf '%s\n' "$pinned" | sed '/^$/d; s/^/      /'
          if grep -q ' paused$' <<<"$pinned"; then
            say "  a paused invocation waits for a human: restate invocations resume <id> --deployment latest, or restate invocations kill <id>"
          fi
        fi
        say "  it is pruned by a later deploy once drained"
        ;;
      *) say "$id ($worker) is $status (the latest for some service): left as it is" ;;
    esac
  done <<<"$plan"
  # Engine Workers Restate doesn't know (a deploy that stopped before registering one, or one
  # removed from Restate by hand): listed, not deleted.
  scripts="$(cf_api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/scripts")"
  if [ "$(status_of "$scripts")" != 200 ]; then
    warn "couldn't list the account's Workers: HTTP $(status_of "$scripts")"
    return 0
  fi
  body_of "$scripts" | node -e '
    const [base, current, known] = process.argv.slice(1);
    const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const engine = new RegExp(`^${esc(base)}-[0-9]+-[0-9]+-[0-9]+(?:-[a-z0-9-]+)?$`);
    const skip = new Set([current, ...known.split(" ")]);
    for (const s of JSON.parse(require("fs").readFileSync(0, "utf8")).result ?? []) {
      if (engine.test(s.id) && !skip.has(s.id)) {
        console.error(`warning: the Worker ${s.id} looks like an engine Worker Restate has no deployment for: delete it by hand if it is (wrangler delete ${s.id})`);
      }
    }' "$ENGINE_BASE" "$ENGINE_NAME" "$known" || warn "couldn't read the account's Workers"
}

dry_run() {
  step "app Worker $APP_NAME: wrangler deploy --dry-run"
  run_wrangler deploy --dry-run -c "$APP_CONFIG"
  step "engine Worker $ENGINE_NAME: wrangler deploy --dry-run"
  run_wrangler deploy --dry-run -c "$ENGINE_CONFIG" --name "$ENGINE_NAME"
}

main() {
  local mode=deploy strict=1 app_state url registered
  case "${1:-}" in
    "") ;;
    --dry-run) mode=dry-run strict=0 ;;
    --check) mode=check strict=0 ;;
    -h | --help)
      usage
      return 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  [ "$#" -le 1 ] || {
    usage >&2
    exit 2
  }

  RELEASE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
  APP_CONFIG="$PWD/wrangler.toml"
  ENGINE_CONFIG="$PWD/engine.wrangler.toml"

  step "release $RELEASE ($mode)"
  VERSION="$(release_version "$RELEASE")" || exit 1
  load_configs "$APP_CONFIG" "$ENGINE_CONFIG" "$RELEASE" "$strict"
  ENGINE_NAME="$(engine_worker_name "$ENGINE_BASE" "$VERSION")" || exit 1
  say "Rafi $VERSION"
  say "app Worker: $APP_NAME ($APP_CONFIG)"
  say "engine Worker: $ENGINE_NAME ($ENGINE_CONFIG, named $ENGINE_BASE there)"
  say "database: $DATABASE_ID; Durable Objects' jurisdiction: $JURISDICTION"
  [ "$mode" != check ] || return 0

  step "the legal documents"
  copy_legal "$PWD/legal" "$APP_ASSETS"

  if [ "$mode" = dry-run ]; then
    dry_run
    step "dry run of Rafi $VERSION done: nothing deployed"
    return 0
  fi

  step "Cloudflare and Restate"
  require_env
  SUBDOMAIN="$(cf_subdomain)" || exit 1
  url="https://$ENGINE_NAME.$SUBDOMAIN.workers.dev/restate"
  say "workers.dev subdomain: $SUBDOMAIN"
  # Restate's admin API answers (before anything changes, not after the app went out).
  registered="$(restate_deployment_at "$url")" || exit 1
  say "Restate admin API: $RESTATE_ADMIN_URL (this release's engine: ${registered:-not registered yet})"

  step "is anything live?"
  preflight "$APP_CONFIG" --remote

  step "D1 migrations"
  run_wrangler d1 migrations apply DB -c "$APP_CONFIG" --remote

  app_state="$(cf_worker_state "$APP_NAME")" || exit 1
  if [ "$app_state" = installed ]; then
    deploy_engine "$url"
    deploy_app
  else
    say "the app Worker $APP_NAME isn't installed yet ($app_state): the app first, its RaffleRoom class for the engine to bind"
    deploy_app
    deploy_engine "$url"
  fi

  prune "$url" || warn "pruning failed; the deploy itself succeeded"

  step "deployed Rafi $VERSION"
  say "app Worker $APP_NAME; engine Worker $ENGINE_NAME at $url (Restate deployment $ENGINE_DEPLOYMENT_ID)"
}

# Sourced (scripts/cloudflare-deploy-test.sh), it only defines its functions.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  main "$@"
fi
