#!/usr/bin/env bash
# Checks the Cloudflare deploy script (scripts/cloudflare-deploy.sh, a release's deploy.sh)
# without a Cloudflare or Restate account: each subcommand run as an installation's workflows
# run it, from a copy of deploy/cloudflare with a release laid out in release/, against a
# stubbed wrangler CLI and Restate CLI (which log their arguments) and a fake Cloudflare and
# Restate admin API (a local node HTTP server answering from a state file and logging each
# request, in one log with the stubs, so the order of calls shows). The configurations are
# read by wrangler's own reader (unstable_readConfig) from the devenv's wrangler package
# (RAFI_WRANGLER_PKG; the wrangler on PATH's when unset), as deploy.sh reads them.
#
#   - check: the release's VERSION and the installation's version file (VERSION.<env> when
#     CLOUDFLARE_ENV is set and it exists, VERSION otherwise; and RAFI_VERSION), the engine
#     Worker's versioned name, the usage, the tools installed once when not given; the three
#     configurations read and cross-checked, broken copies of them refused, placeholders
#     refused (and only warned about by dry-run, which dry-deploys the three Workers);
#     Wrangler environments: none, one without a name of its own (<name>-<env>), one with
#     its own, an environment missing from a configuration, missing vars (the engine's
#     identity keys), rate limits or required secrets, and a binding to the top level's live
#     Worker refused; wrangler given CLOUDFLARE_ENV;
#   - migrate; deploy-live skipping when the deployed version's tag is the fingerprint and
#     deploying when the bundle or the configuration as wrangler reads it changed (not a
#     comment, not another environment), or nothing, or two versions, are deployed;
#     upload-app's first install (deployed directly, app_promoted) and upgrade (a version
#     uploaded, app_version_id from wrangler's ND-JSON output), a missing Worker refused;
#     deploy-engine's name and URL; register-engine's downgrade guard (semantic versioning
#     with pre-releases) and the metadata (instance: the app Worker's name) and "force":
#     false it sends, 5xx retried, 4xx failing; check-remote's same guard before anything is
#     deployed, Restate unreachable or a key or token that doesn't work failing it, and
#     nothing but reads; promote-app's two paths; the outputs (RAFI_OUTPUT_FILE, or printed)
#     and the reports (RAFI_REPORT_FILE);
#   - cleanup: filtering by metadata (the app Worker's name and the account), each drained
#     deployment removed from Restate before its Worker is deleted, a Worker already gone, a
#     deletion failing (the Worker left behind, the run failing), a removal refused (the
#     Worker untouched), the account's leftover engine Workers reported and never deleted,
#     any listing failing stopping it before anything is removed or deleted, and its report;
#     the same in a Wrangler environment.
#
# scripts/cloudflare-release.sh runs it; so can you (in the devenv, for its wrangler):
#
#   ./scripts/cloudflare-deploy-test.sh
#
# Needs node, curl and wrangler 4.148.0's package.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT/scripts/cloudflare-deploy.sh"

# The wrangler package whose configuration reader deploy.sh uses: the devenv's.
if [ -z "${RAFI_WRANGLER_PKG:-}" ]; then
  command -v wrangler >/dev/null || { echo "no wrangler on PATH: run in the devenv, or set RAFI_WRANGLER_PKG" >&2; exit 1; }
  RAFI_WRANGLER_PKG="$(dirname "$(readlink -f "$(command -v wrangler)")")/../lib"
fi
RAFI_WRANGLER_PKG="$(cd "$RAFI_WRANGLER_PKG" && pwd -P)"
[ -f "$RAFI_WRANGLER_PKG/wrangler-dist/cli.js" ] || { echo "$RAFI_WRANGLER_PKG isn't a wrangler package" >&2; exit 1; }

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
# CLOUDFLARE_ENV it was given, on a line of its own). wrangler fails when its arguments
# contain STUB_WRANGLER_FAIL, writes a version-upload entry for `versions upload` (unless
# STUB_NO_VERSION_ID), and answers --dry-run. The Restate CLI lists STUB_RESTATE_LIST (or
# fails with STUB_RESTATE_LIST_FAIL) and refuses to remove the IDs in STUB_RESTATE_REMOVE_FAIL.
# npm (on PATH only where a test puts it) "installs" wrangler as a link to the real package.
BIN="$TMP/bin"
NPM_BIN="$TMP/npm-bin"
mkdir -p "$BIN" "$NPM_BIN"
cat >"$BIN/wrangler" <<'EOF'
#!/usr/bin/env bash
echo "wrangler $*" >>"$STUB_LOG"
[ -z "${CLOUDFLARE_ENV:-}" ] || echo "wrangler-env $CLOUDFLARE_ENV" >>"$STUB_LOG"
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
    || echo '{"type":"version-upload","version":1,"worker_name":"rafi-app","version_id":"1b2c3d4e-0000-4000-8000-000000000001"}' >>"$WRANGLER_OUTPUT_FILE_PATH"
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
cat >"$NPM_BIN/npm" <<'EOF'
#!/usr/bin/env bash
echo "npm $*" >>"$STUB_LOG"
prefix=
while [ "$#" -gt 0 ]; do
  case "$1" in --prefix) prefix="$2"; shift ;; esac
  shift
