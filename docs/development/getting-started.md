# Getting started

Building and running Rafi on your machine. To deploy it, see [Operations](../operations/overview.md).

## Requirements

- Rust stable with the `wasm32-unknown-unknown` target
- `dx` **0.7.10**, which must match the Dioxus version:

  ```sh
  cargo install dioxus-cli@0.7.10 --locked
  ```

- For the participant page: `wasm-bindgen-cli` at the version in `Cargo.lock`, and `wasm-opt` (binaryen):

  ```sh
  cargo install wasm-bindgen-cli --version 0.2.129 --locked
  ```

- Node, for daisyUI (`apps/web/package.json`)

[devenv](https://devenv.sh) provides all of this at the versions CI uses (`devenv.nix`), and runs `npm install` in
`apps/web` when you enter the shell. CI runs every step in it.

## First build

A fresh checkout needs two generated things before everything compiles and runs:

```sh
# the web app's stylesheet (dx builds it too; plain cargo build/clippy of rafi-web needs it to exist)
(cd apps/web && tailwindcss -i tailwind.css -o assets/tailwind.css)

# the participant page's WebAssembly and fallback -> apps/participant/dist
./scripts/participant.sh
```

Rebuild the participant page whenever `apps/participant` or `crates/live-client` change. Without it, the
participant page still renders, but phones can't join or follow the draw.

## The admin and the pages, without draws

Draws need Restate (below). For everything else, run without Clerk, in two terminals:

```sh
# the server; dx proxies /api/, /j/, /rules/, /privacy/ and /participant/ to it (apps/web/Dioxus.toml)
RAFI_DEV_AUTH=1 RAFI_PUBLIC_URL=http://localhost:8787 PORT=8788 cargo run -p rafi

# the web app, with hot reload
cd apps/web && dx serve --port 8787
```

Then open <http://localhost:8787/admin>.

## With draws (Restate)

Run a local `restate-server` (the pinned one), the app and `rafi-restate` on the same database file:

```sh
export RAFI_DATABASE_URL="sqlite://$PWD/rafi.db"

# Restate, in its own terminal: ingress on :8080, admin API and UI on :9070, data in ./restate-data
$(./scripts/restate-server.sh)

# the server (it creates the database) and the web app
RAFI_DEV_AUTH=1 RAFI_PUBLIC_URL=http://localhost:8787 RESTATE_INGRESS_URL=http://localhost:8080 \
  PORT=8788 cargo run -p rafi
(cd apps/web && dx serve --port 8787)

# the raffle engine, in another terminal
RAFI_RESTATE_DATABASE__URL="$RAFI_DATABASE_URL" cargo run -p rafi-restate

# once, and again whenever the services' handlers change (Raffle, EngineInfo)
curl -X POST localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://localhost:9080", "force": true}'
```

### Offline: the mock drand chain

To work without internet, both sides use the mock chain, a public test chain (the `drand` crate's `TestChain`).
Its draws are **not** verifiable.

```sh
RAFI_DRAND_CHAIN=mock … cargo run -p rafi --features mock-drand
RAFI_RESTATE_DRAND__MOCK=true … cargo run -p rafi-restate --features mock-drand
```

The app refuses to start a draw on an engine that uses another chain.

### With Clerk

Export `CLERK_PUBLISHABLE_KEY` when building the web app and `CLERK_SECRET_KEY` for the server, and leave
`RAFI_DEV_AUTH` unset.

## The server alone

Once the web app is bundled, `rafi` serves everything itself:

```sh
(cd apps/web && dx bundle --platform web --release)   # -> target/dx/rafi-web/release/web/public
./scripts/participant.sh                                 # -> apps/participant/dist
RAFI_DEV_AUTH=1 cargo run --release -p rafi              # http://localhost:8080
```

It listens on `IP`:`PORT` (default `127.0.0.1:8080`). It serves the participant scripts at `/participant/` from
`RAFI_PARTICIPANT_DIR` and the web app from `RAFI_PUBLIC_DIR`, both relative to the working directory, falling
back to the web app's `index.html` for its own routes.

## Testing with phones

Set `RAFI_PUBLIC_URL` to your machine's LAN address and run `dx serve --addr 0.0.0.0`. Joining needs HTTPS: phones
solve the proof-of-work with WebCrypto, which browsers only offer on HTTPS and `localhost`. Use a tunnel (for
example `cloudflared tunnel --url http://localhost:8787`) or a certificate for the LAN address (mkcert).

## Cloudflare Workers, locally

Everything runs under `wrangler dev`. The tools are in `devenv.nix`, and `wasm-bindgen-cli` must match `Cargo.lock`.

```sh
./scripts/app-assets.sh                                   # web app bundle + participant scripts -> apps/worker/worker-assets
(cd apps/worker && wrangler d1 migrations apply rafi --local --env dev)
(cd apps/worker && wrangler dev --env dev)                # http://127.0.0.1:8787, dev auth, mock drand
(cd apps/restate-worker && wrangler dev --env dev --port 8791)
# Restate with its ingress on :18080 (the dev app Worker's RESTATE_INGRESS_URL), then register the engine:
RESTATE_INGRESS__BIND_ADDRESS=127.0.0.1:18080 $(./scripts/restate-server.sh)
curl -X POST localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://127.0.0.1:8791", "use_http_11": true}'
```

## Next

- [Testing](testing.md)
- [The web app](web-app.md): UI stack, forms, components, the proof-of-work
- [Architecture](../explanation/architecture.md) and [Crates and apps](../reference/crates.md)
