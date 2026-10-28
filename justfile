wasm := "wasm32-unknown-unknown"

# List the recipes.
[private]
default:
    @just --list --unsorted

# Run all CI jobs but `workers` (long, and it needs a Chromium).
ci: fmt clippy test verify doc audit deny

# Check formatting.
fmt:
    cargo fmt --all --check

# Build the web app's stylesheet (Tailwind CSS + daisyUI); rafi-web's asset!() needs it.
[working-directory: 'apps/web']
css:
    tailwindcss -i tailwind.css -o assets/tailwind.css --minify

# Run clippy, natively and on wasm32.
clippy: clippy-native clippy-wasm

# Run clippy natively, with and without the development features.
clippy-native:
    cargo clippy --workspace --exclude rafi-server --exclude rafi --exclude rafi-web --all-targets --locked -- --deny warnings
    cargo clippy -p rafi-server --all-targets --locked -- --deny warnings
    cargo clippy -p rafi-server --features mock-drand --all-targets --locked -- --deny warnings
    cargo clippy -p rafi --all-targets --locked -- --deny warnings
    cargo clippy -p rafi --features dev,mock-drand --all-targets --locked -- --deny warnings
    cargo clippy -p rafi-restate --features mock-drand --all-targets --locked -- --deny warnings

# Run clippy on wasm32: the web app, the Workers crates and the Worker.
clippy-wasm: css
    cargo clippy --target {{ wasm }} -p rafi-web --locked -- --deny warnings
    cargo clippy --target {{ wasm }} \
        -p rafi-lifecycle -p rafi-raffle -p rafi-api -p rafi-app \
        -p rafi-store -p rafi-store-d1-check -p rafi-live-client -p rafi-live-server -p rafi-cloudflare -p rafi-cloudflare-check \
        -p rafi-engine -p rafi-engine-restate -p rafi-participant --locked -- --deny warnings
    cargo clippy --target {{ wasm }} -p rafi-server --lib --locked -- --deny warnings
    cargo clippy --target {{ wasm }} -p rafi-worker --locked -- --deny warnings
    cargo clippy --target {{ wasm }} -p rafi-worker --features dev --locked -- --deny warnings

# Run tests: the libraries and the engine (end to end against restate-server), then the app.
test:
    cargo test --workspace --exclude rafi-server --exclude rafi-web --locked -- --include-ignored
    cargo test -p rafi-server --locked -- --include-ignored

# Run the independent verifier on the shared test vectors.
verify:
    python3 tools/verify.py --vectors testdata/draw-vectors.json

# Run rustdoc: natively with every feature, and the wasm32-only crates (empty elsewhere) on wasm32.
[env("RUSTDOCFLAGS", "--deny warnings")]
doc:
    cargo doc --workspace --exclude rafi-web --exclude rafi-cloudflare --exclude rafi-participant --exclude rafi-worker --all-features --no-deps --locked
    cargo doc --target {{ wasm }} -p rafi-cloudflare -p rafi-participant -p rafi-worker --no-deps --locked

# Audit dependencies for security advisories.
audit:
    cargo audit

# Check dependency licenses, bans and sources (advisories are `audit`'s job).
deny:
    cargo deny --locked check bans licenses sources

# Run the Workers checks under `wrangler dev` (scripts/workers-check.sh).
workers:
    ./scripts/workers-check.sh

# Build what a Cloudflare deployment deploys (scripts/cloudflare-artifact.sh): what a release ships.
cloudflare-artifact out="target/cloudflare/rafi":
    ./scripts/cloudflare-artifact.sh {{ out }}

# Build and prove a Cloudflare release (scripts/cloudflare-release.sh): the tarball, its checksums, a dry deploy through deploy/cloudflare.
cloudflare-release version:
    ./scripts/cloudflare-release.sh {{ version }}
