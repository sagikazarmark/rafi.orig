# Cloudflare Workers

Rafi on Cloudflare: two Workers and one D1 database, with Restate Cloud running the raffle engine's invocations and
Clerk signing organizers in. This is the production deployment. An installation is a repository of the operator's,
a copy of [`deploy/cloudflare/`](../../deploy/cloudflare/), and GitHub Actions deploys a release of Rafi from it
on every push. CI runs the production shape through `wrangler deploy --dry-run`, a whole evening under `wrangler
dev` and the hourly housekeeping, and rehearses each release's deploy; what it can't check locally is listed under
[Check](#9-check).

Cloudflare is one deployment, not an assumption of the code. The app and the engine are written against small
interfaces (the engine's `Store` and `Observer`, the live `Notifier`, the app's `RateLimit`), and `rafi-cloudflare`
implements them over Workers bindings.

## The two Workers

- **The app Worker**, `rafi` (`apps/worker-app`), on your domain only (`workers_dev` and preview URLs are off:
  a preview would share production's database and Durable Objects). It hosts the app's routes and the static files
  (Workers Assets), and the web app's page (its `index.html`, compiled into the Worker: a path that isn't a file
  comes to the Worker, which serves the page with a Content-Security-Policy naming your Clerk host); the
  `RaffleRoom` Durable Object (one live channel per raffle), the rate limiters and the hourly Cron. It calls the
  raffle engine through Restate Cloud's ingress, and answers 404 under `/restate`.
- **The raffle engine Worker**, `rafi-engine-<version>` (`apps/worker-engine`; `rafi-engine-0-1-0` for 0.1.0),
  one per release, on the account's workers.dev subdomain. It is the Restate endpoint (the `Raffle` object and
  `EngineInfo`) under `/restate`, and 404 everywhere else. It writes the draws to the app's database and nudges
  the app Worker's `RaffleRoom` after every change, through a Durable Object binding to the app's class
  (`script_name`). It holds no secrets: the Restate environment's identity keys are public keys, and the drand
  settings are vars.

Why one engine Worker per release: Restate pins each invocation to the deployment it started on and sends new ones
to the latest. A draw sleeping until its drand round is an invocation in flight, and it must finish on the code
that started it. So each release's engine is a new Worker at a new URL, registered with Restate as a new
deployment and never deployed again; the previous one keeps serving what is pinned to it until Restate reports it
drained, and a later deploy then removes it from Restate and deletes it.

