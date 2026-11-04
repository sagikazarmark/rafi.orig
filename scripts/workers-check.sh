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
#      Cron Trigger would fire it, every page under its Content-Security-Policy
#      with no violation. Needs RESTATE_SERVER_BIN (the devenv shell sets it),
#      Chromium and dx (for the web app bundle, whose index.html the Worker
#      compiles in);
#   5. the production shape (the top level of wrangler.toml, built without
#      the `dev` feature): `wrangler deploy --dry-run` validates the
#      configuration and the bundle against a size budget, and the Worker
#      served without any secret answers 500 "misconfigured", so a deploy
#      can't run open; served with stand-in secrets, without legal/ its
#      /terms and /privacy say the operator hasn't provided them, with this
#      run's own documents in legal/ (read through the assets binding) they
#      show their versions, and with a terms.md without its front matter
#      (version) it refuses every request naming the file.
#      Needs dx too (the dry run uploads the assets);
#   6. testdata/cloudflare-installation, what an installation deploys (the
#      deploy template's configuration, as a test fixture): its
#      wrangler.app.toml, wrangler.engine.toml and wrangler.live.toml agree
#      with the production Workers' (apps/worker-app, apps/worker-engine,
#      apps/worker-live) on the bindings, limits, Durable Object migrations
#      and required secrets, and a release (of the version its VERSION names)
#      is rehearsed the way a tag releases it (scripts/cloudflare-release.sh):
#      the fixture's VERSION, deploy.sh's own checks, worker.tar.gz packaged,
#      each Worker's exports checked, unpacked into a copy of the fixture's
#      release/, and its deploy.sh dry-run there with this wrangler (the
#      Workers' names read from its dry runs), at the top level and as a
#      Wrangler environment (CLOUDFLARE_ENV=eu, with the EU jurisdiction).
#
# Needs: wrangler (WRANGLER overrides the command), worker-build, node, jq, just,
# dx, wasm-bindgen-cli at the lockfile's version and wasm-opt (for the
# participant page), and GNU tar (the release rehearsal).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WRANGLER="${WRANGLER:-wrangler}"
STATE="$(mktemp -d)"
# The evening's Worker: 8787, as in development, unless taken (WORKERS_APP_PORT=8797 just workers).
APP_PORT="${WORKERS_APP_PORT:-8787}"
APP="http://127.0.0.1:$APP_PORT"
LOGS="$STATE/logs"
mkdir -p "$LOGS"
PIDS=()
# Set while the production shape is served with this run's own legal/ (below); removed on exit.
LEGAL_FIXTURE=

cleanup() {
  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  if [ -n "$LEGAL_FIXTURE" ]; then rm -rf "$ROOT/apps/worker/worker-assets/legal"; fi
  rm -rf "$STATE"
}
trap cleanup EXIT

step() { printf '\n== %s\n' "$*"; }

