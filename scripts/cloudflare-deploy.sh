#!/usr/bin/env bash
# Deploys a release of Rafi to Cloudflare, one step per subcommand: the rooms Worker (the
# raffles' RaffleRoom Durable Objects), the app Worker and the release's own raffle engine
# Worker, registered with Restate; and cleans up the engine Workers Restate has drained. Each
# release ships this file as its deploy.sh (the source is scripts/cloudflare-deploy.sh in the
# Rafi repository), so the deploy logic is the release's own, pinned to it and covered by its
# provenance attestation. An installation's workflows (deploy/cloudflare/.github/workflows:
# deploy.yaml and restate-cleanup.yaml) download and verify the release, unpack it into
# release/ and run these subcommands, each job on a fresh runner.
#
# Usage, from the root of an installation's repository (a copy of deploy/cloudflare: VERSION,
# wrangler.toml, engine.wrangler.toml, rooms.wrangler.toml, legal/), with the release unpacked
# into release/ (the directory this script is in is the release; the configurations point into
# it as release/...):
#
#   release/deploy.sh check             the release, VERSION, DEPLOY_VERSION and the configurations; nothing remote
#   release/deploy.sh migrate           the D1 migrations
#   release/deploy.sh deploy-rooms      the rooms Worker, unless its code and configuration are unchanged
#   release/deploy.sh upload-app        the app Worker's version uploaded (deployed directly on a first install)
#   release/deploy.sh deploy-engine     this release's engine Worker, <engine name>-v<version>
#   release/deploy.sh register-engine   the engine Worker registered with Restate (never a downgrade)
#   release/deploy.sh promote-app       the uploaded app version deployed, then its routes and Cron
#   release/deploy.sh cleanup           drained engine Workers deleted, then their Restate deployments
#   release/deploy.sh dry-run           check, legal/ copied in, `wrangler deploy --dry-run` of the three Workers
#
# Environment:
#   RAFI_DEPLOY_VERSION   required: the deploy interface the installation's workflows are written
#                         for; it must be this release's DEPLOY_VERSION (below)
#   RAFI_VERSION          optional: the version the release must be
#   RAFI_INSTANCE         register-engine, cleanup: the installation's name in Restate's deployment
#                         metadata (the workflows: the repository, owner/name)
#   CLOUDFLARE_API_TOKEN, CLOUDFLARE_ACCOUNT_ID   the account; all but check and dry-run
#                         (register-engine needs only the account ID, for the metadata)
#   RESTATE_ADMIN_URL     register-engine, cleanup: the Restate environment's admin API
#                         (https://<env>.env.<region>.restate.cloud:9070)
#   RESTATE_AUTH_TOKEN    register-engine, cleanup: a Restate API key with the Admin role
#   RAFI_APP_PROMOTED, RAFI_APP_VERSION_ID   promote-app: upload-app's outputs
#   RAFI_ENGINE_WORKER, RAFI_ENGINE_URL      register-engine: deploy-engine's outputs
#   GITHUB_OUTPUT         when set, a subcommand's outputs are appended to it as key=value lines
#                         (GitHub Actions' step outputs); otherwise they are printed as such
#   WRANGLER              the wrangler command (default: npx --yes wrangler@4.148.0)
#   RESTATE               the Restate CLI command (default: npx --yes @restatedev/restate@1.8.1)
#   CLOUDFLARE_API_BASE_URL   the Cloudflare API (wrangler's variable too; for tests)
#   RAFI_REGISTER_WAIT    seconds between registration attempts (default 10; for tests)
#
# What each does (each prints its steps):
#   Every subcommand first checks, as `check` does: the release's VERSION is a version
#   (x.y.z, optionally -pre.release) and the installation's VERSION file's (and RAFI_VERSION's,
#   when set); the release's DEPLOY_VERSION is this script's, and RAFI_DEPLOY_VERSION is too
#   (otherwise the installation's workflows are another release's: copy them from the template
#   at this release's tag); and the three configurations fit the release and each other.
#   wrangler.toml (the app), engine.wrangler.toml (the engine) and rooms.wrangler.toml (the
#   rooms) are read with a small line parser: the keys below must be written as `key = "value"`
#   (or 'value', or true/false) on one line, in the tables the template has them in
#   ([[d1_databases]], [[durable_objects.bindings]], [assets], [vars]); [env.*] tables are
#   ignored, as `wrangler deploy` without --env ignores them. They must agree: main, the assets
#   and the migrations point into this release; all three bind DB to one database_id; the rooms
#   Worker defines RAFFLE_ROOM (class RaffleRoom, no script_name) and has no address
#   (workers_dev and preview_urls false), and the app and the engine bind it with script_name =
#   the rooms Worker's name; all three say the same RAFI_DO_JURISDICTION; the engine is on
#   workers.dev and has RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS. The placeholders (the zero
#   database_id, an identity key that isn't publickeyv1_...) are refused, and only warned about
#   by dry-run.
#   migrate: `wrangler d1 migrations apply DB --remote` (the app's configuration).
#   deploy-rooms: the rooms Worker's fingerprint is the sha256 of its bundle's files
#     (release/rooms/build) and rooms.wrangler.toml; its tag, rooms-<32 hex digits of it>, is
#     the version annotation (workers/tag) of each rooms Worker version this deploys. When the
#     Worker's current deployment is one version at 100% with that tag (Cloudflare's API: the
#     script's latest deployment, then that version's annotations), nothing is deployed: the
#     live channels' connections are kept. Otherwise `wrangler deploy` (the class's Durable
#     Object migrations need it).
#   upload-app: legal/terms.md and legal/privacy.md copied into the app's static files; then,
#     if the app Worker has ever run a release's code (a version tagged v<version>, in its
#     current deployment or among its recent versions), `wrangler versions upload --tag
#     v<version>`, the new version not serving yet: output app_version_id (from wrangler's
#     ND-JSON output). Otherwise, a first install (the Worker holds only the secrets `wrangler
#     secret put`/`bulk` created it with): `wrangler deploy`, output app_promoted=true. A
#     Worker that doesn't exist is refused: its secrets come first.
#   deploy-engine: `wrangler deploy` of the engine as <engine name>-v<version, dots as dashes>
#     (rafi-engine-v0-1-0), every time (the same release is the same code, so a re-run changes
#     nothing Restate relies on); outputs engine_worker and engine_url
#     (https://<it>.<the account's workers.dev subdomain>.workers.dev/restate).
#   register-engine: refuses a downgrade: the deployment Restate routes new Raffle and
#     EngineInfo invocations to (GET /services), if its metadata says it is this installation's
#     (app rafi, RAFI_INSTANCE, CLOUDFLARE_ACCOUNT_ID; GET /deployments), must not be of a newer
#     release than this one (semantic versioning, pre-releases before their release); the same
#     release is a re-run. Then POST /deployments of engine_url, "force": false, with the
#     metadata app, instance, version, cloudflare_account and worker; 200 (Restate had it) and
#     201 succeed, a 5xx or no answer is retried (a Worker deployed a moment ago), anything else
#     fails.
#   promote-app: after a first install, nothing; otherwise `wrangler versions deploy
#     <app_version_id>@100%`, then `wrangler triggers deploy` (the routes and the Cron, which a
#     version upload doesn't apply).
#   cleanup: Restate's deployments with their metadata (GET /deployments) and their status
#     (`restate deployments list --extra --json`), joined by ID; only this installation's
#     (the metadata above, its worker <engine name>-v..., its address that Worker's workers.dev
#     one) are touched. Each Drained one: its Worker deleted (DELETE
#     /accounts/{id}/workers/scripts/<worker>; 404 is already gone), then the deployment removed
#     (`restate deployments remove`, which refuses one not drained; never forced). Either
#     listing failing stops it before anything is deleted. Reported: those still draining, with
#     their pinned invocations (a paused one waits for a human: resume it on the latest
#     deployment or kill it), and every deployment that isn't this installation's (never
#     touched).
#   dry-run: check (placeholders warned about), legal/ copied in, and `wrangler deploy
#     --dry-run` of the rooms Worker, the engine Worker under its versioned name and the app
#     Worker; no credentials.
#
# Exit status: 0 on success, 1 on a failure or a refusal, 2 on a usage error. Every subcommand
# can be run again: a failed one is retried by running it again.

