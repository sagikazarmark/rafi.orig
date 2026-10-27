# The checks, as CI runs them (.github/workflows/ci.yaml runs one job per
# recipe) and as they run locally, in the devenv shell.

# List the recipes.
[private]
default:
    @just --list --unsorted

# Run every check CI runs, except the Workers checks (`just workers`: long, and they need a Chromium).
ci: fmt clippy test verify doc deny

# Check formatting.
fmt:
    cargo fmt --all --check

# Build the web app's stylesheet (Tailwind CSS + daisyUI). rafi-web's asset!() needs it to exist.
[working-directory: 'apps/web']
css:
    tailwindcss -i tailwind.css -o assets/tailwind.css --minify

# Run clippy natively and on wasm32-unknown-unknown.
clippy: clippy-native clippy-wasm

# Run clippy natively: the libraries, the engine and the app, with and without the development features.
clippy-native:
    cargo clippy --workspace --exclude rafi-server --exclude rafi --exclude rafi-web --all-targets --locked -- -D warnings
    cargo clippy -p rafi-server --all-targets --locked -- -D warnings
    cargo clippy -p rafi-server --features mock-drand --all-targets --locked -- -D warnings
    cargo clippy -p rafi --all-targets --locked -- -D warnings
    cargo clippy -p rafi --features dev,mock-drand --all-targets --locked -- -D warnings
    cargo clippy -p rafi-restate --features mock-drand --all-targets --locked -- -D warnings

# Run clippy on wasm32-unknown-unknown: the web app, the Workers crates and the Worker.
clippy-wasm: css
    cargo clippy --target wasm32-unknown-unknown -p rafi-web --locked -- -D warnings
    cargo clippy --target wasm32-unknown-unknown \
        -p rafi-lifecycle -p rafi-raffle -p rafi-api -p rafi-app \
        -p rafi-store -p rafi-store-d1-check -p rafi-live-client -p rafi-live-server -p rafi-cloudflare -p rafi-cloudflare-check \
        -p rafi-engine -p rafi-engine-restate -p rafi-participant --locked -- -D warnings
    cargo clippy --target wasm32-unknown-unknown -p rafi-server --lib --locked -- -D warnings
    cargo clippy --target wasm32-unknown-unknown -p rafi-worker --locked -- -D warnings
    cargo clippy --target wasm32-unknown-unknown -p rafi-worker --features dev --locked -- -D warnings

# Run the tests: the libraries and the engine (end to end against restate-server), then the app.
test:
    cargo test --workspace --exclude rafi-server --exclude rafi-web --locked -- --include-ignored
    cargo test -p rafi-server --locked -- --include-ignored

# Check that the independent verifier agrees with the shared test vectors.
verify:
    python3 tools/verify.py --vectors testdata/draw-vectors.json

# Build the documentation, refusing warnings (a broken link fails): natively with every feature, and the Workers crates on wasm32.
[env("RUSTDOCFLAGS", "--deny warnings")]
doc:
    cargo doc --workspace --exclude rafi-web --exclude rafi-cloudflare --exclude rafi-participant --exclude rafi-worker --all-features --no-deps --locked
    cargo doc --target wasm32-unknown-unknown -p rafi-cloudflare -p rafi-participant -p rafi-worker --no-deps --locked

# Check the dependencies: advisories, licenses, bans, sources (deny.toml).
deny:
    cargo deny check

# Run the Cloudflare Workers checks under `wrangler dev`: D1, the live channel, the participant page in Chromium, an evening on the Worker, the production dry run.
workers:
    ./scripts/workers-check.sh