# Starts `wrangler dev` in a directory under ROOT on a port, with this run's local state, and waits for it.
serve() {
  local dir="$1" port="$2" probe="$3" name="${1##*/}"
  shift 3
  # Something else on the port (a developer's own `wrangler dev`, one left
  # over from an earlier run) would answer the checks in this one's place.
  if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    echo "port $port in use: stop whatever listens there (lsof -nP -iTCP:$port -sTCP:LISTEN)" >&2
    return 1
  fi
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
  # A lead of 8 s, not development's 3: the evening watches the countdowns run.
  # RESTATE_INGRESS_URL in wrangler's environment: with a [secrets] section,
  # wrangler dev takes a var's value from there over the configuration's, and
  # the devenv shell exports the Restate CLI's (its own Restate's, on :8080).
  RESTATE_INGRESS_URL=http://127.0.0.1:18080 \
    serve apps/worker "$APP_PORT" /healthz --env dev --test-scheduled --var "RAFI_PUBLIC_URL:$APP" \
    --var RAFI_DRAND_LEAD_SECS:8
  for _ in $(seq 1 60); do curl -s -m 3 -o /dev/null http://127.0.0.1:19070/health && break; sleep 1; done
  # The engine is the same Worker, under /restate; wrangler dev speaks HTTP/1.1 only.
  curl -s -m 30 -X POST http://127.0.0.1:19070/deployments -H 'content-type: application/json' \
    -d '{"uri":"'"$APP/restate"'","use_http_11":true}' >/dev/null
  CHROMIUM="$CHROMIUM" APP="$APP" INGRESS=http://127.0.0.1:18080 \
    node "$ROOT/apps/worker/evening.mjs" "$STATE/chrome-app"

  step "housekeeping, fired as the Cron Trigger fires it"
  # The erasure promise rests on this handler running; it records each
  # success in `meta`, which is what the admin's staleness warning reads. A
  # failed run fails the invocation (500 here, an exception in the Cron history).
  fired="$(curl -s -m 120 -o /dev/null -w '%{http_code}' "$APP/__scheduled?cron=0+*+*+*+*")"
  if [ "$fired" != "200" ]; then
    echo "the scheduled handler failed ($fired); log:" >&2
    tail -30 "$LOGS/worker.log" >&2
    exit 1
  fi
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
  stale="$(curl -s -m 10 -X POST "$APP/api/admin/whoami" -H 'content-type: application/json' -d '{}' \
    | node -e 'process.stdout.write(String(JSON.parse(require("fs").readFileSync(0, "utf8")).retention_stale))')"
  [ "$stale" = "false" ] || { echo "whoami.retention_stale is $stale after a run" >&2; exit 1; }
  echo "ok   the admin sees automatic erasure as running"
  # Every response through the router has a request ID: the Ray ID that
  # came with the request, or a fresh one where none came (wrangler dev).
  request_id="$(curl -s -m 10 -o /dev/null -D - -X POST "$APP/api/admin/whoami" \
    -H 'content-type: application/json' -d '{}' \
    | tr -d '\r' | awk -F': ' 'tolower($1) == "x-request-id" { print $2 }')"
  if ! [[ "$request_id" =~ ^[A-Za-z0-9-]{1,64}$ ]]; then
    echo "the Worker's response has no request ID (x-request-id: '$request_id')" >&2
    exit 1
  fi
  echo "ok   the Worker's responses carry a request ID ($request_id)"
  # And with something due: the evening's raffles dated so long ago that
  # their erasure date and the winner hold have both passed. The next run
  # erases everyone (the raffle never frozen expires, its entrants deleted),
  # purges the raffles and ends their live channels, whose sockets are then
  # refused.
  d1() {
    (cd "$ROOT/apps/worker" && $WRANGLER d1 execute rafi --local --env dev --persist-to "$STATE/wrangler" --json \
      --command "$1") | node -e '
        const out = JSON.parse(require("fs").readFileSync(0, "utf8"));
        process.stdout.write(JSON.stringify(out[0]?.results ?? []));'
  }
  d1 "UPDATE raffles SET event_ends_at = event_ends_at - (retention_days + 400) * 86400" >/dev/null
  fired="$(curl -s -m 120 -o /dev/null -w '%{http_code}' "$APP/__scheduled?cron=0+*+*+*+*")"
  [ "$fired" = "200" ] || { echo "the scheduled handler failed with raffles due ($fired)" >&2; tail -30 "$LOGS/worker.log" >&2; exit 1; }
  left="$(d1 "SELECT
      (SELECT COUNT(*) FROM entrants WHERE erased = 0) AS unerased,
      (SELECT COUNT(*) FROM raffles WHERE state <> 'purged') AS unpurged,
      (SELECT COUNT(*) FROM raffles WHERE live_ended_at IS NULL) AS live,
      (SELECT COUNT(*) FROM raffles) AS raffles")"
  if ! echo "$left" | node -e '
      const [r] = JSON.parse(require("fs").readFileSync(0, "utf8"));
      process.exit(r.raffles >= 3 && r.unerased === 0 && r.unpurged === 0 && r.live === 0 ? 0 : 1);'; then
    echo "housekeeping with raffles due left: $left" >&2
    tail -30 "$LOGS/worker.log" >&2
    exit 1
  fi
  echo "ok   housekeeping with raffles due erased everyone, purged every raffle and ended their channels ($left)"
  raffle="$(d1 "SELECT id FROM raffles ORDER BY created_at LIMIT 1" | node -e 'process.stdout.write(JSON.parse(require("fs").readFileSync(0, "utf8"))[0].id)')"
  code="$(APP="$APP" RAFFLE="$raffle" node -e '
    const ws = new WebSocket(`${process.env.APP.replace(/^http/, "ws")}/partyline/raffle/${process.env.RAFFLE}?v=1`);
    ws.onclose = (e) => { process.stdout.write(String(e.code)); process.exit(0); };
    setTimeout(() => { process.stdout.write("none"); process.exit(0); }, 5000);')"
  [ "$code" = "4403" ] || { echo "a socket to a purged raffle closed with $code, not 4403" >&2; exit 1; }
  echo "ok   a socket to a purged raffle is refused (4403)"
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
  # included, instead of running open with defaults. (Empty: wrangler would
  # hand it the devenv shell's RESTATE_INGRESS_URL as the secret.)
  RESTATE_INGRESS_URL= serve apps/worker 8791 /healthz
  closed="$(curl -s -m 10 -w '\n%{http_code}' http://127.0.0.1:8791/healthz)"
  if [ "$(echo "$closed" | tail -1)" != "500" ] || ! echo "$closed" | head -1 | grep -q misconfigured; then
    echo "the production Worker without secrets answered: $closed" >&2
    tail -20 "$LOGS/worker.log" >&2
    exit 1
  fi
  echo "ok   without secrets the production Worker refuses every request (500 misconfigured)"
  if ! logged "$LOGS/worker.log" "rafi: "; then
    echo "the production Worker logged no reason for refusing; log:" >&2
    tail -20 "$LOGS/worker.log" >&2
    exit 1
  fi
  # The operator's legal documents, each optional, read through the assets
  # binding (Workers Assets answers a missing file with a 404, there being no
  # single-page fallback among the files, which only the real thing shows).
  # Served with stand-in secrets, so the app builds: `just assets` leaves no
  # legal/, so its pages say the operator hasn't provided them; this run's
  # own documents in legal/ are rendered with their versions; and a terms.md
  # without its front matter refuses every request, naming the file. legal/
  # is removed after each (by cleanup too, should this run die in between).
  secrets=(--var RAFI_POW_KEY:workers-check-workers-check-workers-check
    --var RESTATE_INGRESS_API_KEY:workers-check
    --var CLERK_SECRET_KEY:sk_test_workerscheck
    --var CLERK_PUBLISHABLE_KEY:pk_test_cmFmaS10ZXN0LmNsZXJrLmFjY291bnRzLmRldiQ
    --var "RAFI_OPERATOR_NAME:Workers Check" --var RAFI_OPERATOR_EMAIL:privacy@workers-check.invalid)
  legal="$ROOT/apps/worker/worker-assets/legal"
  # documents <port>: the Worker served on <port> with the secrets, the one before stopped.
  documents() {
    kill "${PIDS[${#PIDS[@]}-1]}" 2>/dev/null || true
    # RESTATE_INGRESS_URL from wrangler's environment (above, the evening): nothing listens there.
    RESTATE_INGRESS_URL=http://127.0.0.1:9 serve apps/worker "$1" /healthz --var "RAFI_PUBLIC_URL:http://127.0.0.1:$1" "${secrets[@]}"
  }
  # pages <port>: /healthz's, /terms's and /privacy's body and status, a line each.
  pages() {
    for path in /healthz /terms /privacy; do
      curl -s -m 10 -w ' %{http_code}' "http://127.0.0.1:$1$path" | tr '\n' ' '
      echo
    done
  }
  [ ! -e "$legal" ] || { echo "just assets left a legal/ among the Worker's static files" >&2; exit 1; }
  documents 8792
  pages="$(pages 8792)"
  if ! echo "$pages" | sed -n 1p | grep -q '^ok .* 200$' \
    || ! echo "$pages" | sed -n 2p | grep -q "hasn&#39;t provided terms\..* 200$" \
    || ! echo "$pages" | sed -n 3p | grep -q "hasn&#39;t provided a privacy notice\..* 200$"; then
    echo "the production Worker without legal/ answered:" >&2
    echo "$pages" | cut -c1-300 >&2
    tail -20 "$LOGS/worker.log" >&2
    exit 1
  fi
  echo "ok   without legal documents the production Worker serves, and /terms and /privacy say the operator hasn't provided them"
  LEGAL_FIXTURE=1
  mkdir "$legal"
  printf -- '---\nversion: check-1\n---\n# Check terms\n' >"$legal/terms.md"
  printf -- '---\nversion: check-2\n---\n# Check privacy notice\n' >"$legal/privacy.md"
  documents 8793
  pages="$(pages 8793)"
  if ! echo "$pages" | sed -n 2p | grep -q "Terms · version check-1.*<h1>Check terms</h1>.* 200$" \
    || ! echo "$pages" | sed -n 3p | grep -q "Privacy notice · version check-2.*<h1>Check privacy notice</h1>.* 200$"; then
    echo "the production Worker with legal/ answered:" >&2
    echo "$pages" | cut -c1-300 >&2
    tail -20 "$LOGS/worker.log" >&2
    exit 1
  fi
  echo "ok   the Worker read legal/terms.md and legal/privacy.md through the assets binding, each with its version"
  # A document without its front matter (the version): refused, naming the
  # file, in the answer as well (it names no value).
  printf -- '# Check terms\n' >"$legal/terms.md"
  documents 8794
  pages="$(pages 8794)"
  rm -rf "$legal"
  LEGAL_FIXTURE=
  if ! echo "$pages" | sed -n 1p | grep -q "^misconfigured: legal/terms.md: no front matter.* 500$"; then
    echo "the production Worker with a terms.md without front matter answered:" >&2
    echo "$pages" | cut -c1-300 >&2
    tail -20 "$LOGS/worker.log" >&2
    exit 1
  fi
  echo "ok   with a document without its version the production Worker refuses every request, naming the file"
  kill "${PIDS[${#PIDS[@]}-1]}" 2>/dev/null || true

  step "testdata/cloudflare-installation: the installation's shape, and its release rehearsed"
  # The fixture's three files (the deploy template's) share the production Workers' shapes: their
  # bindings, limits, Cron, addresses and required secrets. A route or a
  # secret added to one and not the other would deploy a Worker that 404s or
  # refuses; a limit or a class name changed in one alone would deploy a
  # different Worker. The app's address is the default in both
  # (workers_dev = true, no routes), which an installation on a custom
  # domain changes in its own copy. The paths differ (the fixture's point
  # into a release), and the identity keys are an
  # installation's, so only their name is compared.
  shape() {
    sed -n \
      -e '/^compatibility_date/p' -e '/^workers_dev/p' -e '/^preview_urls/p' -e '/^routes/p' -e '/^binding = /p' \
      -e '/^database_name/p' -e '/^name = "/p' -e '/^class_name/p' -e '/^script_name/p' \
      -e '/^new_sqlite_classes/p' -e '/^tag = /p' -e '/^namespace_id/p' -e '/^simple = /p' -e '/^crons/p' \
      -e '/^invocation_logs/p' -e 's/^\(RESTATE_IDENTITY_KEYS\) = .*/\1/p' -e '/^RAFI_[A-Z_]* = /p' \
      -e '/^required = \[$/,/^\]/p' "$1"
  }
  for pair in worker-app:wrangler.app.toml worker-engine:wrangler.engine.toml worker-live:wrangler.live.toml; do
    worker="apps/${pair%%:*}/wrangler.toml"
    fixture="testdata/cloudflare-installation/${pair#*:}"
    if ! diff <(shape "$ROOT/$worker") <(shape "$ROOT/$fixture"); then
      echo "$fixture disagrees with $worker (above)" >&2
      exit 1
    fi
  done
  echo "ok   the installation's shapes are the production Workers'"
  # A release, as a tag releases it: the same script, so a broken tarball,
  # deploy.sh or dry deploy shows here, not on the tag. The version is the one
  # the fixture names (testdata/cloudflare-installation/VERSION), as the
  # release workflow's manual run defaults to.
  version="$(tr -d '[:space:]' <"$ROOT/testdata/cloudflare-installation/VERSION")"
  WRANGLER="$WRANGLER" "$ROOT/scripts/cloudflare-release.sh" "$version" "$STATE/release" >"$LOGS/release.log" 2>&1 \
    || { tail -30 "$LOGS/release.log" >&2; exit 1; }
  echo "ok   the installation deploys the release of $version with its deploy.sh (dry run)"
else
  step "the production shape: skipped (needs dx for the assets)"
fi

step "all Workers checks passed"
