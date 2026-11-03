#!/usr/bin/env bash
# Deploys a release of Rafi to Cloudflare, one step per subcommand: the live Worker (the
# raffles' RaffleRoom Durable Objects), the app Worker and the release's own raffle engine
# Worker, registered with Restate; and cleans up the engine Workers Restate has drained. Each
# release ships this file as its deploy.sh (the source is scripts/cloudflare-deploy.sh in the
# Rafi repository), so the deploy logic is the release's own, pinned to it and covered by its
# provenance attestation. An installation's workflows (deploy/cloudflare/.github/workflows:
# deploy.yaml and restate-cleanup.yaml) download and verify the release, unpack it into
# release/ and run these subcommands, each job on a fresh runner; nothing here is specific to
# GitHub Actions.
#
# Usage, from the root of an installation's repository (a copy of deploy/cloudflare: VERSION,
# app.wrangler.toml, engine.wrangler.toml, live.wrangler.toml, legal/), with the release
# unpacked into release/ (the directory this script is in is the release; the configurations
# point into it as release/...):
#
#   release/deploy.sh check             the release, the VERSION file and the configurations; nothing remote
#   release/deploy.sh check-remote      check, then no downgrade (Restate's release) and the token works; reads only
#   release/deploy.sh migrate           the D1 migrations
#   release/deploy.sh deploy-live       the live Worker, unless its code and configuration are unchanged
#   release/deploy.sh upload-app        the app Worker's version uploaded (deployed directly on a first install)
#   release/deploy.sh deploy-engine     this release's engine Worker, <engine name>-v<version>
#   release/deploy.sh register-engine   the engine Worker registered with Restate (never a downgrade)
#   release/deploy.sh promote-app       the uploaded app version deployed, then its routes and Cron
#   release/deploy.sh cleanup           drained engine deployments removed from Restate, then their Workers deleted
#   release/deploy.sh dry-run           check, legal/ copied in, `wrangler deploy --dry-run` of the three Workers
#
# Environment:
#   CLOUDFLARE_ENV        optional: the Wrangler environment ([env.<name>] in each configuration)
#                         this installation is, for several installations in one repository;
#                         wrangler reads it too, so every wrangler call here deploys that
#                         environment's Workers. Unset or empty: the configurations' top level.
#   RAFI_VERSION          optional: the version the release must be
#   CLOUDFLARE_API_TOKEN, CLOUDFLARE_ACCOUNT_ID   the account; all but check and dry-run
#                         (register-engine needs only the account ID, for the metadata)
#   RESTATE_ADMIN_URL     check-remote, register-engine, cleanup: the Restate environment's admin
#                         API (https://<env>.env.<region>.restate.cloud:9070)
#   RESTATE_AUTH_TOKEN    check-remote, register-engine, cleanup: a Restate API key with the Admin
#                         role
#   RAFI_APP_PROMOTED, RAFI_APP_VERSION_ID   promote-app: upload-app's outputs
#   RAFI_ENGINE_WORKER, RAFI_ENGINE_URL      register-engine: deploy-engine's outputs
#   RAFI_OUTPUT_FILE      when set, a subcommand's outputs are appended to this file as key=value
#                         lines (GitHub Actions: $GITHUB_OUTPUT); otherwise they are printed as such
#   RAFI_REPORT_FILE      when set, a subcommand's report (Markdown) is appended to this file
#                         (GitHub Actions: $GITHUB_STEP_SUMMARY); it is printed either way
#   RAFI_TOOLS_DIR        where wrangler is installed (default .rafi-tools, in the installation's
#                         root): `npm install --prefix` of wrangler@4.148.0, once, unless both
#                         overrides below are set
#   WRANGLER              the wrangler command (default: the tools directory's)
#   RAFI_WRANGLER_PKG     a wrangler package directory (with wrangler-dist/cli.js) whose
#                         configuration reader reads the configurations (default: the tools
#                         directory's)
#   RESTATE               the Restate CLI command (default: npx --yes @restatedev/restate@1.8.1)
#   CLOUDFLARE_API_BASE_URL   the Cloudflare API (wrangler's variable too; for tests)
#   RAFI_REGISTER_WAIT    seconds between registration attempts (default 10; for tests)
#
# What each does (each prints its steps):
#   Every subcommand first checks, as `check` does: the release's VERSION is a version
#   (x.y.z, optionally -pre.release) and the installation's version file's (VERSION.<env>
#   when CLOUDFLARE_ENV is set and that file exists, VERSION otherwise; and RAFI_VERSION's, when
#   set); and the three configurations fit the release and each other. app.wrangler.toml (the
#   app), engine.wrangler.toml (the engine) and live.wrangler.toml (the live Worker) are read
#   as wrangler reads them (its own configuration reader, for CLOUDFLARE_ENV's environment: an
#   environment's name is its own `name`, or <top-level name>-<environment>; vars and bindings
#   aren't inherited from the top level, so an environment repeats them). They must agree:
#   main, the assets and the migrations point into this release; all three bind DB to one
#   database_id; the live Worker defines RAFFLE_ROOM (class RaffleRoom, no script_name) with
#   its Durable Object migrations and has no address (workers_dev and preview_urls false), and
#   the app and the engine bind it with script_name = the live Worker's name; all three say the
#   same RAFI_DO_JURISDICTION; the engine is on workers.dev and has
#   RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS; an environment has every var, rate limit and required
#   secret its top level has. The placeholders (the zero database_id, an identity key that isn't
#   publickeyv1_...) are refused, and only warned about by dry-run.
#   The installation is its app Worker's name (unique in the account): Restate's deployment
#   metadata names it as `instance`, and the downgrade guard and the cleanup touch only that
#   instance's deployments in this account.
#   check-remote: what can stop a deploy, checked before it changes anything (the workflow's
#     first job runs it): the downgrade guard (register-engine's, below: Restate runs no newer
#     release of this installation's; none at all on a first install), then the token reads the
#     account (its workers.dev subdomain, which deploy-engine needs). Restate unreachable, or a
#     key or token that doesn't work, fails it. Reads only.
#   migrate: `wrangler d1 migrations apply DB --remote` (the app's configuration).
#   deploy-live: the live Worker's fingerprint is the sha256 of its bundle's files
#     (release/live/build) and its configuration as wrangler reads it for this environment (the
#     top level and the environment's section, not comments or other environments); its tag,
#     live-<32 hex digits of it>, is the version annotation (workers/tag) of each live Worker
#     version this deploys. When the Worker's current deployment is one version at 100% with
#     that tag (Cloudflare's API: the script's latest deployment, then that version's
#     annotations), nothing is deployed: the live channels' connections are kept. Otherwise
#     `wrangler deploy` (the class's Durable Object migrations need it).
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
#   register-engine: refuses a downgrade (again, as check-remote did: another deploy may have
#     registered a newer release since): the deployment Restate routes new Raffle and
#     EngineInfo invocations to (GET /services), if its metadata says it is this installation's
#     (app rafi, instance the app Worker's name, cloudflare_account CLOUDFLARE_ACCOUNT_ID; GET
#     /deployments), must not be of a newer release than this one (semantic versioning,
#     pre-releases before their release); the same release is a re-run. Then POST /deployments
#     of engine_url, "force": false, with the metadata app, instance, version,
#     cloudflare_account and worker; 200 (Restate had it) and 201 succeed, a 5xx or no answer is
#     retried (a Worker deployed a moment ago), anything else fails.
#   promote-app: after a first install, nothing; otherwise `wrangler versions deploy
#     <app_version_id>@100%`, then `wrangler triggers deploy` (the routes and the Cron, which a
#     version upload doesn't apply).
#   cleanup: Restate's deployments with their metadata (GET /deployments) and their status
#     (`restate deployments list --extra --json`), joined by ID, and the account's Workers (GET
#     /accounts/{id}/workers/scripts); any listing failing stops it before anything is removed
#     or deleted. Only this installation's deployments (the metadata above, its worker <engine
#     name>-v..., its address that Worker's workers.dev one) are touched. Each Drained one: the
#     deployment removed from Restate (`restate deployments remove`, which refuses one not
#     drained; never forced), then its Worker deleted (DELETE
#     /accounts/{id}/workers/scripts/<worker>; 404 is already gone); a removal refused leaves
#     the Worker alone, a deletion failing leaves the Worker behind (reported; the run fails).
#     Reported: what was removed and deleted; those still draining, with their active and
#     paused invocations (a paused one waits for a human: resume it on the latest deployment or
#     kill it); the Workers left behind, those whose deletion failed and the account's <engine
#     name>-v... Workers no Restate deployment refers to (never deleted here: delete them by
#     hand); and every deployment that isn't this installation's (never touched).
#   dry-run: check (placeholders warned about), legal/ copied in, and `wrangler deploy
#     --dry-run` of the live Worker, the engine Worker under its versioned name and the app
#     Worker; no credentials.
#
# Exit status: 0 on success, 1 on a failure or a refusal, 2 on a usage error. Every subcommand
# can be run again: a failed one is retried by running it again.

