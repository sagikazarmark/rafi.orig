# Self-hosting

Running Rafi natively: the `rafi` app and the `rafi-restate` raffle engine, sharing one database, with Restate in
front of the engine. There are no container images yet, so this guide builds from source.

## 1. Build

You need the toolchain from [Getting started](../development/getting-started.md#requirements): Rust stable with
`wasm32-unknown-unknown`, `dx` 0.7.10, `wasm-bindgen-cli` at the version in `Cargo.lock`, `wasm-opt` and Node (for
daisyUI).

```sh
# the web app (admin, projector, verification page); CLERK_PUBLISHABLE_KEY is compiled in
(cd apps/web && npm ci && CLERK_PUBLISHABLE_KEY=pk_live_… dx bundle --platform web --release)
#   -> target/dx/rafi-web/release/web/public

# the participant page's WebAssembly and JavaScript fallback
./scripts/participant.sh
#   -> apps/participant/dist

# the two binaries
cargo build --release --locked -p rafi -p rafi-restate
#   -> target/release/rafi, target/release/rafi-restate
```

Never ship a build with `--features mock-drand`.

## 2. Database

Pick one, and point both processes at it.

- **SQLite.** Both processes must open the same file, so they run on the same machine or share a volume. The app
  creates the file.

  ```sh
  RAFI_DATABASE_URL=sqlite:///var/lib/rafi/rafi.db              # the app
  RAFI_RESTATE_DATABASE__URL=sqlite:///var/lib/rafi/rafi.db     # the engine
  ```

- **PostgreSQL.** Create the database first. The app applies the migrations (`migrations/postgres`) on start.
  Until it has, the engine's writes fail and Restate retries them. Transactions run serializable. Connections use
  TLS when the server offers it. Add `?sslmode=verify-full` to require it (against the Mozilla root certificates,
  or `sslrootcert=<file>`).

  ```sh
  RAFI_DATABASE_URL=postgres://rafi:…@db.internal/rafi
  RAFI_RESTATE_DATABASE__URL=postgres://rafi:…@db.internal/rafi
  ```

## 3. Restate

The app reaches the engine through Restate's **ingress**. Restate reaches `rafi-restate` on its endpoint (`:9080`).

- **Self-hosted:** run `restate-server` (the tests pin 1.7.13; `./scripts/restate-server.sh` downloads it). By
  default its ingress is on `:8080`, its admin API on `:9070`, and its data in `./restate-data`. If it shares a
  machine with `rafi`, move one of them off `:8080`.
- **Restate Cloud:** create an environment, an API key with ingress access for the app (`RESTATE_API_KEY`), and set
  the engine's `endpoint.identity_keys` to the environment's public key, so it only accepts requests signed by your
  Restate.

## 4. Run the engine

```sh
RAFI_RESTATE_DATABASE__URL=… \
RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS=publickeyv1_… \
  ./target/release/rafi-restate             # or --config rafi-restate.toml
```

Every setting is in the [configuration reference](../reference/configuration.md#the-raffle-engine-rafi-restate).

Then **register it with Restate**, once and again whenever its handlers change (see
[Upgrades](upgrades.md#re-registering-the-raffle-engine)):

```sh
# self-hosted Restate: its admin API
curl -X POST http://localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://<engine host>:9080", "force": true}'
```

On Restate Cloud, register the engine's public URL as a deployment in the Cloud console or CLI.

## 5. Run the app

```sh
RAFI_PUBLIC_URL=https://raffle.example.org \
RAFI_DATABASE_URL=… \
RESTATE_INGRESS_URL=http://localhost:8080 \
CLERK_SECRET_KEY=sk_live_… \
RAFI_ALTCHA_KEY=$(openssl rand -hex 32) \
IP=127.0.0.1 PORT=8081 \
  ./target/release/rafi
```

- Run it from the repository root, or set `RAFI_PUBLIC_DIR` and `RAFI_PARTICIPANT_DIR` to where you put the web
  bundle and the participant scripts.
- Set `RAFI_ALTCHA_KEY` to a fixed secret. Otherwise it is random per start, and a restart during registration
  invalidates the challenges phones are solving. Don't rotate it during an event.
- Keep `RESTATE_TIMEOUT_SECS` (90) above the engine's `lead_secs + timeout_secs` (66 by default).
- At startup the app warns if dev auth is on, the chain is the mock one, the ingress URL is missing, or the web app
  or participant scripts aren't where it expects.

The app runs the hourly housekeeping itself: it forgets expired proof-of-work challenges and ends live updates a day
after a raffle finishes.

## 6. Reverse proxy

Put a TLS-terminating proxy (Caddy, nginx, …) in front of `rafi` and pass every path through. `rafi` serves the
APIs, the participant pages and the static files itself ([routes](../reference/routes.md)). If the proxy talks plain
HTTP to `rafi` but `RAFI_PUBLIC_URL` is `https://`, the participant cookie is already `Secure`. Set
`RAFI_COOKIE_SECURE` only to override that.

Don't rate-limit by IP aggressively: a whole venue shares one address. Rafi limits joins per raffle and per email.

Expose neither `rafi-restate` (`:9080`) nor Restate's admin API (`:9070`) publicly.

## 7. Check

- `GET /healthz` answers `ok`.
- Create a raffle with **Test mode** on, add fake entrants, freeze, and draw one item. The verification page should
  show **✅ All 1 draws check out**, under a **🧪 TEST MODE** note.
- Erase it, or delete it.

## Backups

Back up the database (the SQLite file, or Postgres as usual). Restate's own state only matters while a raffle is
being drawn: the engine forgets a raffle when it finishes, and the database is the record.

Erasures are logged in the `erasures` table (entrant ID and time only). If you restore a backup taken before an
erasure, erase those entrants again. There is no tool for this yet.