done
mkdir -p "$prefix/node_modules/.bin"
ln -sfn "$STUB_WRANGLER_PKG" "$prefix/node_modules/wrangler"
printf '#!/usr/bin/env bash\necho "tools-wrangler $*" >>"$STUB_LOG"\n' >"$prefix/node_modules/.bin/wrangler"
chmod +x "$prefix/node_modules/.bin/wrangler"
EOF
chmod +x "$BIN/wrangler" "$BIN/restate" "$NPM_BIN/npm"

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
state() { cat >"$TMP/state.json"; }
echo '{}' | state
: >"$LOG"
node "$TMP/fake.mjs" "$TMP/state.json" "$LOG" "$TMP/port" &
SERVER=$!
for _ in $(seq 1 50); do [ -s "$TMP/port" ] && break; sleep 0.1; done
[ -s "$TMP/port" ] || { echo "the fake API didn't start" >&2; exit 1; }
BASE="http://127.0.0.1:$(cat "$TMP/port")"

# A release as `just cloudflare-artifact` lays it out (empty files will do, the live Worker's
# bundle with something in it), unpacked into an installation (a copy of deploy/cloudflare) as
# release/.
INSTALL="$TMP/installation"
mkdir -p "$INSTALL"
cp -R "$ROOT/deploy/cloudflare/." "$INSTALL/"
rm -rf "$INSTALL/release" "$INSTALL/.wrangler" "$INSTALL/.prod.vars" "$INSTALL/.rafi-tools"
RELEASE="$INSTALL/release"
mkdir -p "$RELEASE/app/build" "$RELEASE/app/assets" "$RELEASE/engine/build" "$RELEASE/live/build" "$RELEASE/migrations"
touch "$RELEASE/app/build/index.js" "$RELEASE/engine/build/index.js"
echo 'export { RaffleRoom, default };' >"$RELEASE/live/build/index.js"
echo 'wasm' >"$RELEASE/live/build/index_bg.wasm"
cp "$ROOT"/migrations/sqlite/*.sql "$RELEASE/migrations/"
cp "$SCRIPT" "$RELEASE/deploy.sh"
echo "0.1.0" >"$RELEASE/VERSION"
APP="$INSTALL/app.wrangler.toml"
ENGINE="$INSTALL/engine.wrangler.toml"
LIVE="$INSTALL/live.wrangler.toml"
DB_ID=6f1c2a90-3b7e-4d2a-9c41-0e5b8a7d2f13
KEY=publickeyv1_ChjENKeMvCtRnqG2mrBK1HmPKufgFUc98K8B3ZgXonQQ

# What the workflows give every subcommand; a test's own assignments come after (and win), an
# empty one unsets. No tools are installed: wrangler is the stub, its reader the devenv's.
COMMON=(WRANGLER="$BIN/wrangler" RAFI_WRANGLER_PKG="$RAFI_WRANGLER_PKG" RAFI_TOOLS_DIR="$TMP/tools"
  RESTATE="$BIN/restate" STUB_LOG="$LOG" STUB_WRANGLER_PKG="$RAFI_WRANGLER_PKG"
  STUB_RESTATE_LIST="$TMP/list.json" CLOUDFLARE_API_BASE_URL="$BASE/cf" CLOUDFLARE_API_TOKEN=cf-token
  CLOUDFLARE_ACCOUNT_ID=acct123 RESTATE_ADMIN_URL="$BASE/restate/" RESTATE_AUTH_TOKEN=rs-token
  RAFI_REGISTER_WAIT=0 RAFI_OUTPUT_FILE="$OUTPUTS" RAFI_REPORT_FILE="$REPORT" RAFI_VERSION= CLOUDFLARE_ENV=
  RAFI_APP_PROMOTED= RAFI_APP_VERSION_ID= RAFI_ENGINE_WORKER= RAFI_ENGINE_URL= STUB_WRANGLER_FAIL=
  STUB_NO_VERSION_ID= STUB_RESTATE_LIST_FAIL= STUB_RESTATE_REMOVE_FAIL= GITHUB_OUTPUT= GITHUB_STEP_SUMMARY=)

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

echo "== the template: placeholders"
ds check
ran "check refuses the template's database_id" 1 "database_id is the placeholder"
ran "check refuses the template's identity key" 1 "RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS isn't a list of publickeyv1_"
ds dry-run CLOUDFLARE_API_TOKEN= CLOUDFLARE_ACCOUNT_ID= RESTATE_ADMIN_URL= RESTATE_AUTH_TOKEN=
ran "dry-run only warns about them, and needs no credentials" 0 "warning:" "database_id is the placeholder" \
  "dry run of Rafi 0.1.0 done"
in_order "dry-run: the live Worker, the engine under its versioned name, the app" \
  "wrangler deploy --dry-run -c $LIVE" "wrangler deploy --dry-run -c $ENGINE --name rafi-engine-v0-1-0" \
  "wrangler deploy --dry-run -c $APP"
not_logged "no environment given to wrangler" "wrangler-env"
if cmp -s "$INSTALL/legal/terms.md" "$RELEASE/app/assets/legal/terms.md" \
  && cmp -s "$INSTALL/legal/privacy.md" "$RELEASE/app/assets/legal/privacy.md"; then
  pass "dry-run copies legal/ into the app's static files"
else
  fail "legal/ isn't in $RELEASE/app/assets/legal"
fi
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

echo "== check: the release, the installation's version file, the tools"
ds check
ran "an installation's" 0 "Rafi 0.1.0 (VERSION)" "environment: none" "engine Worker: rafi-engine-v0-1-0 (" \
  "live Worker: rafi-live (" "app Worker: rafi-app (" "database: $DB_ID; Durable Objects' jurisdiction: eu"
if [ -s "$LOG" ]; then fail "check called: $(cat "$LOG")"; else pass "check calls nothing"; fi
if [ -e "$TMP/tools" ]; then fail "tools installed although both were given"; else pass "no tools installed when WRANGLER and RAFI_WRANGLER_PKG are given"; fi
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
echo "0.2.0" >"$INSTALL/VERSION.eu"
ds check
ran "VERSION.eu without CLOUDFLARE_ENV: VERSION is the one" 0 "Rafi 0.1.0 (VERSION)"
rm "$INSTALL/VERSION.eu"
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
ds deploy-rooms
ran "the old deploy-rooms exits 2" 2
ds --help
ran "--help" 0 "release/deploy.sh register-engine" "release/deploy.sh deploy-live"
# Neither given: the pinned wrangler installed into RAFI_TOOLS_DIR (npm, stubbed here), once.
ds check WRANGLER= RAFI_WRANGLER_PKG= PATH="$NPM_BIN:$PATH"
ran "the tools installed when not given" 0 "tools: wrangler 4.148.0 into $TMP/tools" "app Worker: rafi-app ("
in_order "npm install of the pinned wrangler" "npm install --no-save --no-audit --no-fund --prefix $TMP/tools wrangler@4.148.0"
ds migrate WRANGLER= RAFI_WRANGLER_PKG= PATH="$NPM_BIN:$PATH"
ran "the tools used, not installed again" 0
not_logged "not installed again" "npm"
in_order "the tools directory's wrangler" "tools-wrangler d1 migrations apply DB -c $APP --remote"
rm -rf "$TMP/tools"

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
broken "the live Worker on another database" "$LIVE" "s/$DB_ID/11111111-2222-3333-4444-555555555555/" "isn't the app's ($DB_ID)"
broken "the engine binding another Worker's RaffleRoom" "$ENGINE" 's/^script_name = "rafi-live"/script_name = "rafi-app"/' "engine.wrangler.toml: RAFFLE_ROOM's script_name (rafi-app) must be the live Worker's name (rafi-live)"
broken "the app binding another Worker's RaffleRoom" "$APP" 's/^script_name = "rafi-live"/script_name = "live"/' "app.wrangler.toml: RAFFLE_ROOM's script_name (live) must be the live Worker's name (rafi-live)"
broken "the app with the class of its own" "$APP" '/^script_name = "rafi-live"/d' "app.wrangler.toml: RAFFLE_ROOM's script_name (null) must be the live Worker's name"
broken "the live Worker renamed, the others not" "$LIVE" 's/^name = "rafi-live"/name = "raffle-live"/' "must be the live Worker's name (raffle-live)"
broken "the engine in another jurisdiction" "$ENGINE" 's/^RAFI_DO_JURISDICTION = "eu"/RAFI_DO_JURISDICTION = "none"/' "engine.wrangler.toml: RAFI_DO_JURISDICTION (none) must be the live Worker's (eu)"
broken "the app in another jurisdiction" "$APP" 's/^RAFI_DO_JURISDICTION = "eu"/RAFI_DO_JURISDICTION = "fedramp"/' "app.wrangler.toml: RAFI_DO_JURISDICTION (fedramp) must be the live Worker's (eu)"
broken "the live Worker without a jurisdiction" "$LIVE" '/^RAFI_DO_JURISDICTION/d' "live.wrangler.toml: [vars] RAFI_DO_JURISDICTION is missing"
broken "the engine off workers.dev" "$ENGINE" 's/^workers_dev = true/workers_dev = false/' "workers_dev must be true"
broken "the live Worker on workers.dev" "$LIVE" 's/^workers_dev = false/workers_dev = true/' "live.wrangler.toml: workers_dev must be false"
broken "the live Worker without preview_urls = false" "$LIVE" '/^preview_urls/d' "live.wrangler.toml: preview_urls must be false"
broken "the live Worker without its migrations" "$LIVE" 's/^new_sqlite_classes = .*/new_sqlite_classes = ["Other"]/' "live.wrangler.toml: no [[migrations]] creates the RaffleRoom class"
broken "the engine named as the app" "$ENGINE" 's/^name = "rafi-engine"/name = "rafi-app"/' "must differ from the app's"
broken "the engine named as the live Worker" "$ENGINE" 's/^name = "rafi-engine"/name = "rafi-live"/' "must differ from the live Worker's"
broken "the app named as the live Worker" "$APP" 's/^name = "rafi-app"/name = "rafi-live"/' "app.wrangler.toml: name must differ from the live Worker's"
broken "the app's main elsewhere" "$APP" 's|^main = .*|main = "worker/build/index.js"|' "not this release's app/build/index.js"
broken "the engine's main elsewhere" "$ENGINE" 's|^main = .*|main = "release/app/build/index.js"|' "not this release's engine/build/index.js"
broken "the live Worker's main elsewhere" "$LIVE" 's|^main = .*|main = "release/app/build/index.js"|' "not this release's live/build/index.js"
broken "the assets elsewhere" "$APP" 's|^directory = .*|directory = "public"|' "not this release's app/assets"
broken "the migrations elsewhere" "$APP" 's|^migrations_dir = .*|migrations_dir = "release/migrations/sqlite"|' "not this release's migrations"
broken "a database_id that isn't a string (wrangler refuses it)" "$APP" "s/^database_id = .*/database_id = 42/" 'app.wrangler.toml: Processing'
broken "a key set twice (not TOML)" "$ENGINE" 's/^class_name = "RaffleRoom"/class_name = "RaffleRoom"\nclass_name = "RaffleRoom"/' "engine.wrangler.toml: Invalid TOML document"
broken "no RAFFLE_ROOM in the engine" "$ENGINE" 's/^name = "RAFFLE_ROOM"/name = "ROOM"/' 'engine.wrangler.toml: no [[durable_objects.bindings]] with name = "RAFFLE_ROOM"'
broken "no RAFFLE_ROOM in the live Worker" "$LIVE" 's/^name = "RAFFLE_ROOM"/name = "ROOM"/' 'live.wrangler.toml: no [[durable_objects.bindings]] with name = "RAFFLE_ROOM"'
broken "the live Worker's RAFFLE_ROOM with a script_name" "$LIVE" 's/^class_name = "RaffleRoom"/class_name = "RaffleRoom"\nscript_name = "rafi-app"/' "the live Worker's own class: no script_name"
broken "no identity keys" "$ENGINE" '/^RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS/d' "engine.wrangler.toml: [vars] RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS is missing"
for worker in engine live; do
  mv "$RELEASE/$worker/build/index.js" "$TMP/index.js"
  ds check
  ran "a release without the $worker Worker" 1 "the release has no $worker/build/index.js"
  mv "$TMP/index.js" "$RELEASE/$worker/build/index.js"
