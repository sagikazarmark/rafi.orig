# Cloudflare Workers

Rafi on Cloudflare: one Worker and one D1 database, with Restate Cloud running the raffle engine. This is the
production deployment. CI runs its shape through `wrangler deploy --dry-run`, a whole evening under `wrangler
dev` and the hourly housekeeping; what it can't check locally is listed under [First deploy](#first-deploy)'s
check step.

Cloudflare is one deployment, not an assumption of the code. The app and the engine are written against small
interfaces (the engine's `Store` and `Observer`, the live `Notifier`, the app's `RateLimit`), and `rafi-cloudflare`
implements them over Workers bindings.

The Worker, `rafi` (`apps/worker`), hosts:

- the app's routes and the static files (Workers Assets);
- the raffle engine (`Raffle`, `EngineInfo`) under `/restate/*`, called by Restate Cloud over HTTPS with signed
  requests. It writes to the same D1 and nudges `RaffleRoom` after every change;
- the `RaffleRoom` Durable Object (one live channel per raffle), the join rate limiters and the hourly Cron.

`apps/worker/wrangler.toml` has a production top level, plus `[env.dev]` for local development; an installation
deploys the same shape from [`deploy/cloudflare/`](../../deploy/cloudflare/), below.

## You'll need

- a Cloudflare account on the paid Workers plan (Workers, D1, Durable Objects): verifying a BLS signature and
  rendering the participant page don't fit the free plan's CPU time
- a Restate Cloud environment
- a Clerk application
- for a build from source only: the toolchain in `devenv.nix` (`worker-build`, `wrangler`, `wasm-bindgen-cli`
  matching `Cargo.lock`, `dx`, `wasm-opt`, Node)

## What an installation is

Nothing of an installation's is compiled in: one build of the Worker and the web app serves every installation.
The build is a release's tarball (`rafi-cloudflare-<version>.tar.gz`, with `SHA256SUMS`, built by
`.github/workflows/release.yaml` from `scripts/cloudflare-artifact.sh`): the Worker, the static files and the D1
migrations. The installation is [`deploy/cloudflare/`](../../deploy/cloudflare/), a directory of yours:

- `wrangler.toml`, the Worker's shape with your database ID and domain;
- `deploy.sh`, naming the release you deploy (`RAFI_VERSION`): it fetches it, checked against its checksums, puts
  `legal/` in place, applies the D1 migrations and deploys;
- `legal/terms.md` and `legal/privacy.md`, your legal documents, which the Worker reads from its static files;
- your secrets (`.dev.vars.example` lists them), set on the Worker, never in a file you track.

`apps/worker/wrangler.toml` in this repository is the same shape for development and CI (`[env.dev]`, the dry
deploy); `scripts/workers-check.sh` checks that the two agree on the Worker's paths and required secrets.

## First deploy

The short version is `deploy/cloudflare`'s [README](../../deploy/cloudflare/README.md): the Deploy to Cloudflare
button, or the same by hand. What follows is the longer one.

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/sagikazarmark/rafi/tree/main/deploy/cloudflare)

### 1. The installation

Copy `deploy/cloudflare/` into a repository of yours (private is fine), and in it:

```sh
wrangler d1 create rafi --jurisdiction eu   # note the database_id it prints
```

and put the ID into `wrangler.toml`. The quick way is the button, which copies the directory into a repository of
yours, creates the database and the Durable Object, asks for the secrets, and deploys from that repository on every
push ([Workers Builds](https://developers.cloudflare.com/workers/ci-cd/builds/), running `deploy.sh fetch` as the
build and `deploy.sh deploy` as the deploy). Its database has no jurisdiction; if personal data must stay in the EU,
create one as above afterwards, put its ID in, push, and delete the button's.

### 2. The legal documents and the operator

The Worker serves the installation's documents from `legal/`: `terms.md` (with the data processing agreement, which
organizers accept by using the service) and `privacy.md` (the operator's own notice, naming the subprocessors:
Cloudflare, Restate Cloud, Clerk, drand). They ship as engineering drafts marked as templates: fill in the bracketed
parts, have a lawyer review them, and delete the marker. A document that still carries the template marker makes
the Worker refuse every request. Their versions (`RAFI_TERMS_VERSION`, `RAFI_PRIVACY_VERSION`, e.g. `2026-10`) and
the operator's name and email (`RAFI_OPERATOR_NAME`, `RAFI_OPERATOR_EMAIL`) are secrets, next. A changed document is
a new version, shown with it; a change to the processing agreement is notified to organizers directly, since nothing
records who read which version. The retention settings (`RAFI_RETENTION_DAYS` and the others, see the
[configuration](../reference/configuration.md)) have defaults; `RAFI_MIN_AGE` too; set them as secrets if you
change them.