# The node programs below are single-quoted: their ${...} are JavaScript's.
# shellcheck disable=SC2016
set -euo pipefail

# The interface between an installation's workflows and this script (its subcommands, their
# environment and outputs, what it expects of the installation's repository): the release ships
# it as DEPLOY_VERSION (`just cloudflare-artifact` reads it from here), the workflows say theirs
# as RAFI_DEPLOY_VERSION, and it changes when they would no longer fit.
DEPLOY_VERSION=1
WRANGLER_VERSION=4.148.0
RESTATE_CLI_VERSION=1.8.1
# Registering a Worker deployed a moment ago: Restate's discovery may reach it before workers.dev
# serves it.
REGISTER_ATTEMPTS=5
# The services this release's engine exposes, whose latest deployment the downgrade guard reads.
ENGINE_SERVICES="Raffle EngineInfo"

usage() {
  sed -n '/^# Usage/,/^#   release\/deploy.sh dry-run/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
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

# A subcommand's output: a key=value line in GITHUB_OUTPUT when that is set, on stdout otherwise.
output() {
  local key="$1" value="$2"
  [[ "$value" != *$'\n'* ]] || die "the output $key has a line break"
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    printf '%s=%s\n' "$key" "$value" >>"$GITHUB_OUTPUT"
    say "output: $key=$value"
  else
    printf '%s=%s\n' "$key" "$value"
  fi
}

valid_version() { [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; }

# The version in a file (read_version <file> <what it is>): x.y.z with an optional pre-release,
# a leading v dropped.
read_version() {
  local file="$1" what="$2" version
  [ -f "$file" ] || die "$file is missing: $what"
  version="$(tr -d '[:space:]' <"$file")"
  version="${version#v}"
  valid_version "$version" || die "$file says '$version', not a Rafi version (x.y.z or x.y.z-pre.release)"
  printf '%s\n' "$version"
}

# The engine Worker of a release: <engine name>-v<version, dots as dashes, lower case>. A Worker
# name is also its workers.dev host label: [a-z0-9-], at most 63 characters.
engine_worker_name() {
  local base="$1" version="$2" name
  name="$base-v$(printf '%s' "$version" | tr '.' '-' | tr '[:upper:]' '[:lower:]')"
  [[ "$name" =~ ^[a-z0-9][a-z0-9-]*[a-z0-9]$ ]] || die "the engine Worker's name '$name' isn't a valid Worker name"
  [ "${#name}" -le 63 ] || die "the engine Worker's name '$name' is longer than 63 characters: shorten the engine's name"
  printf '%s\n' "$name"
}

# Reads and checks the three configurations (read_configs <app> <engine> <rooms> <release>
# <strict>): prints key=value lines (app_name, app_assets, database_id, engine_base, rooms_name,
# jurisdiction), warnings on stderr. strict=1 refuses placeholders; 0 warns about them.
read_configs() {
  node - "$1" "$2" "$3" "$4" "$5" <<'EOF'
const fs = require("fs");
const path = require("path");
const [appFile, engineFile, roomsFile, release, strict] = process.argv.slice(2);
const errors = [];
const warnings = [];

// The few keys read here, as wrangler.toml has them: `key = "value"` on one line, in [table]
// and [[array.of.tables]] sections. Anything else (numbers, arrays, inline tables, dotted keys)
// is skipped; a key read here written in another form is an error rather than skipped.
const READ = new Set(["name", "main", "workers_dev", "preview_urls", "directory", "binding", "database_id",
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
const rooms = parse(roomsFile);
if (!app || !engine || !rooms) { console.error(errors.join("\n")); process.exit(1); }
const appName = path.basename(appFile);
const engineName = path.basename(engineFile);
const roomsName = path.basename(roomsFile);

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

// The rooms Worker: the RaffleRoom class's, reached only through the other two's bindings.
if (!workerName.test(rooms.top.name ?? "")) errors.push(`${roomsName}: name is missing or not a Worker name (a-z, 0-9, -)`);
inRelease(roomsFile, rooms.top.main, "main", "rooms/build/index.js", "file");
if (rooms.top.workers_dev !== false) errors.push(`${roomsName}: workers_dev must be false (the rooms Worker has no address)`);
if (rooms.top.preview_urls !== false) errors.push(`${roomsName}: preview_urls must be false (the rooms Worker has no address)`);
const roomsDb = one(rooms, "d1_databases", roomsFile, "DB");
const roomsRoom = one(rooms, "durable_objects.bindings", roomsFile, "RAFFLE_ROOM", "name");
if (roomsRoom.class_name !== "RaffleRoom") errors.push(`${roomsName}: RAFFLE_ROOM's class_name must be "RaffleRoom"`);
if ("script_name" in roomsRoom) errors.push(`${roomsName}: RAFFLE_ROOM is the rooms Worker's own class: no script_name`);
const jurisdiction = rooms.tables.vars?.RAFI_DO_JURISDICTION;
if (typeof jurisdiction !== "string" || jurisdiction === "") errors.push(`${roomsName}: [vars] RAFI_DO_JURISDICTION is missing`);

// A Worker binding the rooms Worker's class, by its name.
function bindsRooms(doc, file) {
  const room = one(doc, "durable_objects.bindings", file, "RAFFLE_ROOM", "name");
  if (room.class_name !== "RaffleRoom") errors.push(`${path.basename(file)}: RAFFLE_ROOM's class_name must be "RaffleRoom"`);
  if (room.script_name !== rooms.top.name) {
    errors.push(`${path.basename(file)}: RAFFLE_ROOM's script_name (${room.script_name}) must be the rooms Worker's name (${rooms.top.name})`);
  }
  const vars = doc.tables.vars ?? {};
  if (vars.RAFI_DO_JURISDICTION !== jurisdiction) {
    errors.push(`${path.basename(file)}: RAFI_DO_JURISDICTION (${vars.RAFI_DO_JURISDICTION}) must be the rooms Worker's (${jurisdiction})`);
  }
}

// The app.
if (!workerName.test(app.top.name ?? "")) errors.push(`${appName}: name is missing or not a Worker name (a-z, 0-9, -)`);
else if (app.top.name === rooms.top.name) errors.push(`${appName}: name must differ from the rooms Worker's (${rooms.top.name})`);
inRelease(appFile, app.top.main, "main", "app/build/index.js", "file");
const assets = inRelease(appFile, app.tables.assets?.directory, "[assets] directory", "app/assets", "dir");
const appDb = one(app, "d1_databases", appFile, "DB");
inRelease(appFile, appDb.migrations_dir, "[[d1_databases]] migrations_dir", "migrations", "dir");
bindsRooms(app, appFile);
if (roomsDb.database_id !== appDb.database_id) {
  errors.push(`${roomsName}: DB's database_id (${roomsDb.database_id}) isn't the app's (${appDb.database_id}): the rooms read the app's database`);
}

// The engine, and that it agrees with the app and the rooms.
if (!workerName.test(engine.top.name ?? "")) errors.push(`${engineName}: name is missing or not a Worker name (a-z, 0-9, -)`);
else if (engine.top.name === app.top.name) errors.push(`${engineName}: name must differ from the app's (${app.top.name})`);
else if (engine.top.name === rooms.top.name) errors.push(`${engineName}: name must differ from the rooms Worker's (${rooms.top.name})`);
inRelease(engineFile, engine.top.main, "main", "engine/build/index.js", "file");
if (engine.top.workers_dev !== true) errors.push(`${engineName}: workers_dev must be true (Restate calls the engine at its workers.dev address)`);
const engineDb = one(engine, "d1_databases", engineFile, "DB");
if (engineDb.database_id !== appDb.database_id) {
  errors.push(`${engineName}: DB's database_id (${engineDb.database_id}) isn't the app's (${appDb.database_id}): the engine writes the app's database`);
}
bindsRooms(engine, engineFile);
const keys = (engine.tables.vars ?? {}).RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS;
if (typeof keys !== "string") errors.push(`${engineName}: [vars] RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS is missing`);

// The placeholders an installation replaces.
const placeholders = [];
if (typeof appDb.database_id !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(appDb.database_id)) {
  errors.push(`${appName}: DB's database_id is missing or not a D1 database ID`);
} else if (appDb.database_id === "00000000-0000-0000-0000-000000000000") {
  placeholders.push(`${appName}, ${engineName}, ${roomsName}: DB's database_id is the placeholder (wrangler d1 create)`);
}
if (typeof keys === "string" && !keys.split(",").every((k) => /^publickeyv1_[A-Za-z0-9]+$/.test(k.trim()))) {
  placeholders.push(`${engineName}: RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS isn't a list of publickeyv1_... keys (the Restate environment's)`);
}
(strict === "1" ? errors : warnings).push(...placeholders);

for (const w of warnings) console.error(`warning: ${w}`);
if (errors.length) { console.error(errors.map((e) => `error: ${e}`).join("\n")); process.exit(1); }
const out = { app_name: app.top.name, app_assets: assets, database_id: appDb.database_id,
  engine_base: engine.top.name, rooms_name: rooms.top.name, jurisdiction };
for (const [k, v] of Object.entries(out)) {
  if (/[\n\r]/.test(v)) { console.error(`error: ${k} has a line break`); process.exit(1); }
  console.log(`${k}=${v}`);
}
EOF
}

# Sets APP_NAME, APP_ASSETS, DATABASE_ID, ENGINE_BASE, ROOMS_NAME and JURISDICTION from
# read_configs.
load_configs() {
  local out key value
  out="$(read_configs "$@")" || die "the configurations don't fit this release (above)"
  while IFS='=' read -r key value; do
    case "$key" in
      app_name) APP_NAME="$value" ;;
      app_assets) APP_ASSETS="$value" ;;
      database_id) DATABASE_ID="$value" ;;
      engine_base) ENGINE_BASE="$value" ;;
      rooms_name) ROOMS_NAME="$value" ;;
      jurisdiction) JURISDICTION="$value" ;;
    esac
  done <<<"$out"
}

