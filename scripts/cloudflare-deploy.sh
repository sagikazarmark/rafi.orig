#!/usr/bin/env bash
# Deploys a release of Rafi to Cloudflare, one step per subcommand: the live Worker (the
# raffles' RaffleRoom Durable Objects), the app Worker and the release's own raffle engine
# Worker, registered with Restate; and cleans up the engine Workers Restate has drained. Each
# release ships this file as its deploy.sh (the source is scripts/cloudflare-deploy.sh in the
# Rafi repository), so the deploy logic is the release's own, pinned to it and covered by its
# provenance attestation. An installation's workflows (the deploy template's
# .github/workflows: deploy.yaml and restate-cleanup.yaml) download and verify the release, unpack it into
# release/ and run these subcommands, one step each; nothing here is specific to GitHub
# Actions.
#
# Usage, from the root of an installation's repository (created from the deploy template: VERSION,
# package.json, wrangler.app.toml, wrangler.engine.toml, wrangler.live.toml, and legal/ with the
# operator's terms.md and privacy.md, either or both, if they provide any), with the
# release unpacked into release/ (the directory this script is in is the release; the
# configurations point into it as release/...):
#
#   release/deploy.sh check-remote      no downgrade (Restate's release) and the token works; reads only
#   release/deploy.sh migrate           the D1 migrations
#   release/deploy.sh deploy-live       the live Worker, unless its code and configuration are unchanged
#   release/deploy.sh upload-app        the app Worker's version uploaded (deployed directly on a first install)
#   release/deploy.sh deploy-engine     this release's engine Worker, <engine name>-v<version>
#   release/deploy.sh register-engine   the engine Worker registered with Restate (never a downgrade)
#   release/deploy.sh promote-app       the uploaded app version deployed, then its address and Cron
#   release/deploy.sh cleanup           drained engine deployments removed from Restate, then their Workers deleted
#   release/deploy.sh dry-run           legal/ copied in, `wrangler deploy --dry-run` of the three Workers; nothing remote
#
# Environment:
#   CLOUDFLARE_ENV        optional: the Wrangler environment ([env.<name>] in each configuration)
#                         this installation is, for several installations in one repository;
#                         wrangler reads it too, so every wrangler call here deploys that
#                         environment's Workers. Unset or empty: the configurations' top level.
#   CLOUDFLARE_API_TOKEN, CLOUDFLARE_ACCOUNT_ID   the account; all but dry-run
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
#   RAFI_ARTIFACTS_DIR    when set, the JSON a subcommand reads and writes is kept in
#                         <it>/<subcommand>/ (replaced on each run): wrangler's ND-JSON output of
#                         each wrangler call (wrangler-<what>.ndjson), Restate's services and
#                         deployments (restate-services.json, restate-deployments.json, the
#                         CLI's restate-deployments-list.json), the registration's answer
#                         (restate-register.json), the account's Workers
#                         (cloudflare-workers.json) and the cleanup's summary (cleanup.json).
#                         Never a token: the tokens reach curl on its standard input only.
#   WRANGLER              the wrangler command (default: npx -y wrangler, the installation's
#                         package-lock.json's once `npm ci` installed it)
#   RESTATE               the Restate CLI command (default: npx -y @restatedev/restate, likewise)
#   CLOUDFLARE_API_BASE_URL   the Cloudflare API (wrangler's variable too; for tests)
#   RAFI_REGISTER_WAIT    seconds between registration attempts (default 10; for tests)
#
# Needs bash, curl, jq, and node with npx (for wrangler and the Restate CLI).
#
# What each does (each prints its steps):
#   Every subcommand first reads the release's VERSION (this directory's: x.y.z, optionally
#   -pre.release, a leading v dropped), the version it deploys: the engine Worker's name and
#   the version in Restate's metadata are this. Nothing compares it with the installation's
#   version file (the workflows chose the release to download by it), checks that the release
#   or the configurations have their files, or searches for the template's placeholders: a
#   missing file or a placeholder left fails where wrangler or Cloudflare meets it. Then, but
#   for migrate, the Workers' names: `wrangler deploy --dry-run` of each configuration
#   (wrangler.live.toml, wrangler.engine.toml, wrangler.app.toml, for CLOUDFLARE_ENV's
#   environment: wrangler refuses a configuration that doesn't deploy, an environment it lacks
#   among others), each name the worker_name of the dry run's deploy entry in wrangler's
#   ND-JSON output; this release's engine Worker is <engine name>-v<version>. Nothing compares
#   the configurations with each other: one database_id and one RAFI_DO_JURISDICTION in all
#   three, RAFFLE_ROOM's script_name the live Worker's name and the engine's identity keys in
#   every environment are the installation's to keep (the first test-mode raffle after an
#   install shows a mismatch).
#   The installation is its app Worker's name (unique in the account): Restate's deployment
#   metadata names it as `instance`, and the downgrade guard and the cleanup touch only that
#   instance's deployments in this account.
#   check-remote: what can stop a deploy, checked before it changes anything (the workflow's
#     first deploy step): the downgrade guard (register-engine's, below: Restate runs no newer
#     release of this installation's; none at all on a first install), then the token reads the
#     account (its workers.dev subdomain, which deploy-engine needs). Restate unreachable, or a
#     key or token that doesn't work, fails it. Reads only.
#   migrate: `wrangler d1 migrations apply DB --remote` (the app's configuration).
#   deploy-live: the live Worker's fingerprint is the sha256 of its bundle's files
#     (release/live/build), wrangler.live.toml's bytes and CLOUDFLARE_ENV; its tag,
#     live-<32 hex digits of it>, is the version annotation (workers/tag) of each live Worker
#     version this deploys. When the Worker's current deployment is one version at 100% with
#     that tag (Cloudflare's API: the script's latest deployment, then that version's
#     annotations), nothing is deployed: the live channels' connections are kept. Otherwise
#     `wrangler deploy` (the class's Durable Object migrations need it); any edit of
#     wrangler.live.toml, a comment too, deploys it.
#   upload-app: legal/terms.md and legal/privacy.md, those there are, copied into the app's
#     static files (release/app/assets/legal/, replacing what a run before copied); then, if
#     the app Worker has ever run a release's code (a version tagged v<version>, in its
#     current deployment or among its recent versions),
#     `wrangler versions upload --tag v<version>`, the new version not serving yet: output
#     app_version_id (from wrangler's ND-JSON output). Otherwise, a first install (the Worker
#     holds only the secrets `wrangler secret put`/`bulk` created it with): `wrangler deploy`
#     (its address and Cron too), output app_promoted=true. A Worker that doesn't exist is
#     refused: its secrets come first.
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
#     <app_version_id>@100%`, then `wrangler triggers deploy` (the Worker's address, workers.dev
#     on or off and the routes, none by default, and the Cron, which a version upload doesn't
#     apply).
#   cleanup: Restate's deployments with their metadata (GET /deployments) and their status
#     (`restate deployments list --extra --json`), joined by ID, and the account's Workers (GET
#     /accounts/{id}/workers/scripts); any listing failing stops it before anything is removed
#     or deleted. Only this installation's deployments (the metadata above, its worker <engine
#     name>-v..., its address that Worker's workers.dev one) are touched. Each Drained one: the
#     deployment removed from Restate (`restate deployments remove`, which refuses one not
#     drained; never forced), then its Worker deleted (DELETE
#     /accounts/{id}/workers/scripts/<worker>; 404 is already gone); a removal refused leaves
#     the Worker alone, a deletion failing leaves the Worker behind (reported; the run fails).
#     Reported (and summarized in cleanup.json): what was removed and deleted; those still
#     draining, with their active and paused invocations (a paused one waits for a human:
#     resume it on the latest deployment or kill it); the Workers left behind, those whose
#     deletion failed and the account's <engine name>-v... Workers none of this installation's
#     Restate deployments refers to (by its address or its metadata's worker; another
#     installation's deployment doesn't count; this release's own engine, not registered yet,
#     excepted; never deleted here: delete them by hand); and every deployment that isn't
#     this installation's (never touched).
#   dry-run: legal/ copied in, and the three `wrangler deploy --dry-run` that read the names
#     (the live Worker, the engine Worker, the app Worker); no credentials.
#
# Exit status: 0 on success, 1 on a failure or a refusal, 2 on a usage error. Every subcommand
# can be run again: a failed one is retried by running it again.

