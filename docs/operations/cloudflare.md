# Cloudflare Workers

Rafi on Cloudflare: one Worker and one D1 database, with Restate Cloud running the raffle engine. This is the
planned production deployment. It is verified locally under `wrangler dev`, but hasn't been deployed yet.

Cloudflare is one deployment, not an assumption of the code. The app and the engine are written against small
interfaces (the engine's `Store` and `Observer`, the live `Notifier`, the app's `RateLimit`), and `rafi-cloudflare`
implements them over Workers bindings.

The Worker, `rafi` (`apps/worker`), hosts:

- the app's routes and the static files (Workers Assets);
- the raffle engine (`Raffle`, `EngineInfo`) under `/restate/*`, called by Restate Cloud over HTTPS with signed
  requests. It writes to the same D1 and nudges `RaffleRoom` after every change;
- the `RaffleRoom` Durable Object (one live channel per raffle), the join rate limiters and the hourly Cron.

`wrangler.toml` has a production top level, plus `[env.dev]` for local development.

## You'll need

- a Cloudflare account (Workers, D1, Durable Objects; the paid plan may be needed, depending on BLS CPU time)
- a Restate Cloud environment
- a Clerk application
- the toolchain in `devenv.nix`: `worker-build`, `wrangler`, `wasm-bindgen-cli` matching `Cargo.lock`, `dx`,
  `wasm-opt`, Node

## First deploy

Nothing of your installation's goes into a tracked file, so every upgrade is a plain `git pull`:
[`apps/worker/wrangler.toml`](../../apps/worker/wrangler.toml) is the Worker's *shape*; your database ID goes into
a generated copy of it, your legal documents live where you keep them, and everything else is a secret.

### 1. Database

```sh
wrangler d1 create rafi --jurisdiction eu                 # note the database_id it prints
./scripts/worker-config.sh prod <database_id>             # -> apps/worker/wrangler.prod.toml (untracked)
(cd apps/worker && wrangler d1 migrations apply rafi --remote -c wrangler.prod.toml)
```

`wrangler.prod.toml` is `wrangler.toml` with your ID in it. Never edit it: run the script again after every
upgrade, so its shape is the release's and the ID stays yours. Every `wrangler` command below takes
`-c wrangler.prod.toml`.

### 2. The legal documents and the operator

The Worker bundles the installation's documents at build time: `terms.md` (with the data processing agreement,
which organizers accept by using the service) and `privacy.md` (the operator's own notice, naming the
subprocessors: Cloudflare, Restate Cloud, Clerk, drand). Start from
[`apps/worker/legal/`](../../apps/worker/legal/), which ship as engineering drafts marked as templates: copy them
to a directory of yours (outside the tree, so an upgrade doesn't touch them), fill in the bracketed parts, have a
lawyer review them, and point the build at them with `RAFI_LEGAL_DIR` (below). A document that still carries the
template marker makes the Worker refuse every request. Their versions (`RAFI_TERMS_VERSION`,
`RAFI_PRIVACY_VERSION`, e.g. `2026-10`) and the operator's name and email (`RAFI_OPERATOR_NAME`,
`RAFI_OPERATOR_EMAIL`) are secrets, next. A changed document is a new version, shown with it; a change to the
processing agreement is notified to organizers directly, since nothing records who read which version. The
retention settings (`RAFI_RETENTION_DAYS` and the others, see the
[configuration](../reference/configuration.md)) have defaults; `RAFI_MIN_AGE` too; set them as secrets if you
change them.

### 3. The Worker

The installation's values are secrets. `wrangler.toml` lists them under `[secrets] required`, and a deploy
refuses to go out while one is missing. One JSON file, kept out of the repository:

```json
{
  "RAFI_PUBLIC_URL": "https://raffle.example.org",
  "RAFI_ENGINE_INGRESS_URL": "https://<env>.env.eu.restate.cloud:8080",
  "RAFI_ENGINE_API_KEY": "…",
  "RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS": "publickeyv1_…",
  "RAFI_ALTCHA_KEY": "<openssl rand -hex 32>",
  "CLERK_SECRET_KEY": "sk_live_…",
  "RAFI_OPERATOR_NAME": "Example Meetups e.V.",
  "RAFI_OPERATOR_EMAIL": "privacy@example.org",
  "RAFI_TERMS_VERSION": "2026-10",
  "RAFI_PRIVACY_VERSION": "2026-10"
}
```

Build the static files (web app and participant scripts) with Clerk's publishable key compiled in, build the
Worker with your documents, deploy, and set the secrets:

```sh
CLERK_PUBLISHABLE_KEY=pk_live_… ./scripts/app-assets.sh         # -> apps/worker/worker-assets
cd apps/worker
RAFI_LEGAL_DIR=/path/to/your/legal wrangler deploy -c wrangler.prod.toml
wrangler secret bulk ~/rafi-prod.secrets.json -c wrangler.prod.toml
```