done
mv "$LIVE" "$TMP/live.aside"
ds check
ran "no live.wrangler.toml" 1 "live.wrangler.toml is missing"
mv "$TMP/live.aside" "$LIVE"

echo "== check: Wrangler environments (CLOUDFLARE_ENV)"
# add_env <env> [<the app's own name>]: an [env.<env>] in each configuration, repeating what
# environments don't inherit (vars, bindings, required secrets), RAFFLE_ROOM bound to the
# environment's live Worker.
add_env() {
  local env="$1" app_name="${2:-}" live_name="rafi-live-$1"
  {
    printf '\n[env.%s]\n' "$env"
    [ -z "$app_name" ] || printf 'name = "%s"\n' "$app_name"
    printf 'routes = [{ pattern = "%s.raffles.example", custom_domain = true }]\n' "$env"
    printf '[[env.%s.d1_databases]]\nbinding = "DB"\ndatabase_name = "rafi"\ndatabase_id = "%s"\nmigrations_dir = "release/migrations"\n' "$env" "$DB_ID"
    printf '[[env.%s.durable_objects.bindings]]\nname = "RAFFLE_ROOM"\nclass_name = "RaffleRoom"\nscript_name = "%s"\n' "$env" "$live_name"
    local n=2000 limit
    for limit in JOIN_PER_RAFFLE_LIMITER JOIN_PER_EMAIL_LIMITER CONNECT_LIMITER ADD_LIMITER; do
      n=$((n + 1))
      printf '[[env.%s.ratelimits]]\nname = "%s"\nnamespace_id = "%s"\nsimple = { limit = 10, period = 60 }\n' "$env" "$limit" "$n"
    done
    printf '[env.%s.vars]\nRAFI_DO_JURISDICTION = "eu"\n' "$env"
    printf '[env.%s.secrets]\nrequired = ["RAFI_PUBLIC_URL", "RAFI_ENGINE_INGRESS_URL", "RAFI_ENGINE_API_KEY", "RAFI_POW_KEY", "CLERK_SECRET_KEY", "CLERK_PUBLISHABLE_KEY", "RAFI_OPERATOR_NAME", "RAFI_OPERATOR_EMAIL", "RAFI_TERMS_VERSION", "RAFI_PRIVACY_VERSION"]\n' "$env"
  } >>"$APP"
  {
    printf '\n[env.%s]\n' "$env"
    printf '[[env.%s.d1_databases]]\nbinding = "DB"\ndatabase_name = "rafi"\ndatabase_id = "%s"\n' "$env" "$DB_ID"
    printf '[[env.%s.durable_objects.bindings]]\nname = "RAFFLE_ROOM"\nclass_name = "RaffleRoom"\nscript_name = "%s"\n' "$env" "$live_name"
    printf '[env.%s.vars]\nRAFI_DO_JURISDICTION = "eu"\nRAFI_RESTATE_ENDPOINT__IDENTITY_KEYS = "%s"\n' "$env" "$KEY"
  } >>"$ENGINE"
  {
    printf '\n[env.%s]\n' "$env"
    printf '[[env.%s.d1_databases]]\nbinding = "DB"\ndatabase_name = "rafi"\ndatabase_id = "%s"\n' "$env" "$DB_ID"
    printf '[[env.%s.durable_objects.bindings]]\nname = "RAFFLE_ROOM"\nclass_name = "RaffleRoom"\n' "$env"
    printf '[env.%s.vars]\nRAFI_DO_JURISDICTION = "eu"\n' "$env"
  } >>"$LIVE"
}
add_env staging
ds check
ran "no CLOUDFLARE_ENV: the top level, other environments ignored" 0 "environment: none" "app Worker: rafi-app (" \
  "live Worker: rafi-live (" "engine Worker: rafi-engine-v0-1-0 ("