# The jq programs below are single-quoted: their $... are jq's.
# shellcheck disable=SC2016
set -euo pipefail

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

# The tools: WRANGLER_CMD and RESTATE_CMD, each a command with its first arguments. npx -y runs
# the version package-lock.json pinned (npm ci, from the installation's root or
# below), and fetches the latest only when none is installed.
tools() {
  read -r -a WRANGLER_CMD <<<"${WRANGLER:-npx -y wrangler}"
  read -r -a RESTATE_CMD <<<"${RESTATE:-npx -y @restatedev/restate}"
}

# wrangler, its ND-JSON output kept (run_wrangler <what> <args...>: wrangler-<what>.ndjson among
# the artifacts; wrangler appends, so the file is emptied first).
run_wrangler() {
  local file
  file="$(wrangler_output "$1")"
  shift
  : >"$file"
  WRANGLER_OUTPUT_FILE_PATH="$file" WRANGLER_SEND_METRICS=false command "${WRANGLER_CMD[@]}" "$@"
}
wrangler_output() { printf '%s\n' "$ARTIFACTS/wrangler-$1.ndjson"; }

run_restate() {
  SCARF_ANALYTICS=false command "${RESTATE_CMD[@]}" "$@"
}

# keep <file name> <JSON>: a document among the artifacts.
keep() { printf '%s\n' "$2" >"$ARTIFACTS/$1"; }

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
# A Worker's name is also its workers.dev host label: [a-z0-9-], at most 63 characters.
valid_worker_name() { [[ "$1" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]]; }

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

# Compares two non-negative integers of any length (num_cmp <a> <b>): prints -1, 0 or 1.
num_cmp() {
  local LC_ALL=C a="${1#"${1%%[!0]*}"}" b="${2#"${2%%[!0]*}"}"
  if [ "${#a}" -ne "${#b}" ]; then
    if [ "${#a}" -lt "${#b}" ]; then echo -1; else echo 1; fi
  elif [[ "$a" < "$b" ]]; then
    echo -1
  elif [[ "$a" > "$b" ]]; then
    echo 1
  else
    echo 0
  fi
}

