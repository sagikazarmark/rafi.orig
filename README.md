# Rafi

**Live, verifiable raffles for meetups.**

Participants scan a QR code and join on their phone with a display name and
email: no app, no account. The organizer draws each prize live. Every draw
commits to a future [drand](https://drand.love) randomness round before it
exists, so nobody (the organizer included) can predict or steer the result,
and anyone can re-run every draw afterwards.

- **Projector screen** (`/s/<slug>`): intro, prizes, QR code, live entrant count, countdown, winner reveal.
- **Participant page** (`/j/<CODE>`): server-rendered, works on bad venue wifi; a small WebAssembly (with a JavaScript fallback) keeps it live. It shows "you won" with a private claim code.
- **Organizer admin** (`/admin`): raffles, prizes, people without a smartphone, draw console (Draw / Handed over / Not here / Declined), CSV export.
- **Verification** (`/v/<slug>`): re-runs every draw in the browser; [`tools/verify.py`](tools/verify.py) does the same independently.

See [`docs/PLAN.md`](docs/PLAN.md) for the design and the reasoning behind it.

## How the draw works

1. **Start the draw** closes registration, freezes the entrant list (random IDs only) and opens the
   raffle from its **manifest**: the entrants, one item per prize copy, the algorithm and the beacon
   source (`{"drand": "<chain hash>"}`). It publishes the manifest and `manifest_hash = SHA256(manifest)`.
2. For each item the organizer presses **Draw**. The raffle engine commits to drand
   quicknet round `R` (about 6 s in the future), announces it on screen, waits durably until it is published,
   fetches the beacon from the public relays and verifies its BLS signature against the quicknet key
   ([`drand`](https://crates.io/crates/drand)); `randomness = SHA256(signature)`. A beacon that can't be had
   leaves the draw pending on round `R`: retrying fetches the same round, so nobody can force a redraw.
   The only way out is an explicit abandon, and the verification page shows whom the round would have picked.
3. `seed = SHA256("rafi-draw-v1" ‖ randomness ‖ manifest_hash ‖ k)`. The winner is `pool[uniform(|pool|)]`, using
   SHA-256 counter-mode rejection sampling, so there is no modulo bias.
4. One win per person. "Not here" or "Declined" removes that person and the prize is drawn again with a new round
   (not while another draw is pending, since that could change its pool).

The raffle (its state machine, the draw algorithm and the engine's rules) lives in [`crates/raffle`](crates/raffle)
(pure Rust, no IO): a manifest and an engine run a whole raffle, and nothing of the app is needed to draw or verify
it. What the app adds around it (the lifecycle, starting the draw, entrants, settings) is in
[`crates/lifecycle`](crates/lifecycle). The app's use cases ([`crates/app`](crates/app)) talk to **the
raffle engine** through one interface (`Engine`, in [`crates/engine`](crates/engine); its contract is
documented there) and can't tell one engine from another. The engine is the **Restate virtual object** `Raffle`
(keyed by the raffle's ID; [`crates/engine-restate`](crates/engine-restate)): the app calls it through the Restate
ingress, with the client the Restate SDK generates from the object, and it writes the raffle's log after every
change straight into the app's database.
Production is planned on Cloudflare Workers, where the engine is **`rafi-restate-worker`**
([`apps/restate-worker`](apps/restate-worker)); for self-hosting it is **`rafi-restate`**
([`apps/restate`](apps/restate)), a separate native service. Rafi relies on Restate
for draws: there is no in-process fallback in production (`rafi-engine`'s `local` feature is for tests and
development only). Restate only ever sees anonymous IDs. The app decides which drand chain raffles commit to
(`RAFI_DRAND_CHAIN`); the engine only checks it. See [`docs/engine-plan.md`](docs/engine-plan.md).

The server keeps no state between requests. Natively, phones poll their participant view (`/api/r/<slug>/participant`), and
the projector and the admin poll the full view (`/api/r/<slug>/view`) every few seconds; both carry the raffle's
`version` from the database. On Cloudflare Workers ([`docs/workers-plan.md`](docs/workers-plan.md)) a live channel
per raffle pushes the participant view instead.

```
app ──ingress──▶ Restate ──▶ engine host (Raffle) ──▶ database ◀── app ◀── phones, projector (poll)
```

## Layout

```
crates/raffle                 rafi-raffle: the raffle: draw algorithm, state machine and its admission rule, manifest, verifier
                              (pure, tested, vectors); `mock-chain`: the mock drand chain (development and tests)
crates/lifecycle              rafi-lifecycle: the raffle's lifecycle in the app (pure): its states, starting the draw, who may
                              join, entrants' names and emails, settings, prizes as the raffle's items, what phones see
crates/store                  rafi-store: all SQL, over SQLite, Postgres and D1; the raffle engine's write, reading the snapshot phones are shown
crates/engine                 rafi-engine: the raffle engine's contract (Engine, Store, Observer); behind features,
                              the drand client engines draw with (settings, the round to commit to), an in-process engine
                              (tests, development) and the engine conformance suite
crates/engine-restate         rafi-engine-restate: the `Raffle` virtual object and the `EngineInfo` service, and the app's
                              Engine over the ingress (the SDK's generated clients)
crates/live-client            rafi-live-client: what a phone sees and runs (participant view and channel, how clients follow,
                              page config); `card`: the Dioxus card
crates/live-server            rafi-live-server: the servers' side (the Notifier a deployment pushes through, also the raffle
                              engine's Observer; LiveUpdates; the view built from a raffle as drawn, the live channel's
                              publish rule)
crates/dioxus-altcha          dioxus-altcha: ALTCHA v2 proof-of-work solved in the browser (Web Workers on WebCrypto), with a
                              Dioxus hook; no rafi dependencies (docs/dioxus-altcha-plan.md). The server side is the `altcha` crate
crates/cloudflare             rafi-cloudflare: the Cloudflare glue both Workers share (RaffleRoom Durable Object, notifier, config)
crates/api                    rafi-api: the contract between the app's server and browser: views, and the admin API declared
                              once (`admin_functions!`: the server's `Admin` trait and `dispatch`, the browser's client)
crates/app                    rafi-app: every use case (raffles, prizes, entrants, the draw, the admin's reads), without HTTP
crates/server                 rafi-server: the axum server (participant pages, public JSON, the admin API); `rafi` serves it
                              natively, rafi-worker on Workers

apps/rafi                     rafi: the native binary serving rafi-server and the web app's files; self-hosting
apps/worker                   rafi-worker: the app on Cloudflare Workers (rafi-server's routes, the Cron, RaffleRoom)
apps/restate                  rafi-restate: the native binary serving the Restate services (config, logging, Dockerfile); self-hosting
apps/restate-worker           rafi-restate-worker: the same services as a Cloudflare Worker, on D1, nudging the live channel
apps/web                      rafi-web: the Dioxus 0.7 web app (admin, projector, verification page), bundled by `dx`
apps/participant              rafi-participant: the participant page's WebAssembly, and its JavaScript fallback

migrations                    SQLite (and D1) schema; migrations/postgres: the same for Postgres
testbeds/store-d1             rafi-store-d1-check: a Worker running rafi-store's conformance suite on D1
testbeds/cloudflare           rafi-cloudflare-check: a Worker hosting RaffleRoom for the live channel's checks
                              (testbeds are never deployed; scripts/workers-check.sh runs them under wrangler dev)
tools                         verify.py: independent verifier (Python standard library only)
testdata                      shared test vectors
compose.yaml                  a local Restate server for development
deploy                        self-hosting: docker compose (Caddy, the app, rafi-restate, optionally Restate)
```

## Development

Requirements: Rust stable with `wasm32-unknown-unknown`, and `dx` **0.7.10** (it must match the dioxus version):

```sh
cargo install dioxus-cli@0.7.10 --locked
```

The UI uses **Tailwind CSS v4 + daisyUI 5**, with custom light (`rafi`) and dark (`rafi-dark`) themes defined in
[`apps/web/tailwind.css`](apps/web/tailwind.css). `dx` compiles it automatically with its own standalone Tailwind CLI;
daisyUI comes from npm (`apps/web/package.json`; devenv runs `npm install` on entering the shell). A plain
`cargo build`/`clippy` of the web app needs the compiled stylesheet too: on a fresh checkout, run `dx build` once, or
`tailwindcss -i tailwind.css -o assets/tailwind.css` in `apps/web`.
Organizer forms use [dioform](https://crates.io/crates/dioform) (typed fields, validation, managed async submit)
rendered with components from the [dioxus-daisyui-components](https://github.com/sagikazarmark/dioxus-daisyui-components)
registry (fields, select, switch, buttons, alert dialogs, dropdown menu, tabs, toasts), installed into
`apps/web/src/components/` with `dx components add <name> --path <registry checkout>`. Treat that directory as
vendored: update it by re-installing (`--force`), not by editing.

The app is two crates: the **web app** (`apps/web`, `rafi-web`: admin, projector and verification page; a Dioxus
single-page app built by `dx`) and the **server** (`crates/server`, `rafi-server`: participant pages, the public and
admin JSON APIs, and the web app's files; plain `cargo`, binary `rafi`). The web app calls the server's JSON routes
(`POST /api/admin/<function>`) on the same origin; both sides of that API come from one declaration in `rafi-api`.

The participant page's WebAssembly and its fallback are built into `apps/participant/dist`, which the server serves at
`/participant/` (`RAFI_PARTICIPANT_DIR`). Without it the participant page still renders, but phones can't join or follow the
draw. Build it once, and again when `apps/participant`, `crates/live-client` or `crates/dioxus-altcha` change (it needs `wasm-bindgen-cli` at the
version in `Cargo.lock`, and `wasm-opt` from binaryen):

```sh
cargo install wasm-bindgen-cli --version 0.2.129 --locked
./scripts/participant.sh
```

Draws need Restate (see below). For the admin and the pages alone, run without Clerk, in two terminals:

```sh
# the server; dx proxies /api/, /j/, /rules/, /privacy/ and /participant/ to it (apps/web/Dioxus.toml)
RAFI_DEV_AUTH=1 RAFI_PUBLIC_URL=http://localhost:8787 PORT=8788 cargo run -p rafi

# the web app, with hot reload
cd apps/web && dx serve --port 8787
```

Then open <http://localhost:8787/admin>. To work offline, add `RAFI_DRAND_CHAIN=mock` and `--features mock-drand`
to the server, and the mock chain to the raffle engine: draws then use a test chain with a public key (the `drand`
crate's `TestChain`) and are *not* verifiable.
To test with phones on your network, set `RAFI_PUBLIC_URL` to your machine's LAN address and run `dx serve --addr 0.0.0.0`.
Joining needs HTTPS there: phones solve the proof-of-work with WebCrypto, which browsers only offer on HTTPS and localhost.
Use a tunnel (e.g. `cloudflared tunnel --url http://localhost:8787`) or a certificate for the LAN address (mkcert).

The server alone serves everything once the web app is bundled (this is what the container image runs):

```sh
(cd apps/web && dx bundle --platform web --release)   # -> target/dx/rafi-web/release/web/public
./scripts/participant.sh                                 # -> apps/participant/dist
RAFI_DEV_AUTH=1 cargo run --release -p rafi       # http://localhost:8080
```

It listens on `IP`:`PORT` (default `127.0.0.1:8080`), serves the participant scripts at `/participant/` from
`RAFI_PARTICIPANT_DIR` (default `apps/participant/dist`), and the web app from `RAFI_PUBLIC_DIR`
(default `target/dx/rafi-web/release/web/public`), both relative to the working directory, falling back to the web
app's `index.html` for its own routes.

With Restate: Restate runs in Docker ([`compose.yaml`](compose.yaml)), the app and `rafi-restate` on your
machine, both on the same database file:

```sh
export RAFI_DATABASE_URL="sqlite://$PWD/rafi.db"

docker compose up -d       # Restate: ingress on :8080, admin API and UI on :9070
                           # (RESTATE_INGRESS_PORT / RESTATE_ADMIN_PORT to move them)

# the server (it creates the database) and the web app
RAFI_DEV_AUTH=1 RAFI_PUBLIC_URL=http://localhost:8787 RESTATE_INGRESS_URL=http://localhost:8080 \
  PORT=8788 cargo run -p rafi
(cd apps/web && dx serve --port 8787)

# the raffle engine, in another terminal (offline: add --features mock-drand and RAFI_RESTATE_DRAND__MOCK=true,
# matching the app's RAFI_DRAND_CHAIN=mock; the app refuses to start a draw on an engine on another chain)
RAFI_RESTATE_DATABASE__URL="$RAFI_DATABASE_URL" cargo run -p rafi-restate

# once, and again whenever the services' handlers change (Raffle, EngineInfo)
curl -X POST localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://host.docker.internal:9080", "force": true}'
```

With Clerk, export `CLERK_PUBLISHABLE_KEY` (when building the web app) and `CLERK_SECRET_KEY` (for the server),
and leave `RAFI_DEV_AUTH` unset.

### Tests

```sh
export RESTATE_SERVER_BIN=$(./scripts/restate-server.sh)   # a pinned restate-server for the end-to-end tests
cargo test --workspace --exclude rafi-server --exclude rafi-web -- --include-ignored  # the rules, the app's use cases, drand, store, the engine conformance suite (local runner and the Raffle object on restate-server)
cargo test -p rafi-server -- --include-ignored             # app (with the in-process raffle engine), and app + rafi-restate + restate-server
python3 tools/verify.py --vectors testdata/draw-vectors.json
```

Without `RESTATE_SERVER_BIN` the end-to-end tests skip (and fail on CI).

**The storage conformance suite** (`rafi_store::conformance`) is the contract every database backend must pass:
named scenarios for the query layer (`query.*`: decoding, `NULL` parameters, batch atomicity and snapshots, unique
violations, change counts, ordering, D1's parameter limit, the version triggers) and the domain functions
(`domain.*`). `cargo test -p rafi-store` lists each one per backend, as `<backend>::<scenario>`
(`sqlite::domain.join_cap`), and reports a backend that isn't available, or a capability a scenario needs that the
backend lacks, as ignored with the reason. The tiers:

| Backend | Runs |
|---|---|
| SQLite (in memory) | always |
| embedded Postgres | by default: a local PostgreSQL 18 from prebuilt binaries ([Oliphaunt](https://oliphaunt.dev)), one server per run (started on first use, about 3 s) with a fresh schema per scenario; no install needed. The first build downloads about 13 MB. `RAFI_TEST_EMBEDDED_POSTGRES=off` skips it |
| Postgres | when `RAFI_TEST_POSTGRES_URL` names a throwaway database (a fresh schema per scenario); CI sets it |
| D1 | under `wrangler dev`, through `testbeds/store-d1` (`./scripts/workers-check.sh`, below) |

A new backend implements `rafi_store::conformance::Harness` (a name, its capabilities, and a fresh, migrated
database per scenario) and passes the suite; see the module's documentation.

**The engine conformance suite** (`rafi_engine::conformance`) is the contract every raffle engine must pass,
through the `Engine` interface alone: the fairness rules (the round is committed before its beacon is asked for;
a withheld or forged beacon stalls the draw on its round and a retry fetches the same round; abandons are
logged; nothing changes a pending draw's pool; every log passes the verifier) and the contract
(idempotency, one command at a time per raffle, asynchronous draws, refusals with their kind, a raffle forgotten
once finished). It deploys once per drand profile (instant, with a lead, a withheld relay, a forging relay, a flaky
store) and runs on the local runner (`cargo test -p rafi-engine`) and on the `Raffle` object through the app's
client (`cargo test -p rafi-engine-restate -- --include-ignored`, one `restate-server` per profile).

With `RAFI_TEST_POSTGRES_URL` set, the app's tests (the end-to-end one included) also run against Postgres, each on
a fresh schema, which they leave behind. Locally, for example:

```sh
D=$(mktemp -d); initdb -D $D/pg -U rafi --auth=trust >/dev/null
pg_ctl -D $D/pg -o "-p 54329 -k ''" -l $D/pg.log start && createdb -h 127.0.0.1 -p 54329 -U rafi rafi_test
export RAFI_TEST_POSTGRES_URL=postgres://rafi@127.0.0.1:54329/rafi_test
# afterwards: pg_ctl -D $D/pg stop && rm -rf $D
```

## Configuration

The app:

| Variable | Default | |
|---|---|---|
| `IP`, `PORT` | `127.0.0.1`, `8080` | Listen address (the image sets `0.0.0.0`) |
| `RAFI_PUBLIC_DIR` | `target/dx/rafi-web/release/web/public` | The bundled web app (the image sets `/usr/local/app/public`) |
| `RAFI_PARTICIPANT_DIR` | `apps/participant/dist` | The participant page's WebAssembly and fallback, from `./scripts/participant.sh` (the image sets `/usr/local/app/participant`) |
| `RAFI_PUBLIC_URL` | `http://localhost:8080` | External base URL (QR codes, links) |
| `RAFI_DATABASE_URL` | `sqlite://rafi.db` | The database, shared with `rafi-restate`: SQLite (`sqlite://…`, created if missing) or PostgreSQL (`postgres://user:password@host/db`, the database must exist). The app applies the migrations on start |
| `RAFI_DEV_AUTH` | off | **Development only:** no login, every admin is `dev` |
| `CLERK_SECRET_KEY` | | Clerk backend key (`CLERK_PUBLISHABLE_KEY` at build time) |
| `RESTATE_INGRESS_URL`, `RESTATE_API_KEY` | | The raffle engine's Restate ingress (Restate Cloud: an API key with ingress access). Without it, draws can't start |
| `RESTATE_TIMEOUT_SECS` | `90` | How long a call to the raffle engine waits: a command queued behind a draw waiting for its beacon may take the engine's drand lead plus fetch timeout; past this it is accepted and applies later. Keep it above `lead_secs + timeout_secs` of `rafi-restate` |
| `RAFI_DRAND_CHAIN` | quicknet | The drand chain new raffles commit to (a chain hash). Release builds accept only public drand chains; `mock` (a public test chain, *not* verifiable) needs `--features mock-drand`. The raffle engine must draw from the same chain: the app refuses to start a draw otherwise |
| `RAFI_DRAND_MOCK` | off | Development alias for `RAFI_DRAND_CHAIN=mock` |
| `RAFI_ALTCHA_KEY` | random per start | HMAC key for the proof-of-work challenges (ALTCHA v2); not empty |
| `RAFI_ALTCHA_COST` | `1000` | Proof-of-work difficulty: PBKDF2 iterations per attempt, about 256 attempts per solve (at least 1; provisional, see docs/dioxus-altcha-plan.md M4) |
| `RAFI_MAX_ENTRANTS` | `2000` | The most entrants a raffle takes |
| `RAFI_RETENTION` | off | Erase personal data after the event in the hourly retention run (otherwise erase by hand in the admin). The run always ends live updates a day after a raffle finishes and forgets expired proof-of-work challenges |
| `RAFI_RETENTION_DAYS` / `RAFI_RETENTION_WINNERS_DAYS` | `30` / `90` | Erase personal data after the event. The first erasing run that sees a raffle finished dates its erasure with these; a date once set is kept. The raffle engine needs neither |

`rafi-restate` (draw settings, database, identity keys):
see [`apps/restate`](apps/restate/README.md).

## Self-hosting

The native server, with Docker (production is planned on Cloudflare Workers, below). On a VM with Docker and a
domain pointing at it:

```sh
git clone https://github.com/sagikazarmark/rafi.orig rafi && cd rafi/deploy
cp .env.example .env && $EDITOR .env   # domain, Clerk keys, proof-of-work key, Restate Cloud
docker compose pull && docker compose up -d
restate deployments register https://$RAFI_DOMAIN:9080   # once, and when the handlers change (Raffle, EngineInfo)
```

[`deploy/compose.yaml`](deploy/compose.yaml) runs Caddy (automatic HTTPS), the app and `rafi-restate` from the GHCR
images (`docker compose up -d --build` builds them from the checkout instead). The app and `rafi-restate` share the
SQLite database on a volume. Restate Cloud reaches `rafi-restate` at `https://$RAFI_DOMAIN:9080`, where Caddy
terminates TLS; `rafi-restate` only accepts requests signed with the environment's key (`RESTATE_IDENTITY_KEYS`, the
environment's signing public key). The app calls the Restate Cloud ingress with an API key that has ingress access
(`RESTATE_API_KEY`). Open ports: 22, 80, 443 (TCP and UDP) and 9080.

Without Restate Cloud, `docker compose --profile self-hosted up -d` also runs a Restate server on the VM and registers
`rafi-restate` with it (see the comments in `compose.yaml`).

PostgreSQL instead of SQLite: point both at the same, existing database, the app with
`RAFI_DATABASE_URL=postgres://user:password@host/rafi` and `rafi-restate` with the same URL in `database.url`
(`RAFI_RESTATE_DATABASE__URL`). The app applies the migrations (`migrations/postgres`) on start, as it does for
SQLite; until it has, `rafi-restate`'s writes fail and Restate retries them. Transactions run serializable, as on
SQLite. Connections use TLS when the server offers it; add `?sslmode=verify-full` to require it, against the Mozilla
root certificates or `sslrootcert=<file>`. `deploy/compose.yaml` keeps SQLite; its comments show the Postgres
variant.

To update: build and push new images (CI does it on `main`; by hand, `docker buildx build --platform linux/amd64
--build-arg CLERK_PUBLISHABLE_KEY -t ghcr.io/sagikazarmark/rafi:latest --push .` and the same for
`apps/restate/Dockerfile`), then `docker compose pull && docker compose up -d` on the VM. Avoid it
during a live raffle (`/api/status` shows `live_raffles`).

## Cloudflare Workers

The plan and its decisions: [`docs/workers-plan.md`](docs/workers-plan.md). Cloudflare is one deployment, not an
assumption of the code: the app and the raffle engine are written against small interfaces (the engine's `Store` and
`Observer`, the live `Notifier`, the app's `RateLimit`), and `crates/cloudflare` (`rafi-cloudflare`) implements the
shared ones over Workers bindings. Two Workers wire it up: the app (`apps/worker`, which also hosts the
per-raffle `RaffleRoom` Durable Object and counts joins on its rate-limiting bindings) and the raffle engine
(`apps/restate-worker`), with the D1 database they share.

Local development, everything under `wrangler dev` (the tools are in `devenv.nix`; wasm-bindgen-cli must match
`Cargo.lock`):

```sh
./scripts/app-assets.sh                                   # web app bundle + participant scripts -> apps/worker/worker-assets
(cd apps/worker && wrangler d1 migrations apply rafi --local --env dev)
(cd apps/worker && wrangler dev --env dev)                # http://127.0.0.1:8787, dev auth, mock drand
(cd apps/restate-worker && wrangler dev --env dev --port 8791)
# a local restate-server with its ingress on :18080, then:
restate deployments register http://127.0.0.1:8791 --use-http1.1
```

`./scripts/workers-check.sh` runs every Workers check locally: rafi-store's conformance suite on D1, the live channel over WebSockets
(`testbeds/cloudflare`), the participant page in headless Chromium, a draw through Restate, and a whole evening on the app Worker. CI's `workers`
job runs it.

Deploying (needs a Cloudflare account and Restate Cloud): see the comments at the top of `apps/worker/wrangler.toml` and
`apps/restate-worker/wrangler.toml` (D1 created with `--jurisdiction eu`, the secrets, registering the raffle engine
with Restate Cloud per release). Don't deploy while a raffle is live: a deploy restarts the Durable Objects.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