ds check CLOUDFLARE_ENV=eu
ran "CLOUDFLARE_ENV naming an environment the configurations lack: refused" 1 \
  "CLOUDFLARE_ENV is eu, but app.wrangler.toml has no [env.eu]" "live.wrangler.toml has no [env.eu]"
ds check CLOUDFLARE_ENV='e u'
ran "CLOUDFLARE_ENV not an environment's name: refused" 1 "not an environment's name"
restore
add_env eu
cp "$APP" "$TMP/app.env"
cp "$ENGINE" "$TMP/engine.env"
cp "$LIVE" "$TMP/live.env"
restore_env() {
  restore
  cp "$TMP/app.env" "$APP"
  cp "$TMP/engine.env" "$ENGINE"
  cp "$TMP/live.env" "$LIVE"
}
ds check CLOUDFLARE_ENV=eu
ran "an environment without names of its own: <name>-<environment>" 0 "environment: eu ([env.eu] of each configuration)" \
  "app Worker: rafi-app-eu (" "live Worker: rafi-live-eu (" "engine Worker: rafi-engine-eu-v0-1-0 (" "Rafi 0.1.0 (VERSION)"
ds check
ran "the same configurations without CLOUDFLARE_ENV: the top level" 0 "app Worker: rafi-app (" "engine Worker: rafi-engine-v0-1-0 ("
echo "0.0.9" >"$INSTALL/VERSION.eu"
ds check CLOUDFLARE_ENV=eu
ran "VERSION.eu is the environment's version file" 1 "the installation's VERSION.eu says 0.0.9, but the release in $RELEASE is 0.1.0"
echo "0.1.0" >"$INSTALL/VERSION.eu"
echo "0.0.9" >"$INSTALL/VERSION"
ds check CLOUDFLARE_ENV=eu
ran "VERSION.eu, not VERSION, for the environment" 0 "Rafi 0.1.0 (VERSION.eu)"
ds check
ran "VERSION for the top level, VERSION.eu notwithstanding" 1 "the installation's VERSION says 0.0.9"
echo "0.1.0" >"$INSTALL/VERSION"
rm "$INSTALL/VERSION.eu"
restore
add_env eu raffles-eu
ds check CLOUDFLARE_ENV=eu
ran "an environment's app with a name of its own" 0 "app Worker: raffles-eu (" "live Worker: rafi-live-eu ("
restore_env
sed -i.orig '/^RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS = "publickeyv1_Chj[A-Za-z0-9]*"$/d' "$ENGINE"
ds check CLOUDFLARE_ENV=eu
ran "an environment's engine without the identity keys: refused (vars aren't inherited)" 1 \
  "engine.wrangler.toml: [env.eu] has no RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS in its vars: environments don't inherit vars"
