# Self-hosting

Running Rafi natively: the `rafi` app and the `rafi-restate` raffle engine, sharing one database, with Restate in
front of the engine. There are no container images yet, so this guide builds from source. (On Cloudflare Workers,
nothing is built: see [Cloudflare Workers](cloudflare.md).)

## 1. Build

You need the toolchain from [Getting started](../development/getting-started.md#requirements): Rust stable with
`wasm32-unknown-unknown`, `dx` 0.7.10, `wasm-bindgen-cli` at the version in `Cargo.lock`, `wasm-opt` and Node (for
daisyUI).

```sh
# the web app (admin, projector, verification page); nothing of an installation's is in it
(cd apps/web && npm ci && dx bundle --platform web --release)
#   -> target/dx/rafi-web/release/web/public

# the participant page's WebAssembly and JavaScript fallback; before the binaries, which compile its loader in
just apps/participant/ build
#   -> apps/participant/dist/public

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

The app reaches the engine through Restate's **ingress**. Restate reaches `rafi-restate` on its endpoint
(`127.0.0.1:9080` by default; set `endpoint.listener` if Restate runs on another machine, on a private network).

- **Self-hosted:** run `restate-server` (the tests use 1.8.0, the devenv shell's: see `devenv.lock`'s
  `nix-restate`). By
  default its ingress is on `:8080`, its admin API on `:9070`, and its data in `./restate-data`. If it shares a
  machine with `rafi`, move one of them off `:8080`.
- **Restate Cloud:** create an environment, an API key with ingress access for the app (`RAFI_ENGINE_API_KEY`), and set
  the engine's `endpoint.identity_keys` to the environment's public key, so it only accepts requests signed by your
  Restate.

## 4. Run the engine

```sh
RAFI_RESTATE_DATABASE__URL=… \
RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS=publickeyv1_… \
  ./target/release/rafi-restate             # or --config rafi-restate.toml
```

Every setting is in the [configuration reference](../reference/configuration.md#the-raffle-engine-rafi-restate).

Then **register it with Restate**, without `force`:

```sh
# self-hosted Restate: its admin API
curl -X POST http://localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://<engine host>:9080"}'
```

On Restate Cloud, register the engine's public URL as a deployment in the Cloud console or CLI. A release that
changes the engine's handlers needs a **new URI** (Restate keeps what it already knows about a registered one, and
`force` is never the answer); see [Upgrades](upgrades.md#registering-the-raffle-engine).

## 5. Legal documents and the operator

Every raffle's privacy notice names **you**, the operator of this installation, as the processor, and links the
installation's own privacy notice. Organizers accept your terms, with the data processing agreement, by using the
service; the admin links them. The software can't write these for you:

1. Put `terms.md` and `privacy.md` in a directory (`RAFI_DOCS_DIR`). Start from
   [`deploy/cloudflare/legal/`](../../deploy/cloudflare/legal/), which are engineering drafts marked as templates,
   fill in the bracketed parts and have a lawyer review them.
2. Give each a version (`RAFI_TERMS_VERSION`, `RAFI_PRIVACY_VERSION`, e.g. `2026-10`). Changing a document's text
   means a new version, shown with the document. Nothing records who read which version: a change to the data
   processing agreement has to be notified to organizers directly, not only published.
3. Name yourself: `RAFI_OPERATOR_NAME`, `RAFI_OPERATOR_EMAIL`.

Without these, the app refuses to start (except under `RAFI_DEV_AUTH`, with placeholders marked as such).

The retention settings are yours too, within limits: `RAFI_RETENTION_DAYS` (7: days after the event date before
non-winners are erased, for organizers who choose nothing), `RAFI_RETENTION_MAX_DAYS` (30: the most an organizer may
choose) and `RAFI_RETENTION_WINNERS_DAYS` (90: how long winners are held past that unless their prize is marked
delivered). `RAFI_MIN_AGE` (16) is what entrants confirm by entering. See [Privacy and
abuse](../explanation/privacy-and-abuse.md).

## 6. Run the app

```sh
RAFI_PUBLIC_URL=https://raffle.example.org \
RAFI_DATABASE_URL=… \
RAFI_ENGINE_INGRESS_URL=http://localhost:8080 \
CLERK_SECRET_KEY=sk_live_… CLERK_PUBLISHABLE_KEY=pk_live_… \
RAFI_ALTCHA_KEY=$(openssl rand -hex 32) \
RAFI_DOCS_DIR=/etc/rafi RAFI_TERMS_VERSION=2026-10 RAFI_PRIVACY_VERSION=2026-10 \
RAFI_OPERATOR_NAME="Example Meetups e.V." RAFI_OPERATOR_EMAIL=privacy@example.org \
IP=127.0.0.1 PORT=8081 \
  ./target/release/rafi
```

- Run it from the repository root, or set `RAFI_PUBLIC_DIR` and `RAFI_PARTICIPANT_DIR` to where you put the web
  bundle and the participant build (`dist/public`, as it is). The build's module and glue are named by their
  hash and served `Cache-Control: immutable`, so phones keep them and a deploy links new names; the fallback is
  revalidated (`no-cache`, with an `ETag`). They are served precompressed (`.br` and `.gz`) when the browser
  accepts it: no compressing proxy is needed in front of them. The binary was compiled against one participant
  build, and refuses to start on another: ship the two together.
- Set `RAFI_ALTCHA_KEY` to a fixed secret. Otherwise it is random per start, and a restart during registration
  invalidates the challenges phones are solving. Don't rotate it during an event.
- Keep `RAFI_ENGINE_TIMEOUT_SECS` (90) above the engine's `lead_secs + timeout_secs` (66 by default).
- At startup the app warns if dev auth is on, the chain is the mock one, the ingress URL is missing, or the web app
  isn't where it expects; it refuses to start if the participant build isn't.

The app runs the hourly housekeeping itself: it erases personal data on the dates entrants were shown, forgets
expired proof-of-work challenges and ends live updates a day after a raffle finishes. If no run has succeeded for
two hours (counted from the first raffle), every organizer's admin shows a warning.

## 7. Reverse proxy

Put a TLS-terminating proxy (Caddy, nginx, …) in front of `rafi` and pass every path through. `rafi` serves the
APIs, the participant pages and the static files itself ([routes](../reference/routes.md)). If the proxy talks plain
HTTP to `rafi` but `RAFI_PUBLIC_URL` is `https://`, the participant cookie is already `Secure`. Set
`RAFI_COOKIE_SECURE` only to override that.

Don't rate-limit by IP aggressively: a whole venue shares one address. Rafi limits joins per raffle and per email.

The app logs no personal data; the proxy's access log is where client IPs end up, and the join URL carries the
join code. Keep that log short-lived, don't log request bodies or cookies (the entry cookie is sent to `/j/*`
only), and say what you keep in your privacy notice. Have the proxy send `Strict-Transport-Security`; the app
sets the other security headers itself.

