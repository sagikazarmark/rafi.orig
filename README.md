# Rafi

**Live, verifiable raffles for meetups.**

Attendees scan a QR code and join on their phone with a display name and
email: no app, no account. The organizer draws each prize live. Every draw
commits to a future [drand](https://drand.love) randomness round before it
exists, so nobody (the organizer included) can predict or steer the result,
and anyone can re-run every draw afterwards.

- **Projector screen** (`/s/<slug>`): intro, prizes, QR code, live entrant count, countdown, winner reveal.
- **Attendee page** (`/j/<CODE>`): server-rendered, works on bad venue wifi; a small WebAssembly (with a JavaScript fallback) keeps it live. It shows "you won" with a private claim code.
- **Organizer admin** (`/admin`): raffles, prizes, people without a smartphone, draw console (Draw / Handed over / Not here / Declined), CSV export.
- **Verification** (`/v/<slug>`): re-runs every draw in the browser; [`tools/verify.py`](tools/verify.py) does the same independently.

See [`docs/PLAN.md`](docs/PLAN.md) for the design and the reasoning behind it.

## How the draw works

1. **Start the draw** closes registration, freezes the entrant list (random IDs only) and publishes
   `list_hash = SHA256(commit document)`.
2. For each prize unit the organizer presses **Draw**. The draw engine commits to drand
   quicknet round `R` (about 6 s in the future), announces it on screen, waits durably until it is published,
   fetches the beacon from the public relays and verifies its BLS signature against the quicknet key
   ([`drand`](https://crates.io/crates/drand)); `randomness = SHA256(signature)`. A beacon that can't be had
   leaves the draw pending on round `R`: retrying fetches the same round, so nobody can force a redraw.
   The only ways out are an offline draw or an explicit abandon, and the verification page shows both.
3. `seed = SHA256("rafi-draw-v1" ‖ randomness ‖ list_hash ‖ k)`. The winner is `pool[uniform(|pool|)]`, using
   SHA-256 counter-mode rejection sampling, so there is no modulo bias.
4. One win per person. "Not here" or "Declined" removes that person and the prize is drawn again with a new round
   (not while another draw is pending, since that could change its pool).
The draw state machine lives in [`crates/core`](crates/core) (pure Rust, no IO). In production it runs in
**`restate-rafi`**, a separate service hosting the **Restate virtual object** `Raffle` (keyed by raffle ID):
the app calls it through the Restate ingress, and it writes every new draw state straight into the app's
database. Rafi relies on Restate: there is no in-process fallback. Restate only ever sees anonymous IDs.

The server keeps no state between requests. Phones poll their participant view (`/api/r/<slug>/participant`), and
the projector and the admin poll the full view (`/api/r/<slug>/view`) every few seconds; both carry the raffle's
`version` from the database. On Cloudflare Workers ([`docs/workers-plan.md`](docs/workers-plan.md)) a live channel
per raffle pushes the participant view instead.

```
app ──ingress──▶ Restate ──▶ restate-rafi (Raffle) ──▶ database ◀── app ◀── phones, projector (poll)
```

## Layout

```
crates/core                   rafi-core: draw algorithm, drand math, state machine, verifier (pure, tested, vectors)
crates/drand                  rafi-drand: Rafi's drand settings (quicknet, or the mock test chain) on the drand crate
crates/store                  rafi-store: SQLite schema, connection options, the shared draw state write
crates/restate-rafi           restate-rafi: the `Raffle` virtual object (library)
crates/restate-rafi-endpoint  restate-rafi: the binary serving it (config, logging, Dockerfile)
crates/live                   rafi-live: what a phone sees (participant view, card, proof-of-work)
crates/channel                rafi-channel: the participant view from the database; the live channel (Workers)
crates/attendee               rafi-attendee: the attendee page's WebAssembly, and its JavaScript fallback
crates/app                    rafi: Dioxus 0.7 web app (admin, projector, verify) + axum server (attendee pages, JSON APIs)
                              src/server: the Restate client, attendee SSR pages, proof-of-work, retention
migrations                    SQLite schema
tools                         verify.py: independent verifier (Python standard library only)
testdata                      shared test vectors
compose.yaml                  a local Restate server for development
deploy                        production: docker compose (Caddy, the app, restate-rafi, Restate)
```

## Development

Requirements: Rust stable with `wasm32-unknown-unknown`, and `dx` **0.7.10** (it must match the dioxus version):

```sh
cargo install dioxus-cli@0.7.10 --locked
```

The UI uses **Tailwind CSS v4 + daisyUI 5** (vendored as single-file plugins in `crates/app/vendor/`, no npm),
with custom light (`rafi`) and dark (`rafi-dark`) themes defined in [`crates/app/tailwind.css`](crates/app/tailwind.css).
`dx` compiles it automatically; before a plain `cargo build`/`clippy` of the web app on a fresh checkout run `./scripts/tailwind.sh`.
Organizer forms use [dioform](https://crates.io/crates/dioform) (typed fields, validation, managed async submit)
rendered with components from the [dioxus-daisyui-components](https://github.com/sagikazarmark/dioxus-daisyui-components)
registry (fields, select, switch, buttons, alert dialogs, dropdown menu, tabs, toasts), installed into
`crates/app/src/components/` with `dx components add <name> --path <registry checkout>`. Treat that directory as
vendored: update it by re-installing (`--force`), not by editing.

The app is two builds of one crate (`crates/app`, package `rafi`): the **web app** (admin, projector and
verification page; a Dioxus single-page app built by `dx`, `--features web`) and the **server** (attendee pages,
the public and admin JSON APIs, and the web app's files; plain `cargo`, `--features server`). The web app calls
the server's JSON routes (`POST /api/admin/<function>`) on the same origin.

The attendee page's WebAssembly and its fallback are built into `crates/attendee/dist`, which the server serves at
`/attendee/` (`RAFI_ATTENDEE_DIR`). Without it the attendee page still renders, but phones can't join or follow the
draw. Build it once, and again when `crates/attendee` or `crates/live` change (it needs `wasm-bindgen-cli` at the
version in `Cargo.lock`, and `wasm-opt` from binaryen):

```sh
cargo install wasm-bindgen-cli --version 0.2.129 --locked
./scripts/attendee.sh
```

Draws need Restate (see below). For the admin and the pages alone, run without Clerk, in two terminals:

```sh
# the server; dx proxies /api/, /j/, /rules/, /privacy/ and /attendee/ to it (crates/app/Dioxus.toml)
RAFI_DEV_AUTH=1 RAFI_PUBLIC_URL=http://localhost:8787 PORT=8788 cargo run -p rafi --features server

# the web app, with hot reload
cd crates/app && dx serve --port 8787
```

Then open <http://localhost:8787/admin>. To work offline, add `RAFI_DRAND_MOCK=1` and `--features mock-drand`
to the server, and the same to the draw engine: draws then use a test chain with a public key (the `drand` crate's
`TestChain`) and are *not* verifiable.
To test with phones on your network, set `RAFI_PUBLIC_URL` to your machine's LAN address and run `dx serve --addr 0.0.0.0`.

The server alone serves everything once the web app is bundled (this is what the container image runs):

```sh
(cd crates/app && dx bundle --platform web --release)   # -> target/dx/rafi/release/web/public
./scripts/attendee.sh                                    # -> crates/attendee/dist
RAFI_DEV_AUTH=1 cargo run --release -p rafi --features server   # http://localhost:8080
```

It listens on `IP`:`PORT` (default `127.0.0.1:8080`), serves the attendee scripts at `/attendee/` from
`RAFI_ATTENDEE_DIR` (default `crates/attendee/dist`), and the web app from `RAFI_PUBLIC_DIR`
(default `target/dx/rafi/release/web/public`), both relative to the working directory, falling back to the web
app's `index.html` for its own routes.

With Restate: Restate runs in Docker ([`compose.yaml`](compose.yaml)), the app and `restate-rafi` on your
machine, both on the same database file:

```sh
export RAFI_DATABASE_URL="sqlite://$PWD/rafi.db"

docker compose up -d       # Restate: ingress on :8080, admin API and UI on :9070
                           # (RESTATE_INGRESS_PORT / RESTATE_ADMIN_PORT to move them)

# the server (it creates the database) and the web app
RAFI_DEV_AUTH=1 RAFI_PUBLIC_URL=http://localhost:8787 RESTATE_INGRESS_URL=http://localhost:8080 \
  PORT=8788 cargo run -p rafi --features server
(cd crates/app && dx serve --port 8787)

# the draw engine, in another terminal (offline: add --features mock-drand and RESTATE_RAFI_DRAND__MOCK=true,
# matching the app's RAFI_DRAND_MOCK; the engine refuses raffles committed to another chain)
RESTATE_RAFI_DATABASE__URL="$RAFI_DATABASE_URL" cargo run -p restate-rafi-endpoint

# once, and again whenever the Raffle object's handlers change
curl -X POST localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://host.docker.internal:9080", "force": true}'
```

With Clerk, export `CLERK_PUBLISHABLE_KEY` (when building the web app) and `CLERK_SECRET_KEY` (for the server),
and leave `RAFI_DEV_AUTH` unset.

### Tests

```sh
export RESTATE_SERVER_BIN=$(./scripts/restate-server.sh)   # a pinned restate-server for the end-to-end tests
cargo test --workspace --exclude rafi -- --include-ignored  # core, drand(-beacon, incl. live relays), store, the Raffle object end to end
cargo test -p rafi --features server -- --include-ignored   # app (with an in-process stand-in for Restate), and app + restate-rafi + restate-server
python3 tools/verify.py --vectors testdata/draw-vectors.json
```

Without `RESTATE_SERVER_BIN` the end-to-end tests skip (and fail on CI).

## Configuration

The app:

| Variable | Default | |
|---|---|---|
| `IP`, `PORT` | `127.0.0.1`, `8080` | Listen address (the image sets `0.0.0.0`) |
| `RAFI_PUBLIC_DIR` | `target/dx/rafi/release/web/public` | The bundled web app (the image sets `/usr/local/app/public`) |
| `RAFI_ATTENDEE_DIR` | `crates/attendee/dist` | The attendee page's WebAssembly and fallback, from `./scripts/attendee.sh` (the image sets `/usr/local/app/attendee`) |
| `RAFI_PUBLIC_URL` | `http://localhost:8080` | External base URL (QR codes, links) |
| `RAFI_DATABASE_URL` | `sqlite://rafi.db` | SQLite database (shared with `restate-rafi`) |
| `RAFI_DEV_AUTH` | off | **Development only:** no login, every admin is `dev` |
| `CLERK_SECRET_KEY` | | Clerk backend key (`CLERK_PUBLISHABLE_KEY` at build time) |
| `RESTATE_INGRESS_URL`, `RESTATE_API_KEY` | | The draw engine's Restate ingress (Restate Cloud: an API key with ingress access). Without it, draws can't start |
| `RAFI_DRAND_MOCK` | off | New raffles commit to a public test chain (offline development; needs `--features mock-drand`, and the draw engine on the same chain) |
| `RAFI_ALTCHA_KEY` | random per start | HMAC key for the proof-of-work challenge |
| `RAFI_ALTCHA_MAX` | `30000` | Proof-of-work difficulty |
| `RAFI_MAX_ENTRANTS` | `2000` | The most entrants a raffle takes |
| `RAFI_RETENTION` | off | Erase personal data after the event in the hourly retention run (otherwise erase by hand in the admin). The run always ends live updates a day after a raffle finishes and forgets expired proof-of-work challenges |
| `RAFI_RETENTION_DAYS` / `RAFI_RETENTION_WINNERS_DAYS` | `30` / `90` | Erase personal data after the event |

`restate-rafi` (draw settings, database, identity keys):
see [`crates/restate-rafi-endpoint`](crates/restate-rafi-endpoint/README.md).

## Deployment

On a VM with Docker and a domain pointing at it (production: a Scaleway DEV1-M in Warsaw, `pl-waw-1`):

```sh
git clone https://github.com/sagikazarmark/rafi.orig rafi && cd rafi/deploy
cp .env.example .env && $EDITOR .env   # domain, Clerk keys, proof-of-work key, Restate Cloud
docker compose pull && docker compose up -d
restate deployments register https://$RAFI_DOMAIN:9080   # once, and when the Raffle handlers change
```

[`deploy/compose.yaml`](deploy/compose.yaml) runs Caddy (automatic HTTPS), the app and `restate-rafi` from the GHCR
images (`docker compose up -d --build` builds them from the checkout instead). The app and `restate-rafi` share the
SQLite database on a volume. Restate Cloud reaches `restate-rafi` at `https://$RAFI_DOMAIN:9080`, where Caddy
terminates TLS; `restate-rafi` only accepts requests signed with the environment's key (`RESTATE_IDENTITY_KEYS`, the
environment's signing public key). The app calls the Restate Cloud ingress with an API key that has ingress access
(`RESTATE_API_KEY`). Open ports: 22, 80, 443 (TCP and UDP) and 9080.

Without Restate Cloud, `docker compose --profile self-hosted up -d` also runs a Restate server on the VM and registers
`restate-rafi` with it (see the comments in `compose.yaml`).

To update: build and push new images (CI does it on `main`; by hand, `docker buildx build --platform linux/amd64
--build-arg CLERK_PUBLISHABLE_KEY -t ghcr.io/sagikazarmark/rafi:latest --push .` and the same for
`crates/restate-rafi-endpoint/Dockerfile`), then `docker compose pull && docker compose up -d` on the VM. Avoid it
during a live raffle (`/api/status` shows `live_raffles`).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