# The node programs below are single-quoted: their ${...} are JavaScript's.
# shellcheck disable=SC2016
set -euo pipefail

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

# The tools (ensure_tools): WRANGLER_CMD, the wrangler command, and WRANGLER_PKG, the wrangler
# package whose configuration reader reads the configurations. Unless both are given (WRANGLER,
# RAFI_WRANGLER_PKG), the pinned wrangler is installed into RAFI_TOOLS_DIR, once: node can't
# import a package npx runs from its cache.
ensure_tools() {
  local dir="${RAFI_TOOLS_DIR:-.rafi-tools}" need=no installed
  case "$dir" in /*) ;; *) dir="$PWD/$dir" ;; esac
  if [ -n "${RAFI_WRANGLER_PKG:-}" ]; then
    WRANGLER_PKG="$RAFI_WRANGLER_PKG"
  else
    WRANGLER_PKG="$dir/node_modules/wrangler"
    need=yes
  fi
  if [ -n "${WRANGLER:-}" ]; then
    read -r -a WRANGLER_CMD <<<"$WRANGLER"
  else
    WRANGLER_CMD=("$dir/node_modules/.bin/wrangler")
    need=yes
  fi
  if [ "$need" = yes ]; then
    installed="$(node -e 'try { process.stdout.write(require(process.argv[1]).version ?? "") } catch {}' \
      "$dir/node_modules/wrangler/package.json")"
    if [ "$installed" != "$WRANGLER_VERSION" ]; then
      step "tools: wrangler $WRANGLER_VERSION into $dir"
      npm install --no-save --no-audit --no-fund --prefix "$dir" "wrangler@$WRANGLER_VERSION" >&2 \
        || die "couldn't install wrangler $WRANGLER_VERSION into $dir (npm install)"
    fi
  fi
  [ -f "$WRANGLER_PKG/wrangler-dist/cli.js" ] \
    || die "$WRANGLER_PKG isn't a wrangler package (no wrangler-dist/cli.js): RAFI_WRANGLER_PKG names one, or unset it"
  installed="$(node -e 'try { process.stdout.write(require(process.argv[1]).version ?? "") } catch {}' "$WRANGLER_PKG/package.json")"
  [ "$installed" = "$WRANGLER_VERSION" ] \
    || warn "the configurations are read with wrangler ${installed:-of an unknown version} ($WRANGLER_PKG), not $WRANGLER_VERSION"
}

run_wrangler() {
  WRANGLER_SEND_METRICS=false command "${WRANGLER_CMD[@]}" "$@"
}

run_restate() {
  local cmd
  read -r -a cmd <<<"${RESTATE:-npx --yes @restatedev/restate@$RESTATE_CLI_VERSION}"
  SCARF_ANALYTICS=false command "${cmd[@]}" "$@"
}

# A subcommand's output: a key=value line in RAFI_OUTPUT_FILE when that is set, on stdout
# otherwise.
output() {
  local key="$1" value="$2"
  [[ "$value" != *$'\n'* ]] || die "the output $key has a line break"
  if [ -n "${RAFI_OUTPUT_FILE:-}" ]; then
    printf '%s=%s\n' "$key" "$value" >>"$RAFI_OUTPUT_FILE"
    say "output: $key=$value"
  else
    printf '%s=%s\n' "$key" "$value"
  fi
}

# A subcommand's report (report <Markdown>): printed, and appended to RAFI_REPORT_FILE when that
# is set.
report() {
  step "report"
  printf '%s\n' "$1"
  [ -z "${RAFI_REPORT_FILE:-}" ] || printf '%s\n\n' "$1" >>"$RAFI_REPORT_FILE"
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

# The installation's version file: VERSION.<CLOUDFLARE_ENV> when that is set and the file
# exists, VERSION otherwise (the workflows choose the release to download by the same rule).
version_file() {
  if [ -n "${CLOUDFLARE_ENV:-}" ] && [ -f "VERSION.$CLOUDFLARE_ENV" ]; then
    printf '%s\n' "VERSION.$CLOUDFLARE_ENV"
  else
    printf '%s\n' VERSION
  fi
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

# The configuration reader (read_configs <app> <engine> <live>): the three configurations as
# wrangler reads them (WRANGLER_PKG's unstable_readConfig) for CLOUDFLARE_ENV's environment,
# printed as one JSON object:
#   { "env": <CLOUDFLARE_ENV or null>,
#     "app"|"engine"|"live": { "file", "name", "main", "workers_dev", "preview_urls",
#       "assets": { "directory", "binding" } | null, "d1_databases": [{ "binding",
#       "database_id", "migrations_dir" }], "durable_objects": [{ "name", "class_name",
#       "script_name" }], "migrations": [...], "vars": { "RAFI_DO_JURISDICTION",
#       "RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS" }, "var_names": [...], "ratelimits": [names],
#       "secrets_required": [names],
#       "digest": <sha256 of the configuration's top level and this environment's section> },
#     "top": { the same for the top level, when an environment is read } }
# Paths are absolute (relative ones resolved against the configuration's directory, as wrangler
# does). Errors (a file missing, wrangler refusing it, no such environment) on stderr, exit 1.
read_configs() {
  node --no-deprecation - "$WRANGLER_PKG" "$1" "$2" "$3" <<'EOF'
// wrangler logs through the console: stdout is this program's JSON alone.
for (const k of ["log", "info", "debug", "warn"]) console[k] = console.error;
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const { pathToFileURL } = require("url");
const [pkg, ...files] = process.argv.slice(2);
const env = process.env.CLOUDFLARE_ENV || undefined;

function canonical(v) {
  if (Array.isArray(v)) return `[${v.map(canonical).join(",")}]`;
  if (v && typeof v === "object") {
    return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${canonical(v[k])}`).join(",")}}`;
  }
  return JSON.stringify(v ?? null);
}
function summary(c, file, raw, envName) {
  const dir = path.dirname(path.resolve(file));
  const abs = (p) => (typeof p === "string" ? path.resolve(dir, p) : null);
  const vars = c.vars ?? {};
  const { env: envs, ...top } = raw;
  return {
    file: path.basename(file),
    name: c.name ?? null,
    main: abs(c.main),
    workers_dev: c.workers_dev ?? null,
    preview_urls: c.preview_urls ?? null,
    assets: c.assets ? { directory: abs(c.assets.directory), binding: c.assets.binding ?? null } : null,
    d1_databases: (c.d1_databases ?? []).map((d) => ({ binding: d.binding ?? null,
      database_id: d.database_id ?? null, migrations_dir: abs(d.migrations_dir) })),
    durable_objects: (c.durable_objects?.bindings ?? []).map((b) => ({ name: b.name ?? null,
      class_name: b.class_name ?? null, script_name: b.script_name ?? null })),
    migrations: c.migrations ?? [],
    vars: { RAFI_DO_JURISDICTION: vars.RAFI_DO_JURISDICTION ?? null,
      RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS: vars.RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS ?? null },
    var_names: Object.keys(vars).sort(),
    ratelimits: (c.ratelimits ?? []).map((r) => r.name).sort(),
    secrets_required: [...(c.secrets?.required ?? [])].sort(),
    digest: crypto.createHash("sha256")
      .update(canonical({ top, env: envName ?? null, section: envName ? envs?.[envName] ?? null : null }))
      .digest("hex"),
  };
}

(async () => {
  let wrangler;
  try {
    wrangler = await import(pathToFileURL(path.join(pkg, "wrangler-dist", "cli.js")).href);
  } catch (e) {
    console.error(`error: couldn't load wrangler's configuration reader from ${pkg}: ${e.message}`);
    process.exit(1);
  }
  const out = { env: env ?? null };
  if (env) out.top = {};
  const errors = [];
  for (const [key, file] of [["app", files[0]], ["engine", files[1]], ["live", files[2]]]) {
    const base = path.basename(file);
    if (!fs.existsSync(file)) { errors.push(`${base} is missing`); continue; }
    try {
      const raw = wrangler.experimental_readRawConfig({ config: file }).rawConfig;
      if (env && !Object.hasOwn(raw.env ?? {}, env)) {
        errors.push(`CLOUDFLARE_ENV is ${env}, but ${base} has no [env.${env}]: each configuration defines the environment`);
        continue;
      }
      // env "" is the top level, whatever CLOUDFLARE_ENV says.
      const c = wrangler.unstable_readConfig({ config: file, env: env ?? "" }, { hideWarnings: true });
      out[key] = summary(c, file, raw, env);
      if (env) {
        out.top[key] = summary(wrangler.unstable_readConfig({ config: file, env: "" }, { hideWarnings: true }), file, raw, null);
      }
    } catch (e) {
      errors.push(`${base}: ${String(e.message ?? e).trim().replace(/\n+/g, " ")}`);
    }
  }
  if (errors.length) { console.error(errors.map((e) => `error: ${e}`).join("\n")); process.exit(1); }
  process.stdout.write(`${JSON.stringify(out)}\n`);
})();
EOF
}

# Checks the configurations (check_configs <release> <strict> <read_configs' JSON>):
# prints key=value lines (app_name, app_assets, database_id, engine_base, live_name,
# jurisdiction, live_digest), warnings on stderr. strict=1 refuses placeholders; 0 warns about
# them.
check_configs() {
  node - "$1" "$2" "$3" <<'EOF'
const fs = require("fs");
const path = require("path");
const [release, strict, json] = process.argv.slice(2);
const { env, app, engine, live, top } = JSON.parse(json);
const errors = [];
const warnings = [];
const where = env ? ` ([env.${env}])` : "";

// A path in a configuration (resolved by the reader), which must be this release's.
function inRelease(c, value, key, expected, kind) {
  const want = path.join(release, expected);
  if (typeof value !== "string") { errors.push(`${c.file}${where}: ${key} is missing (${want})`); return null; }
  const real = fs.existsSync(value) ? fs.realpathSync(value) : value;
  if (real !== want) {
    errors.push(`${c.file}${where}: ${key} is ${value}, not this release's ${expected} (${want})`);
    return null;
  }
  const stat = fs.existsSync(want) ? fs.statSync(want) : null;
  if (!stat || (kind === "dir" ? !stat.isDirectory() : !stat.isFile())) {
    errors.push(`the release has no ${expected} (${want})`);
    return null;
  }
  return want;
}
function one(c, list, what, key, value) {
  const found = c[list].filter((t) => t[key] === value);
  if (found.length !== 1) {
    errors.push(`${c.file}${where}: ${found.length ? "more than one" : "no"} ${what} with ${key} = "${value}"${env && !found.length ? " (environments don't inherit bindings: repeat them in the environment)" : ""}`);
    return {};
  }
  return found[0];
}
const workerName = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;

// An environment inherits neither vars, bindings nor required secrets: what the top level has,
// it repeats.
if (env) {
  for (const c of [app, engine, live]) {
    const t = top[c === app ? "app" : c === engine ? "engine" : "live"];
    const vars = t.var_names.filter((v) => !c.var_names.includes(v));
    if (vars.length) {
      errors.push(`${c.file}: [env.${env}] has no ${vars.join(", ")} in its vars, which the top level has: environments don't inherit vars (repeat them under [env.${env}.vars])`);
    }
    const limits = t.ratelimits.filter((r) => !c.ratelimits.includes(r));
    if (limits.length) {
      errors.push(`${c.file}: [env.${env}] has no rate limit ${limits.join(", ")}, which the top level has: environments don't inherit bindings (repeat them as [[env.${env}.ratelimits]])`);
    }
    const secrets = t.secrets_required.filter((v) => !c.secrets_required.includes(v));
    if (secrets.length) {
      errors.push(`${c.file}: [env.${env}] doesn't require ${secrets.join(", ")}, which the top level does: environments don't inherit [secrets] (repeat it as [env.${env}.secrets])`);
    }
  }
}

// The live Worker: the RaffleRoom class's, reached only through the other two's bindings.
if (!workerName.test(live.name ?? "")) errors.push(`${live.file}${where}: name is missing or not a Worker name (a-z, 0-9, -)`);
inRelease(live, live.main, "main", "live/build/index.js", "file");
if (live.workers_dev !== false) errors.push(`${live.file}${where}: workers_dev must be false (the live Worker has no address)`);
if (live.preview_urls !== false) errors.push(`${live.file}${where}: preview_urls must be false (the live Worker has no address)`);
const liveDb = one(live, "d1_databases", "[[d1_databases]]", "binding", "DB");
const liveRoom = one(live, "durable_objects", "[[durable_objects.bindings]]", "name", "RAFFLE_ROOM");
if (liveRoom.class_name !== undefined && liveRoom.class_name !== "RaffleRoom") errors.push(`${live.file}${where}: RAFFLE_ROOM's class_name must be "RaffleRoom"`);
if (liveRoom.script_name) errors.push(`${live.file}${where}: RAFFLE_ROOM is the live Worker's own class: no script_name`);
if (!live.migrations.some((m) => [...(m.new_sqlite_classes ?? []), ...(m.new_classes ?? [])].includes("RaffleRoom"))) {
  errors.push(`${live.file}${where}: no [[migrations]] creates the RaffleRoom class`);
}
const jurisdiction = live.vars.RAFI_DO_JURISDICTION;
if (typeof jurisdiction !== "string" || jurisdiction === "") errors.push(`${live.file}${where}: [vars] RAFI_DO_JURISDICTION is missing`);

// A Worker binding the live Worker's class, by its name.
function bindsLive(c) {
  const room = one(c, "durable_objects", "[[durable_objects.bindings]]", "name", "RAFFLE_ROOM");
  if (room.class_name !== undefined && room.class_name !== "RaffleRoom") errors.push(`${c.file}${where}: RAFFLE_ROOM's class_name must be "RaffleRoom"`);
  if (room.name && room.script_name !== live.name) {
    errors.push(`${c.file}${where}: RAFFLE_ROOM's script_name (${room.script_name}) must be the live Worker's name (${live.name})`);
  }
  if (c.migrations.length) errors.push(`${c.file}${where}: no [[migrations]]: the RaffleRoom class is the live Worker's`);
  if (c.vars.RAFI_DO_JURISDICTION !== jurisdiction) {
    errors.push(`${c.file}${where}: RAFI_DO_JURISDICTION (${c.vars.RAFI_DO_JURISDICTION}) must be the live Worker's (${jurisdiction})`);
  }
}

// The app.
if (!workerName.test(app.name ?? "")) errors.push(`${app.file}${where}: name is missing or not a Worker name (a-z, 0-9, -)`);
else if (app.name === live.name) errors.push(`${app.file}${where}: name must differ from the live Worker's (${live.name})`);
inRelease(app, app.main, "main", "app/build/index.js", "file");
const assets = inRelease(app, app.assets?.directory, "[assets] directory", "app/assets", "dir");
const appDb = one(app, "d1_databases", "[[d1_databases]]", "binding", "DB");
inRelease(app, appDb.migrations_dir, "[[d1_databases]] migrations_dir", "migrations", "dir");
bindsLive(app);
if (liveDb.database_id !== appDb.database_id) {
  errors.push(`${live.file}${where}: DB's database_id (${liveDb.database_id}) isn't the app's (${appDb.database_id}): the live channels read the app's database`);
}

// The engine, and that it agrees with the app and the live Worker.
if (!workerName.test(engine.name ?? "")) errors.push(`${engine.file}${where}: name is missing or not a Worker name (a-z, 0-9, -)`);
else if (engine.name === app.name) errors.push(`${engine.file}${where}: name must differ from the app's (${app.name})`);
else if (engine.name === live.name) errors.push(`${engine.file}${where}: name must differ from the live Worker's (${live.name})`);
inRelease(engine, engine.main, "main", "engine/build/index.js", "file");
if (engine.workers_dev !== true) errors.push(`${engine.file}${where}: workers_dev must be true (Restate calls the engine at its workers.dev address)`);
const engineDb = one(engine, "d1_databases", "[[d1_databases]]", "binding", "DB");
if (engineDb.database_id !== appDb.database_id) {
  errors.push(`${engine.file}${where}: DB's database_id (${engineDb.database_id}) isn't the app's (${appDb.database_id}): the engine writes the app's database`);
}
bindsLive(engine);
const keys = engine.vars.RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS;
if (typeof keys !== "string") {
  errors.push(env
    ? `${engine.file}: [env.${env}] has no RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS in its vars: environments don't inherit vars, so [env.${env}.vars] repeats them (RAFI_DO_JURISDICTION and the identity keys)`
    : `${engine.file}: [vars] RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS is missing`);
}

// The placeholders an installation replaces.
const placeholders = [];
if (typeof appDb.database_id !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(appDb.database_id)) {
  errors.push(`${app.file}${where}: DB's database_id is missing or not a D1 database ID`);
} else if (appDb.database_id === "00000000-0000-0000-0000-000000000000") {
  placeholders.push(`${app.file}, ${engine.file}, ${live.file}${where}: DB's database_id is the placeholder (wrangler d1 create)`);
}
if (typeof keys === "string" && !keys.split(",").every((k) => /^publickeyv1_[A-Za-z0-9]+$/.test(k.trim()))) {
  placeholders.push(`${engine.file}${where}: RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS isn't a list of publickeyv1_... keys (the Restate environment's)`);
}
(strict === "1" ? errors : warnings).push(...placeholders);

for (const w of warnings) console.error(`warning: ${w}`);
if (errors.length) { console.error(errors.map((e) => `error: ${e}`).join("\n")); process.exit(1); }
const out = { app_name: app.name, app_assets: assets, database_id: appDb.database_id,
  engine_base: engine.name, live_name: live.name, jurisdiction, live_digest: live.digest };
for (const [k, v] of Object.entries(out)) {
  if (/[\n\r]/.test(v)) { console.error(`error: ${k} has a line break`); process.exit(1); }
  console.log(`${k}=${v}`);
}
EOF
}

# Sets APP_NAME, APP_ASSETS, DATABASE_ID, ENGINE_BASE, LIVE_NAME, JURISDICTION and LIVE_DIGEST
# from the configurations (load_configs <release> <strict>).
load_configs() {
  local json out key value
  json="$(read_configs "$APP_CONFIG" "$ENGINE_CONFIG" "$LIVE_CONFIG")" \
    || die "couldn't read the configurations (above)"
  out="$(check_configs "$1" "$2" "$json")" || die "the configurations don't fit this release (above)"
  while IFS='=' read -r key value; do
    case "$key" in
      app_name) APP_NAME="$value" ;;
      app_assets) APP_ASSETS="$value" ;;
      database_id) DATABASE_ID="$value" ;;
      engine_base) ENGINE_BASE="$value" ;;
      live_name) LIVE_NAME="$value" ;;
      jurisdiction) JURISDICTION="$value" ;;
      live_digest) LIVE_DIGEST="$value" ;;
    esac
  done <<<"$out"
}

# What every subcommand checks first (check_release <strict>): the release, the installation's
# version file and the configurations. Sets RELEASE, VERSION, VERSION_FILE, the configurations' paths,
# load_configs' variables and ENGINE_WORKER (this release's engine Worker).
check_release() {
  local strict="$1" file wanted
  RELEASE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
  APP_CONFIG="$PWD/app.wrangler.toml"
  ENGINE_CONFIG="$PWD/engine.wrangler.toml"
  LIVE_CONFIG="$PWD/live.wrangler.toml"
  step "release $RELEASE"
  VERSION="$(read_version "$RELEASE/VERSION" "is $RELEASE an unpacked Rafi release?")" || exit 1

  # The release the installation names.
  file="$(version_file)"
  VERSION_FILE="$file"
  wanted="$(read_version "$PWD/$file" "the installation's VERSION file names the Rafi release it deploys (run this from the repository's root)")" || exit 1
  [ "$wanted" = "$VERSION" ] || die "the installation's $file says $wanted, but the release in $RELEASE is $VERSION"
  if [ -n "${RAFI_VERSION:-}" ] && [ "${RAFI_VERSION#v}" != "$VERSION" ]; then
    die "the release in $RELEASE is $VERSION, but RAFI_VERSION says ${RAFI_VERSION#v}"
  fi

  ensure_tools
  load_configs "$RELEASE" "$strict"
  ENGINE_WORKER="$(engine_worker_name "$ENGINE_BASE" "$VERSION")" || exit 1
  say "Rafi $VERSION ($file)"
  if [ -n "${CLOUDFLARE_ENV:-}" ]; then
    say "environment: $CLOUDFLARE_ENV ([env.$CLOUDFLARE_ENV] of each configuration)"
  else
    say "environment: none (the configurations' top level)"
  fi
  say "live Worker: $LIVE_NAME ($LIVE_CONFIG)"
  say "app Worker: $APP_NAME ($APP_CONFIG; the installation's name in Restate)"
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

# Prints the names of the account's Workers, one per line: GET /accounts/{id}/workers/scripts
# (Workers Scripts Read or Write), which answers every Worker of the account at once (the API has
# no pages for it; each one's id is its name). Fails (exit 1, a message on stderr) when it can't be
# read.
cf_workers() {
  local resp
  resp="$(cf_api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/scripts")"
  if [ "$(status_of "$resp")" != 200 ]; then
    printf 'error: listing the account'\''s Workers: HTTP %s %s\n' "$(status_of "$resp")" "$(body_of "$resp" | cf_errors)" >&2
    return 1
  fi
  body_of "$resp" | node -e '
    const d = JSON.parse(require("fs").readFileSync(0, "utf8"));
    if (!Array.isArray(d.result)) throw new Error("no Workers in the answer");
    for (const s of d.result) {
      if (typeof s.id !== "string" || !/^[A-Za-z0-9_-]+$/.test(s.id)) throw new Error(`a Worker named ${s.id}`);
      console.log(s.id);
    }' || {
    echo "error: couldn't read the account's Workers" >&2
    return 1
  }
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

# The live Worker's tag: live-<the first 32 hex digits of the sha256 of its bundle's files (each
# one's path, size and content, in path order) and its configuration's digest (read_configs':
# the top level and this environment's section, as wrangler reads them)>.
live_tag() {
  node - "$RELEASE/live/build" "$LIVE_DIGEST" <<'EOF'
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const [build, digest] = process.argv.slice(2);
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
add("live.wrangler.toml", Buffer.from(digest));
console.log(`live-${hash.digest("hex").slice(0, 32)}`);
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

cmd_deploy_live() {
  local tag
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  step "live Worker $LIVE_NAME"
  tag="$(live_tag)" || die "couldn't fingerprint the live Worker"
  say "this release's live Worker: $tag (its bundle and configuration)"
  cf_deployed "$LIVE_NAME"
  if [ "$WORKER_EXISTS" = no ]; then
    say "not deployed yet"
  elif [ "$DEPLOYED_TAGS" = "100 $tag"$'\n' ]; then
    say "unchanged (it serves $tag): not deployed, so the live channels keep their connections"
    report "### Live Worker \`$LIVE_NAME\`

Unchanged (\`$tag\`): not deployed, so phones keep their live connections."
    return 0
  else
    say "it serves: $(printf '%s' "${DEPLOYED_TAGS:-nothing}" | tr '\n' ',' | sed 's/,$//; s/,/, /g')"
  fi
  run_wrangler deploy -c "$LIVE_CONFIG" --tag "$tag" --message "rafi v$VERSION"
  report "### Live Worker \`$LIVE_NAME\`

Deployed (\`$tag\`, Rafi v$VERSION): phones reconnected to their live channels."
}

cmd_upload_app() {
  local out id installed=no
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  step "the legal documents"
  copy_legal "$PWD/legal" "$APP_ASSETS"
  step "app Worker $APP_NAME"
  cf_deployed "$APP_NAME"
  if [ "$WORKER_EXISTS" = no ]; then
    die "the app Worker $APP_NAME doesn't exist on the account: before the first deploy, create it with its secrets (${CLOUDFLARE_ENV:+CLOUDFLARE_ENV=$CLOUDFLARE_ENV }npx wrangler@$WRANGLER_VERSION secret bulk .prod.vars -c app.wrangler.toml, from .prod.vars.example)"
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
    report "### App Worker \`$APP_NAME\`

Version \`$id\` uploaded (Rafi v$VERSION), not serving yet: promote-app deploys it once the engine is registered."
  else
    say "no release ran here yet (a first install: the Worker has only its secrets): deploying v$VERSION directly"
    run_wrangler deploy -c "$APP_CONFIG" --tag "v$VERSION" --message "rafi v$VERSION"
    output app_promoted true
    report "### App Worker \`$APP_NAME\`

A first install: Rafi v$VERSION deployed directly."
  fi
}

cmd_deploy_engine() {
  local subdomain url
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  step "engine Worker $ENGINE_WORKER"
  subdomain="$(cf_subdomain)" || exit 1
  run_wrangler deploy -c "$ENGINE_CONFIG" --name "$ENGINE_WORKER" --tag "v$VERSION" --message "rafi v$VERSION"
  url="https://$ENGINE_WORKER.$subdomain.workers.dev/restate"
  output engine_worker "$ENGINE_WORKER"
  output engine_url "$url"
  report "### Engine Worker \`$ENGINE_WORKER\`

Deployed (Rafi v$VERSION) at $url; register-engine registers it with Restate."
}

# The downgrade guard: reads Restate's services (stdin: GET /services, then a NUL, then GET
# /deployments) and refuses (exit 3, <refusal> in the message) when the latest deployment of one
# of this engine's services is this installation's and of a newer release than <version>.
restate_guard() {
  node -e '
    const [version, instance, account, services, refusal, file] = process.argv.slice(1);
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
        console.error(`error: ${name} runs on Rafi ${m.version} (${s.deployment_id}), newer than ${version}: ${refusal}. Set ${file} to ${m.version} or later.`);
        refused = true;
      }
    }
    process.exit(refused ? 3 : 0);' "$@"
}

# A Restate admin API answer that isn't 200 (restate_failed <what> <response>), for a message.
restate_failed() {
  local status
  status="$(status_of "$2")"
  if [ "$status" = 000 ]; then
    die "$1: no answer from $RESTATE_ADMIN_URL (RESTATE_ADMIN_URL: the environment's admin API, :9070)"
  fi
  die "$1: HTTP $status $(body_of "$2")"
}

# The downgrade guard (refuse_downgrade <what is refused>), check-remote's and register-engine's:
# the latest deployment of each of the engine's services (GET /services) and its metadata (GET
# /deployments) read, and exits 1 when one is this installation's and of a newer release.
refuse_downgrade() {
  local refusal="$1" services deployments status=0
  step "Restate: the release it runs (installation $APP_NAME)"
  services="$(restate_api GET /services)"
  [ "$(status_of "$services")" = 200 ] || restate_failed "Restate's services" "$services"
  deployments="$(restate_api GET /deployments)"
  [ "$(status_of "$deployments")" = 200 ] || restate_failed "Restate's deployments" "$deployments"
  { body_of "$services"; printf '\0'; body_of "$deployments"; } \
    | restate_guard "$VERSION" "$APP_NAME" "$CLOUDFLARE_ACCOUNT_ID" "$ENGINE_SERVICES" "$refusal" "$VERSION_FILE" || status=$?
  case "$status" in
    0) ;;
    3) exit 1 ;;
    *) die "couldn't read Restate's services and deployments" ;;
  esac
}

cmd_check_remote() {
  local subdomain
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID RESTATE_ADMIN_URL RESTATE_AUTH_TOKEN
  refuse_downgrade "refusing to deploy an older release (a downgrade); nothing was deployed"
  step "Cloudflare: the account $CLOUDFLARE_ACCOUNT_ID"
  subdomain="$(cf_subdomain)" || exit 1
  say "the token reads it: workers.dev subdomain $subdomain"
  step "checked: Rafi $VERSION may be deployed here (nothing changed)"
}

cmd_register_engine() {
  local worker url body resp status attempt id wait
  require_env CLOUDFLARE_ACCOUNT_ID RESTATE_ADMIN_URL RESTATE_AUTH_TOKEN RAFI_ENGINE_WORKER RAFI_ENGINE_URL
  worker="$RAFI_ENGINE_WORKER"
  url="$RAFI_ENGINE_URL"
  [ "$worker" = "$ENGINE_WORKER" ] || die "RAFI_ENGINE_WORKER is $worker, but this release's engine Worker is $ENGINE_WORKER"
  [[ "$url" =~ ^https://${worker}\.[a-z0-9-]+\.workers\.dev/restate$ ]] \
    || die "RAFI_ENGINE_URL is $url, not https://$worker.<subdomain>.workers.dev/restate"
  # check-remote refused a downgrade before anything was deployed; again here, for a newer
  # release registered since (another run, by hand).
  refuse_downgrade "refusing to register an older engine (a downgrade)"

  step "Restate: registering $url"
  body="$(node -e '
    const [uri, instance, version, account, worker] = process.argv.slice(1);
    process.stdout.write(JSON.stringify({ uri, force: false,
      metadata: { app: "rafi", instance, version, cloudflare_account: account, worker } }));' \
    "$url" "$APP_NAME" "$VERSION" "$CLOUDFLARE_ACCOUNT_ID" "$worker")"
  wait="${RAFI_REGISTER_WAIT:-10}"
  for ((attempt = 1; attempt <= REGISTER_ATTEMPTS; attempt++)); do
    resp="$(restate_api POST /deployments "$body")"
    status="$(status_of "$resp")"
    case "$status" in
      200 | 201)
        id="$(body_of "$resp" | node -e 'process.stdout.write(String(JSON.parse(require("fs").readFileSync(0, "utf8")).id ?? ""))')" || id=
        [ -n "$id" ] || die "Restate registered $url but answered no deployment ID: $(body_of "$resp")"
        if [ "$status" = 201 ]; then say "registered: Restate deployment $id"; else say "Restate already had it: deployment $id"; fi
        report "### Restate

The engine Worker \`$worker\` (Rafi v$VERSION) is Restate deployment \`$id\`$([ "$status" = 201 ] && echo ", registered now" || echo " (Restate had it already)"): $url"
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
    report "### App Worker \`$APP_NAME\`

Rafi v$VERSION was deployed directly by upload-app (a first install): nothing to promote."
    return 0
  fi
  [ -n "${RAFI_APP_VERSION_ID:-}" ] || die "neither RAFI_APP_PROMOTED=true nor RAFI_APP_VERSION_ID: upload-app's outputs"
  [[ "$RAFI_APP_VERSION_ID" =~ ^[A-Za-z0-9-]+$ ]] || die "RAFI_APP_VERSION_ID is '$RAFI_APP_VERSION_ID', not a version ID"
  run_wrangler versions deploy "$RAFI_APP_VERSION_ID@100%" --yes -c "$APP_CONFIG" --message "rafi v$VERSION"
  step "app Worker $APP_NAME: routes and Cron"
  run_wrangler triggers deploy -c "$APP_CONFIG"
  report "### App Worker \`$APP_NAME\`

Version \`$RAFI_APP_VERSION_ID\` (Rafi v$VERSION) deployed at 100%, with its routes and Cron."
}

# Restate's deployments and the account's Workers for the cleanup: reads GET /deployments, a
# NUL, the CLI's `deployments list --extra --json`, a NUL, then the account's Workers' names (one
# per line), and prints one tab-separated line per deployment and per leftover Worker:
#   ours <status> <id> <worker> <active invocations>
#   other <id> <address> <why>
#   leftover <worker>      an <engine name>-v<version> Worker no Restate deployment refers to
cleanup_plan() {
  node -e '
    const [base, instance, account] = process.argv.slice(1);
    const [adminJson, cliJson, workersText] = require("fs").readFileSync(0, "utf8").split("\0");
    const admin = JSON.parse(adminJson).deployments;
    // The CLI may say something before its JSON.
    const cli = JSON.parse(cliJson.slice(cliJson.indexOf("{"))).deployments;
    if (!Array.isArray(admin) || !Array.isArray(cli)) throw new Error("no deployments in a listing");
    const status = new Map(cli.map((d) => [d.deployment_id ?? d.id, d]));
    const clean = (s) => String(s ?? "").replace(/[\t\n\r]/g, " ");
    const referred = new Set();
    for (const d of admin) {
      if (!/^dp_[A-Za-z0-9]+$/.test(d.id ?? "")) throw new Error(`a deployment ID: ${d.id}`);
      const m = d.metadata ?? {};
      const address = clean(d.uri ?? d.arn ?? "?");
      const host = /^https:\/\/([a-z0-9-]+)\./.exec(d.uri ?? "");
      if (host) referred.add(host[1]);
      let why = null;
      if (m.app !== "rafi") why = "not a Rafi deployment (no app=rafi metadata)";
      else if (m.instance !== instance) why = `another installation (${clean(m.instance)})`;
      else if (m.cloudflare_account !== account) why = "another Cloudflare account";
      else if (typeof m.worker !== "string" || !m.worker.startsWith(`${base}-v`) || !/^[a-z0-9-]{1,63}$/.test(m.worker)) why = `its worker is not a ${base}-v... engine Worker (${clean(m.worker)})`;
      else if (!new RegExp(`^https://${m.worker}\\.[a-z0-9-]+\\.workers\\.dev/restate/?$`).test(d.uri ?? "")) why = `its address is not the workers.dev one of ${m.worker}`;
      if (why) { console.log(["other", d.id, address, why].join("\t")); continue; }
      referred.add(m.worker);
      const s = status.get(d.id);
      console.log(["ours", s?.status ?? "Unknown", d.id, m.worker, s?.active_invocations ?? "?"].join("\t"));
    }
    const escaped = base.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const engine = new RegExp(`^${escaped}-v[0-9]+-[0-9]+-[0-9]+(-[a-z0-9-]+)?$`);
    for (const w of (workersText ?? "").split("\n").map((l) => l.trim()).filter(Boolean).sort()) {
      if (engine.test(w) && !referred.has(w)) console.log(["leftover", w].join("\t"));
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
  local admin list workers plan kind a b c d resp status pinned paused failed=0
  local removed=() draining=() behind=() refused=() others=() leftovers=() text line
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID RESTATE_ADMIN_URL RESTATE_AUTH_TOKEN
  step "Restate's deployments and the account's Workers (installation $APP_NAME)"
  admin="$(restate_api GET /deployments)"
  [ "$(status_of "$admin")" = 200 ] \
    || die "listing Restate's deployments: HTTP $(status_of "$admin") $(body_of "$admin"): nothing removed or deleted"
  list="$(run_restate deployments list --extra --json)" || die "restate deployments list failed: nothing removed or deleted"
  workers="$(cf_workers)" || die "the account's Workers couldn't be listed: nothing removed or deleted"
  plan="$({ body_of "$admin"; printf '\0'; printf '%s' "$list"; printf '\0'; printf '%s' "$workers"; } \
    | cleanup_plan "$ENGINE_BASE" "$APP_NAME" "$CLOUDFLARE_ACCOUNT_ID")" \
    || die "couldn't read Restate's deployments: nothing removed or deleted"
  [ -n "$plan" ] || say "none"
  while IFS=$'\t' read -r kind a b c d; do
    case "$kind" in
      ours) ;;
      other)
        say "$a at $b: $c; never touched"
        others+=("\`$a\` at $b: $c")
        continue
        ;;
      leftover)
        if [ "$a" = "$ENGINE_WORKER" ]; then
          say "the Worker $a, this release's engine, isn't registered with Restate: a deploy registers it (register-engine)"
          continue
        fi
        say "the Worker $a: no Restate deployment refers to it; left as it is"
        leftovers+=("\`$a\`: no Restate deployment refers to it")
        continue
        ;;
      *) continue ;;
    esac
    # a: status, b: deployment, c: Worker, d: active invocations.
    case "$a" in
      Drained)
        say "$b ($c) is drained: removing the deployment, then deleting the Worker"
        if ! run_restate deployments remove "$b" --yes; then
          warn "Restate didn't remove $b: its Worker $c is left as it is; the next cleanup tries again"
          refused+=("\`$b\` (\`$c\`): Restate refused to remove it; its Worker is untouched, and the next cleanup tries again")
          failed=1
          continue
        fi
        say "  removed the deployment $b"
        resp="$(cf_api DELETE "/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/scripts/$c")"
        status="$(status_of "$resp")"
        case "$status" in
          200)
            say "  deleted the Worker $c"
            removed+=("\`$b\`: removed, its Worker \`$c\` deleted")
            ;;
          404)
            say "  the Worker $c is already gone"
            removed+=("\`$b\`: removed, its Worker \`$c\` already gone")
            ;;
          *)
            warn "deleting the Worker $c: HTTP $status $(body_of "$resp" | cf_errors): it is left behind (its deployment $b is removed)"
            removed+=("\`$b\`: removed, but its Worker \`$c\` wasn't deleted (below)")
            behind+=("\`$c\`: its deployment \`$b\` is removed, but deleting it failed (HTTP $status $(body_of "$resp" | cf_errors))")
            failed=1
            ;;
        esac
        ;;
      Draining)
        paused="?"
        text=
        if pinned="$(pinned_invocations "$b")"; then
          printf '%s\n' "$pinned" | sed '/^$/d; s/^/      /'
          paused="$(grep -c ' paused$' <<<"$pinned" || true)"
          text="$(sed -n 's/^\([^ ]*\) paused$/`\1`/p' <<<"$pinned" | paste -sd ' ' -)"
        fi
        say "$b ($c) is draining: $d active, $paused paused invocation(s) pinned to it"
        line="\`$b\` (\`$c\`): $d active, $paused paused invocation(s)"
        [ -z "$text" ] || line="$line; paused: $text"
        draining+=("$line")
        ;;
      Active) say "$b ($c) is active (the latest for its services): kept" ;;
      *) say "$b ($c) is $a: left as it is" ;;
    esac
  done <<<"$plan"

  text="### Restate cleanup: \`$APP_NAME\` (Rafi v$VERSION)"
  text+=$'\n\n'"**Removed** (drained: the Restate deployment removed, then its Worker deleted):"
  if [ "${#removed[@]}" -eq 0 ]; then text+=$'\n'"- none"; else for line in "${removed[@]}"; do text+=$'\n'"- $line"; done; fi
  if [ "${#refused[@]}" -gt 0 ]; then
    text+=$'\n\n'"**Not removed**:"
    for line in "${refused[@]}"; do text+=$'\n'"- $line"; done
  fi
  text+=$'\n\n'"**Still draining** (a later cleanup removes them once drained):"
  if [ "${#draining[@]}" -eq 0 ]; then
    text+=$'\n'"- none"
  else
    for line in "${draining[@]}"; do text+=$'\n'"- $line"; done
    text+=$'\n\n'"A paused invocation waits for you: resume it on the latest deployment (\`restate invocations resume <id> --deployment latest\`) or kill it (\`restate invocations kill <id>\`)."
  fi
  text+=$'\n\n'"**Workers left behind** (engine Workers no Restate deployment refers to any more: the cleanup neither deletes them nor retries a failed deletion; delete one by hand with \`CLOUDFLARE_API_TOKEN=... CLOUDFLARE_ACCOUNT_ID=... npx wrangler@$WRANGLER_VERSION delete <worker>\`, or on the dashboard):"
  if [ "${#behind[@]}" -eq 0 ] && [ "${#leftovers[@]}" -eq 0 ]; then
    text+=$'\n'"- none"
  else
    for line in ${behind[@]+"${behind[@]}"} ${leftovers[@]+"${leftovers[@]}"}; do text+=$'\n'"- $line"; done
  fi
  text+=$'\n\n'"**Not this installation's** (never touched):"
  if [ "${#others[@]}" -eq 0 ]; then text+=$'\n'"- none"; else for line in "${others[@]}"; do text+=$'\n'"- $line"; done; fi
  report "$text"

  [ "$failed" = 0 ] || die "the cleanup failed in part (above); ${#removed[@]} drained deployment(s) removed"
  step "cleaned up: ${#removed[@]} drained engine deployment(s) removed and their Workers deleted"
}

cmd_dry_run() {
  step "the legal documents"
  copy_legal "$PWD/legal" "$APP_ASSETS"
  step "live Worker $LIVE_NAME: wrangler deploy --dry-run"
  run_wrangler deploy --dry-run -c "$LIVE_CONFIG"
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
    check | check-remote | migrate | deploy-live | upload-app | deploy-engine | register-engine | promote-app | cleanup) ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  # wrangler reads CLOUDFLARE_ENV itself: the same environment for every call, an empty one none.
  if [ -n "${CLOUDFLARE_ENV:-}" ]; then
    [[ "$CLOUDFLARE_ENV" =~ ^[A-Za-z0-9_-]+$ ]] || die "CLOUDFLARE_ENV is '$CLOUDFLARE_ENV', not an environment's name"
    export CLOUDFLARE_ENV
  else
    unset CLOUDFLARE_ENV
  fi
  check_release "$strict"
  "cmd_${command//-/_}"
}

main "$@"