# What every subcommand checks first (check_release <strict>): the release, the installation's
# VERSION, the deploy interface and the configurations. Sets RELEASE, VERSION, the configurations'
# paths, load_configs' variables and ENGINE_WORKER (this release's engine Worker).
check_release() {
  local strict="$1" shipped wanted where
  RELEASE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
  APP_CONFIG="$PWD/wrangler.toml"
  ENGINE_CONFIG="$PWD/engine.wrangler.toml"
  ROOMS_CONFIG="$PWD/rooms.wrangler.toml"
  step "release $RELEASE"
  VERSION="$(read_version "$RELEASE/VERSION" "is $RELEASE an unpacked Rafi release?")" || exit 1

  # The deploy interface: the release's own (packed from this script's), and the workflows'.
  [ -f "$RELEASE/DEPLOY_VERSION" ] || die "$RELEASE/DEPLOY_VERSION is missing: is $RELEASE an unpacked Rafi release?"
  shipped="$(tr -d '[:space:]' <"$RELEASE/DEPLOY_VERSION")"
  [ "$shipped" = "$DEPLOY_VERSION" ] \
    || die "$RELEASE/DEPLOY_VERSION says '$shipped', but this deploy.sh is interface version $DEPLOY_VERSION: the release is broken"
  wanted="${RAFI_DEPLOY_VERSION:-}"
  if [ "$wanted" != "$DEPLOY_VERSION" ]; then
    where="deploy/cloudflare/.github/workflows in the Rafi repository at the tag v$VERSION"
    [ -z "${RAFI_REPOSITORY:-}" ] || where="$where (https://github.com/$RAFI_REPOSITORY/tree/v$VERSION/deploy/cloudflare/.github/workflows)"
    if [ -z "$wanted" ]; then
      die "RAFI_DEPLOY_VERSION isn't set: the installation's workflows set it (env), to the deploy interface they are written for. Rafi $VERSION's is $DEPLOY_VERSION: copy .github/workflows/deploy.yaml and restate-cleanup.yaml from $where"
    fi
    die "Rafi $VERSION's deploy interface is $DEPLOY_VERSION, but your workflows are written for $wanted (RAFI_DEPLOY_VERSION): copy .github/workflows/deploy.yaml and restate-cleanup.yaml from $where, then deploy again"
  fi

  # The release the installation names.
  wanted="$(read_version "$PWD/VERSION" "the installation's VERSION file names the Rafi release it deploys (run this from the repository's root)")" || exit 1
  [ "$wanted" = "$VERSION" ] || die "the installation's VERSION says $wanted, but the release in $RELEASE is $VERSION"
  if [ -n "${RAFI_VERSION:-}" ] && [ "${RAFI_VERSION#v}" != "$VERSION" ]; then
    die "the release in $RELEASE is $VERSION, but RAFI_VERSION says ${RAFI_VERSION#v}"
  fi

  load_configs "$APP_CONFIG" "$ENGINE_CONFIG" "$ROOMS_CONFIG" "$RELEASE" "$strict"
  ENGINE_WORKER="$(engine_worker_name "$ENGINE_BASE" "$VERSION")" || exit 1
  say "Rafi $VERSION (deploy interface $DEPLOY_VERSION)"
  say "rooms Worker: $ROOMS_NAME ($ROOMS_CONFIG)"
  say "app Worker: $APP_NAME ($APP_CONFIG)"
  say "engine Worker: $ENGINE_WORKER ($ENGINE_CONFIG, named $ENGINE_BASE there)"
  say "database: $DATABASE_ID; Durable Objects' jurisdiction: $JURISDICTION"
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