restore_env
sed -i.orig '/^\[env.eu.vars\]$/,$d' "$ENGINE"
ds check CLOUDFLARE_ENV=eu
ran "an environment's engine without vars at all: refused" 1 "engine.wrangler.toml: [env.eu] has no RAFI_DO_JURISDICTION, RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS in its vars"
restore_env
sed -i.orig 's/^script_name = "rafi-live-eu"$/script_name = "rafi-live"/' "$APP"
ds check CLOUDFLARE_ENV=eu
ran "an environment's app bound to the top level's live Worker: refused" 1 \
  "app.wrangler.toml ([env.eu]): RAFFLE_ROOM's script_name (rafi-live) must be the live Worker's name (rafi-live-eu)"
restore_env
node -e '
  const fs = require("fs");
  const f = process.argv[1];
  fs.writeFileSync(f, fs.readFileSync(f, "utf8").replace(/\[\[env\.eu\.ratelimits\]\]\nname = "ADD_LIMITER"\n[^\n]*\n[^\n]*\n/, ""));' "$APP"
ds check CLOUDFLARE_ENV=eu
ran "an environment's app without a rate limit: refused" 1 "app.wrangler.toml: [env.eu] has no rate limit ADD_LIMITER"
restore_env
sed -i.orig '/^\[env.eu.secrets\]$/,$d' "$APP"
ds check CLOUDFLARE_ENV=eu
ran "an environment's app without its required secrets: refused" 1 "app.wrangler.toml: [env.eu] doesn't require"
restore_env
sed -i.orig '/^\[\[env.eu.d1_databases\]\]$/,/^database_id/d' "$LIVE"
ds check CLOUDFLARE_ENV=eu
ran "an environment's live Worker without the database: refused" 1 \
  "live.wrangler.toml ([env.eu]): no [[d1_databases]] with binding = \"DB\" (environments don't inherit bindings"
