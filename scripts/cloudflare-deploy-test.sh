#!/usr/bin/env bash
# Checks the Cloudflare deploy script (scripts/cloudflare-deploy.sh, a release's deploy.sh)
# without a Cloudflare or Restate account: each subcommand run as an installation's workflows
# run it, from a copy of testdata/cloudflare-installation with a release laid out in release/, against a
# stubbed wrangler CLI and Restate CLI (which log their arguments; the wrangler stub's dry runs
# write a deploy entry with the Worker's name into its ND-JSON output, as wrangler does) and a
# fake Cloudflare and Restate admin API (a local node HTTP server answering from a state file
# and logging each request, in one log with the stubs, so the order of calls shows).
#
#   - dry-run (no credentials; legal/ copied in: both documents, one, or none): the release's
#     VERSION, the version deployed (the installation's version files and the template's placeholders not looked at), the
#     usage (check is gone), jq missing; the Workers' names read from the dry runs' ND-JSON
#     deploy entries (whatever the files say), a dry run failing or reporting no name, the
#     engine Worker's versioned name; Wrangler environments: none, one without a name of its
#     own (<name>-<env>), one with its own, an environment wrangler refuses; wrangler given
#     CLOUDFLARE_ENV; the tools' defaults (npx -y wrangler, npx -y @restatedev/restate) and
#     their overrides;
#   - migrate; deploy-live skipping when the deployed version's tag is the fingerprint and
#     deploying when the bundle or wrangler.live.toml changed, or nothing, or two versions, are
#     deployed; upload-app's first install (deployed directly, app_promoted) and upgrade (a
#     version uploaded, app_version_id from wrangler's ND-JSON output), a missing Worker
#     refused; deploy-engine's name and URL; register-engine's downgrade guard (semantic
#     versioning with pre-releases) and the metadata (instance: the app Worker's name) and
#     "force": false it sends, 5xx retried, 4xx failing; check-remote's same guard before
#     anything is deployed, Restate unreachable or a key or token that doesn't work failing it,
#     and nothing but reads; promote-app's two paths; the outputs (RAFI_OUTPUT_FILE, or
#     printed) and the reports (RAFI_REPORT_FILE);
#   - cleanup: filtering by metadata (the app Worker's name and the account), each drained
#     deployment removed from Restate before its Worker is deleted, a Worker already gone, a
#     deletion failing (the Worker left behind, the run failing), a removal refused (the
#     Worker untouched), the account's leftover engine Workers (none of this installation's
#     deployments referring to them; another installation's don't count) reported and never
#     deleted, any listing failing stopping it before anything is removed or deleted, and its
#     report; the same in a Wrangler environment;
#   - the artifacts (RAFI_ARTIFACTS_DIR): wrangler's ND-JSON per call, Restate's listings, the
#     registration's answer, the Workers listing and cleanup.json, and no token among them.
#
# CI runs it (`just cloudflare-deploy-test`); so can you:
#
#   ./scripts/cloudflare-deploy-test.sh
#
# Needs node (the fake API), curl and jq.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT/scripts/cloudflare-deploy.sh"
command -v jq >/dev/null || { echo "cloudflare-deploy-test: jq isn't installed (deploy.sh needs it)" >&2; exit 1; }

TMP="$(mktemp -d)"
TMP="$(cd "$TMP" && pwd -P)"
SERVER=
cleanup() {
  if [ -n "$SERVER" ]; then
    kill "$SERVER" 2>/dev/null || true
    wait "$SERVER" 2>/dev/null || true
    SERVER=
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT
# Killed (or its output closed: | head), it still stops the fake API.
trap 'cleanup; exit 1' INT TERM HUP PIPE
FAILED=0
PASSED=0

pass() {
  printf 'ok   %s\n' "$1"
  PASSED=$((PASSED + 1))
}
fail() {
  printf 'FAIL %s\n' "$1" >&2
  FAILED=$((FAILED + 1))
}

# The stubs: wrangler and the Restate CLI log their arguments (wrangler also the
# CLOUDFLARE_ENV it was given, on a line of its own). wrangler writes its ND-JSON output
# (WRANGLER_OUTPUT_FILE_PATH) as wrangler does: a session entry, then for `deploy --dry-run` a
# deploy entry with the Worker's name (--name's; or the configuration's top-level name, or for
# CLOUDFLARE_ENV its [env.<env>]'s name or <name>-<env>, refusing an environment the file lacks;
# STUB_DRY_NAMES="<file>=<name> ..." overrides it, STUB_DRY_NO_NAME leaves the entry out) and
# for `versions upload` a version-upload entry (unless STUB_NO_VERSION_ID). It fails when its
# arguments contain STUB_WRANGLER_FAIL. The Restate CLI lists STUB_RESTATE_LIST (or fails with
# STUB_RESTATE_LIST_FAIL) and refuses to remove the IDs in STUB_RESTATE_REMOVE_FAIL. npx (on
# PATH only where a test puts it) logs its arguments and runs the stub its package names.
BIN="$TMP/bin"
NPX_BIN="$TMP/npx-bin"
NOJQ_BIN="$TMP/nojq-bin"
mkdir -p "$BIN" "$NPX_BIN" "$NOJQ_BIN"
cat >"$BIN/wrangler" <<'EOF'
#!/usr/bin/env bash
echo "wrangler $*" >>"$STUB_LOG"
[ -z "${CLOUDFLARE_ENV:-}" ] || echo "wrangler-env $CLOUDFLARE_ENV" >>"$STUB_LOG"
out="${WRANGLER_OUTPUT_FILE_PATH:-/dev/null}"
echo '{"type":"wrangler-session","version":1,"wrangler_version":"4.148.0","log_file_path":"/tmp/wrangler.log"}' >>"$out"
if [ -n "${STUB_WRANGLER_FAIL:-}" ] && [[ " $* " == *" $STUB_WRANGLER_FAIL "* ]]; then
  echo "stub wrangler: failing" >&2
  echo '{"type":"command-failed","version":1,"message":"stub"}' >>"$out"
  exit 1
fi
config= name=
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
  case "${args[i]}" in
    -c) config="${args[i + 1]}" ;;
    --name) name="${args[i + 1]}" ;;
  esac
done
case " $* " in *" --dry-run "*)
  if [ -z "$name" ]; then
    name="$(sed -n 's/^name = "\(.*\)"$/\1/p' "$config" | head -1)"
    if [ -n "${CLOUDFLARE_ENV:-}" ]; then
      if ! grep -qxF "[env.$CLOUDFLARE_ENV]" "$config"; then
        echo "stub wrangler: Processing $(basename "$config") configuration: No environment found in configuration with name \"$CLOUDFLARE_ENV\"." >&2
        exit 1
      fi
      own="$(sed -n "/^\[env\.$CLOUDFLARE_ENV\]\$/,/^\[\[*env\.$CLOUDFLARE_ENV\./s/^name = \"\(.*\)\"\$/\1/p" "$config" | head -1)"
      name="${own:-$name-$CLOUDFLARE_ENV}"
    fi
    for pair in ${STUB_DRY_NAMES:-}; do
      [ "${pair%%=*}" != "$(basename "$config")" ] || name="${pair#*=}"
    done
  fi
  echo "dry run: $*"
  [ -n "${STUB_DRY_NO_NAME:-}" ] \
    || echo "{\"type\":\"deploy\",\"version\":1,\"worker_name\":\"$name\",\"worker_tag\":null,\"version_id\":null}" >>"$out"
  exit 0
  ;;
esac
if [ "$1 $2" = "versions upload" ]; then
  [ -n "${STUB_NO_VERSION_ID:-}" ] \
    || echo '{"type":"version-upload","version":1,"worker_name":"rafi-app","version_id":"1b2c3d4e-0000-4000-8000-000000000001"}' >>"$out"
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
cat >"$NPX_BIN/npx" <<EOF
#!/usr/bin/env bash
echo "npx \$*" >>"\$STUB_LOG"
[ "\$1" = -y ] || exit 1
case "\$2" in
  wrangler) shift 2; exec "$BIN/wrangler" "\$@" ;;
  @restatedev/restate) shift 2; exec "$BIN/restate" "\$@" ;;
  *) exit 1 ;;
esac
EOF
chmod +x "$BIN/wrangler" "$BIN/restate" "$NPX_BIN/npx"
# A PATH with bash alone: no jq.
ln -s "$(command -v bash)" "$NOJQ_BIN/bash"

