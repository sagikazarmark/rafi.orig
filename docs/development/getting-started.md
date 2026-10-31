# Getting started

Building and running Rafi on your machine. To deploy it, see [Operations](../operations/overview.md).

## Requirements

[devenv](https://devenv.sh) provides everything at the versions CI uses ([`devenv.nix`](../../devenv.nix)), and
runs `npm install` in `apps/web` when you enter the shell. CI runs every step in it, and every step is a recipe of
the [`justfile`](../../justfile): `just` lists them, `just ci` runs the same checks CI does ([Testing](testing.md)).

Without devenv, you need:

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
- For draws: `restate-server` 1.8 ([Restate's releases](https://github.com/restatedev/restate/releases))
- For Workers: `wrangler` and `worker-build`
- For the browser checks (`just workers`): a Chromium, found as `chromium` or `google-chrome` on the path, or
  named in `CHROMIUM`. devenv installs one on Linux; on macOS, point `CHROMIUM` at your browser.

## With devenv

```sh
devenv up
```

This runs everything natively, with dev auth (no Clerk) and live reload:

| Process | What it is |
|---|---|
| `restate` | Restate: ingress on `:8080`, admin API and UI on <http://localhost:9070/ui> |
| `engine` | The raffle engine (`rafi-restate`) on `:9080`, registered with Restate once it's up |
| `rafi` | The server on `:8788`, behind `dx serve` (`apps/web/Dioxus.toml` proxies to it) |
| `dioxus` | `dx serve`: the web app on `:8787`, with hot reload |
| `participant` | Builds and checks the participant page (`just apps/participant/ check`), and again when it changes; `rafi` starts after it, and restarts on a new build (it compiles the build's loader in) |

Then open <http://localhost:8787/admin>. Draws use real drand, so they need internet.

- `rafi` and `engine` restart when their Rust code changes. Restate only discovers the engine's handlers when it
  registers it, on start: after changing them (`Raffle`, `EngineInfo`), register it again with
  `restate deployments register --force http://127.0.0.1:9080`. `--force` overwrites what Restate knows about the
  URI, which is fine here, where no raffle in flight matters, and never in production (see
  [Upgrades](../operations/upgrades.md#registering-the-raffle-engine)).
- The database (`rafi.db`) and Restate's data are in `.devenv/state`.
- Ports other than the server's are allocated: if one is taken, devenv takes the next free one.
- `devenv up -d` runs in the background: `devenv processes logs rafi`, then `devenv down`.

The stacks use dev auth: the server runs with `RAFI_DEV_AUTH`, and tells the web app so (`/config.js`). The `clerk`
profile uses Clerk instead.

### Profiles

```sh
devenv --profile mock up                     # offline: the mock drand chain
devenv --profile workers up                  # on Cloudflare Workers, under wrangler dev
devenv --profile clerk up                    # Clerk instead of dev auth
devenv --profile workers --profile clerk up  # profiles combine
```

Each combination of profiles has its own state (`.devenv/profiles/<names>/state`): its own database, Restate and
local Workers state. Pass the same `--profile` flags to `devenv down` and `devenv processes`.

- **`mock`** builds both sides with `mock-drand`, and draws from the mock chain, a public test chain (the `drand`
  crate's `TestChain`). Its draws are **not** verifiable. The app refuses to start a draw on an engine that uses
  another chain. (The `workers` stack always uses it.)
- **`workers`** runs the Worker (`wrangler-app`, <http://127.0.0.1:8787>; the raffle engine is in it, under
  `/restate`, registered with Restate over HTTP/1.1 once the Worker answers `/healthz`) with
  `wrangler dev --env dev`: dev auth and the mock chain. Before starting it, it applies the D1 migrations
  (devenv-extras' `wrangler.dev.app.d1.migrate`), and builds the static files (`just apps/worker/ assets`) when
  their sources changed, when something else built them, or for the other auth. The first build takes a few
  minutes. The Worker's local data (D1, Durable Objects) lives in the profile's `state/wrangler/state/app` and is
  kept across restarts.
- **`clerk`** signs the organizer in with Clerk. It needs `CLERK_PUBLISHABLE_KEY` and `CLERK_SECRET_KEY`, from
  [Clerk's dashboard](https://dashboard.clerk.com) (API keys; a development instance accepts `localhost`), in `.env`
  at the project root, which the shell loads. Both keys are the server's (or the app Worker's, which reads `.env`
  with `--env-file`): it runs without `RAFI_DEV_AUTH` and hands the publishable key to the web app at `/config.js`.
  The web app is built the same way either way.

## Without devenv

### First build

A fresh checkout needs two generated things before everything compiles and runs:

```sh
# the web app's stylesheet (dx builds it too; plain cargo build/clippy of rafi-web needs it to exist)
just apps/web/ css

# the participant page's WebAssembly and fallback -> apps/participant/dist/public
just apps/participant/ build
```

The server compiles the participant build's loader in (`dist/public/index.html`, with the hashed names of that
build), so `rafi-server`, `rafi` and the Worker don't build without it: run it first, and again whenever
`apps/participant` or `crates/live-client` change, then build the server again (`cargo run` does, and
`devenv up` restarts it). The `just` recipes that build the server run it first.

### The admin and the pages, without draws

Draws need Restate (below). For everything else, run without Clerk, in two terminals:

```sh
# the server; dx proxies /api/, /j/, /rules/, /privacy/ and /participant/ to it (apps/web/Dioxus.toml).
# `--features dev` is what allows RAFI_DEV_AUTH: a build without it refuses to start with the flag.
RAFI_DEV_AUTH=1 RAFI_PUBLIC_URL=http://localhost:8787 PORT=8788 cargo run -p rafi --features dev

# the web app, with hot reload
cd apps/web && dx serve --port 8787
```

Then open <http://localhost:8787/admin>.

### With draws (Restate)

Run `restate-server`, the app and `rafi-restate` on the same database file:

```sh
export RAFI_DATABASE_URL="sqlite://$PWD/rafi.db"

# Restate, in its own terminal: ingress on :8080, admin API and UI on :9070, data in ./restate-data
restate-server

# the server (it creates the database) and the web app
RAFI_DEV_AUTH=1 RAFI_PUBLIC_URL=http://localhost:8787 RAFI_ENGINE_INGRESS_URL=http://localhost:8080 \
  PORT=8788 cargo run -p rafi --features dev
(cd apps/web && dx serve --port 8787)

# the raffle engine, in another terminal (on the loopback address, with Restate alone: no request signing)
RAFI_RESTATE_DATABASE__URL="$RAFI_DATABASE_URL" RAFI_RESTATE_ENDPOINT__INSECURE=true cargo run -p rafi-restate

# once, and again whenever the services' handlers change (Raffle, EngineInfo);
# `force` overwrites the registration: fine in development, never in production
curl -X POST localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://localhost:9080", "force": true}'
```

For the mock drand chain (offline):

```sh
RAFI_DRAND_CHAIN=mock … cargo run -p rafi --features dev,mock-drand
RAFI_RESTATE_DRAND__MOCK=true … cargo run -p rafi-restate --features mock-drand
```

### Cloudflare Workers

`wasm-bindgen-cli` must match `Cargo.lock`. `--persist-to` keeps the Worker's local state (D1, Durable Objects)
across restarts:

```sh
just apps/worker/ assets                                  # web app bundle + participant build + template legal documents -> apps/worker/worker-assets
(cd apps/worker && wrangler d1 migrations apply rafi --local --env dev --persist-to ../../.wrangler-state)
(cd apps/worker && wrangler dev --env dev --persist-to ../../.wrangler-state)   # http://127.0.0.1:8787, dev auth, mock drand
# Restate with its ingress on :18080 (the dev Worker's RAFI_ENGINE_INGRESS_URL), then register the engine, which is
# the same Worker under /restate (wrangler dev speaks HTTP/1.1 only):
RESTATE_INGRESS__BIND_ADDRESS=127.0.0.1:18080 restate-server
curl -X POST localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://127.0.0.1:8787/restate", "use_http_11": true}'
```

## With Clerk

Give the server (or the app Worker) `CLERK_PUBLISHABLE_KEY` and `CLERK_SECRET_KEY`, and leave `RAFI_DEV_AUTH`
unset. The web app needs nothing: it reads the server's `/config.js`.

## The server alone

Once the web app is bundled, `rafi` serves everything itself:

```sh
(cd apps/web && dx bundle --platform web --release)   # -> target/dx/rafi-web/release/web/public
just apps/participant/ build                             # -> apps/participant/dist/public
RAFI_DEV_AUTH=1 cargo run --release -p rafi --features dev   # http://localhost:8080
```

It listens on `IP`:`PORT` (default `127.0.0.1:8080`). It serves the participant build at `/participant/` from
`RAFI_PARTICIPANT_DIR` (which must be the build it was compiled against, or it doesn't start) and the web app from
`RAFI_PUBLIC_DIR`, both relative to the working directory: its files at `/assets/`, and its `index.html`, read at
startup, at every other path, under the web app's Content-Security-Policy. The Worker compiles that `index.html` in
instead, so building `rafi-worker` (`worker-build`, `just clippy-wasm`, `just doc`) needs the web app bundled first
(`just web`).

## Testing with phones

Set `RAFI_PUBLIC_URL` to your machine's LAN address and run `dx serve --addr 0.0.0.0`. Joining needs HTTPS: phones
solve the proof-of-work with WebCrypto, which browsers only offer on HTTPS and `localhost`. Use a tunnel (for
example `cloudflared tunnel --url http://localhost:8787`) or a certificate for the LAN address (mkcert).

## Next

- [Testing](testing.md)
- [The web app](web-app.md): UI stack, forms, components, the proof-of-work
- [Architecture](../explanation/architecture.md) and [Crates and apps](../reference/crates.md)