Expose none of these publicly: `rafi-restate`'s endpoint (`:9080`; whoever reaches it supplies the raffle's state,
which is why it listens on the loopback address by default and refuses to start without `identity_keys` unless
told the network is trusted), Restate's admin API (`:9070`), or Restate's **ingress** (`:8080`): the ingress has
no authentication of its own, and anyone reaching it can send any raffle's `draw`, `decide` or `finish`, bypassing
the app's ownership checks. Only `rafi` talks to the ingress.

## 8. Check

- `GET /healthz` answers `ok`, `/terms` and `/privacy` show your documents with their versions.
- Create a raffle with **Test mode** on, add fake entrants, freeze, and draw one item. The verification page should
  show **✅ All 1 draws check out**, under a **🧪 TEST MODE** note.
- Erase it, or delete it.

## Reviewing held winner data

Winners' names and emails are held past the erasure date until the organizer marks the prize delivered, at most
for the winner hold. Nothing in the software keeps them longer, but an organizer who never marks anything delivered
keeps every winner for the whole hold. Look at what is held now and then, and ask organizers to mark prizes
delivered (or do it for them, with their agreement, in the People tab):

```sql
SELECT r.id, r.title, r.owner_id, r.erase_at, COUNT(*) AS held
FROM entrants e JOIN raffles r ON r.id = e.raffle_id
WHERE e.erased = 0 AND e.delivered_at IS NULL
  AND e.id IN (SELECT entrant_id FROM draws WHERE raffle_id = r.id AND outcome = 'claimed')
GROUP BY r.id, r.title, r.owner_id, r.erase_at
ORDER BY r.erase_at;
```

Note each review and what you did about it: that record is part of showing the hold is justified.

## Backups

Back up the database (the SQLite file, or Postgres as usual). Restate's own state only matters while a raffle is
being drawn: the engine forgets a raffle when it finishes, and the database is the record.

A backup holds personal data that may have been erased since. After a restore:

1. If you can, carry over from the live database what the restore would undo: the `erasures` table (entrant IDs
   and times only) and, from `raffles`, the `purged_at` and `organizer_left_at` of raffles that were purged or
   whose organizer left: `sqlite3 live.db ".dump erasures"` or `pg_dump -t erasures`, plus
   `SELECT id, purged_at, organizer_left_at FROM raffles WHERE purged_at IS NOT NULL`, then import and `UPDATE`.
2. Run `rafi reapply-erasures` with the app's environment. It erases again every entrant the log names, purges
   again every raffle that was purged, blanks again every organizer who removed their data, and runs housekeeping
   once, so whatever came due in the meantime is erased too (the erasure dates are in the raffles themselves, so
   the clock needs no log). Then start the app.

Erasures the organizer made before the date, purges and organizers who left that aren't in what you restored, are
the only things the clock won't redo: that is what step 1 is for.