### 3. The Worker

The installation's values are secrets. `wrangler.toml` lists them under `[secrets] required`, and a deploy
refuses to go out while one is missing; `.dev.vars.example` says what each is. The button asks for them; by
hand, one file, kept out of your repository (`.dev.vars` is ignored):

```ini
RAFI_PUBLIC_URL=https://raffle.example.org
RAFI_ENGINE_INGRESS_URL=https://<env>.env.eu.restate.cloud:8080
RAFI_ENGINE_API_KEY=…
RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS=publickeyv1_…
RAFI_ALTCHA_KEY=<openssl rand -hex 32>
CLERK_SECRET_KEY=sk_live_…
CLERK_PUBLISHABLE_KEY=pk_live_…
RAFI_OPERATOR_NAME=Example Meetups e.V.
RAFI_OPERATOR_EMAIL=privacy@example.org
RAFI_TERMS_VERSION=2026-10
RAFI_PRIVACY_VERSION=2026-10
```

`CLERK_PUBLISHABLE_KEY` is not a secret (the web app gets it from the Worker at `/config.js`), but it is the
installation's, so it travels with the others. Then:

```sh
./deploy.sh fetch                  # the release into rafi/, checked against SHA256SUMS
wrangler deploy                    # the first time: creates the Worker and stops at the missing secrets
wrangler secret bulk .dev.vars
./deploy.sh deploy                 # legal/ into place, the D1 migrations, then the deploy
```

The very first `deploy` creates the Worker and stops at the missing secrets (there is no Worker to hold them yet);
`secret bulk` then sets them, and the deploy is run again. From then on, a deploy that is missing one refuses, and
`./deploy.sh` is every deploy: the release, the migrations, the Worker. Never rotate `RAFI_ALTCHA_KEY` during an
event.

The Worker refuses every request without its documents, `RAFI_ALTCHA_KEY`, `RAFI_ENGINE_INGRESS_URL`,
`CLERK_SECRET_KEY`, `CLERK_PUBLISHABLE_KEY`, `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS`, the operator and the document
versions. Release builds reject dev auth and the mock chain.

Attach a custom domain (`routes` in `wrangler.toml`, or the dashboard). `workers_dev` and preview URLs are off on
purpose, because previews would share production's database and Durable Objects.

### 4. The raffle engine

In Restate Cloud, **register `https://<the Worker's hostname>/restate` as a deployment**, without `force`.
Restate appends `/discover` and `/invoke/…` to that URL; the Worker verifies each request's signature before
anything else happens to it. A release that changes the engine's handlers comes with a new path under `/restate/`
and says so; register that, and drain the old deployment (see
[Upgrades](upgrades.md#registering-the-raffle-engine)). Re-registering a URL Restate already knows changes
nothing, silently.

### 5. Check

Open `/healthz`, `/terms` and `/privacy`, then run a test-mode raffle end to end, as in
[Self-hosting](self-hosting.md#8-check). `/healthz` answering `ok` means the Worker has every secret and its
documents; it does not check the database's schema against the build (apply the migrations first, as above).

Nothing in this repository's CI holds a Cloudflare token: deploys are yours (Workers Builds from your repository,
or `./deploy.sh`). CI does run the production shape through `wrangler deploy --dry-run`, serves it without
secrets, and deploys (dry) the artifact through `deploy/cloudflare` ([Testing](../development/testing.md#workers)),
so what a release ships has been built and validated the way `wrangler deploy` builds and validates it.

What only a real deploy can check, so check it on the first one:

- the Worker reaches Restate Cloud's ingress on its port (`:8080`): `workerd` allows any port, production has port
  rules for proxied hosts; Restate Cloud's `:443` alias is the fallback;
- the startup time `wrangler deploy` reports stays under the 400 ms limit (the module is about 5 MB);
- the Durable Objects' EU jurisdiction (`ns.jurisdiction("eu")`) is reached only in production: the test-mode
  raffle's first phone connect exercises it;
- Restate Cloud calls `/restate/*` over HTTP/2 (locally HTTP/1.1 only);
- a zone-level Bot Fight Mode or WAF rule that challenges `POST /restate/*` (a non-browser client) needs a skip
  rule for that path;
- the paid Workers plan: BLS verification and server-side rendering don't fit the free plan's CPU time.

## From source

Instead of a release, build the artifact here and point your copy of `deploy/cloudflare` at it:

```sh
./scripts/cloudflare-artifact.sh /path/to/your/installation/rafi    # the Worker, the static files, the migrations
(cd /path/to/your/installation && ./deploy.sh deploy)
```

`cloudflare-artifact.sh` is what the release workflow runs; `deploy.sh deploy` alone skips the fetch.

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
  wrangler d1 execute rafi --remote --command "
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