restore_env
ds dry-run CLOUDFLARE_ENV=eu
ran "dry-run in an environment" 0 "dry run of Rafi 0.1.0 done"
in_order "wrangler given the environment, the engine under the environment's versioned name" \
  "wrangler deploy --dry-run -c $LIVE" "wrangler-env eu" \
  "wrangler deploy --dry-run -c $ENGINE --name rafi-engine-eu-v0-1-0" "wrangler-env eu" \
  "wrangler deploy --dry-run -c $APP" "wrangler-env eu"
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
not_logged "nothing deployed" "wrangler deploy"
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
ran "a comment added to live.wrangler.toml: still unchanged" 0 "unchanged (it serves $TAG)"
restore
add_env staging
ds deploy-live
ran "another environment added to the configurations: still unchanged" 0 "unchanged (it serves $TAG)"
restore
sed -i.orig 's/^compatibility_date = .*/compatibility_date = "2026-10-02"/' "$LIVE"
ds deploy-live
ran "live.wrangler.toml changed: deployed" 0
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
not_logged "nothing deployed when Cloudflare fails" "wrangler deploy"

echo "== upload-app"
echo '{}' | state
ds upload-app
ran "no app Worker: refused, its secrets first" 1 "the app Worker rafi-app doesn't exist" "secret bulk .prod.vars -c app.wrangler.toml"
not_logged "nothing uploaded without an app Worker" "wrangler"
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
not_logged "not deployed" "wrangler deploy"
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
not_logged "nothing deployed without a subdomain" "wrangler"

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
echo "0.1.0" >"$INSTALL/VERSION.eu"
restate_state "$(ours 0.2.0)" "$(ours 0.0.9 rafi-app-eu)"
ds register-engine CLOUDFLARE_ENV=eu RAFI_ENGINE_WORKER=rafi-engine-eu-v0-1-0 RAFI_ENGINE_URL=https://rafi-engine-eu-v0-1-0.acme.workers.dev/restate
ran "an environment's engine: the top level's newer release isn't its" 0 "Raffle: on dp_raffle, not a deployment of this installation" \
  "EngineInfo: on dp_info, Rafi 0.0.9" "installation rafi-app-eu"
