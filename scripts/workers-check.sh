#!/usr/bin/env bash
# Runs rafi's Cloudflare Workers checks locally, under `wrangler dev`:
#
#   1. rafi-store's conformance suite against D1 (testbeds/store-d1), failing
#      if any scenario fails;
#   2. the live channel driven over real WebSockets (testbeds/cloudflare/scenario.mjs);
#   3. the participant page in headless Chromium (crates/participant/smoke.mjs), when
#      CHROMIUM (or `chromium`) is available;
#   4. a draw through Restate into the live channel (crates/restate-worker/e2e.mjs),
#      when RESTATE_SERVER_BIN is set (./scripts/restate-server.sh prints it);
#   5. a whole evening on the app Worker itself (crates/app-worker/evening.mjs): the
#      admin API, a phone joining in Chromium, draws through Restate, the end.
#      Needs RESTATE_SERVER_BIN, Chromium and dx (for the web app bundle).
#
# Needs: wrangler (WRANGLER overrides the command), worker-build, node,
# wasm-bindgen-cli at the lockfile's version and wasm-opt (for the participant page).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WRANGLER="${WRANGLER:-wrangler}"
STATE="$(mktemp -d)"
LOGS="$STATE/logs"
mkdir -p "$LOGS"
PIDS=()

cleanup() {
  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  rm -rf "$STATE"
}
trap cleanup EXIT

step() { printf '\n== %s\n' "$*"; }

# Starts `wrangler dev` in a directory under ROOT on a port, sharing local state, and waits for it.
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

step "build"
"$ROOT/scripts/participant.sh" "$ROOT/testbeds/cloudflare/site/participant"
for dir in testbeds/store-d1 testbeds/cloudflare; do
  (cd "$ROOT/$dir" && worker-build --release) >"$LOGS/build-${dir##*/}.log" 2>&1
done
(cd "$ROOT/crates/restate-worker" && worker-build --release --features dev) >"$LOGS/build-restate-worker.log" 2>&1
migrate testbeds/store-d1 rafi-store-check
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
  CHROMIUM="$CHROMIUM" BASE=http://127.0.0.1:8790 node "$ROOT/crates/participant/smoke.mjs" "$STATE/chrome"
else
  step "the participant page: skipped (no Chromium)"
fi

if [ -n "${RESTATE_SERVER_BIN:-}" ]; then
  step "a draw through Restate"
  (cd "$STATE" && RESTATE_BASE_DIR="$STATE/restate" RESTATE_NODE_NAME=workers-check \
    RESTATE_LISTEN_MODE=tcp RESTATE_ADMIN__LISTEN_MODE=tcp RESTATE_INGRESS__LISTEN_MODE=tcp \
    RESTATE_BIND_ADDRESS=127.0.0.1:15122 RESTATE_ADVERTISED_ADDRESS=http://127.0.0.1:15122 \
    RESTATE_INGRESS__BIND_ADDRESS=127.0.0.1:18080 RESTATE_ADMIN__BIND_ADDRESS=127.0.0.1:19070 \
    exec "$RESTATE_SERVER_BIN") >"$LOGS/restate-server.log" 2>&1 &
  PIDS+=($!)
  serve crates/restate-worker 8791 /discover --env check
  for _ in $(seq 1 60); do curl -s -m 3 -o /dev/null http://127.0.0.1:19070/health && break; sleep 1; done
  curl -s -m 30 -X POST http://127.0.0.1:19070/deployments -H 'content-type: application/json' \
    -d '{"uri":"http://127.0.0.1:8791","use_http_11":true}' >/dev/null
  INGRESS=http://127.0.0.1:18080 APP=http://127.0.0.1:8790 node "$ROOT/crates/restate-worker/e2e.mjs"
else
  step "a draw through Restate: skipped (RESTATE_SERVER_BIN not set)"
fi

if [ -n "${RESTATE_SERVER_BIN:-}" ] && [ -n "$CHROMIUM" ] && command -v dx >/dev/null; then
  step "an evening on the app Worker"
  "$ROOT/scripts/app-assets.sh" >"$LOGS/app-assets.log" 2>&1 || { tail -20 "$LOGS/app-assets.log" >&2; exit 1; }
  (cd "$ROOT/crates/app-worker" && worker-build --release --features dev) >"$LOGS/build-app.log" 2>&1
  (cd "$ROOT/crates/app-worker" && $WRANGLER d1 migrations apply rafi --local --env dev --persist-to "$STATE/wrangler") >/dev/null
  serve crates/app-worker 8787 /healthz --env dev
  serve crates/restate-worker 8792 /discover --env dev
  curl -s -m 30 -X POST http://127.0.0.1:19070/deployments -H 'content-type: application/json' \
    -d '{"uri":"http://127.0.0.1:8792","use_http_11":true,"force":true}' >/dev/null
  CHROMIUM="$CHROMIUM" APP=http://127.0.0.1:8787 node "$ROOT/crates/app-worker/evening.mjs" "$STATE/chrome-app"
else
  step "an evening on the app Worker: skipped (needs RESTATE_SERVER_BIN, Chromium and dx)"
fi

step "all Workers checks passed"