# require_env <name...>: each set; the tokens fit curl's configuration (quoted, below).
require_env() {
  local name missing=()
  for name in "$@"; do
    [ -n "${!name:-}" ] || missing+=("$name")
  done
  [ "${#missing[@]}" -eq 0 ] || die "not set: ${missing[*]}"
  for name in "$@"; do
    case "$name" in
      *_TOKEN)
        if [[ "${!name}" == *'"'* || "${!name}" == *"\\"* || "${!name}" =~ [[:space:]] ]]; then
          die "$name has a quote, a backslash or whitespace in it"
        fi
        ;;
    esac
  done
  if [ -n "${RESTATE_ADMIN_URL:-}" ]; then
    RESTATE_ADMIN_URL="${RESTATE_ADMIN_URL%/}"
    export RESTATE_ADMIN_URL
  fi
}

# http <method> <url> <token> [<json body>]: prints the response body, a line break and the HTTP
# status (000 when there was no response). The token goes to curl on stdin, never in its
# arguments.
http() {
  local method="$1" url="$2" token="$3" data="${4:-}" args
  args=(-sS -X "$method" -K - -w '\n%{http_code}' -H 'accept: application/json' --max-time 60)
  [ -z "$data" ] || args+=(-H 'content-type: application/json' --data-binary "$data")
  printf 'header = "Authorization: Bearer %s"\n' "$token" | curl "${args[@]}" "$url" || printf '\n000'
}
cf_api() { http "$1" "${CLOUDFLARE_API_BASE_URL:-https://api.cloudflare.com/client/v4}$2" "$CLOUDFLARE_API_TOKEN" "${3:-}"; }
restate_api() { http "$1" "$RESTATE_ADMIN_URL$2" "$RESTATE_AUTH_TOKEN" "${3:-}"; }
status_of() { printf '%s\n' "${1##*$'\n'}"; }
body_of() { printf '%s\n' "${1%$'\n'*}"; }