# Semantic versioning precedence (semver_cmp <a> <b>, both valid_version): prints -1, 0 or 1.
# The core numbers, then a pre-release before its release, pre-release identifiers numeric (by
# value) before alphanumeric (ASCII), fewer before more.
semver_cmp() {
  local LC_ALL=C a="$1" b="$2" ap="" bp="" i c x y
  local -a ac bc ai bi
  [[ "$a" != *-* ]] || ap="${a#*-}"
  [[ "$b" != *-* ]] || bp="${b#*-}"
  IFS=. read -r -a ac <<<"${a%%-*}"
  IFS=. read -r -a bc <<<"${b%%-*}"
  for i in 0 1 2; do
    c="$(num_cmp "${ac[i]}" "${bc[i]}")"
    [ "$c" = 0 ] || { echo "$c"; return; }
  done
  if [ -z "$ap" ] && [ -z "$bp" ]; then echo 0; return; fi
  [ -n "$ap" ] || { echo 1; return; }
  [ -n "$bp" ] || { echo -1; return; }
  IFS=. read -r -a ai <<<"$ap"
  IFS=. read -r -a bi <<<"$bp"
  for ((i = 0; ; i++)); do
    if [ "$i" -ge "${#ai[@]}" ] && [ "$i" -ge "${#bi[@]}" ]; then echo 0; return; fi
    [ "$i" -lt "${#ai[@]}" ] || { echo -1; return; }
    [ "$i" -lt "${#bi[@]}" ] || { echo 1; return; }
    x="${ai[i]}"
    y="${bi[i]}"
    if [[ "$x" =~ ^[0-9]+$ && "$y" =~ ^[0-9]+$ ]]; then
      c="$(num_cmp "$x" "$y")"
    elif [[ "$x" =~ ^[0-9]+$ ]]; then
      c=-1
    elif [[ "$y" =~ ^[0-9]+$ ]]; then
      c=1
    elif [[ "$x" < "$y" ]]; then
      c=-1
    elif [[ "$x" > "$y" ]]; then
      c=1
    else
      c=0
    fi
    [ "$c" = 0 ] || { echo "$c"; return; }
  done
}

# The engine Worker of a release: <engine name>-v<version, dots as dashes, lower case>.
engine_worker_name() {
  local base="$1" version="$2" name
  name="$base-v$(printf '%s' "$version" | tr '.' '-' | tr '[:upper:]' '[:lower:]')"
  [ "${#name}" -le 63 ] || die "the engine Worker's name '$name' is longer than 63 characters: shorten the engine's name"
  valid_worker_name "$name" || die "the engine Worker's name '$name' isn't a valid Worker name"
  printf '%s\n' "$name"
}

# The release (read_release): its VERSION, the version this deploys, nothing compared with it.
# Sets RELEASE, VERSION and the configurations' paths.
read_release() {
  RELEASE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
  APP_CONFIG="$PWD/wrangler.app.toml"
  ENGINE_CONFIG="$PWD/wrangler.engine.toml"
  LIVE_CONFIG="$PWD/wrangler.live.toml"
  step "release $RELEASE"
  VERSION="$(read_version "$RELEASE/VERSION" "is $RELEASE an unpacked Rafi release?")" || exit 1
  say "Rafi $VERSION (the release's VERSION)"
  if [ -n "${CLOUDFLARE_ENV:-}" ]; then
    say "environment: $CLOUDFLARE_ENV ([env.$CLOUDFLARE_ENV] of each configuration)"
  else
    say "environment: none (the configurations' top level)"
  fi
}

# The Workers' names (read_names): `wrangler deploy --dry-run` of each configuration, for
# CLOUDFLARE_ENV's environment (which checks it as a deploy would, uploading nothing), and the
# worker_name of the dry run's deploy entry in wrangler's ND-JSON output. Sets LIVE_NAME,
# ENGINE_BASE, APP_NAME and ENGINE_WORKER (this release's engine Worker).
read_names() {
  local key config file name
  for key in live engine app; do
    case "$key" in
      live) config="$LIVE_CONFIG" ;;
      engine) config="$ENGINE_CONFIG" ;;
      *) config="$APP_CONFIG" ;;
    esac
    step "$key Worker ($(basename "$config")): wrangler deploy --dry-run"
    run_wrangler "dry-run-$key" deploy -c "$config" --dry-run \
      || die "wrangler deploy --dry-run -c $(basename "$config")${CLOUDFLARE_ENV:+ (CLOUDFLARE_ENV=$CLOUDFLARE_ENV)} failed (above): the configuration doesn't deploy"
    file="$(wrangler_output "dry-run-$key")"
    name="$(jq -r 'select(.type == "deploy") | .worker_name // empty' "$file" | tail -n 1)" || name=
    valid_worker_name "$name" \
      || die "wrangler's dry run of $(basename "$config") reported no Worker's name (its deploy entry's worker_name: '$name')"
    case "$key" in
      live) LIVE_NAME="$name" ;;
      engine) ENGINE_BASE="$name" ;;
      *) APP_NAME="$name" ;;
    esac
  done
  ENGINE_WORKER="$(engine_worker_name "$ENGINE_BASE" "$VERSION")" || exit 1
  step "the Workers (their names from wrangler's dry runs)"
  say "live Worker: $LIVE_NAME ($LIVE_CONFIG)"
  say "app Worker: $APP_NAME ($APP_CONFIG; the installation's name in Restate)"
  say "engine Worker: $ENGINE_WORKER ($ENGINE_CONFIG, named $ENGINE_BASE there)"
}

