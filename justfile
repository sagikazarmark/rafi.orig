set lazy
set default-list
set ignore-comments

wasm := "wasm32-unknown-unknown"

# The time of reproducible builds' files: SOURCE_DATE_EPOCH, or the commit's.
source_date_epoch := env("SOURCE_DATE_EPOCH", `git log -1 --format=%ct 2>/dev/null || echo 0`)

# Run all CI jobs but `workers` (long, and it needs a Chromium) and `cloudflare-tarball-check` (long).
ci: fmt clippy test verify doc audit deny cloudflare-deploy-test

# Check formatting.
fmt:
    cargo fmt --all --check

# Audit dependencies for security advisories.
audit:
    cargo audit

# Check dependency licenses, bans and sources (advisories are `audit`'s job).
deny:
    cargo deny --locked check bans licenses sources

# Build the web app's stylesheet (apps/web); rafi-web's asset!() needs it.
css:
    {{ just_executable() }} apps/web/ css

# Build the participant page (apps/participant); rafi-server compiles its loader in.
participant:
    {{ just_executable() }} apps/participant/ build

# Bundle the web app (apps/web); rafi-cloudflare-app compiles its index.html in.
web:
    {{ just_executable() }} apps/web/ build

# Run clippy, natively and on wasm32.
clippy: clippy-native clippy-wasm

# Run clippy natively, with and without the development features.
clippy-native: css participant
    cargo clippy --workspace --exclude rafi-server --exclude rafi --exclude rafi-web --all-targets --locked -- --deny warnings
    cargo clippy -p rafi-server --all-targets --locked -- --deny warnings
    cargo clippy -p rafi-server --features mock-drand --all-targets --locked -- --deny warnings
    cargo clippy -p rafi --all-targets --locked -- --deny warnings
    cargo clippy -p rafi --features dev,mock-drand --all-targets --locked -- --deny warnings
    cargo clippy -p rafi-restate --features mock-drand --all-targets --locked -- --deny warnings
    cargo clippy -p rafi-web --all-targets --locked -- --deny warnings

# Run clippy on wasm32: the web app, the Workers crates and the Workers (the local combined one, production's app, engine and live).
clippy-wasm: css participant web
    cargo clippy --target {{ wasm }} -p rafi-web --locked -- --deny warnings
    cargo clippy --target {{ wasm }} \
        -p rafi-lifecycle -p rafi-raffle -p rafi-api -p rafi-app \
        -p rafi-store -p rafi-store-d1-check -p rafi-live-client -p rafi-live-server -p rafi-cloudflare -p rafi-cloudflare-check \
        -p rafi-engine -p rafi-engine-restate -p rafi-participant --locked -- --deny warnings
    cargo clippy --target {{ wasm }} -p rafi-server --lib --locked -- --deny warnings
    cargo clippy --target {{ wasm }} -p rafi-worker --locked -- --deny warnings
    cargo clippy --target {{ wasm }} -p rafi-worker --features dev --locked -- --deny warnings
    # Production's three Workers, each on its own: checked with the others, the features they turn on would hide what one lacks.
    cargo clippy --target {{ wasm }} -p rafi-worker-app --locked -- --deny warnings
    cargo clippy --target {{ wasm }} -p rafi-worker-engine --locked -- --deny warnings
    cargo clippy --target {{ wasm }} -p rafi-worker-live --locked -- --deny warnings
    cargo clippy --target {{ wasm }} -p rafi-cloudflare-engine --features dev --locked -- --deny warnings
    cargo clippy --target {{ wasm }} -p rafi-cloudflare --features connect --locked -- --deny warnings
    cargo clippy --target {{ wasm }} -p rafi-cloudflare --features object --locked -- --deny warnings
    # What each production Worker links. The engine's: nothing of the app's, and no RaffleRoom (partyline-worker is rafi-cloudflare's `connect` and `object`).
    @if cargo tree --target {{ wasm }} -p rafi-worker-engine -e normal --prefix none --locked | grep -E '^(rafi-server|rafi-app|rafi-api|rafi-cloudflare-app|dioxus[a-z-]*|axum|partyline-worker) v'; then echo "rafi-worker-engine depends on the app's crates (above)" >&2; exit 1; fi
    # The live Worker's: the class and what it reads with (rafi-store, rafi-live-server, partyline-worker), nothing of the app's or the engine's.
    @if cargo tree --target {{ wasm }} -p rafi-worker-live -e normal --prefix none --locked | grep -E '^(rafi-server|rafi-app|rafi-api|rafi-cloudflare-app|rafi-cloudflare-engine|rafi-engine-restate|restate-sdk[a-z-]*|dioxus[a-z-]*|[a-z-]*clerk[a-z-]*|axum|tower-worker-fetch|tracing-subscriber) v'; then echo "rafi-worker-live depends on the app's or the engine's crates (above)" >&2; exit 1; fi
    @for crate in rafi-store rafi-live-server partyline-worker; do cargo tree --target {{ wasm }} -p rafi-worker-live -e normal --prefix none --locked | grep -q "^$crate v" || { echo "rafi-worker-live doesn't link $crate: is RaffleRoom still in it?" >&2; exit 1; }; done
    # The RaffleRoom class (rafi-cloudflare's `object`) is the live Worker's alone: the app's and the engine's Workers would export it too.
    @for crate in rafi-worker-app rafi-worker-engine; do if cargo tree --target {{ wasm }} -p $crate -e features -i rafi-cloudflare --prefix none --locked | grep -F 'rafi-cloudflare feature "object"'; then echo "$crate turns on rafi-cloudflare's object feature: it would export RaffleRoom" >&2; exit 1; fi; done
    @cargo tree --target {{ wasm }} -p rafi-worker-live -e features -i rafi-cloudflare --prefix none --locked | grep -qF 'rafi-cloudflare feature "object"' || { echo "rafi-worker-live doesn't turn on rafi-cloudflare's object feature" >&2; exit 1; }