# Cloudflare's errors in a response body (stdin), for a message.
cf_errors() {
  node -e '
    let d; try { d = JSON.parse(require("fs").readFileSync(0, "utf8")); } catch { process.exit(0); }
    process.stdout.write((d.errors ?? []).map((e) => `${e.code}: ${e.message}`).join("; "));'
}

# Succeeds when a 404's body (stdin) says there is no such Worker (10007, as wrangler checks it;
# 10090), not another 404 (a wrong account, a wrong path).
cf_no_such_worker() {
  node -e '
    let d; try { d = JSON.parse(require("fs").readFileSync(0, "utf8")); } catch { process.exit(1); }
    process.exit((d.errors ?? []).some((e) => e.code === 10007 || e.code === 10090) ? 0 : 1);'
}

# Prints the account's workers.dev subdomain.
cf_subdomain() {
  local resp sub
  resp="$(cf_api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/subdomain")"
  if [ "$(status_of "$resp")" = 200 ]; then
    sub="$(body_of "$resp" | node -e '
      const d = JSON.parse(require("fs").readFileSync(0, "utf8"));
      process.stdout.write(d.result?.subdomain ?? "");')" || sub=
    if [[ "$sub" =~ ^[a-z0-9][a-z0-9-]*$ ]]; then
      printf '%s\n' "$sub"
      return 0
    fi
  fi
  die "the account's workers.dev subdomain: HTTP $(status_of "$resp") $(body_of "$resp" | cf_errors). An account without one registers it at https://dash.cloudflare.com/$CLOUDFLARE_ACCOUNT_ID/workers/onboarding"
}

# What a Worker serves (cf_deployed <worker>): sets WORKER_EXISTS (no when Cloudflare has no such
# Worker) and DEPLOYED_TAGS, its latest deployment's versions, one per line: <percentage>
# <workers/tag annotation, - for none>; empty when it has no deployment. Cloudflare's API, as
# `wrangler deployments status` and `wrangler versions view` read it: the script's deployments,
# latest first, then each of the latest's versions.
cf_deployed() {
  local worker="$1" resp status versions pct id tag
  WORKER_EXISTS=yes
  DEPLOYED_TAGS=
  resp="$(cf_api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/scripts/$worker/deployments")"
  status="$(status_of "$resp")"
  if [ "$status" = 404 ] && body_of "$resp" | cf_no_such_worker; then
    WORKER_EXISTS=no
    return 0
  fi
  [ "$status" = 200 ] || die "the Worker $worker's deployments: HTTP $status $(body_of "$resp" | cf_errors)"
  versions="$(body_of "$resp" | node -e '
    const d = JSON.parse(require("fs").readFileSync(0, "utf8"));
    const deployments = d.result?.deployments;
    if (!Array.isArray(deployments)) throw new Error("no deployments in the answer");
    for (const v of deployments[0]?.versions ?? []) console.log(`${v.percentage}\t${v.version_id}`);')" \
    || die "couldn't read the Worker $worker's deployments"
  while IFS=$'\t' read -r pct id; do
    [ -n "$id" ] || continue
    [[ "$id" =~ ^[A-Za-z0-9-]+$ ]] || die "the Worker $worker's deployment has a version '$id'"
    resp="$(cf_api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/scripts/$worker/versions/$id")"
    [ "$(status_of "$resp")" = 200 ] \
      || die "the Worker $worker's version $id: HTTP $(status_of "$resp") $(body_of "$resp" | cf_errors)"
    tag="$(body_of "$resp" | node -e '
      const d = JSON.parse(require("fs").readFileSync(0, "utf8"));
      process.stdout.write(String(d.result?.annotations?.["workers/tag"] ?? "").replace(/\s/g, ""));')" \
      || die "couldn't read the Worker $worker's version $id"
    DEPLOYED_TAGS+="$pct ${tag:--}"$'\n'
  done <<<"$versions"
}

# Prints yes when one of a Worker's recent versions (Cloudflare's list, newest first) has a
# release's tag, v<version>, and no otherwise.
cf_release_version_among_recent() {
  local worker="$1" resp
  resp="$(cf_api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/scripts/$worker/versions")"
  [ "$(status_of "$resp")" = 200 ] || die "the Worker $worker's versions: HTTP $(status_of "$resp") $(body_of "$resp" | cf_errors)"
  body_of "$resp" | node -e '
    const d = JSON.parse(require("fs").readFileSync(0, "utf8"));
    const items = d.result?.items ?? d.result;
    if (!Array.isArray(items)) throw new Error("no versions in the answer");
    console.log(items.some((v) => /^v[0-9]/.test(v.annotations?.["workers/tag"] ?? "")) ? "yes" : "no");' \
    || die "couldn't read the Worker $worker's versions"
}

# The rooms Worker's tag: rooms-<the first 32 hex digits of the sha256 of its bundle's files
# (each one's path, size and content, in path order) and rooms.wrangler.toml>.
rooms_tag() {
  node - "$RELEASE/rooms/build" "$ROOMS_CONFIG" <<'EOF'
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const [build, config] = process.argv.slice(2);
const files = [];
(function walk(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full);
    else if (entry.isFile()) files.push(full);
    else throw new Error(`${full}: neither a file nor a directory`);
  }
})(build);
const hash = crypto.createHash("sha256");
const add = (name, content) => {
  hash.update(`${name}\0${content.length}\0`);
  hash.update(content);
};
const rel = (f) => path.relative(build, f).split(path.sep).join("/");
for (const f of files.sort((a, b) => (rel(a) < rel(b) ? -1 : rel(a) > rel(b) ? 1 : 0))) add(`build/${rel(f)}`, fs.readFileSync(f));
add("rooms.wrangler.toml", fs.readFileSync(config));
console.log(`rooms-${hash.digest("hex").slice(0, 32)}`);
EOF
}

