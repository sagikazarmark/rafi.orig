#!/usr/bin/env bash
# Runs rafi's Cloudflare Workers checks locally, under `wrangler dev`:
#
#   1. rafi-store's conformance suite against D1 (testbeds/store-d1), failing
#      if any scenario fails;
#   2. the live channel driven over real WebSockets (testbeds/cloudflare/scenario.mjs);
#   3. the participant page in headless Chromium (apps/participant/smoke.mjs), when
#      CHROMIUM (or `chromium`) is available;
#   4. a whole evening on the Worker itself (apps/worker/evening.mjs): the admin
#      API, a phone joining in Chromium, draws through Restate (a local
#      restate-server calling the engine in the same Worker, under /restate),
#      the live channel, the end, and the hourly housekeeping fired as the
#      Cron Trigger would fire it. Needs RESTATE_SERVER_BIN (the devenv shell
#      sets it), Chromium and dx (for the web app bundle);
#   5. the production shape (the top level of wrangler.toml, built without
#      the `dev` feature): `wrangler deploy --dry-run` validates the
#      configuration and the bundle against a size budget, and the Worker
#      served without any secret answers 500 "misconfigured", so a deploy
#      can't run open; it has read the legal documents through the assets
#      binding by then, and served without legal/ it refuses naming them
#      (seeing through Assets' single-page fallback). Needs dx too (the dry
#      run uploads the assets);
#   6. deploy/cloudflare, what an installation deploys: its wrangler.toml
#      agrees with the Worker's on the paths and the required secrets, and
#      the release of the version it pins is rehearsed the way a tag releases
#      it (scripts/cloudflare-release.sh): packaged, fetched by a copy of the
#      template with its own deploy.sh, and deployed (dry).
#
# Needs: wrangler (WRANGLER overrides the command), worker-build, node, just,
# dx, wasm-bindgen-cli at the lockfile's version and wasm-opt (for the
# participant page), python3 and curl (the release rehearsal).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WRANGLER="${WRANGLER:-wrangler}"
STATE="$(mktemp -d)"
LOGS="$STATE/logs"
mkdir -p "$LOGS"
PIDS=()
# Where the Worker's legal/ is while the production shape is served without it (below); restored on exit.
LEGAL_ASIDE=

cleanup() {
  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  [ -z "$LEGAL_ASIDE" ] || [ ! -d "$LEGAL_ASIDE" ] || mv "$LEGAL_ASIDE" "$ROOT/apps/worker/worker-assets/legal"
  rm -rf "$STATE"
}
trap cleanup EXIT

step() { printf '\n== %s\n' "$*"; }

# Starts `wrangler dev` in a directory under ROOT on a port, with this run's local state, and waits for it.
serve() {
  local dir="$1" port="$2" probe="$3" name="${1##*/}"
  shift 3
  (cd "$ROOT/$dir" && exec $WRANGLER dev "$@" --port "$port" --ip 127.0.0.1 --local \
    --persist-to "$STATE/wrangler" --inspector-port "$((port + 1000))") >"$LOGS/$name.log" 2>&1 &
  PIDS+=($!)
  for _ in $(seq 1 120); do
    if curl -s -m 3 -o /dev/null "http://127.0.0.1:$port$probe"; then return 0; fi
    sleep 1
  done
  echo "$dir didn't start; log:" >&2
  tail -30 "$LOGS/$name.log" >&2
  return 1
}

migrate() {
  (cd "$ROOT/$1" && $WRANGLER d1 migrations apply "$2" --local --persist-to "$STATE/wrangler") >/dev/null
}

# Waits for a line matching $2 in the log $1: wrangler relays the Worker's console a moment after the response.
logged() {
  for _ in $(seq 1 20); do
    if grep -q -- "$2" "$1"; then return 0; fi
    sleep 1
  done
  return 1
}