# Run tests: the libraries and the engine (end to end against restate-server), then the app, then the web app's router.
test: css participant
    cargo test --workspace --exclude rafi-server --exclude rafi-web --locked -- --include-ignored
    cargo test -p rafi-server --locked -- --include-ignored
    cargo test -p rafi-web --locked
    # The end-to-end tests pass without a server to run against: say so, or a green run proves less than it looks (CI sets the variable, and fails them without it).
    @[ -n "${RESTATE_SERVER_BIN:-}" ] || echo "SKIPPED: $(grep -rho 'ignore = "needs RESTATE_SERVER_BIN"' crates | wc -l | tr -d ' ') end-to-end tests ran nothing: RESTATE_SERVER_BIN is unset (the devenv shell sets it)"

# Run the independent verifier on the shared test vectors.
verify:
    python3 tools/verify.py --vectors testdata/draw-vectors.json

# Run rustdoc: natively with every feature, and the wasm32-only crates (empty elsewhere) on wasm32.
[env("RUSTDOCFLAGS", "--deny warnings")]
doc: participant web
    cargo doc --workspace --exclude rafi-web --exclude rafi-cloudflare --exclude rafi-cloudflare-app --exclude rafi-cloudflare-engine --exclude rafi-participant --exclude rafi-worker --exclude rafi-worker-app --exclude rafi-worker-engine --exclude rafi-worker-live --all-features --no-deps --locked
    cargo doc --target {{ wasm }} -p rafi-cloudflare -p rafi-cloudflare-app -p rafi-cloudflare-engine -p rafi-participant -p rafi-worker -p rafi-worker-app -p rafi-worker-engine -p rafi-worker-live --no-deps --locked

# Run the Workers checks under `wrangler dev` (scripts/workers-check.sh).
workers:
    ./scripts/workers-check.sh

# Nothing of an installation's is in the artifact: its configuration (the deploy
# template's wrangler.app.toml, wrangler.engine.toml and wrangler.live.toml, which
# point into this layout as release/) and its legal documents, which deploy.sh
# copies into app/assets/legal/, are its own. The Workers' recipes run in their
# own directories: they get `out` as an absolute path, and RAFI_VERSION, which
# crates/server/build.rs compiles in, as `version`.

# Build what a Cloudflare release ships, into one directory: VERSION (`version`: RAFI_VERSION, or git describe), app/ (apps/worker-app: build/ and assets/, without legal/), engine/build/ (apps/worker-engine), live/build/ (apps/worker-live), migrations/ (the D1 migrations, migrations/sqlite's files) and deploy.sh (scripts/cloudflare-deploy.sh).
cloudflare-artifact version=env("RAFI_VERSION", `git describe --tags --always --dirty 2>/dev/null || echo unknown`) out="build/cloudflare/worker":
    rm -rf "{{ out }}"
    mkdir -p "{{ out }}/migrations"
    RAFI_VERSION="{{ version }}" {{ just_executable() }} apps/worker-app/ artifact "{{ absolute_path(out / "app") }}"
    RAFI_VERSION="{{ version }}" {{ just_executable() }} apps/worker-engine/ artifact "{{ absolute_path(out / "engine") }}"
    RAFI_VERSION="{{ version }}" {{ just_executable() }} apps/worker-live/ artifact "{{ absolute_path(out / "live") }}"
    cp migrations/sqlite/*.sql "{{ out }}/migrations/"
    cp scripts/cloudflare-deploy.sh "{{ out }}/deploy.sh"
    chmod 755 "{{ out }}/deploy.sh"
    echo "{{ version }}" >"{{ out }}/VERSION"

# Package a Cloudflare release of `version`.
cloudflare-tarball version out="build/cloudflare": (cloudflare-artifact version out / "worker")
    rm -f "{{ absolute_path(out / "worker.tar.gz") }}"

    # The same bytes from the same files (sorted, every time source_date_epoch,
    # no owner, modes 644 and 755, gzip without a name or a time): two builds
    # of a tag can be compared. GNU tar (the devenv shell's). No leading
    # directory: an installation unpacks it into its release/ as it is.
    # tar runs gzip itself: piped, sh (no pipefail) would hide tar's failure.
    tar -C "{{ absolute_path(out / "worker") }}" \
        --sort=name --mtime="@{{ source_date_epoch }}" \
        --owner=0 --group=0 --numeric-owner --mode=u+rwX,go=rX \
        --pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime \
        --use-compress-program="gzip -9 -n" \
        -cf "{{ absolute_path(out / "worker.tar.gz") }}" VERSION app deploy.sh engine live migrations

# Check deploy.sh (scripts/cloudflare-deploy-test.sh): each subcommand against a stubbed wrangler and Restate.
cloudflare-deploy-test:
    ./scripts/cloudflare-deploy-test.sh

# Build the tarball of `version` and check it (scripts/cloudflare-tarball-check.sh): its layout, and its deploy.sh run (dry) from a copy of testdata/cloudflare-installation, in build/cloudflare/check.
cloudflare-tarball-check version="0.0.0": (cloudflare-tarball version)
    ./scripts/cloudflare-tarball-check.sh build/cloudflare/worker.tar.gz build/cloudflare/check