# wrangler with its ND-JSON output (with_output <file> <args...>).
wrangler_with_output() {
  local file="$1"
  shift
  : >"$file"
  WRANGLER_OUTPUT_FILE_PATH="$file" run_wrangler "$@"
}

cmd_check() {
  step "checked: Rafi $VERSION fits this installation"
}

cmd_migrate() {
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  step "D1 migrations"
  run_wrangler d1 migrations apply DB -c "$APP_CONFIG" --remote
}

cmd_deploy_rooms() {
  local tag
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  step "rooms Worker $ROOMS_NAME"
  tag="$(rooms_tag)" || die "couldn't fingerprint the rooms Worker"
  say "this release's rooms Worker: $tag (its bundle and rooms.wrangler.toml)"
  cf_deployed "$ROOMS_NAME"
  if [ "$WORKER_EXISTS" = no ]; then
    say "not deployed yet"
  elif [ "$DEPLOYED_TAGS" = "100 $tag"$'\n' ]; then
    say "unchanged (it serves $tag): not deployed, so the live channels keep their connections"
    return 0
  else
    say "it serves: $(printf '%s' "${DEPLOYED_TAGS:-nothing}" | tr '\n' ',' | sed 's/,$//; s/,/, /g')"
  fi
  run_wrangler deploy -c "$ROOMS_CONFIG" --tag "$tag" --message "rafi v$VERSION"
}

cmd_upload_app() {
  local out id installed=no
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  step "the legal documents"
  copy_legal "$PWD/legal" "$APP_ASSETS"
  step "app Worker $APP_NAME"
  cf_deployed "$APP_NAME"
  if [ "$WORKER_EXISTS" = no ]; then
    die "the app Worker $APP_NAME doesn't exist on the account: before the first deploy, create it with its secrets (npx wrangler secret bulk .prod.vars, from .prod.vars.example)"
  fi
  # A release's code has run here when a version tagged v<version> is deployed, or was (a secret
  # changed since makes a new version, untagged).
  if grep -qE '^[0-9.]+ v[0-9]' <<<"$DEPLOYED_TAGS"; then
    installed=yes
  else
    installed="$(cf_release_version_among_recent "$APP_NAME")" || exit 1
  fi
  out="$(mktemp)"
  # shellcheck disable=SC2064 # the file's name, now
  trap "rm -f '$out'" EXIT
  if [ "$installed" = yes ]; then
    say "a release ran here before: uploading v$VERSION as a version, deployed once the engine is registered (promote-app)"
    wrangler_with_output "$out" versions upload -c "$APP_CONFIG" --tag "v$VERSION" --message "rafi v$VERSION"
    id="$(node -e '
      const lines = require("fs").readFileSync(process.argv[1], "utf8").split("\n").filter((l) => l.trim());
      const ups = lines.map((l) => JSON.parse(l)).filter((e) => e.type === "version-upload");
      process.stdout.write(String(ups.at(-1)?.version_id ?? ""));' "$out")" || id=
    [[ "$id" =~ ^[A-Za-z0-9-]+$ ]] || die "wrangler versions upload reported no version ID (its output: $(tr '\n' ' ' <"$out"))"
    output app_version_id "$id"
  else
    say "no release ran here yet (a first install: the Worker has only its secrets): deploying v$VERSION directly"
    run_wrangler deploy -c "$APP_CONFIG" --tag "v$VERSION" --message "rafi v$VERSION"
    output app_promoted true
  fi
}

cmd_deploy_engine() {
  local subdomain
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  step "engine Worker $ENGINE_WORKER"
  subdomain="$(cf_subdomain)" || exit 1
  run_wrangler deploy -c "$ENGINE_CONFIG" --name "$ENGINE_WORKER" --tag "v$VERSION" --message "rafi v$VERSION"
  output engine_worker "$ENGINE_WORKER"
  output engine_url "https://$ENGINE_WORKER.$subdomain.workers.dev/restate"
}

# The downgrade guard: reads Restate's services (stdin: GET /services, then a NUL, then GET
# /deployments) and refuses (exit 3) when the latest deployment of one of this engine's services
# is this installation's and of a newer release than <version>.
restate_guard() {
  node -e '
    const [version, instance, account, services] = process.argv.slice(1);
    const [servicesJson, deploymentsJson] = require("fs").readFileSync(0, "utf8").split("\0");
    function parse(v) {
      const m = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/.exec(v ?? "");
      return m && { core: [BigInt(m[1]), BigInt(m[2]), BigInt(m[3])], pre: m[4] ? m[4].split(".") : [] };
    }
    // Semantic versioning precedence: the core numbers, then a pre-release before its release,
    // pre-release identifiers numeric (by value) before alphanumeric (ASCII), fewer before more.
    function compare(a, b) {
      for (let i = 0; i < 3; i++) if (a.core[i] !== b.core[i]) return a.core[i] < b.core[i] ? -1 : 1;
      if (!a.pre.length && !b.pre.length) return 0;
      if (!a.pre.length) return 1;
      if (!b.pre.length) return -1;
      for (let i = 0; i < Math.max(a.pre.length, b.pre.length); i++) {
        if (i >= a.pre.length) return -1;
        if (i >= b.pre.length) return 1;
        const [x, y] = [a.pre[i], b.pre[i]];
        const [xn, yn] = [/^\d+$/.test(x), /^\d+$/.test(y)];
        if (xn && yn) { if (BigInt(x) !== BigInt(y)) return BigInt(x) < BigInt(y) ? -1 : 1; }
        else if (xn !== yn) return xn ? -1 : 1;
        else if (x !== y) return x < y ? -1 : 1;
      }
      return 0;
    }
    const mine = parse(version);
    const svc = JSON.parse(servicesJson).services;
    const deps = JSON.parse(deploymentsJson).deployments;
    if (!Array.isArray(svc) || !Array.isArray(deps)) throw new Error("no services or deployments in the answers");
    let refused = false;
    for (const name of services.split(" ")) {
      const s = svc.find((x) => x.name === name);
      if (!s) { console.log(`    ${name}: not registered yet`); continue; }
      const d = deps.find((x) => x.id === s.deployment_id);
      const m = d?.metadata ?? {};
      if (m.app !== "rafi" || m.instance !== instance || m.cloudflare_account !== account) {
        console.log(`    ${name}: on ${s.deployment_id}, not a deployment of this installation (no downgrade check)`);
        continue;
      }
      const theirs = parse(m.version);
      if (!theirs) throw new Error(`${s.deployment_id}: its metadata version is "${m.version}"`);
      const c = compare(mine, theirs);
      console.log(`    ${name}: on ${s.deployment_id}, Rafi ${m.version}${c === 0 ? " (this release: a re-run)" : ""}`);
      if (c < 0) {
        console.error(`error: ${name} runs on Rafi ${m.version} (${s.deployment_id}), newer than ${version}: refusing to register an older engine (a downgrade). Set VERSION to ${m.version} or later.`);
        refused = true;
      }
    }
    process.exit(refused ? 3 : 0);' "$@"
}