copy_legal() {
  local from="$1" to="$2/legal" doc copied=()
  # What a run before this one copied goes first: a document the installation has since removed
  # is gone from the app too.
  rm -rf "$to"
  for doc in terms.md privacy.md; do
    [ -f "$from/$doc" ] || continue
    mkdir -p "$to"
    cp "$from/$doc" "$to/"
    copied+=("$doc")
  done
  if [ "${#copied[@]}" -eq 0 ]; then
    say "no legal documents in $from: the app's /terms and /privacy say the operator hasn't provided them"
  else
    say "copied ${copied[*]} from $from into $to"
  fi
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
  jq -r '[(.errors // [])[] | "\(.code): \(.message)"] | join("; ")' 2>/dev/null || true
}

# Succeeds when a 404's body (stdin) says there is no such Worker (10007, as wrangler checks it;
# 10090), not another 404 (a wrong account, a wrong path).
cf_no_such_worker() {
  jq -e 'any((.errors // [])[]; .code == 10007 or .code == 10090)' >/dev/null 2>&1
}

# Prints the account's workers.dev subdomain.
cf_subdomain() {
  local resp sub
  resp="$(cf_api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/subdomain")"
  if [ "$(status_of "$resp")" = 200 ]; then
    sub="$(body_of "$resp" | jq -r '.result.subdomain // empty' 2>/dev/null)" || sub=
    if [[ "$sub" =~ ^[a-z0-9][a-z0-9-]*$ ]]; then
      printf '%s\n' "$sub"
      return 0
    fi
  fi
  die "the account's workers.dev subdomain: HTTP $(status_of "$resp") $(body_of "$resp" | cf_errors). An account without one registers it at https://dash.cloudflare.com/$CLOUDFLARE_ACCOUNT_ID/workers/onboarding"
}

# The account's Workers, kept as cloudflare-workers.json: GET /accounts/{id}/workers/scripts
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
  keep cloudflare-workers.json "$(body_of "$resp")"
  jq -e '(.result | type) == "array"
    and all(.result[]; (.id | type) == "string" and (.id | test("^[A-Za-z0-9_-]+$")))' \
    "$ARTIFACTS/cloudflare-workers.json" >/dev/null 2>&1 || {
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
  versions="$(body_of "$resp" | jq -r '.result.deployments
    | if type == "array" then (.[0].versions // [])[] | "\(.percentage)\t\(.version_id)"
      else error("no deployments in the answer") end')" \
    || die "couldn't read the Worker $worker's deployments"
  while IFS=$'\t' read -r pct id; do
    [ -n "$id" ] || continue
    [[ "$id" =~ ^[A-Za-z0-9-]+$ ]] || die "the Worker $worker's deployment has a version '$id'"
    resp="$(cf_api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/scripts/$worker/versions/$id")"
    [ "$(status_of "$resp")" = 200 ] \
      || die "the Worker $worker's version $id: HTTP $(status_of "$resp") $(body_of "$resp" | cf_errors)"
    tag="$(body_of "$resp" | jq -r '.result.annotations["workers/tag"] // "" | tostring | gsub("\\s"; "")')" \
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
  body_of "$resp" | jq -r '(.result.items // .result)
    | if type != "array" then error("no versions in the answer")
      elif any(.[]; (.annotations["workers/tag"] // "") | tostring | test("^v[0-9]")) then "yes"
      else "no" end' \
    || die "couldn't read the Worker $worker's versions"
}

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum; else shasum -a 256; fi | cut -d' ' -f1
}

# The live Worker's tag: live-<the first 32 hex digits of the sha256 of its bundle's files (each
# one's path, size and content, in path order), wrangler.live.toml (its size and bytes) and
# CLOUDFLARE_ENV>.
live_tag() {
  local build="$RELEASE/live/build" rel hash
  hash="$(
    {
      while IFS= read -r rel; do
        printf 'build/%s\0%s\0' "$rel" "$(wc -c <"$build/$rel" | tr -d '[:space:]')"
        cat "$build/$rel"
      done < <(cd "$build" && find . -type f -print | sed 's|^\./||' | LC_ALL=C sort)
      printf 'wrangler.live.toml\0%s\0' "$(wc -c <"$LIVE_CONFIG" | tr -d '[:space:]')"
      cat "$LIVE_CONFIG"
      printf 'CLOUDFLARE_ENV\0%s\0' "${CLOUDFLARE_ENV:-}"
    } | sha256
  )"
  [[ "$hash" =~ ^[0-9a-f]{64}$ ]] || return 1
  printf 'live-%s\n' "${hash:0:32}"
}

