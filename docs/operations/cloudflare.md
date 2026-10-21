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

### 1. Database

```sh
cd apps/worker
wrangler d1 create rafi --jurisdiction eu
```

Put the new database's ID into `database_id` in `apps/worker/wrangler.toml`. Then apply the schema:

```sh
wrangler d1 migrations apply rafi --remote
```

### 2. The Worker

Set `RAFI_PUBLIC_URL` and `RESTATE_INGRESS_URL` in `[vars]` of `apps/worker/wrangler.toml`, then the secrets:

```sh
cd apps/worker
wrangler secret put RAFI_ALTCHA_KEY          # random, 32+ bytes; never rotate during an event
wrangler secret put CLERK_SECRET_KEY
wrangler secret put RESTATE_API_KEY          # Restate Cloud, ingress access
wrangler secret put RESTATE_IDENTITY_KEYS    # your Restate environment's public key(s), comma separated
```

Build the static files (web app and participant scripts) with Clerk's publishable key compiled in, then deploy:

```sh
CLERK_PUBLISHABLE_KEY=pk_live_… ./scripts/app-assets.sh   # -> apps/worker/worker-assets
(cd apps/worker && wrangler deploy)
```

The Worker refuses every request without `RAFI_ALTCHA_KEY`, `RESTATE_INGRESS_URL`, `CLERK_SECRET_KEY` or
`RESTATE_IDENTITY_KEYS`. Release builds reject dev auth and the mock chain.

Attach a custom domain. `workers_dev` and preview URLs are off on purpose, because previews would share
production's database and Durable Objects.

### 3. The raffle engine

In Restate Cloud, **register `https://<the Worker's hostname>/restate` as a new deployment**. Do this again for
every release (see [Upgrades](upgrades.md)). Restate appends `/discover` and `/invoke/…` to that URL; the Worker
verifies each request's signature before anything else happens to it.

### 4. Check

Open `/healthz`, then run a test-mode raffle end to end, as in [Self-hosting](self-hosting.md#7-check).

## Data residency

- D1 is created with `--jurisdiction eu`.
- Durable Objects come from an EU-jurisdiction namespace (`RAFI_DO_JURISDICTION=eu`, fixed at creation).
- Invocation logs are off, because they would record client IPs and cookies.
- D1 Time Travel keeps erased data for up to 30 more days. Say so in your privacy notice.

## Limits

The join limits are Workers rate-limiting bindings (600/min per raffle, 5/min per raffle and email, 600/min live
connections). They are soft, per Cloudflare location. The hard limit is `RAFI_MAX_ENTRANTS`, enforced in D1.

## Local development

`devenv --profile workers up` (see [Getting started](../development/getting-started.md#profiles), or
[without devenv](../development/getting-started.md#cloudflare-workers)), and `./scripts/workers-check.sh` in
[Testing](../development/testing.md#workers).