cmd_register_engine() {
  local worker url services deployments body resp status attempt id wait
  require_env CLOUDFLARE_ACCOUNT_ID RESTATE_ADMIN_URL RESTATE_AUTH_TOKEN RAFI_INSTANCE RAFI_ENGINE_WORKER RAFI_ENGINE_URL
  worker="$RAFI_ENGINE_WORKER"
  url="$RAFI_ENGINE_URL"
  [ "$worker" = "$ENGINE_WORKER" ] || die "RAFI_ENGINE_WORKER is $worker, but this release's engine Worker is $ENGINE_WORKER"
  [[ "$url" =~ ^https://${worker}\.[a-z0-9-]+\.workers\.dev/restate$ ]] \
    || die "RAFI_ENGINE_URL is $url, not https://$worker.<subdomain>.workers.dev/restate"
  step "Restate: the engine's services now"
  services="$(restate_api GET /services)"
  [ "$(status_of "$services")" = 200 ] || die "Restate's services: HTTP $(status_of "$services") $(body_of "$services")"
  deployments="$(restate_api GET /deployments)"
  [ "$(status_of "$deployments")" = 200 ] || die "Restate's deployments: HTTP $(status_of "$deployments") $(body_of "$deployments")"
  status=0
  { body_of "$services"; printf '\0'; body_of "$deployments"; } \
    | restate_guard "$VERSION" "$RAFI_INSTANCE" "$CLOUDFLARE_ACCOUNT_ID" "$ENGINE_SERVICES" || status=$?
  case "$status" in
    0) ;;
    3) exit 1 ;;
    *) die "couldn't read Restate's services and deployments" ;;
  esac

  step "Restate: registering $url"
  body="$(node -e '
    const [uri, instance, version, account, worker] = process.argv.slice(1);
    process.stdout.write(JSON.stringify({ uri, force: false,
      metadata: { app: "rafi", instance, version, cloudflare_account: account, worker } }));' \
    "$url" "$RAFI_INSTANCE" "$VERSION" "$CLOUDFLARE_ACCOUNT_ID" "$worker")"
  wait="${RAFI_REGISTER_WAIT:-10}"
  for ((attempt = 1; attempt <= REGISTER_ATTEMPTS; attempt++)); do
    resp="$(restate_api POST /deployments "$body")"
    status="$(status_of "$resp")"
    case "$status" in
      200 | 201)
        id="$(body_of "$resp" | node -e 'process.stdout.write(String(JSON.parse(require("fs").readFileSync(0, "utf8")).id ?? ""))')" || id=
        [ -n "$id" ] || die "Restate registered $url but answered no deployment ID: $(body_of "$resp")"
        if [ "$status" = 201 ]; then say "registered: Restate deployment $id"; else say "Restate already had it: deployment $id"; fi
        return 0
        ;;
      000 | 5??) ;;
      *) die "registering $url with Restate: HTTP $status $(body_of "$resp")" ;;
    esac
    warn "registering $url with Restate (attempt $attempt of $REGISTER_ATTEMPTS): HTTP $status $(body_of "$resp")"
    [ "$attempt" -eq "$REGISTER_ATTEMPTS" ] || sleep "$wait"
  done
  die "Restate couldn't register $url: the engine Worker is deployed but not registered. Running this again retries."
}

cmd_promote_app() {
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  step "app Worker $APP_NAME"
  if [ "${RAFI_APP_PROMOTED:-}" = true ]; then
    say "deployed directly by upload-app (a first install): nothing to promote"
    return 0
  fi
  [ -n "${RAFI_APP_VERSION_ID:-}" ] || die "neither RAFI_APP_PROMOTED=true nor RAFI_APP_VERSION_ID: upload-app's outputs"
  [[ "$RAFI_APP_VERSION_ID" =~ ^[A-Za-z0-9-]+$ ]] || die "RAFI_APP_VERSION_ID is '$RAFI_APP_VERSION_ID', not a version ID"
  run_wrangler versions deploy "$RAFI_APP_VERSION_ID@100%" --yes -c "$APP_CONFIG" --message "rafi v$VERSION"
  step "app Worker $APP_NAME: routes and Cron"
  run_wrangler triggers deploy -c "$APP_CONFIG"
}