step "build"
# The participant build first: the testbed and the Worker compile its loader in.
just "$ROOT/apps/participant/" check
site="$ROOT/testbeds/cloudflare/site/participant"
rm -rf "${site:?}" && mkdir -p "$site"
cp -R "$ROOT/apps/participant/dist/public/assets" "$site/assets"
cp "$ROOT/apps/participant/dist/public/fallback.js" "$site/"
for dir in testbeds/store-d1 testbeds/cloudflare; do
  (cd "$ROOT/$dir" && worker-build --release) >"$LOGS/build-${dir##*/}.log" 2>&1
done
migrate testbeds/store-d1 rafi-store-d1-check
migrate testbeds/cloudflare rafi-cloudflare-check

step "rafi-store's conformance suite on D1"
serve testbeds/store-d1 8789 /
report="$(curl -s -m 60 -w '\n%{http_code}' http://127.0.0.1:8789/)"
echo "$report" | head -1
[ "$(echo "$report" | tail -1)" = "200" ] || { echo "store conformance scenarios failed" >&2; exit 1; }

step "the live channel"
serve testbeds/cloudflare 8790 '/participant?r=probe'
node "$ROOT/testbeds/cloudflare/scenario.mjs" http://127.0.0.1:8790

CHROMIUM="${CHROMIUM:-$(command -v chromium || command -v google-chrome || true)}"
if [ -n "$CHROMIUM" ]; then
  step "the participant page in Chromium"
  CHROMIUM="$CHROMIUM" BASE=http://127.0.0.1:8790 node "$ROOT/apps/participant/smoke.mjs" "$STATE/chrome"
else
  step "the participant page: skipped (no Chromium)"
fi

if [ -n "${RESTATE_SERVER_BIN:-}" ] && [ -n "$CHROMIUM" ] && command -v dx >/dev/null; then
  step "an evening on the Worker"
  just "$ROOT/apps/worker/" assets >"$LOGS/app-assets.log" 2>&1 || { tail -20 "$LOGS/app-assets.log" >&2; exit 1; }
  assets_built=1
  (cd "$ROOT/apps/worker" && worker-build --release --features dev) >"$LOGS/build-app.log" 2>&1
  (cd "$ROOT/apps/worker" && $WRANGLER d1 migrations apply rafi --local --env dev --persist-to "$STATE/wrangler") >/dev/null
  (cd "$STATE" && RESTATE_BASE_DIR="$STATE/restate" RESTATE_NODE_NAME=workers-check \
    RESTATE_LISTEN_MODE=tcp RESTATE_ADMIN__LISTEN_MODE=tcp RESTATE_INGRESS__LISTEN_MODE=tcp \
    RESTATE_BIND_ADDRESS=127.0.0.1:15122 RESTATE_ADVERTISED_ADDRESS=http://127.0.0.1:15122 \
    RESTATE_INGRESS__BIND_ADDRESS=127.0.0.1:18080 RESTATE_ADMIN__BIND_ADDRESS=127.0.0.1:19070 \
    exec "$RESTATE_SERVER_BIN") >"$LOGS/restate-server.log" 2>&1 &
  PIDS+=($!)
  # --test-scheduled: the Cron Trigger can be fired by hand (/__scheduled).
  serve apps/worker 8787 /healthz --env dev --test-scheduled
  for _ in $(seq 1 60); do curl -s -m 3 -o /dev/null http://127.0.0.1:19070/health && break; sleep 1; done
  # The engine is the same Worker, under /restate; wrangler dev speaks HTTP/1.1 only.
  curl -s -m 30 -X POST http://127.0.0.1:19070/deployments -H 'content-type: application/json' \
    -d '{"uri":"http://127.0.0.1:8787/restate","use_http_11":true}' >/dev/null
  CHROMIUM="$CHROMIUM" APP=http://127.0.0.1:8787 INGRESS=http://127.0.0.1:18080 \
    node "$ROOT/apps/worker/evening.mjs" "$STATE/chrome-app"

  step "housekeeping, fired as the Cron Trigger fires it"
  # The erasure promise rests on this handler running; it records each
  # success in `meta`, which is what the admin's staleness warning reads.
  curl -s -m 120 -o /dev/null "http://127.0.0.1:8787/__scheduled?cron=0+*+*+*+*"
  last_ok="$(cd "$ROOT/apps/worker" && $WRANGLER d1 execute rafi --local --env dev --persist-to "$STATE/wrangler" --json \
    --command "SELECT value FROM meta WHERE key = 'last_retention_ok_at'" | node -e '
      const out = JSON.parse(require("fs").readFileSync(0, "utf8"));
      const rows = out[0]?.results ?? [];
      process.stdout.write(rows[0]?.value ?? "");')"
  if [ -z "$last_ok" ]; then
    echo "the scheduled handler didn't record a successful housekeeping run; log:" >&2
    tail -30 "$LOGS/worker.log" >&2
    exit 1
  fi
  echo "ok   housekeeping ran and recorded its success (last_retention_ok_at=$last_ok)"
  stale="$(curl -s -m 10 -X POST http://127.0.0.1:8787/api/admin/whoami -H 'content-type: application/json' -d '{}' \
    | node -e 'process.stdout.write(String(JSON.parse(require("fs").readFileSync(0, "utf8")).retention_stale))')"
  [ "$stale" = "false" ] || { echo "whoami.retention_stale is $stale after a run" >&2; exit 1; }
  echo "ok   the admin sees automatic erasure as running"
  # The dev Worker is done: the production shape takes the port's build dir.
  kill "${PIDS[${#PIDS[@]}-1]}" 2>/dev/null || true