Locally the two are one Worker, `apps/worker` (`rafi-worker`), with the engine under `/restate` beside the app: see
[Local development](#local-development).

### What the engine Worker needs

From [`engine.wrangler.toml`](../../deploy/cloudflare/engine.wrangler.toml), which the deploy checks against
`wrangler.toml`:

- `DB`, bound to the app's database (the same `database_id`);
- `RAFFLE_ROOM`, class `RaffleRoom` with `script_name` the app Worker's name: the class is the app's, so the app
  Worker must exist first (a first deploy deploys the app before the engine);
- `RAFI_DO_JURISDICTION`, the app's;
- `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS`: the Restate environment's public key (`publickeyv1_…`), or several, comma
  separated, while rotating it ([Runbook](runbook.md#rotating-secrets)). Without it the engine refuses every
  request;
- optionally `RAFI_DRAND_RELAYS`, `RAFI_DRAND_LEAD_SECS`, `RAFI_DRAND_TIMEOUT_SECS`
  ([configuration](../reference/configuration.md#workers-only));
- `workers_dev = true`: Restate calls it at `https://rafi-engine-<version>.<subdomain>.workers.dev/restate`.

Since a registered engine Worker is never deployed again, a change to `engine.wrangler.toml` takes effect with the
next release's engine.

### The engine on workers.dev

The engine's address is public and its name predictable. That is fine because nothing reaches its handlers without
Restate's signature: every request to `/restate/*` must carry a request identity signed by your Restate
environment's private key, which the Restate SDK checks against `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` before any
routing, discovery included; anything else is refused, and every other path is a 404. The engine has no secrets to
leak, and it sees anonymous IDs only. Your zone's rules (WAF, Bot Fight Mode) don't apply to workers.dev, so
none of them can challenge Restate's calls either.

## What an installation is

Nothing of an installation's is compiled in: one build of the Workers and the web app serves every installation.
The build is a release's one asset, `worker.tar.gz` (built and attested by `.github/workflows/release.yaml`):
`VERSION`, `app/` (the app Worker and its static files), `engine/` (the engine Worker), `migrations/sqlite/` (D1)
and `deploy.sh`, the deploy itself (`scripts/cloudflare-deploy.sh`), so a release deploys with its own logic. The
installation is a repository of yours, a copy of [`deploy/cloudflare/`](../../deploy/cloudflare/):

- `wrangler.toml`, the app Worker's shape with your database ID and domain;
- `engine.wrangler.toml`, the engine Worker's, with the same database ID and your Restate environment's public key;
- `legal/terms.md` and `legal/privacy.md`, your legal documents, which the deploy copies into the app's static
  files;
- `.dev.vars.example`, listing the app Worker's secrets, which are set on the Worker (by the deploy from a GitHub
  secret, or by you), never in a file you track;
- `.github/workflows/deploy.yml`, the deploy: the release it deploys and your repository's secrets.

The deploy reads the two configurations with a small line parser: the keys it reads (each file's header lists
them) are written as `key = "value"` on one line, as the template has them, and `[env.*]` tables are ignored.
`apps/worker-app/wrangler.toml` and `apps/worker-engine/wrangler.toml` in this repository are the same shapes;
`scripts/workers-check.sh` checks that the template's agree with them on the bindings, limits and required
secrets.

## The deploy workflow

`deploy/cloudflare/.github/workflows/deploy.yml` runs on every push to `main` (but one changing only Markdown outside
`legal/`: a deploy restarts the live channels and stops while a raffle is live, which a typo fix shouldn't do) and
by hand (**Actions > Deploy > Run workflow**, with `deploy_while_live`). It is one job calling this repository's
[`.github/workflows/deploy-cloudflare.yml`](../../.github/workflows/deploy-cloudflare.yml) at the release's tag:

```yaml
uses: sagikazarmark/rafi/.github/workflows/deploy-cloudflare.yml@v0.1.0
with:
  version: "0.1.0"
```

The two name the same release (the release rehearsal checks the template's do). The tag is immutable once its
release is published, so the workflow that runs is the one released with it.

| Input | |
|---|---|
| `version` | The release, e.g. `0.1.0` (required) |
| `deploy_while_live` | Deploy even though raffles are live (default off) |
| `environment` | A GitHub environment of yours, for its protection rules (required reviewers) and secrets; none when empty |

| Secret | |
|---|---|
| `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID` | The account, with the token [below](#7-the-cloudflare-api-token) |
| `RESTATE_ADMIN_URL` | The Restate environment's admin API, `https://<env>.env.<region>.restate.cloud:9070` |
| `RESTATE_AUTH_TOKEN` | A Restate Cloud API key with the Admin role |
| `RAFI_SECRETS` | Optional: the app Worker's secrets as a dotenv file's content; without it they are yours to set ([below](#6-the-app-workers-secrets)) |

The job, on `ubuntu-latest`, one at a time per repository (concurrency group `rafi-deploy`; a run waits for the one
in progress, never cancels it):

1. checks the version's format;
2. checks out your repository and sets up Node.js (wrangler and the Restate CLI come through `npx`, at the versions
   `deploy.sh` pins);
3. downloads `worker.tar.gz` from the GitHub release `v<version>`;
4. **verifies its provenance**: `gh attestation verify` with the Rafi repository, the signer workflow
   `.github/workflows/release.yaml`, the source ref `refs/tags/v<version>` and GitHub-hosted runners only. The
   release workflow attests the tarball from a run on the tag (it refuses to publish from anything else), so a
   tarball that this repository's release workflow didn't build from that tag stops the deploy;
5. unpacks it into `release/` (fresh each run; the template's `.gitignore` lists it);
6. writes `RAFI_SECRETS`, when set, to a file only the runner's user reads, and runs `release/deploy.sh` with it as
   `RAFI_SECRETS_FILE`, `RAFI_VERSION` the version (the release's `VERSION` must agree) and
   `RAFI_DEPLOY_WHILE_LIVE=1` when asked.

The run's log in your repository's Actions is the deploy's log. The job needs only `contents: read`: the release
and its attestation are public.

## What each deploy does

`release/deploy.sh`, printing each step (its header in
[`scripts/cloudflare-deploy.sh`](../../scripts/cloudflare-deploy.sh) is the reference):

1. **Checks** the release and the two configurations: `main`, the assets and the migrations point into
   `release/`; one `database_id` for both; the engine's `RAFFLE_ROOM` names the app Worker; the same jurisdiction;
   the engine on workers.dev with identity keys; no placeholders left.
2. **Copies `legal/`** into the app's static files.
3. **Looks up** the account's workers.dev subdomain and Restate's deployments, before changing anything.
4. **Is anything live?** The runbook's [preflight query](runbook.md#before-a-deploy-is-anything-live) on the
   database: live raffles stop the deploy, listed, unless `deploy_while_live`. A database without the tables yet
   (the first deploy) has nothing live.
5. **The D1 migrations** (`wrangler d1 migrations apply DB --remote`).
6. **The engine, then the app.** The release's engine Worker is deployed and registered with Restate (`POST
   /deployments` without `force`, retried while the new Worker comes up), then the app Worker is deployed, so the
   app never calls a handler the registered engine lacks. On the first deploy the app goes first, since the engine
   binds its class. An engine Worker Restate already has is left as it is; one deployed by a run that stopped
   before registering it is deployed again and registered. A failed registration fails the deploy before the app
   is deployed.
7. **Pruning.** Every older engine deployment of this installation that Restate reports **drained** is removed from
   Restate (never forced: Restate refuses one that isn't drained), and then its Worker is deleted. One still
   **draining** is listed with the invocations pinned to it, and pruned by a later deploy. A **paused** invocation
   (Restate stopped retrying it) keeps its deployment draining until someone resumes it on the latest deployment
   (`restate invocations resume <id> --deployment latest`) or kills it (`restate invocations kill <id>`): see
   [Upgrades](upgrades.md#registering-the-raffle-engine). Engine Workers Restate doesn't know are listed, not
   deleted. A pruning failure warns; the deploy has succeeded.

A failed run leaves what it finished (migrations apply forward and stay; a registered engine stays registered);
running it again continues from there.

## You'll need

- a Cloudflare account on the paid Workers plan (Workers, D1, Durable Objects): verifying a BLS signature and
  rendering the participant page don't fit the free plan's CPU time
- a zone (your domain) on that account, and the account's workers.dev subdomain enabled
- a Restate Cloud environment, with an API key with ingress access (the app's) and one with the Admin role (the
  deploy's)
- a Clerk application
- a GitHub repository for the installation, with Actions
- Node.js 22 or later on your machine for the one-time `npx wrangler` steps
- for a build from source only: the toolchain in `devenv.nix` (`worker-build`, `wrangler`, `wasm-bindgen-cli`
  matching `Cargo.lock`, `dx`, `wasm-opt`, Node)

## First deploy

The short version is `deploy/cloudflare`'s [README](../../deploy/cloudflare/README.md). What follows is the longer
one.

### 1. The installation

Copy `deploy/cloudflare/`, `.github/` included, into a new repository of yours (private is fine), as its root, at
the release's tag. Its workflow runs on every push to `main`; until the steps below are done it stops at its first
check (the placeholders), having changed nothing.

Pick the origin organizers and phones will use (`https://raffle.example.org`) and put it into `routes` in
`wrangler.toml` (`{ pattern = "raffle.example.org", custom_domain = true }`): the deploy attaches it as the app
Worker's custom domain. From a workflow (not a terminal), wrangler takes over an existing DNS record or custom
domain for that hostname instead of asking, so pick a hostname nothing else uses.

### 2. Clerk

Create an application and its production instance for your domain, and add the DNS records Clerk lists to your zone
(*DNS only*). Its keys, `pk_live_…` and `sk_live_…`, are app secrets (step 6).

### 3. Restate Cloud

Create an environment (in the EU if personal data must stay there). From its page in the console:

- the ingress URL (`https://<env>.env.<region>.restate.cloud:8080`) and an API key with ingress access: the app
  Worker's `RAFI_ENGINE_INGRESS_URL` and `RAFI_ENGINE_API_KEY`;
- the admin URL (`https://<env>.env.<region>.restate.cloud:9070`) and an API key with the **Admin** role: the
  workflow's `RESTATE_ADMIN_URL` and `RESTATE_AUTH_TOKEN`. The deploy registers each release's engine with it,
  lists the deployments, asks which invocations are pinned to a draining one (`/query`) and removes drained ones;
- the request identity's public key (`publickeyv1_…`): `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` in
  `engine.wrangler.toml`.

Nothing is registered by hand: each deploy registers its release's engine.

### 4. The database

```sh
npx wrangler@4.148.0 login
npx wrangler@4.148.0 d1 create rafi --jurisdiction eu   # note the database_id it prints
```

Put the ID into both `wrangler.toml` and `engine.wrangler.toml`.

### 5. The legal documents and the operator

The Worker serves the installation's documents from `legal/`: `terms.md` (with the data processing agreement, which
organizers accept by using the service) and `privacy.md` (the operator's own notice, naming the subprocessors:
Cloudflare, Restate Cloud, Clerk, drand). They ship as engineering drafts marked as templates: fill in the bracketed
parts, have a lawyer review them, and delete the marker. A document that still carries the template marker makes
the Worker refuse every request. Their versions (`RAFI_TERMS_VERSION`, `RAFI_PRIVACY_VERSION`, e.g. `2026-10`) and
the operator's name and email (`RAFI_OPERATOR_NAME`, `RAFI_OPERATOR_EMAIL`) are secrets, next. A changed document is
a new version, shown with it; a change to the processing agreement is notified to organizers directly, since nothing
records who read which version. Your record of processing, and what changed in the templates, are in [the
operator's records](overview.md#the-operators-records). The retention settings (`RAFI_RETENTION_DAYS` and the
others, see the [configuration](../reference/configuration.md)) have defaults; `RAFI_MIN_AGE` too; set them as
secrets if you change them.

### 6. The app Worker's secrets

The installation's values are the app Worker's secrets. `wrangler.toml` lists them under `[secrets] required`, and
wrangler refuses to deploy the app while one is missing; `.dev.vars.example` says what each is. One file, kept out
of your repository (`.dev.vars` is ignored):

```ini
RAFI_PUBLIC_URL=https://raffle.example.org
RAFI_ENGINE_INGRESS_URL=https://<env>.env.eu.restate.cloud:8080
RAFI_ENGINE_API_KEY=…
RAFI_POW_KEY=<openssl rand -hex 32>
CLERK_SECRET_KEY=sk_live_…
CLERK_PUBLISHABLE_KEY=pk_live_…
RAFI_OPERATOR_NAME=Example Meetups e.V.
RAFI_OPERATOR_EMAIL=privacy@example.org
RAFI_TERMS_VERSION=2026-10
RAFI_PRIVACY_VERSION=2026-10
```

`CLERK_PUBLISHABLE_KEY` is not a secret (the web app gets it from the Worker at `/config.js`), but it is the
installation's, so it travels with the others. Two ways to set them:

- **From GitHub**: the file's content as the repository secret `RAFI_SECRETS`. Each deploy passes it to the app's
  `wrangler deploy --secrets-file`, which creates the Worker with them on the first deploy and sets them on every
  later one. Secrets it doesn't name are kept, so deleting a line doesn't delete the secret (`npx wrangler secret
  delete <NAME>` does).
- **By hand**, once, before the first deploy, from your repository: `npx wrangler@4.148.0 secret bulk .dev.vars`.
  The Worker doesn't exist yet, so wrangler creates it holding only the secrets (it asks first); the deploy sees no
  Durable Object on it yet, so it still deploys the app first. Later, `npx wrangler secret put <NAME>`.

With neither, wrangler can't deploy a new Worker with required secrets missing: the first deploy stops at the app,
after the migrations, naming them; set them and run it again. Never rotate `RAFI_POW_KEY` during an event.

The Worker refuses every request without its documents, `RAFI_POW_KEY`, `RAFI_ENGINE_INGRESS_URL`,
`CLERK_SECRET_KEY`, `CLERK_PUBLISHABLE_KEY`, the operator and the document versions. Release builds reject dev auth
and the mock chain.

### 7. The Cloudflare API token

A custom token (**Manage account > Account API tokens > Create token**) for your account and your domain's zone:

| Permission | What the deploy uses it for |
|---|---|
| Account · Workers Scripts · Edit | Uploading both Workers (code, static files, secrets, the Cron, the Durable Object migration, the workers.dev address); the app's custom domain; looking up the workers.dev subdomain (`GET /accounts/{id}/workers/subdomain`), the app Worker (`GET …/workers/services/{name}`) and the account's Workers (`GET …/workers/scripts`); deleting a drained engine Worker |
| Account · D1 · Edit | The "is anything live?" query and the migrations (`wrangler d1 execute`, `wrangler d1 migrations apply`) |
| Zone · Workers Routes · Edit (your zone) | The app Worker's route on your domain |

The API reference lists Workers Scripts as the permission for the subdomain, the scripts, their schedules,
workers.dev and domain endpoints, and D1 for queries; Cloudflare's **Edit Cloudflare Workers** token template (what
[its GitHub Actions guide](https://developers.cloudflare.com/workers/ci-cd/external-cicd/github-actions/)
recommends for wrangler) adds Workers Routes on the zone, and KV, R2, tail, account settings and user permissions
the deploy doesn't use. That template plus Account · D1 · Edit is a superset, if you'd rather start from it.

### 8. GitHub, and the first run

In the repository's **Settings > Secrets and variables > Actions**: `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`,
`RESTATE_ADMIN_URL`, `RESTATE_AUTH_TOKEN`, and `RAFI_SECRETS` if you chose it. If your account or organization
restricts which actions run, allow `sagikazarmark/rafi`'s workflows, `actions/checkout` and `actions/setup-node`.
For an approval before each deploy, make a GitHub environment with required reviewers and pass its name as
`environment` (commented out in `deploy.yml`); its secrets are used in place of the repository's.

Commit your configuration and documents, and push to `main` (or run the workflow by hand). The first run creates
the database's tables, the app Worker (with its Durable Object class and Cron), the engine Worker
`rafi-engine-0-1-0`, and registers the engine with Restate.

### 9. Check

Open `/healthz`, `/terms` and `/privacy`, then run a test-mode raffle end to end, as in
[Self-hosting](self-hosting.md#8-check). `/healthz` answering `ok` and the version means the app Worker has every
secret and its documents (anything missing answers `500 misconfigured`, naming it when that names no value); it
does not check the database's schema against the build (the deploy applies the migrations first). The engine's
log is under its own Worker (`npx wrangler tail rafi-engine-0-1-0`), the app's under `rafi`.

Nothing in this repository's CI holds a Cloudflare token: deploys run in installations' repositories. CI does run
the production shapes through `wrangler deploy --dry-run`, serves the app without secrets, and rehearses a
release's `deploy.sh --dry-run` from a copy of `deploy/cloudflare` ([Testing](../development/testing.md#workers)),
so what a release ships has been built and validated the way `wrangler deploy` builds and validates it.

What only a real deploy can check, so check it on the first one:

- the token's permissions cover every step (the custom domain and a deleted Worker included), and the Admin
  key every Restate call (registering, listing, `/query`, removing);
- the workflow reads the release and its attestation with the repository's own `GITHUB_TOKEN`;
- the app Worker reaches Restate Cloud's ingress on its port (`:8080`): `workerd` allows any port, production has
  port rules for proxied hosts; Restate Cloud's `:443` alias is the fallback;
- the startup time `wrangler deploy` reports stays under the 400 ms limit for both (the app's module is about
  3.7 MB, the engine's 2.6 MB);
- the Durable Objects' EU jurisdiction (`ns.jurisdiction("eu")`) is reached only in production, from the app (the
  test-mode raffle's first phone connect) and from the engine through its binding to the app's class (the first
  draw's nudge);
- Restate Cloud calls the engine's `/restate/*` on workers.dev over HTTP/2 (locally HTTP/1.1 only);
- the paid Workers plan: BLS verification and server-side rendering don't fit the free plan's CPU time;
- the next release's deploy registers its engine, and once the first engine has drained, a later deploy removes it
  from Restate and deletes its Worker.

## Upgrading

Change the release in `.github/workflows/deploy.yml`, both the `uses: …@v<version>` and `version:`, and push,
between events. Never to an older release ([Upgrades](upgrades.md#database-migrations)). The release notes say when
the configurations, `.dev.vars.example` or the workflow changed shape; compare yours with `deploy/cloudflare` at the
new tag then.

A release has one asset, `worker.tar.gz`; the release page shows its SHA256 digest (GitHub computes one for every
upload, and the release workflow checks it is the build's); releases are immutable once published, so neither the
asset nor the tag under a version changes. The deploy verifies the attestation; by hand:

```sh
gh release download v<version> -R sagikazarmark/rafi -p worker.tar.gz
gh attestation verify worker.tar.gz -R sagikazarmark/rafi \
  --signer-workflow github.com/sagikazarmark/rafi/.github/workflows/release.yaml \
  --source-ref refs/tags/v<version> --deny-self-hosted-runners
```

## From source

Instead of a release, build one here into your installation's `release/` and run its `deploy.sh` from your
installation, with the workflow's environment:

```sh
RAFI_VERSION=<version> just cloudflare-artifact /path/to/your/installation/release
cd /path/to/your/installation
CLOUDFLARE_API_TOKEN=… CLOUDFLARE_ACCOUNT_ID=… RESTATE_ADMIN_URL=… RESTATE_AUTH_TOKEN=… release/deploy.sh
```

`just cloudflare-artifact` is what the release workflow packs. The version names the engine Worker, so a build of
your own deploys an engine of its own; the rule against older builds holds the same.

## Data residency

- D1 is created with `--jurisdiction eu`.
- Durable Objects come from an EU-jurisdiction namespace (`RAFI_DO_JURISDICTION=eu`, fixed at creation).
- The automatic per-invocation logs are off, because they would record client IPs and cookies; the application's
  own lines (warnings and errors, with raffle IDs and error text) are kept.
- D1 Time Travel keeps erased data for up to 30 more days. Say so in your privacy notice (the template does). Time
  Travel is also the backup: the [runbook](runbook.md#backups-and-restore) has the restore, in order. After
  restoring with Time Travel, re-apply the erasures. The hourly housekeeping erases again whatever is past its date
  on its own (the dates are in the raffles themselves), and whoever took themselves off a raffle that is over, and
  forgets the removals at their request of raffles that are frozen, purged or past their date (the last statement
  below does the same at once). What it can't know is what organizers erased before the
  date, or who removed their data: for those, put back what the live database had (the runbook's steps 1 and 3:
  the `erasures` log, `purged_at` and `organizer_left_at`, and the rest), then run the restore tool's statements
  against D1 (the same SQL `rafi reapply-erasures` runs natively, see [Self-hosting](self-hosting.md#backups)):

  ```sh
  wrangler d1 execute rafi --remote --command "
    -- Entrants the log names (if the log came along).
    UPDATE entrants SET display_name = '#' || substr(id, 1, 6), email_raw = NULL, email_norm = NULL,
      list_ref = NULL, token_hash = NULL, claim_code = NULL, erased = 1
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
      list_ref = NULL, token_hash = NULL, claim_code = NULL, erased = 1
    WHERE erased = 0 AND EXISTS (SELECT 1 FROM raffles r WHERE r.id = entrants.raffle_id
      AND (r.purged_at IS NOT NULL OR r.organizer_left_at IS NOT NULL));
    UPDATE raffles SET state = 'purged' WHERE purged_at IS NOT NULL AND state <> 'purged';
    UPDATE raffles SET organizer_name = '', contact_email = '', privacy_email = '', privacy_text = '',
      privacy_url = '' WHERE organizer_left_at IS NOT NULL AND (organizer_name <> '' OR contact_email <> ''
      OR privacy_email <> '' OR privacy_text <> '' OR privacy_url <> '');
    -- Removals at their request no add can need any more.
    DELETE FROM add_removals WHERE raffle_id IN (SELECT id FROM raffles
      WHERE manifest IS NOT NULL OR purged_at IS NOT NULL OR erase_at <= unixepoch());"
  ```

## Limits

The rate limits are Workers rate-limiting bindings (600 joins/min per raffle, 5/min per raffle and email, 20 adds
or undos/min per organizer, 600/min live connections). They are soft, per Cloudflare location. The hard limit is
`RAFI_MAX_ENTRANTS`, enforced in D1.

## Local development

`devenv --profile workers up` (see [Getting started](../development/getting-started.md#profiles), or
[without devenv](../development/getting-started.md#cloudflare-workers)), and `./scripts/workers-check.sh` in
[Testing](../development/testing.md#workers).