# Restate's deployments for the cleanup: reads GET /deployments, a NUL, then the CLI's `deployments
# list --extra --json` (stdin), and prints one tab-separated line per deployment:
#   ours <status> <id> <worker> <active invocations>
#   other <id> <address> <why>
cleanup_plan() {
  node -e '
    const [base, instance, account] = process.argv.slice(1);
    const [adminJson, cliJson] = require("fs").readFileSync(0, "utf8").split("\0");
    const admin = JSON.parse(adminJson).deployments;
    // The CLI may say something before its JSON.
    const cli = JSON.parse(cliJson.slice(cliJson.indexOf("{"))).deployments;
    if (!Array.isArray(admin) || !Array.isArray(cli)) throw new Error("no deployments in a listing");
    const status = new Map(cli.map((d) => [d.deployment_id ?? d.id, d]));
    const clean = (s) => String(s ?? "").replace(/[\t\n\r]/g, " ");
    for (const d of admin) {
      if (!/^dp_[A-Za-z0-9]+$/.test(d.id ?? "")) throw new Error(`a deployment ID: ${d.id}`);
      const m = d.metadata ?? {};
      const address = clean(d.uri ?? d.arn ?? "?");
      let why = null;
      if (m.app !== "rafi") why = "not a Rafi deployment (no app=rafi metadata)";
      else if (m.instance !== instance) why = `another installation (${clean(m.instance)})`;
      else if (m.cloudflare_account !== account) why = "another Cloudflare account";
      else if (typeof m.worker !== "string" || !m.worker.startsWith(`${base}-v`) || !/^[a-z0-9-]{1,63}$/.test(m.worker)) why = `its worker is not a ${base}-v... engine Worker (${clean(m.worker)})`;
      else if (!new RegExp(`^https://${m.worker}\\.[a-z0-9-]+\\.workers\\.dev/restate/?$`).test(d.uri ?? "")) why = `its address is not the workers.dev one of ${m.worker}`;
      if (why) { console.log(["other", d.id, address, why].join("\t")); continue; }
      const s = status.get(d.id);
      console.log(["ours", s?.status ?? "Unknown", d.id, m.worker, s?.active_invocations ?? "?"].join("\t"));
    }' "$@"
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

cmd_cleanup() {
  local admin list plan kind a b c d resp status pinned failed=0 removed=0
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID RESTATE_ADMIN_URL RESTATE_AUTH_TOKEN RAFI_INSTANCE
  step "Restate's deployments"
  admin="$(restate_api GET /deployments)"
  [ "$(status_of "$admin")" = 200 ] \
    || die "listing Restate's deployments: HTTP $(status_of "$admin") $(body_of "$admin"): nothing deleted"
  list="$(run_restate deployments list --extra --json)" || die "restate deployments list failed: nothing deleted"
  plan="$({ body_of "$admin"; printf '\0'; printf '%s' "$list"; } | cleanup_plan "$ENGINE_BASE" "$RAFI_INSTANCE" "$CLOUDFLARE_ACCOUNT_ID")" \
    || die "couldn't read Restate's deployments: nothing deleted"
  [ -n "$plan" ] || say "none"
  while IFS=$'\t' read -r kind a b c d; do
    [ "$kind" = ours ] || continue
    # a: status, b: deployment, c: Worker, d: active invocations.
    case "$a" in
      Drained)
        say "$b ($c) is drained: deleting the Worker, then the deployment"
        resp="$(cf_api DELETE "/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/scripts/$c")"
        status="$(status_of "$resp")"
        case "$status" in
          200) say "  deleted the Worker $c" ;;
          404) say "  the Worker $c is already gone" ;;
          *)
            warn "deleting the Worker $c: HTTP $status $(body_of "$resp" | cf_errors): its deployment $b stays"
            failed=1
            continue
            ;;
        esac
        if run_restate deployments remove "$b" --yes; then
          say "  removed the deployment $b"
          removed=$((removed + 1))
        else
          warn "Restate didn't remove $b (its Worker $c is deleted): the next cleanup tries again"
          failed=1
        fi
        ;;
      Draining)
        say "$b ($c) is draining: $d invocation(s) still pinned to it"
        if pinned="$(pinned_invocations "$b")"; then
          printf '%s\n' "$pinned" | sed '/^$/d; s/^/      /'
        fi
        say "  a paused invocation waits for a human: restate invocations resume <id> --deployment latest, or restate invocations kill <id>"
        ;;
      Active) say "$b ($c) is active (the latest for its services): kept" ;;
      *) say "$b ($c) is $a: left as it is" ;;
    esac
  done <<<"$plan"
  while IFS=$'\t' read -r kind a b c _; do
    [ "$kind" = other ] || continue
    say "$a at $b: $c; never touched"
  done <<<"$plan"
  [ "$failed" = 0 ] || die "the cleanup failed in part (above); $removed deployment(s) removed"
  step "cleaned up: $removed drained engine deployment(s) and their Workers removed"
}

cmd_dry_run() {
  step "the legal documents"
  copy_legal "$PWD/legal" "$APP_ASSETS"
  step "rooms Worker $ROOMS_NAME: wrangler deploy --dry-run"
  run_wrangler deploy --dry-run -c "$ROOMS_CONFIG"
  step "engine Worker $ENGINE_WORKER: wrangler deploy --dry-run"
  run_wrangler deploy --dry-run -c "$ENGINE_CONFIG" --name "$ENGINE_WORKER"
  step "app Worker $APP_NAME: wrangler deploy --dry-run"
  run_wrangler deploy --dry-run -c "$APP_CONFIG"
  step "dry run of Rafi $VERSION done: nothing deployed"
}

main() {
  local command="${1:-}" strict=1
  if [ "$#" -ne 1 ]; then
    usage >&2
    exit 2
  fi
  case "$command" in
    -h | --help)
      usage
      return 0
      ;;
    dry-run) strict=0 ;;
    check | migrate | deploy-rooms | upload-app | deploy-engine | register-engine | promote-app | cleanup) ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  check_release "$strict"
  "cmd_${command//-/_}"
}

main "$@"