in_order "registered with the environment's app Worker as instance" \
  'http POST /restate/deployments {"uri":"https://rafi-engine-eu-v0-1-0.acme.workers.dev/restate","force":false,"metadata":{"app":"rafi","instance":"rafi-app-eu","version":"0.1.0","cloudflare_account":"acct123","worker":"rafi-engine-eu-v0-1-0"}}'
restate_state "$(ours 0.0.9)" "$(ours 0.2.0 rafi-app-eu)"
ds register-engine CLOUDFLARE_ENV=eu RAFI_ENGINE_WORKER=rafi-engine-eu-v0-1-0 RAFI_ENGINE_URL=https://rafi-engine-eu-v0-1-0.acme.workers.dev/restate
ran "an environment's own newer release: refused, naming its version file" 1 "EngineInfo runs on Rafi 0.2.0" "Set VERSION.eu to 0.2.0 or later"
restore
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

echo "== check-remote"
# The deploy's first job: Restate's release read and the token tried, nothing changed.
echo '{ "subdomain": "acme", "services": [], "deployments": [] }' | state
ds check-remote
ran "a first install (nothing registered): allowed" 0 "Raffle: not registered yet" "EngineInfo: not registered yet" \
  "the token reads it: workers.dev subdomain acme" "checked: Rafi 0.1.0 may be deployed here"
in_order "Restate's services and deployments, then the account" \
  "http GET /restate/services" "http GET /restate/deployments" "http GET /cf/accounts/acct123/workers/subdomain"
not_logged "nothing changed or deployed" "POST" "DELETE" "wrangler" "restate "
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
  not_logged "nothing past Restate after the refusal ($newer)" "/cf/" "wrangler" "POST"
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
if [ -s "$LOG" ]; then fail "something ran after Restate didn't answer: $(cat "$LOG")"; else pass "nothing else ran after Restate didn't answer"; fi
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
not_logged "nothing deployed with a bad token" "wrangler" "POST"
ds check-remote RESTATE_ADMIN_URL=
ran "no RESTATE_ADMIN_URL: refused" 1 "not set: RESTATE_ADMIN_URL"
not_logged "nothing asked without it" "http"
sed -i.orig "s/$DB_ID/00000000-0000-0000-0000-000000000000/" "$APP" "$ENGINE" "$LIVE"
ds check-remote
ran "the placeholders refused before any secret is used" 1 "database_id is the placeholder"
if [ -s "$LOG" ]; then fail "check-remote asked with placeholders: $(cat "$LOG")"; else pass "nothing asked with placeholders"; fi
restore

echo "== promote-app"
ds promote-app RAFI_APP_PROMOTED=true
ran "after a first install: nothing" 0 "nothing to promote"
not_logged "nothing deployed after a first install" "wrangler"
reported "its report" "deployed directly by upload-app (a first install): nothing to promote"
ds promote-app RAFI_APP_VERSION_ID=1b2c3d4e-0000-4000-8000-000000000001
ran "an uploaded version deployed" 0
in_order "the version at 100%, then the routes and Cron" \
  "wrangler versions deploy 1b2c3d4e-0000-4000-8000-000000000001@100% --yes -c $APP --message rafi v0.1.0" \
  "wrangler triggers deploy -c $APP"
reported "promote-app's report" "### App Worker \`rafi-app\`" "Version \`1b2c3d4e-0000-4000-8000-000000000001\` (Rafi v0.1.0) deployed at 100%"
ds promote-app
ran "neither output: refused" 1 "neither RAFI_APP_PROMOTED=true nor RAFI_APP_VERSION_ID"
ds promote-app RAFI_APP_VERSION_ID=1b2c3d4e-0000-4000-8000-000000000001 STUB_WRANGLER_FAIL="versions deploy"
ran "the version's deploy failing: failed" 1
not_logged "no triggers without the version" "triggers deploy"