else
  step "an evening on the Worker: skipped (needs RESTATE_SERVER_BIN, Chromium and dx)"
fi

if command -v dx >/dev/null; then
  step "the production shape: a dry deploy within budget, and fail-closed without secrets"
  # The static files of this run's participant build, not an earlier one's.
  [ -n "${assets_built:-}" ] || just "$ROOT/apps/worker/" assets >"$LOGS/app-assets.log" 2>&1 || { tail -20 "$LOGS/app-assets.log" >&2; exit 1; }
  # The top level of wrangler.toml, built as `wrangler deploy` builds it (no
  # `dev` feature). The dry run validates every binding and uploads nothing.
  dry="$(cd "$ROOT/apps/worker" && $WRANGLER deploy --dry-run --outdir "$STATE/dry" 2>&1)" \
    || { echo "$dry" | tail -30 >&2; exit 1; }
  gzip_kib="$(echo "$dry" | sed -n 's/.*gzip: \([0-9.]*\) KiB.*/\1/p' | head -1 | cut -d. -f1)"
  [ -n "$gzip_kib" ] || { echo "no upload size in wrangler's output:" >&2; echo "$dry" | tail -20 >&2; exit 1; }
  # Workers' limit is 3 MiB compressed on the free plan, 10 MiB paid; the
  # budget keeps a deploy from sliding towards either unnoticed.
  budget=2560
  if [ "$gzip_kib" -gt "$budget" ]; then
    echo "the Worker is $gzip_kib KiB gzipped, over the $budget KiB budget" >&2
    exit 1
  fi
  echo "ok   wrangler deploy --dry-run: $gzip_kib KiB gzipped (budget $budget KiB)"
  # Served with no secret at all: every request is refused, the health check
  # included, instead of running open with defaults.
  serve apps/worker 8791 /healthz
  closed="$(curl -s -m 10 -w '\n%{http_code}' http://127.0.0.1:8791/healthz)"
  if [ "$(echo "$closed" | tail -1)" != "500" ] || ! echo "$closed" | head -1 | grep -q misconfigured; then
    echo "the production Worker without secrets answered: $closed" >&2
    tail -20 "$LOGS/worker.log" >&2
    exit 1
  fi
  echo "ok   without secrets the production Worker refuses every request (500 misconfigured)"
  # The legal documents are read through the assets binding before anything
  # else is checked, so that refusal was for a secret, not for them: a
  # Markdown file served by Workers Assets was taken for a document, not for
  # the single-page fallback.
  missing_docs="missing from the Worker's static files"
  if ! logged "$LOGS/worker.log" "rafi: "; then
    echo "the production Worker logged no reason for refusing; log:" >&2
    tail -20 "$LOGS/worker.log" >&2
    exit 1
  fi
  if grep -q "$missing_docs" "$LOGS/worker.log"; then
    echo "the production Worker didn't find legal/ among its static files:" >&2
    grep "$missing_docs" "$LOGS/worker.log" | head -1 >&2
    exit 1
  fi
  echo "ok   the Worker read legal/terms.md and legal/privacy.md through the assets binding"
  # And without them, the refusal is theirs, with the message that says what
  # to do. Workers Assets answers a missing path with the web app's index.html
  # and a 200 (not_found_handling = "single-page-application"), which the
  # Worker has to see through; this is the one place that is checked against
  # the real thing. legal/ is moved aside while this Worker starts and answers,
  # and put back right after (by cleanup too, should this run die in between).
  kill "${PIDS[${#PIDS[@]}-1]}" 2>/dev/null || true
  LEGAL_ASIDE="$STATE/legal-aside"
  mv "$ROOT/apps/worker/worker-assets/legal" "$LEGAL_ASIDE"
  serve apps/worker 8792 /healthz
  closed="$(curl -s -m 10 -w '\n%{http_code}' http://127.0.0.1:8792/healthz)"
  named=
  if logged "$LOGS/worker.log" "$missing_docs"; then named=1; fi
  mv "$LEGAL_ASIDE" "$ROOT/apps/worker/worker-assets/legal"
  LEGAL_ASIDE=
  if [ "$(echo "$closed" | tail -1)" != "500" ] || [ -z "$named" ]; then
    echo "the production Worker without legal/ answered: $closed" >&2
    tail -20 "$LOGS/worker.log" >&2
    exit 1
  fi
  echo "ok   without legal/ the production Worker refuses every request, naming the documents"

  step "deploy/cloudflare: the installation's shape, and its release rehearsed"
  # The two wrangler.toml files share the Worker's shape: its paths, bindings,
  # limits, Cron and required secrets. A route or a secret added to one and
  # not the other would deploy a Worker that 404s or refuses; a limit or a
  # class name changed in one alone would deploy a different Worker. The
  # Worker's file is compared up to [env.dev], which repeats the bindings.
  shape() {
    sed '/^\[env\.dev\]/q' "$1" | sed -n \
      -e '/^compatibility_date/p' -e '/^run_worker_first/p' -e '/^not_found_handling/p' -e '/^binding = /p' \
      -e '/^database_name/p' -e '/^name = "/p' -e '/^class_name/p' -e '/^new_sqlite_classes/p' -e '/^tag = /p' \
      -e '/^namespace_id/p' -e '/^simple = /p' -e '/^crons/p' -e '/^invocation_logs/p' -e '/^RAFI_/p' \
      -e '/^required = \[$/,/^\]/p'
  }
  if ! diff <(shape "$ROOT/apps/worker/wrangler.toml") <(shape "$ROOT/deploy/cloudflare/wrangler.toml"); then
    echo "deploy/cloudflare/wrangler.toml disagrees with apps/worker/wrangler.toml (above)" >&2
    exit 1
  fi
  echo "ok   the template's shape is the Worker's"
  # The release of the version the template pins, as a tag releases it: the
  # same script, so a broken fetch, checksum or tarball shows here, not on
  # the tag.
  pinned="$(sed -n 's/^RAFI_VERSION=//p' "$ROOT/deploy/cloudflare/deploy.sh")"
  WRANGLER="$WRANGLER" "$ROOT/scripts/cloudflare-release.sh" "$pinned" "$STATE/release" >"$LOGS/release.log" 2>&1 \
    || { tail -30 "$LOGS/release.log" >&2; exit 1; }
  echo "ok   deploy/cloudflare fetches and deploys the release of $pinned (dry run)"
else
  step "the production shape: skipped (needs dx for the assets)"
fi

step "all Workers checks passed"