cmd_migrate() {
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  step "D1 migrations"
  run_wrangler d1-migrations d1 migrations apply DB -c "$APP_CONFIG" --remote
}

cmd_deploy_live() {
  local tag
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  step "live Worker $LIVE_NAME"
  tag="$(live_tag)" || die "couldn't fingerprint the live Worker"
  say "this release's live Worker: $tag (its bundle and wrangler.live.toml)"
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
  run_wrangler deploy-live deploy -c "$LIVE_CONFIG" --tag "$tag" --message "rafi v$VERSION"
  report "### Live Worker \`$LIVE_NAME\`

Deployed (\`$tag\`, Rafi v$VERSION): phones reconnected to their live channels."
}

cmd_upload_app() {
  local id installed=no
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID
  step "the legal documents"
  copy_legal "$PWD/legal" "$RELEASE/app/assets"
  step "app Worker $APP_NAME"
  cf_deployed "$APP_NAME"
  if [ "$WORKER_EXISTS" = no ]; then
    die "the app Worker $APP_NAME doesn't exist on the account: before the first deploy, create it with its secrets (${CLOUDFLARE_ENV:+CLOUDFLARE_ENV=$CLOUDFLARE_ENV }npx -y wrangler secret bulk .dev.vars -c wrangler.app.toml, from .dev.vars.example)"
  fi
  # A release's code has run here when a version tagged v<version> is deployed, or was (a secret
  # changed since makes a new version, untagged).
  if grep -qE '^[0-9.]+ v[0-9]' <<<"$DEPLOYED_TAGS"; then
    installed=yes
  else
    installed="$(cf_release_version_among_recent "$APP_NAME")" || exit 1
  fi
  if [ "$installed" = yes ]; then
    say "a release ran here before: uploading v$VERSION as a version, deployed once the engine is registered (promote-app)"
    run_wrangler versions-upload-app versions upload -c "$APP_CONFIG" --tag "v$VERSION" --message "rafi v$VERSION"
    id="$(jq -r 'select(.type == "version-upload") | .version_id // empty' "$(wrangler_output versions-upload-app)" \
      | tail -n 1)" || id=
    [[ "$id" =~ ^[A-Za-z0-9-]+$ ]] \
      || die "wrangler versions upload reported no version ID (its output: $(tr '\n' ' ' <"$(wrangler_output versions-upload-app)"))"
    output app_version_id "$id"
    report "### App Worker \`$APP_NAME\`

Version \`$id\` uploaded (Rafi v$VERSION), not serving yet: promote-app deploys it once the engine is registered."
  else
    say "no release ran here yet (a first install: the Worker has only its secrets): deploying v$VERSION directly"
    run_wrangler deploy-app deploy -c "$APP_CONFIG" --tag "v$VERSION" --message "rafi v$VERSION"
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
  run_wrangler deploy-engine deploy -c "$ENGINE_CONFIG" --name "$ENGINE_WORKER" --tag "v$VERSION" --message "rafi v$VERSION"
  url="https://$ENGINE_WORKER.$subdomain.workers.dev/restate"
  output engine_worker "$ENGINE_WORKER"
  output engine_url "$url"
  report "### Engine Worker \`$ENGINE_WORKER\`

Deployed (Rafi v$VERSION) at $url; register-engine registers it with Restate."
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

# The latest deployment of each of the engine's services (restate_guard_rows <GET /services>
# <GET /deployments>, files), one tab-separated line each: <service> none (not registered),
# <service> other <deployment> (not this installation's), or <service> ours <deployment>
# <its metadata version>.
restate_guard_rows() {
  jq -r --arg services "$ENGINE_SERVICES" --arg instance "$APP_NAME" --arg account "$CLOUDFLARE_ACCOUNT_ID" \
    --slurpfile deployments "$2" '
    .services as $svc
    | ($deployments[0].deployments) as $deps
    | if ($svc | type) != "array" or ($deps | type) != "array"
      then error("no services or deployments in the answers") else . end
    | $services | split(" ")[] as $name
    | ([$svc[] | select(.name == $name)] | first) as $s
    | if $s == null then [$name, "none"]
      else ([$deps[] | select(.id == $s.deployment_id)] | first | .metadata // {}) as $m
      | if $m.app == "rafi" and $m.instance == $instance and $m.cloudflare_account == $account
        then [$name, "ours", ($s.deployment_id | tostring), ($m.version // "" | tostring)]
        else [$name, "other", ($s.deployment_id | tostring)] end
      end
    | @tsv' "$1"
}

# The downgrade guard (refuse_downgrade <what is refused>), check-remote's and register-engine's:
# the latest deployment of each of the engine's services (GET /services) and its metadata (GET
# /deployments) read, and exits 1 when one is this installation's and of a newer release.
refuse_downgrade() {
  local refusal="$1" resp rows name kind id version c refused=0 file=VERSION
  # The version file the workflows read: VERSION.<CLOUDFLARE_ENV> when it exists, VERSION otherwise.
  [ -z "${CLOUDFLARE_ENV:-}" ] || file="VERSION.$CLOUDFLARE_ENV (VERSION without one)"
  step "Restate: the release it runs (installation $APP_NAME)"
  resp="$(restate_api GET /services)"
  [ "$(status_of "$resp")" = 200 ] || restate_failed "Restate's services" "$resp"
  keep restate-services.json "$(body_of "$resp")"
  resp="$(restate_api GET /deployments)"
  [ "$(status_of "$resp")" = 200 ] || restate_failed "Restate's deployments" "$resp"
  keep restate-deployments.json "$(body_of "$resp")"
  rows="$(restate_guard_rows "$ARTIFACTS/restate-services.json" "$ARTIFACTS/restate-deployments.json")" \
    || die "couldn't read Restate's services and deployments"
  while IFS=$'\t' read -r name kind id version; do
    case "$kind" in
      none) say "$name: not registered yet" ;;
      other) say "$name: on $id, not a deployment of this installation (no downgrade check)" ;;
      ours)
        valid_version "$version" || die "couldn't read Restate's services and deployments: $id's metadata version is '$version'"
        c="$(semver_cmp "$VERSION" "$version")"
        if [ "$c" = 0 ]; then
          say "$name: on $id, Rafi $version (this release: a re-run)"
        else
          say "$name: on $id, Rafi $version"
        fi
        if [ "$c" = -1 ]; then
          printf 'error: %s runs on Rafi %s (%s), newer than %s: %s. Set %s to %s or later.\n' \
            "$name" "$version" "$id" "$VERSION" "$refusal" "$file" "$version" >&2
          refused=1
        fi
        ;;
    esac
  done <<<"$rows"
  [ "$refused" = 0 ] || exit 1
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
  body="$(jq -nc --arg uri "$url" --arg instance "$APP_NAME" --arg version "$VERSION" \
    --arg account "$CLOUDFLARE_ACCOUNT_ID" --arg worker "$worker" \
    '{ uri: $uri, force: false,
       metadata: { app: "rafi", instance: $instance, version: $version, cloudflare_account: $account, worker: $worker } }')"
  wait="${RAFI_REGISTER_WAIT:-10}"
  for ((attempt = 1; attempt <= REGISTER_ATTEMPTS; attempt++)); do
    resp="$(restate_api POST /deployments "$body")"
    status="$(status_of "$resp")"
    keep restate-register.json "$(body_of "$resp")"
    case "$status" in
      200 | 201)
        id="$(body_of "$resp" | jq -r '.id // empty | tostring')" || id=
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
  run_wrangler versions-deploy-app versions deploy "$RAFI_APP_VERSION_ID@100%" --yes -c "$APP_CONFIG" --message "rafi v$VERSION"
  step "app Worker $APP_NAME: its address (workers.dev or its routes) and Cron"
  run_wrangler triggers-deploy-app triggers deploy -c "$APP_CONFIG"
  report "### App Worker \`$APP_NAME\`

Version \`$RAFI_APP_VERSION_ID\` (Rafi v$VERSION) deployed at 100%, with its address (workers.dev or its routes) and Cron."
}

# The cleanup's plan from its listings among the artifacts (restate-deployments.json: GET
# /deployments; restate-deployments-list.json: the CLI's `deployments list --extra --json`;
# cloudflare-workers.json: the account's Workers): one tab-separated line per deployment and
# per leftover Worker:
#   ours <status> <id> <worker> <active invocations>
#   other <id> <address> <why>
#   leftover <worker>      an <engine name>-v<version> Worker none of this installation's Restate
#                          deployments (app rafi, instance the app Worker's name, this account)
#                          refers to by its address or its metadata's worker; another
#                          installation's deployment referring to it doesn't count
cleanup_plan() {
  jq -nr --arg base "$ENGINE_BASE" --arg instance "$APP_NAME" --arg account "$CLOUDFLARE_ACCOUNT_ID" \
    --slurpfile admin "$ARTIFACTS/restate-deployments.json" \
    --slurpfile cli "$ARTIFACTS/restate-deployments-list.json" \
    --slurpfile workers "$ARTIFACTS/cloudflare-workers.json" '
    def clean: if . == null then "" else tostring end | gsub("[\t\n\r]"; " ");
    def mine($m): $m.app == "rafi" and $m.instance == $instance and $m.cloudflare_account == $account;
    def why($d; $m):
      if $m.app != "rafi" then "not a Rafi deployment (no app=rafi metadata)"
      elif $m.instance != $instance then "another installation (\($m.instance | clean))"
      elif $m.cloudflare_account != $account then "another Cloudflare account"
      elif ($m.worker | type) != "string" or ($m.worker | startswith("\($base)-v") | not)
        or ($m.worker | test("^[a-z0-9-]{1,63}$") | not)
        then "its worker is not a \($base)-v... engine Worker (\($m.worker | clean))"
      elif (($d.uri // "") | test("^https://\($m.worker)\\.[a-z0-9-]+\\.workers\\.dev/restate/?$") | not)
        then "its address is not the workers.dev one of \($m.worker)"
      else null end;
    $admin[0].deployments as $deps
    | $cli[0].deployments as $list
    | $workers[0].result as $scripts
    | if ($deps | type) != "array" or ($list | type) != "array" or ($scripts | type) != "array"
      then error("no deployments or Workers in a listing") else . end
    | ($list | map({ key: ((.deployment_id // .id) | tostring), value: . }) | from_entries) as $status
    | [ $deps[]
        | if ((.id // "") | tostring | test("^dp_[A-Za-z0-9]+$")) then . else error("a deployment ID: \(.id)") end
        | . as $d | (.metadata // {}) as $m | why($d; $m) as $why
        | if $why != null then ["other", .id, ((.uri // .arn // "?") | clean), $why]
          else ["ours", ($status[.id].status // "Unknown" | clean), .id, $m.worker,
            ($status[.id].active_invocations // "?" | clean)] end ] as $rows
    | ([ $deps[] | select(mine(.metadata // {}))
         | (.uri // "") | tostring | capture("^https://(?<host>[a-z0-9-]+)\\.").host ]
       + [ $rows[] | select(.[0] == "ours") | .[3] ]) as $referred
    | ($rows[] | @tsv),
      ([ $scripts[] | .id ] | sort | .[]
        | select(test("^\($base)-v[0-9]+-[0-9]+-[0-9]+(-[a-z0-9-]+)?$"))
        | . as $w | select(any($referred[]; . == $w) | not)
        | ["leftover", .] | @tsv)'
}

# The invocations pinned to a deployment and not completed, one per line: <id> <status>.
pinned_invocations() {
  local resp query
  query="SELECT id, status FROM sys_invocation WHERE pinned_deployment_id = '$1' AND status <> 'completed' ORDER BY id"
  resp="$(restate_api POST /query "$(jq -nc --arg query "$query" '{ query: $query }')")"
  [ "$(status_of "$resp")" = 200 ] || return 1
  body_of "$resp" | jq -r '(.rows // [])[] | "\(.id) \(.status)"'
}

# note <kind> <jq filter> [<jq args>...]: one item of the cleanup's summary (cleanup.json).
note() {
  local kind="$1" filter="$2"
  shift 2
  jq -nc --arg kind "$kind" "$@" "$filter + { kind: \$kind }" >>"$CLEANUP_NOTES"
}

# The cleanup's summary, cleanup.json among the artifacts (cleanup_summary <failed: 0 or 1>).
cleanup_summary() {
  jq -s --arg instance "$APP_NAME" --arg version "$VERSION" --arg account "$CLOUDFLARE_ACCOUNT_ID" \
    --argjson failed "$1" '
    def of($k): [.[] | select(.kind == $k) | del(.kind)];
    { installation: $instance, version: $version, cloudflare_account: $account, failed: ($failed == 1),
      removed: of("removed"),
      deleted: [of("removed")[] | select(.worker_status != "left_behind") | .worker],
      not_removed: of("not_removed"),
      draining: of("draining"),
      kept: of("kept"),
      left_behind: of("left_behind"),
      leftover: [of("leftover")[] | .worker],
      not_ours: of("not_ours") }' "$CLEANUP_NOTES" >"$ARTIFACTS/cleanup.json"
}

cmd_cleanup() {
  local admin list plan kind a b c d resp status pinned paused failed=0 ids
  local removed=() draining=() behind=() refused=() others=() leftovers=() text line
  require_env CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID RESTATE_ADMIN_URL RESTATE_AUTH_TOKEN
  CLEANUP_NOTES="$WORK/cleanup.ndjson"
  : >"$CLEANUP_NOTES"
  step "Restate's deployments and the account's Workers (installation $APP_NAME)"
  admin="$(restate_api GET /deployments)"
  [ "$(status_of "$admin")" = 200 ] \
    || die "listing Restate's deployments: HTTP $(status_of "$admin") $(body_of "$admin"): nothing removed or deleted"
  keep restate-deployments.json "$(body_of "$admin")"
  list="$(run_restate deployments list --extra --json)" || die "restate deployments list failed: nothing removed or deleted"
  # The CLI may say something before its JSON.
  [[ "$list" != *"{"* ]] || list="{${list#*\{}"
  keep restate-deployments-list.json "$list"
  cf_workers || die "the account's Workers couldn't be listed: nothing removed or deleted"
  plan="$(cleanup_plan)" || die "couldn't read Restate's deployments: nothing removed or deleted"
  [ -n "$plan" ] || say "none"
  while IFS=$'\t' read -r kind a b c d; do
    case "$kind" in
      ours) ;;
      other)
        say "$a at $b: $c; never touched"
        others+=("\`$a\` at $b: $c")
        note not_ours '{ deployment: $d, address: $a, reason: $r }' --arg d "$a" --arg a "$b" --arg r "$c"
        continue
        ;;
      leftover)
        if [ "$a" = "$ENGINE_WORKER" ]; then
          say "the Worker $a, this release's engine, isn't registered with Restate: a deploy registers it (register-engine)"
          continue
        fi
        say "the Worker $a: no Restate deployment of this installation refers to it; left as it is"
        leftovers+=("\`$a\`: no Restate deployment of this installation refers to it")
        note leftover '{ worker: $w }' --arg w "$a"
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
          note not_removed '{ deployment: $d, worker: $w, reason: "Restate refused to remove it; its Worker is untouched" }' \
            --arg d "$b" --arg w "$c"
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
            note removed '{ deployment: $d, worker: $w, worker_status: "deleted" }' --arg d "$b" --arg w "$c"
            ;;
          404)
            say "  the Worker $c is already gone"
            removed+=("\`$b\`: removed, its Worker \`$c\` already gone")
            note removed '{ deployment: $d, worker: $w, worker_status: "already_gone" }' --arg d "$b" --arg w "$c"
            ;;
          *)
            warn "deleting the Worker $c: HTTP $status $(body_of "$resp" | cf_errors): it is left behind (its deployment $b is removed)"
            removed+=("\`$b\`: removed, but its Worker \`$c\` wasn't deleted (below)")
            behind+=("\`$c\`: its deployment \`$b\` is removed, but deleting it failed (HTTP $status $(body_of "$resp" | cf_errors))")
            note removed '{ deployment: $d, worker: $w, worker_status: "left_behind" }' --arg d "$b" --arg w "$c"
            note left_behind '{ worker: $w, deployment: $d, error: $e }' --arg d "$b" --arg w "$c" \
              --arg e "HTTP $status $(body_of "$resp" | cf_errors)"
            failed=1
            ;;
        esac
        ;;
      Draining)
        paused="?"
        text=
        ids=null
        if pinned="$(pinned_invocations "$b")"; then
          printf '%s\n' "$pinned" | sed '/^$/d; s/^/      /'
          paused="$(grep -c ' paused$' <<<"$pinned" || true)"
          text="$(sed -n 's/^\([^ ]*\) paused$/`\1`/p' <<<"$pinned" | paste -sd ' ' -)"
          ids="$(sed -n 's/^\([^ ]*\) paused$/\1/p' <<<"$pinned" | jq -Rnc '[inputs]')"
        fi
        say "$b ($c) is draining: $d active, $paused paused invocation(s) pinned to it"
        line="\`$b\` (\`$c\`): $d active, $paused paused invocation(s)"
        [ -z "$text" ] || line="$line; paused: $text"
        draining+=("$line")
        note draining '{ deployment: $d, worker: $w, active_invocations: ($n | tonumber? // $n), paused_invocations: $p }' \
          --arg d "$b" --arg w "$c" --arg n "$d" --argjson p "$ids"
        ;;
      Active)
        say "$b ($c) is active (the latest for its services): kept"
        note kept '{ deployment: $d, worker: $w, status: $s }' --arg d "$b" --arg w "$c" --arg s "$a"
        ;;
      *)
        say "$b ($c) is $a: left as it is"
        note kept '{ deployment: $d, worker: $w, status: $s }' --arg d "$b" --arg w "$c" --arg s "$a"
        ;;
    esac
  done <<<"$plan"
  cleanup_summary "$failed"

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
  text+=$'\n\n'"**Workers left behind** (engine Workers no Restate deployment of this installation refers to any more: the cleanup neither deletes them nor retries a failed deletion; delete one by hand from the repository with \`CLOUDFLARE_API_TOKEN=... CLOUDFLARE_ACCOUNT_ID=... npx -y wrangler delete <worker>\`, or on the dashboard):"
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
  copy_legal "$PWD/legal" "$RELEASE/app/assets"
  read_names
  step "dry run of Rafi $VERSION done: nothing deployed"
}

main() {
  local command="${1:-}"
  if [ "$#" -ne 1 ]; then
    usage >&2
    exit 2
  fi
  case "$command" in
    -h | --help)
      usage
      return 0
      ;;
    check-remote | migrate | deploy-live | upload-app | deploy-engine | register-engine | promote-app | cleanup | dry-run) ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  command -v jq >/dev/null 2>&1 \
    || die "jq isn't installed: deploy.sh reads every JSON answer with it (https://jqlang.org; GitHub's runners have it)"
  # wrangler reads CLOUDFLARE_ENV itself: the same environment for every call, an empty one none.
  if [ -n "${CLOUDFLARE_ENV:-}" ]; then
    [[ "$CLOUDFLARE_ENV" =~ ^[A-Za-z0-9_-]+$ ]] || die "CLOUDFLARE_ENV is '$CLOUDFLARE_ENV', not an environment's name"
    export CLOUDFLARE_ENV
  else
    unset CLOUDFLARE_ENV
  fi
  tools
  WORK="$(mktemp -d)"
  # shellcheck disable=SC2064 # the directory's name, now
  trap "rm -rf '$WORK'" EXIT
  if [ -n "${RAFI_ARTIFACTS_DIR:-}" ]; then
    case "$RAFI_ARTIFACTS_DIR" in
      /*) ARTIFACTS="$RAFI_ARTIFACTS_DIR/$command" ;;
      *) ARTIFACTS="$PWD/$RAFI_ARTIFACTS_DIR/$command" ;;
    esac
    rm -rf "$ARTIFACTS"
  else
    ARTIFACTS="$WORK/artifacts"
  fi
  mkdir -p "$ARTIFACTS" || die "couldn't create $ARTIFACTS (RAFI_ARTIFACTS_DIR)"
  read_release
  # The names, from wrangler's dry runs: all but migrate need them (dry-run's are its own).
  case "$command" in
    migrate | dry-run) ;;
    *) read_names ;;
  esac
  "cmd_${command//-/_}"
}

main "$@"
