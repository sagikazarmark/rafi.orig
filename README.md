# Rafi

**Live, verifiable raffles for meetups.**

Attendees scan a QR code and join on their phone with a display name and
email: no app, no account. The organizer draws each prize live. Every draw
commits to a future [drand](https://drand.love) randomness round before it
exists, so nobody (the organizer included) can predict or steer the result,
and anyone can re-run every draw afterwards.

- **Projector screen** (`/s/<slug>`): intro, prizes, QR code, live entrant count, countdown, winner reveal.
- **Attendee page** (`/j/<CODE>`): server-rendered, no WASM, works on bad venue wifi. It shows "you won" with a private claim code.
- **Organizer admin** (`/admin`): raffles, prizes, people without a smartphone, draw console (Draw / Handed over / Not here / Declined), CSV export.
- **Verification** (`/v/<slug>`): re-runs every draw in the browser; [`tools/verify.py`](tools/verify.py) does the same independently.

See [`docs/PLAN.md`](docs/PLAN.md) for the design and the reasoning behind it.

## How the draw works

1. **Start the draw** closes registration, freezes the entrant list (random IDs only) and publishes
   `list_hash = SHA256(commit document)`.
2. For each prize unit the organizer presses **Draw**. The draw engine commits to drand
   quicknet round `R` (about 6 s in the future), announces it on screen, waits durably until it is published,
   fetches the beacon from two relays and checks `randomness = SHA256(signature)`.
3. `seed = SHA256("rafi-draw-v1" ‖ randomness ‖ list_hash ‖ k)`. The winner is `pool[uniform(|pool|)]`, using
   SHA-256 counter-mode rejection sampling, so there is no modulo bias.
4. One win per person. "Not here" or "Declined" removes that person and the prize is drawn again with a new round.
The draw state machine lives in [`crates/core`](crates/core) (pure Rust, no IO). In production it runs in
**`restate-rafi`**, a separate service hosting the **Restate virtual object** `Raffle` (keyed by raffle ID):
the app calls it through the Restate ingress, and it writes every new draw state straight into the app's
SQLite database, where the app picks it up and pushes it to the screens. For development, and as an emergency
fallback, the same state machine runs in-process. Restate only ever sees anonymous IDs.

```
app ──ingress──▶ Restate ──▶ restate-rafi (Raffle) ──▶ SQLite ◀── app (watches, pushes to screens)
```

## Layout

```
crates/core                   rafi-core: draw algorithm, drand math, state machine, verifier (pure, tested, vectors)
crates/drand                  rafi-drand: drand quicknet client (several relays, offline mock)
crates/store                  rafi-store: SQLite schema, connection options, the shared draw state write
crates/restate-rafi           restate-rafi: the `Raffle` virtual object (library)
crates/restate-rafi-endpoint  restate-rafi: the binary serving it (config, logging, Dockerfile)
crates/app                    rafi: Dioxus 0.7 fullstack app (admin, projector, verify) + axum server
                              src/server: SQLite (sqlx), SSE hub, draw engines (Restate client + local),
                                          attendee SSR pages, proof-of-work, retention
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
`dx` compiles it automatically; before a plain `cargo build`/`cargo test` on a fresh checkout run `./scripts/tailwind.sh`.
Organizer forms use [dioform](https://crates.io/crates/dioform) (typed fields, validation, managed async submit)
rendered with components from the [dioxus-daisyui-components](https://github.com/sagikazarmark/dioxus-daisyui-components)
registry (fields, select, switch, buttons, alert dialogs, dropdown menu, tabs, toasts), installed into
`crates/app/src/components/` with `dx components add <name> --path <registry checkout>`. Treat that directory as
vendored: update it by re-installing (`--force`), not by editing.

Run locally without Clerk or Restate (in-process draw engine, real drand):

```sh
cd crates/app
RAFI_DEV_AUTH=1 RAFI_PUBLIC_URL=http://localhost:8787 dx serve --port 8787
```

Then open <http://localhost:8787/admin>. Add `RAFI_DRAND_MOCK=1` to work offline (draws are then *not* verifiable).
To test with phones on your network, set `RAFI_PUBLIC_URL` to your machine's LAN address and run `dx serve --addr 0.0.0.0`.

With Restate: Restate runs in Docker ([`compose.yaml`](compose.yaml)), the app and `restate-rafi` on your
machine, both on the same database file:

```sh
export RAFI_DATABASE_URL="sqlite://$PWD/rafi.db"

docker compose up -d       # Restate: ingress on :8080, admin API and UI on :9070
                           # (RESTATE_INGRESS_PORT / RESTATE_ADMIN_PORT to move them)

# the app (it creates the database); new raffles are drawn by Restate
(cd crates/app && RAFI_DEV_AUTH=1 RAFI_PUBLIC_URL=http://localhost:8787 \
  RESTATE_INGRESS_URL=http://localhost:8080 dx serve --port 8787)

# the draw engine, in another terminal
RESTATE_RAFI_DATABASE__URL="$RAFI_DATABASE_URL" cargo run -p restate-rafi-endpoint

# once, and again whenever the Raffle object's handlers change
curl -X POST localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://host.docker.internal:9080", "force": true}'
```

With Clerk, export `CLERK_PUBLISHABLE_KEY` (at build time) and `CLERK_SECRET_KEY`, and leave `RAFI_DEV_AUTH` unset.

### Tests

```sh
./scripts/tailwind.sh                                # once, so the stylesheet asset exists
export RESTATE_SERVER_BIN=$(./scripts/restate-server.sh)   # a pinned restate-server for the end-to-end tests
cargo test --workspace --exclude rafi -- --include-ignored  # core, drand, store, the Raffle object end to end
cargo test -p rafi --features server -- --include-ignored   # app, and app + restate-rafi + restate-server
python3 tools/verify.py --vectors testdata/draw-vectors.json
```

Without `RESTATE_SERVER_BIN` the end-to-end tests skip (and fail on CI).

## Configuration

The app:

| Variable | Default | |
|---|---|---|
| `RAFI_PUBLIC_URL` | `http://localhost:8080` | External base URL (QR codes, links) |
| `RAFI_DATABASE_URL` | `sqlite://rafi.db` | SQLite database (shared with `restate-rafi`) |
| `RAFI_DEV_AUTH` | off | **Development only:** no login, every admin is `dev` |
| `CLERK_SECRET_KEY` | | Clerk backend key (`CLERK_PUBLISHABLE_KEY` at build time) |
| `RAFI_DRAW_ENGINE` | `local`, or `restate` if `RESTATE_INGRESS_URL` is set | Draw engine for new raffles |
| `RESTATE_INGRESS_URL`, `RESTATE_API_KEY` | | Restate ingress (Restate Cloud: an API key with ingress access) |
| `RAFI_DRAND_RELAYS` | `https://api.drand.sh,https://drand.cloudflare.com` | Local engine: all answering relays must agree |
| `RAFI_DRAW_LEAD_SECS` | `6` | Local engine: seconds until the committed round (≥ 3) |
| `RAFI_DRAND_TIMEOUT_SECS` | `60` | Local engine: give up and log a failed draw after this |
| `RAFI_DRAND_MOCK` | off | Local engine: fake beacons for offline development |
| `RAFI_ALTCHA_KEY` | random per start | HMAC key for the proof-of-work challenge |
| `RAFI_ALTCHA_MAX` | `30000` | Proof-of-work difficulty |
| `RAFI_RETENTION` | off | Run the hourly job that erases personal data after the event (otherwise erase by hand in the admin) |
| `RAFI_RETENTION_DAYS` / `RAFI_RETENTION_WINNERS_DAYS` | `30` / `90` | Erase personal data after the event |

`restate-rafi` (draw settings, database, identity keys):
see [`crates/restate-rafi-endpoint`](crates/restate-rafi-endpoint/README.md).

## Deployment

On a VM with Docker and a domain pointing at it:

```sh
git clone https://github.com/sagikazarmark/rafi && cd rafi/deploy
cp .env.example .env && $EDITOR .env   # domain, Clerk keys, proof-of-work key
docker compose up -d --build
```

[`deploy/compose.yaml`](deploy/compose.yaml) builds both images from the checkout and runs Caddy (automatic HTTPS),
the app, `restate-rafi` and a Restate server; a one-shot container registers `restate-rafi` with Restate on every
`up`. The app and `restate-rafi` share the SQLite database on a volume. Only ports 80 and 443 are published; the
Restate UI is on the VM's loopback (`ssh -L 9070:localhost:9070 <vm>`, then <http://localhost:9070>).

To update: `git pull && docker compose up -d --build`. Avoid it during a live raffle (`/api/status` shows
`live_raffles`).

CI also builds `ghcr.io/sagikazarmark/rafi` and `ghcr.io/sagikazarmark/restate-rafi`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