# The fake APIs: /cf/... is Cloudflare's (account acct123, token cf-token), /restate/... the
# Restate admin API (token rs-token). They answer from state.json, read on each request:
#   subdomain; scripts: { <name>: { deployments: [...], versions: [...] } } (a name absent is
#   no such Worker; GET .../workers/scripts lists them); status: { "<METHOD> <path>": <status> }
#   overrides; services, deployments (Restate's), register: [statuses of successive POST
#   /deployments], query: rows.
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
      if (req.method === "GET" && p === "/cf/accounts/acct123/workers/scripts") {
        return send(res, 200, { success: true, errors: [], messages: [], result: Object.keys(scripts).map((id) => ({ id, etag: "x" })) });
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
OUTPUTS="$TMP/outputs"
REPORT="$TMP/report"
ARTIFACTS="$TMP/artifacts"
state() { cat >"$TMP/state.json"; }
echo '{}' | state
: >"$LOG"
node "$TMP/fake.mjs" "$TMP/state.json" "$LOG" "$TMP/port" &
SERVER=$!
for _ in $(seq 1 50); do [ -s "$TMP/port" ] && break; sleep 0.1; done
[ -s "$TMP/port" ] || { echo "the fake API didn't start" >&2; exit 1; }
BASE="http://127.0.0.1:$(cat "$TMP/port")"

# A release as `just cloudflare-artifact` lays it out (empty files will do, the live Worker's
# bundle with something in it), unpacked into an installation (a copy of the fixture) as
# release/.
INSTALL="$TMP/installation"
mkdir -p "$INSTALL"
cp -R "$ROOT/testdata/cloudflare-installation/." "$INSTALL/"
rm -rf "$INSTALL/release" "$INSTALL/.wrangler" "$INSTALL/.dev.vars" "$INSTALL/node_modules"
# The operator's documents, the tests' own.
rm -rf "$INSTALL/legal"
mkdir "$INSTALL/legal"
printf -- '---\nversion: test-1\n---\n# Terms\n' >"$INSTALL/legal/terms.md"
printf -- '---\nversion: test-1\n---\n# Privacy notice\n' >"$INSTALL/legal/privacy.md"
RELEASE="$INSTALL/release"
mkdir -p "$RELEASE/app/build" "$RELEASE/app/assets" "$RELEASE/engine/build" "$RELEASE/live/build" "$RELEASE/migrations"
touch "$RELEASE/app/build/index.js" "$RELEASE/engine/build/index.js"
echo 'export { RaffleRoom, default };' >"$RELEASE/live/build/index.js"
echo 'wasm' >"$RELEASE/live/build/index_bg.wasm"
cp "$ROOT"/migrations/sqlite/*.sql "$RELEASE/migrations/"
cp "$SCRIPT" "$RELEASE/deploy.sh"
echo "0.1.0" >"$RELEASE/VERSION"
APP="$INSTALL/wrangler.app.toml"
ENGINE="$INSTALL/wrangler.engine.toml"
LIVE="$INSTALL/wrangler.live.toml"
DB_ID=6f1c2a90-3b7e-4d2a-9c41-0e5b8a7d2f13
KEY=publickeyv1_ChjENKeMvCtRnqG2mrBK1HmPKufgFUc98K8B3ZgXonQQ

# What the workflows give every subcommand; a test's own assignments come after (and win), an
# empty one unsets. wrangler and the Restate CLI are the stubs.
COMMON=(WRANGLER="$BIN/wrangler" RESTATE="$BIN/restate" STUB_LOG="$LOG"
  STUB_RESTATE_LIST="$TMP/list.json" CLOUDFLARE_API_BASE_URL="$BASE/cf" CLOUDFLARE_API_TOKEN=cf-token
  CLOUDFLARE_ACCOUNT_ID=acct123 RESTATE_ADMIN_URL="$BASE/restate/" RESTATE_AUTH_TOKEN=rs-token
  RAFI_REGISTER_WAIT=0 RAFI_OUTPUT_FILE="$OUTPUTS" RAFI_REPORT_FILE="$REPORT" RAFI_ARTIFACTS_DIR=
  CLOUDFLARE_ENV= RAFI_APP_PROMOTED= RAFI_APP_VERSION_ID= RAFI_ENGINE_WORKER= RAFI_ENGINE_URL=
  STUB_WRANGLER_FAIL= STUB_NO_VERSION_ID= STUB_DRY_NAMES= STUB_DRY_NO_NAME= STUB_RESTATE_LIST_FAIL=
  STUB_RESTATE_REMOVE_FAIL= GITHUB_OUTPUT= GITHUB_STEP_SUMMARY=)

# ds <subcommand> [<VAR=value>...]: runs release/deploy.sh <subcommand> in the installation, its
# output in $TMP/out, its exit status in CODE; the log, the outputs and the report fresh.
ds() {
  local sub="$1"
  shift
  : >"$LOG"
  : >"$OUTPUTS"
  : >"$REPORT"
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
    n="$(grep -nF -- "$text" "$LOG" | awk -F: -v p="$prev" '$1 > p { print $1; exit }' || true)"
    if [ -z "$n" ]; then
      fail "$name: no '$text' in the log after line $prev; the log: $(cat "$LOG")"
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
# outputs_are <name> <the output file's content>
outputs_are() {
  if [ "$(cat "$OUTPUTS")" = "$2" ]; then pass "$1"; else fail "$1: the outputs are '$(cat "$OUTPUTS")', not '$2'"; fi
}
# reported <name> <text>...: each in the report file.
reported() {
  local name="$1" text
  shift
  for text in "$@"; do
    grep -qF -- "$text" "$REPORT" || { fail "$name: no '$text' in the report: $(cat "$REPORT")"; return; }
  done
  pass "$name"
}
# not_reported <name> <text>...: none of them in the report file.
not_reported() {
  local name="$1" text
  shift
  for text in "$@"; do
    if grep -qF -- "$text" "$REPORT"; then
      fail "$name: '$text' in the report: $(cat "$REPORT")"
      return
    fi
  done
  pass "$name"
}
# json_is <name> <file> <jq expression>: the expression is true of the file.
json_is() {
  if [ -f "$2" ] && jq -e "$3" "$2" >/dev/null 2>&1; then
    pass "$1"
  else
    fail "$1: not $3 of $2: $(cat "$2" 2>/dev/null || echo 'no such file')"
  fi
}
# only_dry_runs <name>: the log has wrangler's three dry runs and nothing else (but CLOUDFLARE_ENV).
only_dry_runs() {
  local other
  other="$(grep -vE '^wrangler deploy -c [^ ]+ --dry-run$|^wrangler-env ' "$LOG" || true)"
  if [ -z "$other" ] && [ "$(grep -c -- '--dry-run' "$LOG")" = 3 ]; then pass "$1"; else fail "$1: the log: $(cat "$LOG")"; fi
}

echo "== dry-run: the template as it is"
ds dry-run CLOUDFLARE_API_TOKEN= CLOUDFLARE_ACCOUNT_ID= RESTATE_ADMIN_URL= RESTATE_AUTH_TOKEN=
ran "dry-run needs no credentials, and nothing looks for the template's placeholders" 0 "dry run of Rafi 0.1.0 done"
if grep -qE "warning|placeholder" "$TMP/out"; then fail "dry-run warned: $(cat "$TMP/out")"; else pass "no warning about the placeholders"; fi
in_order "dry-run: the live Worker, the engine, the app" \
  "wrangler deploy -c $LIVE --dry-run" "wrangler deploy -c $ENGINE --dry-run" "wrangler deploy -c $APP --dry-run"
only_dry_runs "dry-run: nothing but the three dry runs"
not_logged "no environment given to wrangler" "wrangler-env"
ran "dry-run: the names from the dry runs" 0 "live Worker: rafi-live (" "app Worker: rafi-app (" \
  "engine Worker: rafi-engine-v0-1-0 ("
if cmp -s "$INSTALL/legal/terms.md" "$RELEASE/app/assets/legal/terms.md" \
  && cmp -s "$INSTALL/legal/privacy.md" "$RELEASE/app/assets/legal/privacy.md"; then
  pass "dry-run copies legal/ into the app's static files"
else
  fail "legal/ isn't in $RELEASE/app/assets/legal"
fi
# The documents are the operator's, each optional: one alone is copied alone, and what a run
# before copied doesn't outlive its removal.
mv "$INSTALL/legal" "$TMP/legal.aside"
mkdir "$INSTALL/legal"
cp "$TMP/legal.aside/terms.md" "$INSTALL/legal/"
ds dry-run
ran "dry-run with the terms alone" 0 "copied terms.md from" "dry run of Rafi 0.1.0 done"
if [ -f "$RELEASE/app/assets/legal/terms.md" ] && [ ! -e "$RELEASE/app/assets/legal/privacy.md" ]; then
  pass "the terms alone are copied, the privacy notice a run before copied gone"
else
  fail "the app's legal/ after the terms alone: $(ls "$RELEASE/app/assets/legal" 2>&1)"
fi
rm -rf "$INSTALL/legal"
ds dry-run
ran "dry-run with no legal/ at all" 0 "no legal documents in" "dry run of Rafi 0.1.0 done"
if [ ! -e "$RELEASE/app/assets/legal" ]; then pass "no legal/ among the app's static files"; else fail "the app's legal/ outlived the installation's"; fi
mv "$TMP/legal.aside" "$INSTALL/legal"
rm -rf "$RELEASE/app/assets/legal"

# Filled in, as an installation does.
sed -i.orig "s/00000000-0000-0000-0000-000000000000/$DB_ID/" "$APP" "$ENGINE" "$LIVE"
sed -i.orig "s/\"publickeyv1_…\"/\"$KEY, publickeyv1_9n4BzZ8\"/" "$ENGINE"
rm -f "$INSTALL"/*.orig
cp "$APP" "$TMP/app.good"
cp "$ENGINE" "$TMP/engine.good"
cp "$LIVE" "$TMP/live.good"
restore() {
  cp "$TMP/app.good" "$APP"
  cp "$TMP/engine.good" "$ENGINE"
  cp "$TMP/live.good" "$LIVE"
  rm -f "$INSTALL"/*.orig "$INSTALL"/VERSION.*
}
echo "== the release's VERSION, the usage, the tools"
ds dry-run
ran "the release's VERSION" 0 "Rafi 0.1.0 (the release's VERSION)" "environment: none" "engine Worker: rafi-engine-v0-1-0 (" \
  "live Worker: rafi-live (" "app Worker: rafi-app ("
only_dry_runs "dry-run runs wrangler's three dry runs, nothing else"
echo "0.2.0" >"$INSTALL/VERSION"
ds dry-run
ran "the installation's VERSION isn't read: the release's VERSION is the one deployed" 0 "Rafi 0.1.0 (the release's VERSION)" \
  "engine Worker: rafi-engine-v0-1-0 ("
rm "$INSTALL/VERSION"
ds dry-run
ran "nor needed" 0 "Rafi 0.1.0 (the release's VERSION)"
echo "0.1.0" >"$INSTALL/VERSION"
mv "$RELEASE/VERSION" "$TMP/VERSION.aside"
ds dry-run
ran "a release without its VERSION: nothing to deploy" 1 "$RELEASE/VERSION is missing"
echo "0.1" >"$RELEASE/VERSION"
ds dry-run
ran "a release VERSION that isn't a version" 1 "says '0.1', not a Rafi version"
mv "$TMP/VERSION.aside" "$RELEASE/VERSION"
echo "v0.2.0-RC.1" >"$RELEASE/VERSION"
ds dry-run
ran "a pre-release, a leading v dropped: its engine Worker" 0 "engine Worker: rafi-engine-v0-2-0-rc-1"
echo "0.1.0" >"$RELEASE/VERSION"
sed -i.orig "s/^name = \"rafi-engine\"/name = \"rafi-engine-$(printf 'x%.0s' $(seq 1 50))\"/" "$ENGINE"
ds dry-run
ran "an engine Worker's name over 63 characters" 1 "longer than 63"
restore
ds nope
ran "an unknown subcommand exits 2" 2
ds ""
ran "no subcommand exits 2" 2
ds --check
ran "the old --check exits 2" 2
ds check
ran "check, gone, exits 2" 2
ds deploy-rooms
ran "the old deploy-rooms exits 2" 2
ds --help
ran "--help" 0 "release/deploy.sh register-engine" "release/deploy.sh deploy-live"
ds dry-run PATH="$NOJQ_BIN"
ran "no jq: a clear failure" 1 "jq isn't installed"
if [ -s "$LOG" ]; then fail "something ran without jq: $(cat "$LOG")"; else pass "nothing runs without jq"; fi
# The defaults: npx -y wrangler and npx -y @restatedev/restate (the installation's package.json's).
ds dry-run WRANGLER= RESTATE= PATH="$NPX_BIN:$PATH"
ran "the tools by default: npx -y" 0 "app Worker: rafi-app ("
in_order "npx -y wrangler for wrangler" "npx -y wrangler deploy -c $LIVE --dry-run" "wrangler deploy -c $LIVE --dry-run" \
  "npx -y wrangler deploy -c $APP --dry-run"
ds migrate WRANGLER= PATH="$NPX_BIN:$PATH"
in_order "npx -y wrangler for the migrations" "npx -y wrangler d1 migrations apply DB -c $APP --remote"
ds migrate WRANGLER="$BIN/wrangler --stub-arg" PATH="$NPX_BIN:$PATH"
in_order "WRANGLER overrides it, with its arguments" "wrangler --stub-arg d1 migrations apply DB -c $APP --remote"
not_logged "no npx when WRANGLER is given" "npx"

echo "== the names"
ds dry-run STUB_DRY_NAMES="wrangler.app.toml=raffles-app wrangler.live.toml=raffles-live wrangler.engine.toml=raffles-engine"
ran "the names are the dry runs' worker_name, not read from the files" 0 "app Worker: raffles-app (" \
  "live Worker: raffles-live (" "engine Worker: raffles-engine-v0-1-0 ("
ds dry-run STUB_WRANGLER_FAIL="$APP"
ran "a dry run failing: refused" 1 "wrangler deploy --dry-run -c wrangler.app.toml failed (above): the configuration doesn't deploy"
ds dry-run STUB_DRY_NO_NAME=1
ran "a dry run without a deploy entry: refused" 1 "wrangler's dry run of wrangler.live.toml reported no Worker's name"
ds dry-run STUB_DRY_NAMES="wrangler.engine.toml=Not_A_Name"
ran "a name that isn't a Worker's: refused" 1 "wrangler's dry run of wrangler.engine.toml reported no Worker's name (its deploy entry's worker_name: 'Not_A_Name')"

echo "== Wrangler environments (CLOUDFLARE_ENV)"
# add_env <env> [<the app's own name>]: an [env.<env>] in each configuration, as the README's
# (the vars, bindings and required secrets repeated: environments don't inherit them).
add_env() {
  local env="$1" app_name="${2:-}" live_name="rafi-live-$1"
  {
    printf '\n[env.%s]\n' "$env"
    [ -z "$app_name" ] || printf 'name = "%s"\n' "$app_name"
    # Its own address, as INSTALL.md's [env.eu] writes it out (a custom domain here).
    printf 'workers_dev = false\nroutes = [{ pattern = "%s.raffles.example", custom_domain = true }]\n' "$env"
    printf '[[env.%s.d1_databases]]\nbinding = "DB"\ndatabase_name = "rafi"\ndatabase_id = "%s"\nmigrations_dir = "release/migrations"\n' "$env" "$DB_ID"
    printf '[[env.%s.durable_objects.bindings]]\nname = "RAFFLE_ROOM"\nclass_name = "RaffleRoom"\nscript_name = "%s"\n' "$env" "$live_name"
    printf '[env.%s.vars]\nRAFI_DO_JURISDICTION = "eu"\n' "$env"
  } >>"$APP"
  {
    printf '\n[env.%s]\n' "$env"
    printf '[[env.%s.d1_databases]]\nbinding = "DB"\ndatabase_name = "rafi"\ndatabase_id = "%s"\n' "$env" "$DB_ID"
    printf '[[env.%s.durable_objects.bindings]]\nname = "RAFFLE_ROOM"\nclass_name = "RaffleRoom"\nscript_name = "%s"\n' "$env" "$live_name"
    printf '[env.%s.vars]\nRAFI_DO_JURISDICTION = "eu"\nRESTATE_IDENTITY_KEYS = "%s"\n' "$env" "$KEY"
  } >>"$ENGINE"
  {
    printf '\n[env.%s]\n' "$env"
    printf '[[env.%s.d1_databases]]\nbinding = "DB"\ndatabase_name = "rafi"\ndatabase_id = "%s"\n' "$env" "$DB_ID"
    printf '[[env.%s.durable_objects.bindings]]\nname = "RAFFLE_ROOM"\nclass_name = "RaffleRoom"\n' "$env"
    printf '[env.%s.vars]\nRAFI_DO_JURISDICTION = "eu"\n' "$env"
  } >>"$LIVE"
}
add_env staging
ds dry-run
ran "no CLOUDFLARE_ENV: the top level, other environments ignored" 0 "environment: none" "app Worker: rafi-app (" \
  "live Worker: rafi-live (" "engine Worker: rafi-engine-v0-1-0 ("
ds dry-run CLOUDFLARE_ENV=eu
ran "CLOUDFLARE_ENV naming an environment the configurations lack: wrangler refuses it" 1 \
  "No environment found in configuration with name \"eu\"" "wrangler deploy --dry-run -c wrangler.live.toml (CLOUDFLARE_ENV=eu) failed"
ds dry-run CLOUDFLARE_ENV='e u'
ran "CLOUDFLARE_ENV not an environment's name: refused" 1 "not an environment's name"
restore
add_env eu
ds dry-run CLOUDFLARE_ENV=eu
ran "an environment without names of its own: <name>-<environment>" 0 "environment: eu ([env.eu] of each configuration)" \
  "app Worker: rafi-app-eu (" "live Worker: rafi-live-eu (" "engine Worker: rafi-engine-eu-v0-1-0 (" "Rafi 0.1.0 (the release's VERSION)"
in_order "wrangler given the environment for each dry run" \
  "wrangler deploy -c $LIVE --dry-run" "wrangler-env eu" "wrangler deploy -c $ENGINE --dry-run" "wrangler-env eu" \
  "wrangler deploy -c $APP --dry-run" "wrangler-env eu"
echo "0.3.0" >"$INSTALL/VERSION.eu"
ds dry-run CLOUDFLARE_ENV=eu
ran "VERSION.eu isn't read either (the workflows choose the release by it)" 0 "Rafi 0.1.0 (the release's VERSION)" \
  "engine Worker: rafi-engine-eu-v0-1-0 ("
rm "$INSTALL/VERSION.eu"
ds dry-run
ran "the same configurations without CLOUDFLARE_ENV: the top level" 0 "app Worker: rafi-app (" "engine Worker: rafi-engine-v0-1-0 ("
restore
add_env eu raffles-eu
ds dry-run CLOUDFLARE_ENV=eu
ran "an environment's app with a name of its own" 0 "app Worker: raffles-eu (" "live Worker: rafi-live-eu ("
restore
add_env eu
ds dry-run CLOUDFLARE_ENV=eu
ran "dry-run in an environment" 0 "dry run of Rafi 0.1.0 done" "engine Worker: rafi-engine-eu-v0-1-0 ("
rm -rf "$RELEASE/app/assets/legal"
restore
echo "== migrate"
ds migrate
ran "migrate" 0
in_order "the D1 migrations, remote, with the app's configuration" "wrangler d1 migrations apply DB -c $APP --remote"
ds migrate CLOUDFLARE_API_TOKEN=
ran "migrate without credentials" 1 "not set: CLOUDFLARE_API_TOKEN"
not_logged "nothing ran without them" "wrangler"

echo "== deploy-live"
echo '{}' | state
ds deploy-live
ran "a live Worker not deployed yet is deployed" 0 "not deployed yet"
TAG="$(grep -o 'live-[0-9a-f]\{32\}' "$LOG" | head -1 || true)"
in_order "wrangler deploy of the live Worker, tagged with its fingerprint" \
  "wrangler deploy -c $LIVE --tag $TAG --message rafi v0.1.0"
if [[ "$TAG" =~ ^live-[0-9a-f]{32}$ ]]; then pass "the fingerprint's tag: $TAG"; else fail "no live-<32 hex> tag: '$TAG'"; fi
reported "deploy-live's report" "### Live Worker \`rafi-live\`" "Deployed (\`$TAG\`, Rafi v0.1.0)"
# live_state <tag> [<second tag>] [<Worker>]: the live Worker serving a version with that tag (two at 50% each).
live_state() {
  local worker="${3:-rafi-live}"
  if [ -z "${2:-}" ]; then
    state <<EOF
{ "scripts": { "$worker": {
  "deployments": [ { "id": "d2", "versions": [ { "version_id": "r2", "percentage": 100 } ] },
                   { "id": "d1", "versions": [ { "version_id": "r1", "percentage": 100 } ] } ],
  "versions": [ { "id": "r2", "annotations": { "workers/tag": "$1", "workers/message": "rafi v0.0.9" } },
                { "id": "r1", "annotations": { "workers/tag": "live-0" } } ] } } }
EOF
  else
    state <<EOF
{ "scripts": { "$worker": {
  "deployments": [ { "id": "d2", "versions": [ { "version_id": "r2", "percentage": 50 }, { "version_id": "r1", "percentage": 50 } ] } ],
  "versions": [ { "id": "r2", "annotations": { "workers/tag": "$1" } }, { "id": "r1", "annotations": { "workers/tag": "$2" } } ] } } }
EOF
  fi
}
live_state "$TAG"
ds deploy-live
ran "a live Worker serving this fingerprint is left alone" 0 "unchanged (it serves $TAG)" "keep their connections"
in_order "its latest deployment's version read" "http GET /cf/accounts/acct123/workers/scripts/rafi-live/deployments" \
  "http GET /cf/accounts/acct123/workers/scripts/rafi-live/versions/r2"
not_logged "nothing deployed" "--tag"
reported "deploy-live's report when unchanged" "Unchanged (\`$TAG\`)"
live_state "live-0123456789abcdef0123456789abcdef"
ds deploy-live
ran "another fingerprint deployed" 0 "it serves: 100 live-0123456789abcdef0123456789abcdef"
in_order "the live Worker deployed" "wrangler deploy -c $LIVE --tag $TAG"
live_state "$TAG" "live-0123456789abcdef0123456789abcdef"
ds deploy-live
ran "a gradual deployment, half this fingerprint, deployed" 0
in_order "the live Worker deployed over a gradual one" "wrangler deploy -c $LIVE --tag $TAG"
live_state "$TAG"
echo "# a comment" >>"$LIVE"
ds deploy-live
ran "a comment added to wrangler.live.toml: deployed (its bytes are in the fingerprint)" 0 "it serves: 100 $TAG"
not_logged "a comment: another tag" "--tag $TAG "
restore
sed -i.orig 's/^compatibility_date = .*/compatibility_date = "2026-10-02"/' "$LIVE"
ds deploy-live
ran "wrangler.live.toml changed: deployed" 0
not_logged "with another tag" "--tag $TAG "
in_order "the live Worker deployed after its configuration changed" "wrangler deploy -c $LIVE --tag live-"
restore
echo 'more' >>"$RELEASE/live/build/index_bg.wasm"
ds deploy-live
ran "the bundle changed: deployed" 0
not_logged "the bundle changed: with another tag" "--tag $TAG "
in_order "the live Worker deployed after its bundle changed" "wrangler deploy -c $LIVE --tag live-"
echo 'wasm' >"$RELEASE/live/build/index_bg.wasm"
ds deploy-live
ran "the bundle as it was: the same fingerprint" 0 "unchanged (it serves $TAG)"
add_env eu
live_state "$TAG" "" rafi-live-eu
ds deploy-live CLOUDFLARE_ENV=eu
ran "an environment's live Worker: its own name and fingerprint" 0 "live Worker rafi-live-eu"
in_order "the environment's live Worker read, then deployed with the environment" \
  "http GET /cf/accounts/acct123/workers/scripts/rafi-live-eu/deployments" "wrangler deploy -c $LIVE --tag live-" "wrangler-env eu"
not_logged "the environment's fingerprint isn't the top level's" "--tag $TAG "
restore
state <<'EOF'
{ "status": { "GET /cf/accounts/acct123/workers/scripts/rafi-live/deployments": 500 } }
EOF
ds deploy-live
ran "Cloudflare failing: refused" 1 "the Worker rafi-live's deployments: HTTP 500"
not_logged "nothing deployed when Cloudflare fails" "--tag"

echo "== upload-app"
rm -rf "$RELEASE/app/assets/legal"
echo '{}' | state
ds upload-app
ran "no app Worker: refused, its secrets first" 1 "the app Worker rafi-app doesn't exist" "secret bulk .dev.vars -c wrangler.app.toml"
not_logged "nothing uploaded without an app Worker" "--tag" "versions upload"
# app_state <tag of the deployed version> <tag of an older version> [<Worker>]: the app Worker.
app_state() {
  state <<EOF
{ "scripts": { "${3:-rafi-app}": {
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
reported "a first install's report" "### App Worker \`rafi-app\`" "A first install: Rafi v0.1.0 deployed directly."
if cmp -s "$INSTALL/legal/privacy.md" "$RELEASE/app/assets/legal/privacy.md"; then pass "legal/ copied in"; else fail "legal/ not copied"; fi
app_state "v0.0.9" ""
ds upload-app
ran "an upgrade: a version uploaded" 0 "a release ran here before"
in_order "wrangler versions upload, tagged" "wrangler versions upload -c $APP --tag v0.1.0 --message rafi v0.1.0"
not_logged "not deployed" "wrangler deploy -c $APP --tag"
outputs_are "the uploaded version's ID" "app_version_id=1b2c3d4e-0000-4000-8000-000000000001"
reported "an upgrade's report" "Version \`1b2c3d4e-0000-4000-8000-000000000001\` uploaded (Rafi v0.1.0), not serving yet"
app_state "" "v0.0.9"
ds upload-app
ran "a secret changed since the last release: still an upgrade" 0 "a release ran here before"
in_order "its recent versions read" "http GET /cf/accounts/acct123/workers/scripts/rafi-app/versions" "wrangler versions upload"
ds upload-app STUB_NO_VERSION_ID=1
ran "wrangler reporting no version ID: failed" 1 "wrangler versions upload reported no version ID"
outputs_are "no output then" ""
ds upload-app STUB_WRANGLER_FAIL=upload
ran "wrangler failing: failed" 1
outputs_are "no output when wrangler fails" ""
ds upload-app RAFI_OUTPUT_FILE= RAFI_REPORT_FILE=
ran "without RAFI_OUTPUT_FILE the output is printed" 0 "app_version_id=1b2c3d4e-0000-4000-8000-000000000001" "==> report"
if [ -s "$OUTPUTS" ] || [ -s "$REPORT" ]; then fail "files written without RAFI_OUTPUT_FILE and RAFI_REPORT_FILE"; else pass "no files written without them"; fi
add_env eu
app_state "v0.0.9" "" rafi-app-eu
ds upload-app CLOUDFLARE_ENV=eu
ran "an environment's app Worker" 0 "app Worker rafi-app-eu" "a release ran here before"
in_order "the environment's app Worker read, then uploaded with the environment" \
  "http GET /cf/accounts/acct123/workers/scripts/rafi-app-eu/deployments" "wrangler versions upload -c $APP" "wrangler-env eu"
restore

echo "== deploy-engine"
echo '{ "subdomain": "acme" }' | state
ds deploy-engine
ran "the engine Worker deployed" 0
in_order "under its versioned name, tagged" "http GET /cf/accounts/acct123/workers/subdomain" \
  "wrangler deploy -c $ENGINE --name rafi-engine-v0-1-0 --tag v0.1.0 --message rafi v0.1.0"
outputs_are "its name and address" "engine_worker=rafi-engine-v0-1-0
engine_url=https://rafi-engine-v0-1-0.acme.workers.dev/restate"
reported "deploy-engine's report" "### Engine Worker \`rafi-engine-v0-1-0\`" "https://rafi-engine-v0-1-0.acme.workers.dev/restate"
add_env eu
ds deploy-engine CLOUDFLARE_ENV=eu
ran "an environment's engine Worker" 0
in_order "under the environment's versioned name, with the environment" \
  "wrangler deploy -c $ENGINE --name rafi-engine-eu-v0-1-0 --tag v0.1.0" "wrangler-env eu"
outputs_are "its name and address in the environment" "engine_worker=rafi-engine-eu-v0-1-0
engine_url=https://rafi-engine-eu-v0-1-0.acme.workers.dev/restate"
restore
echo '{}' | state
ds deploy-engine
ran "no workers.dev subdomain: refused" 1 "the account's workers.dev subdomain: HTTP 404"
not_logged "nothing deployed without a subdomain" "--name"

echo "== register-engine"
ENGINE_URL=https://rafi-engine-v0-1-0.acme.workers.dev/restate
REG=(RAFI_ENGINE_WORKER=rafi-engine-v0-1-0 RAFI_ENGINE_URL="$ENGINE_URL")
REGISTERED='"uri":"https://rafi-engine-v0-1-0.acme.workers.dev/restate","force":false,"metadata":{"app":"rafi","instance":"rafi-app","version":"0.1.0","cloudflare_account":"acct123","worker":"rafi-engine-v0-1-0"}'
# restate_state <Raffle's deployment's metadata> [<EngineInfo's>] [<register statuses>]
restate_state() {
  state <<EOF
{ "subdomain": "acme",
  "services": [ { "name": "Raffle", "deployment_id": "dp_raffle" }, { "name": "EngineInfo", "deployment_id": "dp_info" },
                { "name": "Other", "deployment_id": "dp_other" } ],
  "deployments": [ { "id": "dp_raffle", "uri": "https://x.acme.workers.dev/restate", "metadata": $1 },
                   { "id": "dp_info", "uri": "https://y.acme.workers.dev/restate", "metadata": ${2:-$1} },
                   { "id": "dp_other", "uri": "https://example.org", "metadata": { "app": "rafi", "instance": "rafi-app", "cloudflare_account": "acct123", "version": "9.0.0" } } ],
  "register": ${3:-[201]} }
EOF
}
# attempts <n> <name>: Restate was asked to register n times.
attempts() {
  local n
  n="$(grep -c 'POST /restate/deployments' "$LOG" || true)"
  if [ "$n" = "$1" ]; then pass "$2"; else fail "$2: $n attempts"; fi
}
ours() { printf '{ "app": "rafi", "instance": "%s", "cloudflare_account": "acct123", "version": "%s", "worker": "w" }' "${2:-rafi-app}" "$1"; }
echo '{ "services": [], "deployments": [] }' | state
ds register-engine "${REG[@]}"
ran "a first registration" 0 "Raffle: not registered yet" "registered: Restate deployment dp_new"
in_order "the services and deployments read, then the engine registered with its metadata (instance: the app Worker's name), never forced" \
  "http GET /restate/services" "http GET /restate/deployments" "http POST /restate/deployments {$REGISTERED}"
not_logged "no token in the stubs' arguments" "rs-token" "cf-token"
reported "register-engine's report" "### Restate" "The engine Worker \`rafi-engine-v0-1-0\` (Rafi v0.1.0) is Restate deployment \`dp_new\`, registered now" "$ENGINE_URL"
restate_state "$(ours 0.2.0)"
ds register-engine "${REG[@]}"
ran "a downgrade refused" 1 "Raffle runs on Rafi 0.2.0 (dp_raffle), newer than 0.1.0" "Set VERSION to 0.2.0 or later"
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
restate_state "$(ours 9.0.0 rafi-app-eu)"
ds register-engine "${REG[@]}"
ran "another installation's deployment (another app Worker): no guard" 0 "not a deployment of this installation"
restate_state '{ "app": "rafi", "instance": "rafi-app", "cloudflare_account": "acct999", "version": "9.0.0" }'
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
ds register-engine "${REG[@]}" RESTATE_ADMIN_URL=
ran "no RESTATE_ADMIN_URL: refused" 1 "not set: RESTATE_ADMIN_URL"
not_logged "nothing asked without it" "http"
# In an environment: its app Worker's name is the instance, and its guard ignores the top level's.
add_env eu
restate_state "$(ours 0.2.0)" "$(ours 0.0.9 rafi-app-eu)"
ds register-engine CLOUDFLARE_ENV=eu RAFI_ENGINE_WORKER=rafi-engine-eu-v0-1-0 RAFI_ENGINE_URL=https://rafi-engine-eu-v0-1-0.acme.workers.dev/restate
ran "an environment's engine: the top level's newer release isn't its" 0 "Raffle: on dp_raffle, not a deployment of this installation" \
  "EngineInfo: on dp_info, Rafi 0.0.9" "installation rafi-app-eu"
in_order "registered with the environment's app Worker as instance" \
  'http POST /restate/deployments {"uri":"https://rafi-engine-eu-v0-1-0.acme.workers.dev/restate","force":false,"metadata":{"app":"rafi","instance":"rafi-app-eu","version":"0.1.0","cloudflare_account":"acct123","worker":"rafi-engine-eu-v0-1-0"}}'
restate_state "$(ours 0.0.9)" "$(ours 0.2.0 rafi-app-eu)"
ds register-engine CLOUDFLARE_ENV=eu RAFI_ENGINE_WORKER=rafi-engine-eu-v0-1-0 RAFI_ENGINE_URL=https://rafi-engine-eu-v0-1-0.acme.workers.dev/restate
ran "an environment's own newer release: refused, naming its version file" 1 "EngineInfo runs on Rafi 0.2.0" \
  "Set VERSION.eu (VERSION without one) to 0.2.0 or later"
restore
# A pre-release's precedence among pre-releases.
echo "0.2.0-rc.10" >"$RELEASE/VERSION"
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

echo "== check-remote"
# The deploy's first step: Restate's release read and the token tried, nothing changed.
echo '{ "subdomain": "acme", "services": [], "deployments": [] }' | state
ds check-remote
ran "a first install (nothing registered): allowed" 0 "Raffle: not registered yet" "EngineInfo: not registered yet" \
  "the token reads it: workers.dev subdomain acme" "checked: Rafi 0.1.0 may be deployed here"
in_order "Restate's services and deployments, then the account" \
  "http GET /restate/services" "http GET /restate/deployments" "http GET /cf/accounts/acct123/workers/subdomain"
not_logged "nothing changed or deployed" "POST" "DELETE" "--tag" "--name" "versions" "restate "
restate_state "$(ours 0.1.0)"
ds check-remote
ran "the same release (a re-run): allowed" 0 "Rafi 0.1.0 (this release: a re-run)"
for older in 0.0.9 0.1.0-rc.1; do
  restate_state "$(ours "$older")"
  ds check-remote
  ran "a newer release than Restate's $older: allowed" 0 "Raffle: on dp_raffle, Rafi $older" "checked: Rafi 0.1.0"
done
for newer in 0.2.0 0.1.1-alpha; do
  restate_state "$(ours "$newer")"
  ds check-remote
  ran "an older release than Restate's $newer: refused" 1 \
    "Raffle runs on Rafi $newer (dp_raffle), newer than 0.1.0: refusing to deploy an older release (a downgrade); nothing was deployed" \
    "Set VERSION to $newer or later"
  not_logged "nothing past Restate after the refusal ($newer)" "/cf/" "--tag" "POST"
done
restate_state "$(ours 0.1.0)" "$(ours 0.3.0)"
ds check-remote
ran "EngineInfo on a newer release: refused" 1 "EngineInfo runs on Rafi 0.3.0"
restate_state "$(ours 9.0.0 other-app)"
ds check-remote
ran "another installation's newer deployment: no guard" 0 "not a deployment of this installation"
restate_state "$(ours 0.0.9)"
ds check-remote RESTATE_ADMIN_URL=http://127.0.0.1:1/restate
ran "Restate unreachable: failed" 1 "Restate's services: no answer from http://127.0.0.1:1/restate"
not_logged "nothing else ran after Restate didn't answer" "http " "--tag"
state <<'EOF'
{ "subdomain": "acme", "status": { "GET /restate/deployments": 503 } }
EOF
ds check-remote
ran "Restate's deployments unreadable: failed" 1 "Restate's deployments: HTTP 503"
not_logged "Cloudflare not asked when Restate fails" "/cf/"
ds check-remote RESTATE_AUTH_TOKEN=wrong
ran "a Restate key that doesn't work: failed" 1 "Restate's services: HTTP 401"
restate_state "$(ours 0.0.9)"
ds check-remote CLOUDFLARE_API_TOKEN=wrong
ran "a Cloudflare token that doesn't work: failed" 1 "the account's workers.dev subdomain: HTTP 401" "Authentication error"
not_logged "nothing deployed with a bad token" "--tag" "POST"
ds check-remote RESTATE_ADMIN_URL=
ran "no RESTATE_ADMIN_URL: refused" 1 "not set: RESTATE_ADMIN_URL"
not_logged "nothing asked without it" "http"

echo "== promote-app"
ds promote-app RAFI_APP_PROMOTED=true
ran "after a first install: nothing" 0 "nothing to promote"
not_logged "nothing deployed after a first install" "versions deploy" "triggers"
reported "its report" "deployed directly by upload-app (a first install): nothing to promote"
ds promote-app RAFI_APP_VERSION_ID=1b2c3d4e-0000-4000-8000-000000000001
ran "an uploaded version deployed" 0
in_order "the version at 100%, then its address and Cron" \
  "wrangler versions deploy 1b2c3d4e-0000-4000-8000-000000000001@100% --yes -c $APP --message rafi v0.1.0" \
  "wrangler triggers deploy -c $APP"
reported "promote-app's report" "### App Worker \`rafi-app\`" "Version \`1b2c3d4e-0000-4000-8000-000000000001\` (Rafi v0.1.0) deployed at 100%, with its address (workers.dev or its routes) and Cron"
ds promote-app
ran "neither output: refused" 1 "neither RAFI_APP_PROMOTED=true nor RAFI_APP_VERSION_ID"
ds promote-app RAFI_APP_VERSION_ID=1b2c3d4e-0000-4000-8000-000000000001 STUB_WRANGLER_FAIL="versions deploy"
ran "the version's deploy failing: failed" 1
not_logged "no triggers without the version" "triggers deploy"

echo "== cleanup"
meta() { printf '{ "app": "rafi", "instance": "%s", "cloudflare_account": "%s", "version": "0.0.1", "worker": "%s" }' "$1" "$2" "$3"; }
state <<EOF
{ "scripts": { "rafi-engine-v0-0-8": {}, "rafi-engine-v0-0-9": {}, "rafi-engine-v0-1-0": {}, "rafi-engine-v0-0-6": {},
               "rafi-engine-v0-0-5": {}, "rafi-engine-v0-0-4": {}, "rafi-engine-v0-0-3": {}, "rafi-engine-eu-v0-0-4": {}, "rafi-engine-old": {},
               "rafi-app": {}, "rafi-live": {} },
  "query": [ { "id": "inv_1", "status": "paused" }, { "id": "inv_2", "status": "suspended" } ],
  "deployments": [
    { "id": "dp_drained", "uri": "https://rafi-engine-v0-0-8.acme.workers.dev/restate", "metadata": $(meta rafi-app acct123 rafi-engine-v0-0-8) },
    { "id": "dp_gone", "uri": "https://rafi-engine-v0-0-7.acme.workers.dev/restate", "metadata": $(meta rafi-app acct123 rafi-engine-v0-0-7) },
    { "id": "dp_draining", "uri": "https://rafi-engine-v0-0-9.acme.workers.dev/restate", "metadata": $(meta rafi-app acct123 rafi-engine-v0-0-9) },
    { "id": "dp_active", "uri": "https://rafi-engine-v0-1-0.acme.workers.dev/restate", "metadata": $(meta rafi-app acct123 rafi-engine-v0-1-0) },
    { "id": "dp_nometa", "uri": "https://raffle.example.org/restate" },
    { "id": "dp_otherinst", "uri": "https://rafi-engine-eu-v0-0-5.acme.workers.dev/restate", "metadata": $(meta rafi-app-eu acct123 rafi-engine-eu-v0-0-5) },
    { "id": "dp_otheracct", "uri": "https://rafi-engine-v0-0-5.acme.workers.dev/restate", "metadata": $(meta rafi-app acct999 rafi-engine-v0-0-5) },
    { "id": "dp_appworker", "uri": "https://rafi-app.acme.workers.dev/restate", "metadata": $(meta rafi-app acct123 rafi-app) },
    { "id": "dp_elsewhere", "uri": "https://evil.acme.workers.dev/restate", "metadata": $(meta rafi-app acct123 rafi-engine-v0-0-6) },
    { "id": "dp_otherapp", "uri": "https://rafi-engine-v0-0-3.acme.workers.dev/restate", "metadata": $(meta acme-app acct123 rafi-engine-v0-0-3) }
  ] }
EOF
cat >"$TMP/list.json" <<'EOF'
{ "deployments": [
  { "deployment_id": "dp_drained", "endpoint": "https://rafi-engine-v0-0-8.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_gone", "endpoint": "https://rafi-engine-v0-0-7.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_draining", "endpoint": "https://rafi-engine-v0-0-9.acme.workers.dev/restate", "status": "Draining", "active_invocations": 2, "services": [] },
  { "deployment_id": "dp_active", "endpoint": "https://rafi-engine-v0-1-0.acme.workers.dev/restate", "status": "Active", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_nometa", "endpoint": "https://raffle.example.org/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_otherinst", "endpoint": "https://rafi-engine-eu-v0-0-5.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_otheracct", "endpoint": "https://rafi-engine-v0-0-5.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_appworker", "endpoint": "https://rafi-app.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_elsewhere", "endpoint": "https://evil.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] },
  { "deployment_id": "dp_otherapp", "endpoint": "https://rafi-engine-v0-0-3.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] }
] }
EOF
cp "$TMP/state.json" "$TMP/cleanup.json"
cp "$TMP/list.json" "$TMP/list.good"
# cleanup_state <JSON patch>: the cleanup's state with the patch's keys over it.
cleanup_state() {
  jq --argjson patch "$1" '. + $patch' "$TMP/cleanup.json" >"$TMP/state.json"
}
ds cleanup
ran "cleanup" 0 "dp_drained (rafi-engine-v0-0-8) is drained" "deleted the Worker rafi-engine-v0-0-8" \
  "removed the deployment dp_drained" "the Worker rafi-engine-v0-0-7 is already gone" "removed the deployment dp_gone" \
  "dp_draining (rafi-engine-v0-0-9) is draining: 2 active, 1 paused invocation(s)" "inv_1 paused" \
  "dp_active (rafi-engine-v0-1-0) is active" \
  "dp_nometa at https://raffle.example.org/restate: not a Rafi deployment" "dp_otherinst at" "another installation (rafi-app-eu)" \
  "dp_otheracct at" "another Cloudflare account" "dp_appworker at" "its worker is not a rafi-engine-v... engine Worker (rafi-app)" \
  "dp_elsewhere at" "its address is not the workers.dev one of rafi-engine-v0-0-6" "dp_otherapp at" "another installation (acme-app)" \
  "the Worker rafi-engine-v0-0-4: no Restate deployment of this installation refers to it" "2 drained engine deployment(s) removed"
in_order "every listing first, then each drained deployment removed from Restate before its Worker is deleted" \
  "http GET /restate/deployments" "restate deployments list --extra --json" "http GET /cf/accounts/acct123/workers/scripts" \
  "restate deployments remove dp_drained --yes" "http DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-8" \
  "restate deployments remove dp_gone --yes" "http DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-7"
first_remove="$(grep -nF 'restate deployments remove dp_drained --yes' "$LOG" | head -1 | cut -d: -f1 || true)"
first_delete="$(grep -nF 'http DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-8' "$LOG" | head -1 | cut -d: -f1 || true)"
if [ -n "$first_remove" ] && [ -n "$first_delete" ] && [ "$first_remove" -lt "$first_delete" ]; then
  pass "no deletion of a Worker before its deployment's removal"
else
  fail "the Worker rafi-engine-v0-0-8 deleted (line $first_delete) before dp_drained was removed (line $first_remove)"
fi
not_logged "nothing else deleted or removed" "scripts/rafi-engine-v0-0-9" "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-1-0" \
  "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-5" "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-6" \
  "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-4" "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-eu" \
  "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-3" \
  "remove dp_draining" "remove dp_active" "remove dp_nometa" \
  "remove dp_otherinst" "remove dp_otheracct" "remove dp_appworker" "remove dp_elsewhere" "remove dp_otherapp" "--force"
if grep -qE '^http DELETE /cf/accounts/acct123/workers/scripts/rafi-(app|live)$' "$LOG"; then fail "the app or live Worker deleted"; else pass "the app and live Workers never deleted"; fi
reported "the cleanup's report" "### Restate cleanup: \`rafi-app\` (Rafi v0.1.0)" \
  "\`dp_drained\`: removed, its Worker \`rafi-engine-v0-0-8\` deleted" "\`dp_gone\`: removed, its Worker \`rafi-engine-v0-0-7\` already gone" \
  "**Still draining**" "\`dp_draining\` (\`rafi-engine-v0-0-9\`): 2 active, 1 paused invocation(s); paused: \`inv_1\`" \
  "restate invocations resume <id> --deployment latest" "restate invocations kill <id>" \
  "**Workers left behind**" "\`rafi-engine-v0-0-4\`: no Restate deployment of this installation refers to it" \
  "\`rafi-engine-v0-0-6\`: no Restate deployment of this installation refers to it" "delete <worker>" \
  "**Not this installation's** (never touched)" "\`dp_otherinst\`" "\`dp_nometa\`" "\`dp_otherapp\`"
# A Worker of the engine's name that only another installation's deployment refers to (another app
# Worker's, another account's) is a leftover of this one's all the same.
reported "a Worker only another installation's deployment refers to: reported" \
  "\`rafi-engine-v0-0-3\`: no Restate deployment of this installation refers to it" \
  "\`rafi-engine-v0-0-5\`: no Restate deployment of this installation refers to it"
not_reported "leftovers: only the engine name's Workers, not those this installation's deployments refer to" \
  "\`rafi-engine-eu-v0-0-4\`" "\`rafi-engine-v0-1-0\`:" "\`rafi-engine-v0-0-9\`:" "rafi-engine-old" "\`rafi-app\`:" "\`rafi-live\`:"
cleanup_state '{ "status": { "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-8": 500 } }'
ds cleanup
ran "a Worker's deletion failing: left behind, the rest go on, failed" 1 \
  "deleting the Worker rafi-engine-v0-0-8: HTTP 500" "it is left behind (its deployment dp_drained is removed)" \
  "removed the deployment dp_gone" "the cleanup failed in part"
in_order "its deployment removed first, the others still cleaned up" "restate deployments remove dp_drained --yes" \
  "http DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-8" "restate deployments remove dp_gone --yes"
reported "the Worker reported left behind" "\`rafi-engine-v0-0-8\`: its deployment \`dp_drained\` is removed, but deleting it failed (HTTP 500"
cp "$TMP/cleanup.json" "$TMP/state.json"
ds cleanup STUB_RESTATE_REMOVE_FAIL=dp_drained
ran "Restate refusing a removal: its Worker untouched, failed, the next cleanup retries" 1 "Restate didn't remove dp_drained" \
  "removed the deployment dp_gone"
not_logged "the refused deployment's Worker untouched" "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-8"
reported "the refusal reported" "**Not removed**" "\`dp_drained\` (\`rafi-engine-v0-0-8\`): Restate refused to remove it; its Worker is untouched"
cleanup_state '{ "status": { "GET /restate/deployments": 500 } }'
ds cleanup
ran "the deployments' metadata unlisted: stopped" 1 "listing Restate's deployments: HTTP 500" "nothing removed or deleted"
not_logged "nothing removed or deleted without the metadata" "DELETE" "deployments remove"
cp "$TMP/cleanup.json" "$TMP/state.json"
ds cleanup STUB_RESTATE_LIST_FAIL=1
ran "the deployments' status unlisted: stopped" 1 "restate deployments list failed: nothing removed or deleted"
not_logged "nothing removed or deleted without the status" "DELETE" "deployments remove"
cleanup_state '{ "status": { "GET /cf/accounts/acct123/workers/scripts": 403 } }'
ds cleanup
ran "the account's Workers unlisted: stopped" 1 "listing the account's Workers: HTTP 403" "nothing removed or deleted"
not_logged "nothing removed or deleted without the Workers" "DELETE" "deployments remove"
if [ -s "$REPORT" ]; then fail "a report when a listing failed: $(cat "$REPORT")"; else pass "no report when a listing failed"; fi
cp "$TMP/cleanup.json" "$TMP/state.json"
echo 'not json' >"$TMP/list.json"
ds cleanup
ran "a status listing that isn't one: stopped" 1 "couldn't read Restate's deployments: nothing removed or deleted"
not_logged "nothing removed or deleted on a listing that isn't one" "DELETE" "deployments remove"
cp "$TMP/list.good" "$TMP/list.json"
ds cleanup RESTATE_AUTH_TOKEN=
ran "cleanup without Restate's token" 1 "not set: RESTATE_AUTH_TOKEN"
# This release's engine Worker, deployed but not registered yet: not a leftover to delete.
cleanup_state '{ "deployments": [] }'
echo '{ "deployments": [] }' >"$TMP/list.json"
ds cleanup
ran "this release's unregistered engine Worker isn't a leftover" 0 "rafi-engine-v0-1-0, this release's engine, isn't registered with Restate"
not_reported "not reported to delete" "\`rafi-engine-v0-1-0\`:"
cp "$TMP/list.good" "$TMP/list.json"
# In an environment: its own instance and engine Workers only.
add_env eu
cleanup_state "{ \"deployments\": [
    { \"id\": \"dp_eu\", \"uri\": \"https://rafi-engine-eu-v0-0-8.acme.workers.dev/restate\", \"metadata\": $(meta rafi-app-eu acct123 rafi-engine-eu-v0-0-8) },
    { \"id\": \"dp_drained\", \"uri\": \"https://rafi-engine-v0-0-8.acme.workers.dev/restate\", \"metadata\": $(meta rafi-app acct123 rafi-engine-v0-0-8) } ],
  \"scripts\": { \"rafi-engine-eu-v0-0-8\": {}, \"rafi-engine-eu-v0-0-4\": {}, \"rafi-engine-v0-0-4\": {}, \"rafi-engine-v0-0-8\": {} } }"
echo '{ "deployments": [
  { "deployment_id": "dp_eu", "status": "Drained", "active_invocations": 0 },
  { "deployment_id": "dp_drained", "status": "Drained", "active_invocations": 0 } ] }' >"$TMP/list.json"
ds cleanup CLOUDFLARE_ENV=eu
ran "an environment's cleanup" 0 "installation rafi-app-eu" "dp_drained at" "another installation (rafi-app)" \
  "the Worker rafi-engine-eu-v0-0-4: no Restate deployment of this installation refers to it"
in_order "the environment's drained deployment removed, then its Worker deleted" \
  "restate deployments remove dp_eu --yes" "http DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-eu-v0-0-8"
not_logged "the top level's untouched" "remove dp_drained" "scripts/rafi-engine-v0-0-8"
reported "the environment's report" "### Restate cleanup: \`rafi-app-eu\`" "\`rafi-engine-eu-v0-0-4\`: no Restate deployment of this installation refers to it"
not_reported "the top level's Workers not the environment's leftovers" "\`rafi-engine-v0-0-4\`"
cp "$TMP/list.good" "$TMP/list.json"
restore
# The Restate CLI by default: npx -y @restatedev/restate.
cp "$TMP/cleanup.json" "$TMP/state.json"
ds cleanup RESTATE= PATH="$NPX_BIN:$PATH"
ran "the Restate CLI by default: npx -y" 0 "2 drained engine deployment(s) removed"
in_order "npx -y @restatedev/restate for the listing and the removals" \
  "npx -y @restatedev/restate deployments list --extra --json" "restate deployments list --extra --json" \
  "npx -y @restatedev/restate deployments remove dp_drained --yes"

echo "== the artifacts (RAFI_ARTIFACTS_DIR)"
rm -rf "$ARTIFACTS"
mkdir -p "$ARTIFACTS/dry-run"
echo stray >"$ARTIFACTS/dry-run/stray"
ds dry-run RAFI_ARTIFACTS_DIR="$ARTIFACTS"
ran "dry-run with RAFI_ARTIFACTS_DIR" 0
for key in live engine app; do
  json_is "dry-run: the $key Worker's dry run kept (wrangler's ND-JSON, its deploy entry)" \
    "$ARTIFACTS/dry-run/wrangler-dry-run-$key.ndjson" 'select(.type == "deploy") | .worker_name == "rafi-'"$key"'"'
done
if [ -e "$ARTIFACTS/dry-run/stray" ]; then fail "an earlier run's file kept"; else pass "each run replaces its subcommand's artifacts"; fi
ds dry-run
if [ -e "$INSTALL/dry-run" ] || [ -n "$(find "$INSTALL" -name 'wrangler-*.ndjson' -print -quit)" ]; then
  fail "artifacts written without RAFI_ARTIFACTS_DIR"
else
  pass "no artifacts left without RAFI_ARTIFACTS_DIR"
fi
echo '{ "subdomain": "acme" }' | state
ds deploy-engine RAFI_ARTIFACTS_DIR="$ARTIFACTS"
json_is "deploy-engine: wrangler's ND-JSON of the deploy" "$ARTIFACTS/deploy-engine/wrangler-deploy-engine.ndjson" \
  'select(.type == "wrangler-session") | .version == 1'
app_state "v0.0.9" ""
ds upload-app RAFI_ARTIFACTS_DIR="$ARTIFACTS"
json_is "upload-app: wrangler's ND-JSON of the upload" "$ARTIFACTS/upload-app/wrangler-versions-upload-app.ndjson" \
  'select(.type == "version-upload") | .version_id == "1b2c3d4e-0000-4000-8000-000000000001"'
rm -rf "$RELEASE/app/assets/legal"
ds promote-app RAFI_APP_VERSION_ID=1b2c3d4e-0000-4000-8000-000000000001 RAFI_ARTIFACTS_DIR="$ARTIFACTS"
for what in versions-deploy-app triggers-deploy-app; do
  json_is "promote-app: wrangler's ND-JSON of $what" "$ARTIFACTS/promote-app/wrangler-$what.ndjson" '.type == "wrangler-session"'
done
restate_state "$(ours 0.0.9)"
ds register-engine "${REG[@]}" RAFI_ARTIFACTS_DIR="$ARTIFACTS"
ran "register-engine with RAFI_ARTIFACTS_DIR" 0
json_is "register-engine: Restate's services" "$ARTIFACTS/register-engine/restate-services.json" '.services | length == 3'
json_is "register-engine: Restate's deployments" "$ARTIFACTS/register-engine/restate-deployments.json" '.deployments | length == 3'
json_is "register-engine: the registration's answer" "$ARTIFACTS/register-engine/restate-register.json" '.id == "dp_new"'
# A relative RAFI_ARTIFACTS_DIR is the installation's root's.
cp "$TMP/cleanup.json" "$TMP/state.json"
ds cleanup RAFI_ARTIFACTS_DIR=rafi-artifacts
ran "cleanup with a relative RAFI_ARTIFACTS_DIR" 0
C="$INSTALL/rafi-artifacts/cleanup"
json_is "cleanup: Restate's deployments" "$C/restate-deployments.json" '.deployments | length == 10'
json_is "cleanup: the CLI's listing" "$C/restate-deployments-list.json" '.deployments | length == 10'
json_is "cleanup: the account's Workers" "$C/cloudflare-workers.json" '.result | length == 11'
json_is "cleanup.json: removed" "$C/cleanup.json" '.removed == [
  { "deployment": "dp_drained", "worker": "rafi-engine-v0-0-8", "worker_status": "deleted" },
  { "deployment": "dp_gone", "worker": "rafi-engine-v0-0-7", "worker_status": "already_gone" } ]'
json_is "cleanup.json: deleted" "$C/cleanup.json" '.deleted == ["rafi-engine-v0-0-8", "rafi-engine-v0-0-7"]'
json_is "cleanup.json: draining, with the paused invocations" "$C/cleanup.json" '.draining == [
  { "deployment": "dp_draining", "worker": "rafi-engine-v0-0-9", "active_invocations": 2, "paused_invocations": ["inv_1"] } ]'
json_is "cleanup.json: kept" "$C/cleanup.json" '.kept == [ { "deployment": "dp_active", "worker": "rafi-engine-v0-1-0", "status": "Active" } ]'
json_is "cleanup.json: leftovers" "$C/cleanup.json" '.leftover == ["rafi-engine-v0-0-3", "rafi-engine-v0-0-4", "rafi-engine-v0-0-5", "rafi-engine-v0-0-6"]'
json_is "cleanup.json: nothing left behind, not failed" "$C/cleanup.json" '.left_behind == [] and .not_removed == [] and .failed == false'
json_is "cleanup.json: not ours" "$C/cleanup.json" \
  '[.not_ours[].deployment] == ["dp_nometa", "dp_otherinst", "dp_otheracct", "dp_appworker", "dp_elsewhere", "dp_otherapp"]
   and (.not_ours[1].reason == "another installation (rafi-app-eu)")'
json_is "cleanup.json: the installation" "$C/cleanup.json" \
  '.installation == "rafi-app" and .version == "0.1.0" and .cloudflare_account == "acct123"'
cleanup_state '{ "status": { "DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-v0-0-8": 500 } }'
ds cleanup RAFI_ARTIFACTS_DIR="$ARTIFACTS" STUB_RESTATE_REMOVE_FAIL=dp_gone
ran "a failing cleanup with RAFI_ARTIFACTS_DIR" 1 "the cleanup failed in part"
json_is "cleanup.json of a failing cleanup: left behind, not removed, failed" "$ARTIFACTS/cleanup/cleanup.json" \
  '.failed == true and .deleted == []
   and .removed == [ { "deployment": "dp_drained", "worker": "rafi-engine-v0-0-8", "worker_status": "left_behind" } ]
   and .left_behind[0].worker == "rafi-engine-v0-0-8" and (.left_behind[0].error | startswith("HTTP 500"))
   and .not_removed[0].deployment == "dp_gone"'
if grep -rqF -e cf-token -e rs-token "$ARTIFACTS" "$INSTALL/rafi-artifacts"; then
  fail "a token among the artifacts: $(grep -rlF -e cf-token -e rs-token "$ARTIFACTS" "$INSTALL/rafi-artifacts")"
else
  pass "no token among the artifacts ($(find "$ARTIFACTS" "$INSTALL/rafi-artifacts" -type f | wc -l | tr -d ' ') files)"
fi
rm -rf "$INSTALL/rafi-artifacts" "$RELEASE/app/assets/legal"

if [ "$FAILED" != 0 ]; then
  echo "cloudflare-deploy-test: $FAILED FAILED, $PASSED passed" >&2
  exit 1
fi
echo "cloudflare-deploy-test: all $PASSED passed"