echo "== cleanup"
meta() { printf '{ "app": "rafi", "instance": "%s", "cloudflare_account": "%s", "version": "0.0.1", "worker": "%s" }' "$1" "$2" "$3"; }
state <<EOF
{ "scripts": { "rafi-engine-v0-0-8": {}, "rafi-engine-v0-0-9": {}, "rafi-engine-v0-1-0": {}, "rafi-engine-v0-0-6": {},
               "rafi-engine-v0-0-5": {}, "rafi-engine-v0-0-4": {}, "rafi-engine-eu-v0-0-4": {}, "rafi-engine-old": {},
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
    { "id": "dp_elsewhere", "uri": "https://evil.acme.workers.dev/restate", "metadata": $(meta rafi-app acct123 rafi-engine-v0-0-6) }
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
  { "deployment_id": "dp_elsewhere", "endpoint": "https://evil.acme.workers.dev/restate", "status": "Drained", "active_invocations": 0, "services": [] }
] }
EOF
cp "$TMP/state.json" "$TMP/cleanup.json"
cp "$TMP/list.json" "$TMP/list.good"
# cleanup_state <JSON patch>: the cleanup's state with the patch's keys over it.
cleanup_state() {
  node -e '
    const fs = require("fs");
    const s = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    fs.writeFileSync(process.argv[2], JSON.stringify({ ...s, ...JSON.parse(process.argv[3]) }));' \
    "$TMP/cleanup.json" "$TMP/state.json" "$1"
}
ds cleanup
ran "cleanup" 0 "dp_drained (rafi-engine-v0-0-8) is drained" "deleted the Worker rafi-engine-v0-0-8" \
  "removed the deployment dp_drained" "the Worker rafi-engine-v0-0-7 is already gone" "removed the deployment dp_gone" \
  "dp_draining (rafi-engine-v0-0-9) is draining: 2 active, 1 paused invocation(s)" "inv_1 paused" \
  "dp_active (rafi-engine-v0-1-0) is active" \
  "dp_nometa at https://raffle.example.org/restate: not a Rafi deployment" "dp_otherinst at" "another installation (rafi-app-eu)" \
  "dp_otheracct at" "another Cloudflare account" "dp_appworker at" "its worker is not a rafi-engine-v... engine Worker (rafi-app)" \
  "dp_elsewhere at" "its address is not the workers.dev one of rafi-engine-v0-0-6" \
  "the Worker rafi-engine-v0-0-4: no Restate deployment refers to it" "2 drained engine deployment(s) removed"
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
  "remove dp_draining" "remove dp_active" "remove dp_nometa" \
  "remove dp_otherinst" "remove dp_otheracct" "remove dp_appworker" "remove dp_elsewhere" "--force"
if grep -qE '^http DELETE /cf/accounts/acct123/workers/scripts/rafi-(app|live)$' "$LOG"; then fail "the app or live Worker deleted"; else pass "the app and live Workers never deleted"; fi
reported "the cleanup's report" "### Restate cleanup: \`rafi-app\` (Rafi v0.1.0)" \
  "\`dp_drained\`: removed, its Worker \`rafi-engine-v0-0-8\` deleted" "\`dp_gone\`: removed, its Worker \`rafi-engine-v0-0-7\` already gone" \
  "**Still draining**" "\`dp_draining\` (\`rafi-engine-v0-0-9\`): 2 active, 1 paused invocation(s); paused: \`inv_1\`" \
  "restate invocations resume <id> --deployment latest" "restate invocations kill <id>" \
  "**Workers left behind**" "\`rafi-engine-v0-0-4\`: no Restate deployment refers to it" \
  "\`rafi-engine-v0-0-6\`: no Restate deployment refers to it" "delete <worker>" \
  "**Not this installation's** (never touched)" "\`dp_otherinst\`" "\`dp_nometa\`"
not_reported "leftovers: only this installation's engine Workers, not those referred to" \
  "\`rafi-engine-eu-v0-0-4\`" "\`rafi-engine-v0-0-5\`:" "\`rafi-engine-v0-1-0\`:" "\`rafi-engine-v0-0-9\`:" "rafi-engine-old" "\`rafi-app\`:" "\`rafi-live\`:"
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
  "the Worker rafi-engine-eu-v0-0-4: no Restate deployment refers to it"
in_order "the environment's drained deployment removed, then its Worker deleted" \
  "restate deployments remove dp_eu --yes" "http DELETE /cf/accounts/acct123/workers/scripts/rafi-engine-eu-v0-0-8"
not_logged "the top level's untouched" "remove dp_drained" "scripts/rafi-engine-v0-0-8"
reported "the environment's report" "### Restate cleanup: \`rafi-app-eu\`" "\`rafi-engine-eu-v0-0-4\`: no Restate deployment refers to it"
not_reported "the top level's Workers not the environment's leftovers" "\`rafi-engine-v0-0-4\`"
cp "$TMP/list.good" "$TMP/list.json"
restore

if [ "$FAILED" != 0 ]; then
  echo "cloudflare-deploy-test: $FAILED FAILED, $PASSED passed" >&2
  exit 1
fi
echo "cloudflare-deploy-test: all $PASSED passed"