The very first `deploy` creates the Worker and stops at the missing secrets (there is no Worker to hold them yet);
`secret bulk` then sets them, and the deploy is run again. From then on, a deploy that is missing one refuses.
Never rotate `RAFI_ALTCHA_KEY` during an event.

The Worker refuses every request without `RAFI_ALTCHA_KEY`, `RAFI_ENGINE_INGRESS_URL`, `CLERK_SECRET_KEY`,
`RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS`, the operator and the document versions. Release builds reject dev auth and
the mock chain.

Attach a custom domain. `workers_dev` and preview URLs are off on purpose, because previews would share
production's database and Durable Objects.

### 4. The raffle engine

In Restate Cloud, **register `https://<the Worker's hostname>/restate` as a deployment**, without `force`.
Restate appends `/discover` and `/invoke/…` to that URL; the Worker verifies each request's signature before
anything else happens to it. A release that changes the engine's handlers comes with a new path under `/restate/`
and says so; register that, and drain the old deployment (see
[Upgrades](upgrades.md#registering-the-raffle-engine)). Re-registering a URL Restate already knows changes
nothing, silently.

### 5. Check

Open `/healthz`, `/terms` and `/privacy`, then run a test-mode raffle end to end, as in
[Self-hosting](self-hosting.md#8-check).

## Data residency

- D1 is created with `--jurisdiction eu`.
- Durable Objects come from an EU-jurisdiction namespace (`RAFI_DO_JURISDICTION=eu`, fixed at creation).
- The automatic per-invocation logs are off, because they would record client IPs and cookies; the application's
  own lines (warnings and errors, with raffle IDs and error text) are kept.
- D1 Time Travel keeps erased data for up to 30 more days. Say so in your privacy notice (the template does). After
  restoring with Time Travel, re-apply the erasures. The hourly housekeeping erases again whatever is past its date
  on its own (the dates are in the raffles themselves). What it can't know is what organizers erased before the
  date, or who removed their data: for those, carry the live `erasures` table and the `organizer_left_at` values
  over if you can, and run the restore tool's statements against D1 (the same SQL `rafi reapply-erasures` runs
  natively, see [Self-hosting](self-hosting.md#backups)):

  ```sh
  wrangler d1 execute rafi --remote -c wrangler.prod.toml --command "
    -- Entrants the log names (if the log came along).
    UPDATE entrants SET display_name = '#' || substr(id, 1, 6), email_raw = NULL, email_norm = NULL,
      token_hash = NULL, claim_code = NULL, erased = 1
    WHERE erased = 0 AND EXISTS (SELECT 1 FROM erasures
      WHERE erasures.raffle_id = entrants.raffle_id AND erasures.entrant_id = entrants.id);
    -- Entrants of a purged raffle, or one whose organizer left: nobody in it is
    -- un-erased, whether or not the log came along. Logged, then erased.
    INSERT INTO erasures (raffle_id, entrant_id, at)
      SELECT raffle_id, id, unixepoch() FROM entrants
      WHERE erased = 0
        AND EXISTS (SELECT 1 FROM raffles r WHERE r.id = entrants.raffle_id
                      AND (r.purged_at IS NOT NULL OR r.organizer_left_at IS NOT NULL))
        AND NOT EXISTS (SELECT 1 FROM erasures e WHERE e.raffle_id = entrants.raffle_id AND e.entrant_id = entrants.id);
    UPDATE entrants SET display_name = '#' || substr(id, 1, 6), email_raw = NULL, email_norm = NULL,
      token_hash = NULL, claim_code = NULL, erased = 1
    WHERE erased = 0 AND EXISTS (SELECT 1 FROM raffles r WHERE r.id = entrants.raffle_id
      AND (r.purged_at IS NOT NULL OR r.organizer_left_at IS NOT NULL));
    UPDATE raffles SET state = 'purged' WHERE purged_at IS NOT NULL AND state <> 'purged';
    UPDATE raffles SET organizer_name = '', contact_email = '', privacy_email = '', privacy_text = '',
      privacy_url = '' WHERE organizer_left_at IS NOT NULL;"
  ```

## Limits

The join limits are Workers rate-limiting bindings (600/min per raffle, 5/min per raffle and email, 600/min live
connections). They are soft, per Cloudflare location. The hard limit is `RAFI_MAX_ENTRANTS`, enforced in D1.

## Local development

`devenv --profile workers up` (see [Getting started](../development/getting-started.md#profiles), or
[without devenv](../development/getting-started.md#cloudflare-workers)), and `./scripts/workers-check.sh` in
[Testing](../development/testing.md#workers).
