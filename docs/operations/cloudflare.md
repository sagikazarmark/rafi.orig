# Cloudflare Workers

Rafi on Cloudflare: three Workers and one D1 database, with Restate Cloud running the raffle engine's invocations
and Clerk signing organizers in. This is the production deployment. An installation is a repository of the
operator's, a copy of [`deploy/cloudflare/`](../../deploy/cloudflare/), whose GitHub Actions workflows deploy the
Rafi release its `VERSION` names. The first deploy, step by step, is that directory's
[README](../../deploy/cloudflare/README.md); this page is the reference behind it: what runs where, what the
deploy does and why, the permissions it needs, and what to do when something goes wrong.

Cloudflare is one deployment, not an assumption of the code. The app and the engine are written against small
interfaces (the engine's `Store` and `Observer`, the live `Notifier`, the app's `RateLimit`), and `rafi-cloudflare`
implements them over Workers bindings.

## The three Workers

| | App | Engine | Rooms |
|---|---|---|---|
| Worker | `rafi` | `rafi-engine-v<version, dashed>` (`rafi-engine-v0-1-0`), one per release | `rafi-rooms` |
| Built from | `apps/worker-app` | `apps/worker-engine` | `apps/worker-rooms` |
| Configuration | `wrangler.toml` | `engine.wrangler.toml` (named `rafi-engine` there) | `rooms.wrangler.toml` |
| Address | your domain only (a custom domain; no workers.dev, no preview URLs) | `https://<it>.<subdomain>.workers.dev/restate` | none |
| Serves | the web app and its API, the static files, phones' live-channel sockets (gated, then handed to the raffle's object), `/healthz`; 404 under `/restate` | the Restate endpoint (`Raffle`, `EngineInfo`) under `/restate`; 404 elsewhere | 404 to every request: its objects are reached through the bindings |
| Bindings | `ASSETS`, `DB`, `RAFFLE_ROOM` (`script_name = "rafi-rooms"`), four rate limiters, Cron `0 * * * *` | `DB`, `RAFFLE_ROOM` (`script_name = "rafi-rooms"`) | `DB`, `RAFFLE_ROOM` (its own class `RaffleRoom`, migration `v1`) |
| Vars | `RAFI_DO_JURISDICTION` | `RAFI_DO_JURISDICTION`, `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS`; optionally `RAFI_DRAND_RELAYS`, `RAFI_DRAND_LEAD_SECS`, `RAFI_DRAND_TIMEOUT_SECS` | `RAFI_DO_JURISDICTION` |
| Secrets | the installation's ([below](#the-app-workers-secrets)) | none | none |

All three bind the one D1 database (the same `database_id`) and say the same `RAFI_DO_JURISDICTION`. Every
variable, and which Worker reads it, is in the [configuration reference](../reference/configuration.md#workers-only).
Locally the three are one Worker, `apps/worker` (`rafi-worker`), with the engine under `/restate` and the class
beside the app: see [Local development](#local-development).

### Why the engine is a Worker per release

Restate treats a deployment as immutable: it pins each invocation to the deployment it started on, sends new ones
to the latest, and, re-registering a URI it knows, keeps the handlers it discovered there the first time
([Upgrades](upgrades.md#registering-the-raffle-engine)). A draw sleeping until its drand round is an invocation in
flight, and it must finish on the code that started it. So each release's engine is a Worker of its own at an
address of its own, registered as a new deployment, and only ever running that release's code: a later deploy of
the same release redeploys it with the same code (with `engine.wrangler.toml`'s vars as they are then), and a
deploy of another release never touches it. The previous release's engine keeps serving what is pinned to it until
Restate reports it drained; the [cleanup](#the-restate-cleanup) then deletes it.

While an older engine drains, it writes to the database the newer release migrated and nudges the rooms Worker's
objects, so a release's migrations stay compatible with the previous release's engine, and its notes say so when
they can't ([Releasing](../development/releasing.md#before-the-tag)).

### Why the rooms Worker is separate

The raffles' live channels are `RaffleRoom` Durable Objects, one per raffle, holding phones' sockets. Deploying the
Worker that defines a Durable Object class restarts its objects, which drops every connection; and a new class or
migration needs `wrangler deploy` (a version upload refuses a pending Durable Object migration). In a Worker of its
own, the class is deployed only when its code or `rooms.wrangler.toml` changed (the deploy compares a fingerprint,
below), so most releases keep every phone connected, and the app Worker, which has no class, is uploaded as a
version and switched to only once the engine is registered.

The objects' requests (`/nudge`, `/destroy`, the forwarded socket upgrade) are an interface between Workers
deployed apart: a new rooms Worker accepts the previous app's and engine's requests, so a change to it takes two
releases.

### The engine on workers.dev

The engine's address is public and its name predictable. That is fine because nothing reaches its handlers without
Restate's signature: every request to `/restate/*` must carry a request identity signed by your Restate
environment's private key, which the Restate SDK checks against `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` before any
routing, discovery included; anything else is refused, and every other path is a 404. The engine holds no secrets
(the identity keys are public keys, the drand settings vars), and it sees anonymous IDs only. Your zone's rules (WAF,
Bot Fight Mode) don't apply to workers.dev, so none of them can challenge Restate's calls either. Its name is at
most 63 characters (a DNS label), so the engine's name in `engine.wrangler.toml` leaves room for `-v<version>`.

## What an installation is

Nothing of an installation's is compiled in: one build serves every installation. A release's one asset,
`worker.tar.gz`, built and attested by `.github/workflows/release.yaml`, holds `VERSION`, `DEPLOY_VERSION`, `app/`
(the app Worker and its static files), `engine/`, `rooms/`, `migrations/` (the D1 migrations) and `deploy.sh`
(`scripts/cloudflare-deploy.sh`), so a release deploys with its own logic ([Releasing](../development/releasing.md)).
The installation is the operator's repository, a copy of `deploy/cloudflare` at a release's tag:

- `VERSION`: the release it deploys;
- `wrangler.toml`, `engine.wrangler.toml`, `rooms.wrangler.toml`: the three Workers' shapes as this repository
  ships them (`apps/worker-app/wrangler.toml` and the others; `scripts/workers-check.sh` checks they agree),
  pointed at `release/`, with the operator's database ID, domain and Restate public key;
- `legal/terms.md` and `legal/privacy.md`, which the deploy copies into the app's static files;
- `.prod.vars.example`: the app Worker's secrets, set on the Worker once, never committed (`.prod.vars` is
  git-ignored, as is `release/`);
- `.github/workflows/deploy.yaml` and `restate-cleanup.yaml`.

`deploy.sh` reads the three configurations with a small line parser: the keys it reads (each file's header lists
them) are written as `key = "value"` on one line, as the template has them, and `[env.*]` tables are ignored. It
refuses configurations that don't fit the release or each other: `main`, the assets and the migrations inside
`release/`; one `database_id` in all three; the rooms Worker defining `RAFFLE_ROOM` with no address, the other two
binding it by its name; one jurisdiction; three different names; the engine on workers.dev with identity keys; and
no placeholder left (the zero database ID, an identity key that isn't `publickeyv1_…`).

## The workflows

Both are the installation's own, thin, and name the Rafi repository once (`RAFI_REPOSITORY`). Their environment:

| | |
|---|---|
| `RAFI_REPOSITORY` | `sagikazarmark/rafi`: where releases come from, and whose `release.yaml` must have attested them |
| `RAFI_DEPLOY_VERSION` | The deploy interface the workflows are written for; `deploy.sh` refuses to run under another than its release's `DEPLOY_VERSION` ([Upgrading](#upgrading)) |
| `RAFI_INSTANCE` | `${{ github.repository }}`: the installation's name in Restate's deployment metadata |

Their secrets: `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` ([the token](#the-api-token)),
`RESTATE_ADMIN_URL` (`https://<env-id>.env.<region>.restate.cloud:9070`) and `RESTATE_AUTH_TOKEN` (an Admin-role
API key, [below](#the-restate-keys)). Each job gets only the ones its subcommand uses. The jobs need `contents:
read` only (the release and its attestation are public), run on `ubuntu-latest` with Node.js 24 (wrangler and the
Restate CLI come through `npx`, at the versions `deploy.sh` pins: wrangler 4.148.0, `@restatedev/restate` 1.8.1),
and can each name a GitHub environment (`environment: production`, commented out) for required reviewers or its
secrets.

**`deploy.yaml`** runs on a push to `main` that changes `VERSION`, `legal/**` or one of the three configurations,
and by hand (**Actions > Deploy > Run workflow**, `gh workflow run deploy.yaml`). Its jobs, each on a fresh runner:

| Job | Runs | Needs |
|---|---|---|
| **Prepare the release** | reads `VERSION`; `gh release download v<version>` of `worker.tar.gz`; `gh attestation verify` with `--signer-workflow github.com/<RAFI_REPOSITORY>/.github/workflows/release.yaml`, `--source-ref refs/tags/v<version>` and `--deny-self-hosted-runners`; unpacks it into `release/`; `deploy.sh check`; hands the tarball on as an artifact with its sha256 | nothing secret |
| **D1 migrations** | `deploy.sh migrate` | Cloudflare |
| **Rooms Worker** | `deploy.sh deploy-rooms` | Cloudflare |
| **App Worker (upload)** | `deploy.sh upload-app`; outputs `app_version_id` or `app_promoted` | Cloudflare |
| **Engine Worker** (beside the app's) | `deploy.sh deploy-engine`; outputs `engine_worker`, `engine_url` | Cloudflare |
| **Register the engine, deploy the app** | `deploy.sh register-engine`, then `deploy.sh promote-app` | Restate, the account ID; Cloudflare |

Every later job checks the artifact against prepare's sha256 before unpacking it, so all of them deploy the bytes
prepare verified. The order is the point: the migrations before any code that needs them, the class before the
Workers binding it, and the engine registered before the app that calls it switches over, so the app never calls a
handler the engine Restate runs lacks.

**`restate-cleanup.yaml`** runs daily at 04:37 UTC and by hand: a prepare job like deploy's, then `deploy.sh
cleanup` ([below](#the-restate-cleanup)).

The two share the concurrency group `rafi` without cancelling: one deploy or cleanup at a time, a run waiting for
the one in progress. GitHub keeps one waiting run per group, so a newer run replaces a waiting one, which shows as
cancelled (a scheduled cleanup queued behind a running deploy can cancel another deploy waiting there): run it
again. Scheduled workflows run only on the default branch, and in a public repository GitHub disables them after
60 days without repository activity.

The run's log in the installation's **Actions** is the deploy's log, each step printed.

## What each subcommand does

`release/deploy.sh <subcommand>`, from the installation's root, with the release unpacked into `release/`. Its
header ([`scripts/cloudflare-deploy.sh`](../../scripts/cloudflare-deploy.sh)) is the reference; in short:

| Subcommand | Does | Outputs |
|---|---|---|
| `check` | The release's `VERSION` is the installation's (and `RAFI_VERSION`'s, when set); its `DEPLOY_VERSION` is `deploy.sh`'s and `RAFI_DEPLOY_VERSION`; the configurations fit (above). Every subcommand does this first. Nothing remote. | |
| `migrate` | `wrangler d1 migrations apply DB --remote` | |
| `deploy-rooms` | Fingerprints the rooms Worker (sha256 over its bundle and `rooms.wrangler.toml`, as the tag `rooms-<32 hex>`) and reads what the Worker serves (its latest deployment's versions and their `workers/tag` annotations). One version at 100 % with this tag: nothing deployed, the live channels keep their connections. Anything else (no Worker, another tag, a gradual deployment): `wrangler deploy --tag <it>`, which applies the class's migrations. | |
| `upload-app` | Copies `legal/` into the app's static files. Refuses an app Worker that doesn't exist (its secrets come first). If a release has run on it (a version tagged `v<version>`, deployed or among its recent versions): `wrangler versions upload --tag v<version>`, not serving yet. Otherwise, a first install: `wrangler deploy`. | `app_version_id`, or `app_promoted=true` |
| `deploy-engine` | Looks up the account's workers.dev subdomain, then `wrangler deploy --name rafi-engine-v<version> --tag v<version>`, every time (the same release is the same code). | `engine_worker`, `engine_url` |
| `register-engine` | Refuses a downgrade: if the deployment Restate routes new `Raffle` and `EngineInfo` invocations to is this installation's (its metadata) and of a newer release, it stops. Then `POST /deployments` of the engine's URL, `"force": false`, with the metadata `app` (`rafi`), `instance`, `version`, `cloudflare_account` and `worker`; 200 (Restate had it) and 201 succeed, a 5xx or no answer is retried five times (a Worker deployed a moment ago), anything else fails. | |
| `promote-app` | After a first install, nothing. Otherwise `wrangler versions deploy <id>@100%`, then `wrangler triggers deploy` (the custom domain and the Cron, which a version upload doesn't apply). | |
| `cleanup` | [Below](#the-restate-cleanup). | |
| `dry-run` | `check` (placeholders only warned about), `legal/` copied in, `wrangler deploy --dry-run` of the three Workers. No credentials. | |

Outputs go to `GITHUB_OUTPUT` as `key=value` lines when it is set, and to standard output otherwise; the later
subcommands take them as `RAFI_APP_VERSION_ID`, `RAFI_APP_PROMOTED`, `RAFI_ENGINE_WORKER` and `RAFI_ENGINE_URL`. The
exit status is 0, 1 on a failure or refusal, 2 on a usage error. Tokens reach `curl` only on its standard input.

Every subcommand can run again, and a failed run is retried by running it again (**Re-run failed jobs**). What a
failed run leaves: migrations apply forward and stay; a rooms Worker deployed stays (a re-run skips it); an
uploaded app version not deployed is harmless; an engine Worker deployed but not registered serves nothing until a
re-run registers it; a registered engine stays the latest deployment even if the app's switch then failed (the
previous app calls the same handlers unless the release notes say otherwise), and a re-run finishes the switch.

## The Restate cleanup

`deploy.sh cleanup` lists Restate's deployments twice, with their metadata (the admin API's `GET /deployments`) and
with their status (`restate deployments list --extra --json`), and stops before deleting anything if either fails.
It touches only this installation's: metadata `app` `rafi`, `instance` `RAFI_INSTANCE`, `cloudflare_account` the
account, `worker` named `<engine name>-v…`, and its address that Worker's workers.dev one. For each:

- **Drained**: deletes its Worker (`DELETE /accounts/{account_id}/workers/scripts/<worker>`; already gone is
  fine), then removes the deployment (`restate deployments remove`, never forced: Restate refuses one that isn't
  drained). It carries on with the others when one fails, and fails at the end.
- **Draining**: lists the invocations still pinned to it (`/query`). A **paused** one (Restate stopped retrying
  it, as its retry policy says, and waits for a human) keeps it draining until someone resumes it on the latest
  deployment or kills it ([Upgrades](upgrades.md#on-workers-automatic)).
- **Active**: kept.

Deployments that aren't this installation's (another installation sharing the Restate environment, one registered
by hand) are listed and never touched. Neither are engine Workers Restate doesn't know.

**Don't rename or transfer the repository**: `RAFI_INSTANCE` is its name, so the engines registered under the old
one become another installation's to the cleanup (listed, never deleted) and to the downgrade check (no check).
Delete those engine Workers yourself once Restate reports them drained (`restate deployments remove <id>`, then
`npx wrangler delete <worker>`).

## The API token

An account API token (owned by the account rather than a user, so it survives its creator leaving; the README's
[step 2](../../deploy/cloudflare/README.md#2-a-cloudflare-api-token) creates it with `cf` or by hand), for the
account and the installation's zone:

| Permission (dashboard · API name) | What uses it |
|---|---|
| Account · Workers Scripts · Edit (`Workers Scripts Write`) | Uploading and deploying the three Workers (`PUT …/workers/scripts/{name}`, versions and deployments), their secrets, Cron (`…/schedules`), settings, workers.dev on or off (`…/subdomain`), the Durable Object class's migrations (part of the upload), the app's custom domain; reading deployments and versions (`deploy-rooms`, `upload-app`); the account's workers.dev subdomain (`GET /accounts/{id}/workers/subdomain`); deleting a drained engine Worker (`DELETE …/workers/scripts/{name}`) |
| Account · D1 · Edit (`D1 Write`) | `wrangler d1 migrations apply DB --remote` (the database's details and queries) |
| Zone · Workers Routes · Edit (`Workers Routes Write`), the installation's zone | The app Worker's hostname on the zone |

Sources: the "Accepted Permissions" of each endpoint in Cloudflare's API reference (the OpenAPI schema's
`x-api-token-group`): Workers Scripts Write for the scripts, their versions, deployments, secrets, schedules,
settings and subdomain, the account's workers.dev subdomain and the Workers custom-domain endpoints; D1 Write for a
database and its queries; Workers Routes Write for a zone's Workers routes. Wrangler attaches a custom domain through
an account-level Workers endpoint, and Cloudflare creates its DNS record and certificate on the zone; Cloudflare's
**Edit Cloudflare Workers** token template, which its CI guide recommends for wrangler, includes Workers Routes on
zones, and the token keeps it for the hostname. Not needed: Workers Tail Read (only `wrangler tail`, run by you),
Account Settings Read, Memberships and User Details (wrangler reads `/memberships` and `/user` only for `wrangler
whoami` and to explain an authentication error, and the account ID is given), KV, R2. That template plus Account ·
D1 · Edit is a superset, if you'd rather start from it.

Creating account tokens takes a Super Administrator, or a member with API token provisioning, and a token can't get
a permission its creator lacks. The API calls are List Permission Groups (`GET
/accounts/{account_id}/tokens/permission_groups`), List Zones (`GET /zones?name=`) and Create Token (`POST
/accounts/{account_id}/tokens`, with the policies: the account resource `com.cloudflare.api.account.<id>`, the zone
resource `com.cloudflare.api.account.zone.<id>`, and permission groups by ID); `cf` 1.0.0-beta.27 has them as `cf
accounts tokens permission-groups list`, `cf zones list` and `cf accounts tokens create`, and Roll Token (`PUT
/accounts/{account_id}/tokens/{token_id}/value`) as `cf accounts tokens roll`. Rotating it is in the
[runbook](runbook.md#rotating-secrets).

## The Restate keys

Restate Cloud's API keys each have a role.

- **The app's key**, `RAFI_ENGINE_API_KEY`, a secret on the app Worker: the app only invokes the engine through the
  ingress (`https://<env-id>.env.<region>.restate.cloud:8080`, `RAFI_ENGINE_INGRESS_URL`), never the admin API. Give
  it the narrowest role the console offers that can call the ingress; if none narrower than **Full** is listed,
  **Full**.
- **The workflows' key**, `RESTATE_AUTH_TOKEN`, a GitHub secret, with the **Admin** role, as Restate's deployment
  guides say for registering: `register-engine` reads `GET /services` and `GET /deployments` and calls `POST
  /deployments`; `cleanup` reads `GET /deployments`, `POST /query` (the pinned invocations), and runs the Restate CLI's
  `deployments list` and `deployments remove` (with `RESTATE_ADMIN_URL` and `RESTATE_AUTH_TOKEN` in its environment).

The engine's `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` is the environment's request identity public key
(`publickeyv1_…`, **Developers > Security > HTTP endpoints** in Restate Cloud), not a key to call Restate with.

## The app Worker's secrets

The installation's values are the app Worker's secrets, set once before the first deploy (`wrangler secret bulk
.prod.vars`, which creates the Worker holding only them; `upload-app` refuses a Worker that doesn't exist) and
changed with `wrangler secret put <NAME>`; deploys keep them. `wrangler.toml` lists them under `[secrets]
required`, and wrangler refuses to deploy the app while one is missing. What each is:
[`.prod.vars.example`](../../deploy/cloudflare/.prod.vars.example) and the
[configuration reference](../reference/configuration.md). The Worker refuses every request without its documents,
`RAFI_POW_KEY`, `RAFI_ENGINE_INGRESS_URL`, `CLERK_SECRET_KEY`, `CLERK_PUBLISHABLE_KEY`, the operator and the
document versions (`/healthz` answers `500 misconfigured`, naming the reason when that names no value). Release
builds reject dev auth and the mock chain. Never rotate `RAFI_POW_KEY` during an event.

## The legal documents and the operator

The Worker serves the installation's documents from `legal/`: `terms.md` (with the data processing agreement, which
organizers accept by using the service) and `privacy.md` (the operator's own notice, naming the subprocessors:
Cloudflare, Restate Cloud, Clerk, drand). They ship as engineering drafts marked as templates: fill in the bracketed
parts, have a lawyer review them, and delete the marker. A document that still carries the template marker makes
the Worker refuse every request. Their versions (`RAFI_TERMS_VERSION`, `RAFI_PRIVACY_VERSION`, e.g. `2026-11`) and
the operator's name and email (`RAFI_OPERATOR_NAME`, `RAFI_OPERATOR_EMAIL`) are secrets. A changed document is a
new version, shown with it, and deploys with a push of `legal/`; a change to the processing agreement is notified
to organizers directly, since nothing records who read which version. Your record of processing, and what changed
in the templates, are in [the operator's records](overview.md#the-operators-records). The retention settings
(`RAFI_RETENTION_DAYS` and the others, see the [configuration](../reference/configuration.md)) have defaults;
`RAFI_MIN_AGE` too; set them as secrets if you change them.

## Upgrading

Change `VERSION` and push, between events; never to an older release ([Upgrades](upgrades.md#database-migrations)):
the migrations only go forward, and `register-engine` refuses an engine older than the one Restate runs. The release
notes say when a configuration, `.prod.vars.example` or a secret changed shape, when the rooms Worker changed (that
deploy drops the live connections), and when `DEPLOY_VERSION` changed.

`DEPLOY_VERSION` is the interface between the workflows and the release's `deploy.sh`: its subcommands, their
environment and outputs, and what it expects of the installation's repository. The workflows say which one they are
written for (`RAFI_DEPLOY_VERSION`), and `deploy.sh` stops at `check` under any other, naming the template's
workflows at the new release's tag (with their URL). Copy `deploy.yaml` and `restate-cleanup.yaml` from
`deploy/cloudflare/.github/workflows` at that tag, then push `VERSION`; a release that keeps the interface needs
nothing of the workflows.

A release has one asset, `worker.tar.gz`; the release page shows its SHA256 digest (GitHub computes one for every
upload, and the release workflow checks it is the build's), and releases are immutable once published, so neither
the asset nor the tag under a version changes. The deploy verifies the attestation; by hand:

```sh
gh release download v<version> -R sagikazarmark/rafi -p worker.tar.gz
gh attestation verify worker.tar.gz -R sagikazarmark/rafi \
  --signer-workflow github.com/sagikazarmark/rafi/.github/workflows/release.yaml \
  --source-ref refs/tags/v<version> --deny-self-hosted-runners
```

## What only a real deploy can check

Nothing in this repository's CI holds a Cloudflare token: deploys run in installations' repositories. CI runs the
production shapes through `wrangler deploy --dry-run`, serves the app without secrets, runs `deploy.sh`'s
subcommands against a stubbed wrangler and Restate CLI and a fake Cloudflare and Restate API
(`scripts/cloudflare-deploy-test.sh`), and rehearses each release's `deploy.sh dry-run` from a copy of
`deploy/cloudflare` ([Testing](../development/testing.md#workers)). What only a real deploy can check, so check it
on the first one:

- the token's permissions cover every step (the custom domain, the workers.dev subdomain lookup, a deleted engine
  Worker included), and the Admin key every Restate call (registering, listing, `/query`, removing);
- the workflows read the release and its attestation with the repository's own `GITHUB_TOKEN`;
- the API shapes `deploy.sh` reads (deployments, versions and their annotations, the subdomain) are as wrangler
  4.148.0's source has them;
- the app Worker reaches Restate Cloud's ingress on its port (`:8080`): `workerd` allows any port, production has
  port rules for proxied hosts; Restate Cloud's `:443` alias is the fallback;
- the startup time `wrangler deploy` reports stays under the 400 ms limit for each Worker;
- the Durable Objects' EU jurisdiction (`ns.jurisdiction("eu")`) is reached only in production: from the app (a
  phone's first connect) and from the engine (the first draw's nudge), both through their bindings to the rooms
  Worker's class;
- Restate Cloud calls the engine's `/restate/*` on workers.dev over HTTP/2 (locally HTTP/1.1 only);
- the paid Workers plan: BLS verification and server-side rendering don't fit the free plan's CPU time;
- a second release's deploy registers its engine, and once the first engine has drained, the cleanup deletes its
  Worker and removes it from Restate; a deploy that doesn't change the rooms Worker keeps phones connected.

## From source

Instead of a release, build one here into your installation's `release/` and run its `deploy.sh` there, in the
workflows' order, with their environment (`RAFI_INSTANCE` your repository's `owner/name`, as the workflows have it,
so the cleanup and the downgrade check see the same installation):

```sh
RAFI_VERSION=<version> just cloudflare-artifact /path/to/your/installation/release
cd /path/to/your/installation
export RAFI_DEPLOY_VERSION=1 RAFI_INSTANCE=<owner>/<name> CLOUDFLARE_API_TOKEN=… CLOUDFLARE_ACCOUNT_ID=…
release/deploy.sh migrate
release/deploy.sh deploy-rooms
release/deploy.sh upload-app        # prints app_version_id=… (or app_promoted=true)
release/deploy.sh deploy-engine     # prints engine_worker=… and engine_url=…
RESTATE_ADMIN_URL=… RESTATE_AUTH_TOKEN=… RAFI_ENGINE_WORKER=… RAFI_ENGINE_URL=… release/deploy.sh register-engine
RAFI_APP_VERSION_ID=… release/deploy.sh promote-app    # or RAFI_APP_PROMOTED=true
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

## Troubleshooting

| Symptom | Cause, and what to do |
|---|---|
| Prepare fails at **Check it against this repository**: a placeholder | `wrangler.toml`, `engine.wrangler.toml` or `rooms.wrangler.toml` still has the zero `database_id`, or the identity key isn't `publickeyv1_…`: README steps 3 and 6. |
| … `database_id` isn't the app's, `script_name` isn't the rooms Worker's name, `RAFI_DO_JURISDICTION` differs | The three configurations disagree: one database, the rooms Worker's name as the others' `script_name`, one jurisdiction. |
| … `write <key> as <key> = "..." on one line` | A key `deploy.sh` reads is written in another TOML form: write it as the template does. |
| … the deploy interface: `copy .github/workflows/deploy.yaml and restate-cleanup.yaml from …` | The release's `DEPLOY_VERSION` changed: copy the two workflows from the template at that tag ([Upgrading](#upgrading)). |
| … `VERSION says …, but the release … is …` | `VERSION` isn't a version, or was edited during the run: fix it and run again. |
| Prepare fails at **Download the release** or **Verify its provenance** | No release `v<VERSION>` (a typo, or a version not released yet), or a tarball that Rafi's release workflow didn't build from that tag on GitHub's runners: never work around this. |
| **App Worker (upload)**: `the app Worker rafi doesn't exist on the account` | Its secrets come first: README step 8. |
| **App Worker**: wrangler names missing secrets | One of `[secrets] required` isn't set: `wrangler secret put <NAME>`. |
| **Engine Worker**: `the account's workers.dev subdomain: HTTP …` | The account has no workers.dev subdomain (register it on the dashboard's **Workers & Pages** page), or the token lacks Workers Scripts. |
| Any Cloudflare step: `Authentication error` (10000) or 403 | The token is missing a permission or the zone (above), expired, or rolled: replace the `CLOUDFLARE_API_TOKEN` secret. |
| **Register the engine**: `refusing to register an older engine (a downgrade)` | `VERSION` is older than the release Restate runs. Set it to that release or later; never roll back. |
| **Register the engine**: 401 or 403 | `RESTATE_AUTH_TOKEN` isn't an Admin-role key of this environment, or `RESTATE_ADMIN_URL` is another environment's (or the ingress, `:8080`). |
| **Register the engine**: Restate couldn't discover the engine (at once, or after five attempts) | Restate couldn't reach the engine (workers.dev not serving it yet), or the engine refused the discovery because the identity key in `engine.wrangler.toml` isn't this environment's: check the engine Worker's logs, fix, re-run the failed jobs. |
| A deploy shows as cancelled | A newer run replaced it while it waited (the concurrency group): run it again. |
| `/healthz` answers `500 misconfigured` | Missing secrets or documents, or a document still a template: the reason is in the answer when it names no value, and in the app Worker's logs. |
| The admin says the raffle engine can't be reached | `RAFI_ENGINE_INGRESS_URL` or `RAFI_ENGINE_API_KEY` wrong, or Restate Cloud down: the [runbook](runbook.md#when-the-raffle-engine-is-down-during-an-event). |
| Phones don't update live | The app's logs (`nudging … failed`), the rooms Worker's logs; `RAFFLE_ROOM` bound with `script_name` the rooms Worker's name. |
| The cleanup reports a deployment draining for days | A paused invocation: resume or kill it ([Upgrades](upgrades.md#on-workers-automatic)). |
| The cleanup doesn't run | Scheduled runs happen on the default branch only, and a public repository's are disabled after 60 days without activity: enable it again (**Actions > Restate cleanup**). |

Logs: each deploy and cleanup is a run in the installation's **Actions**; each Worker has its own Workers Logs (the
dashboard's **Workers & Pages >** the Worker **> Logs**, or `npx wrangler tail <Worker>`): `rafi`, `rafi-rooms`, and
each release's `rafi-engine-v…`. [Troubleshooting](troubleshooting.md) has the rest.

## Local development

`devenv --profile workers up` (see [Getting started](../development/getting-started.md#profiles), or
[without devenv](../development/getting-started.md#cloudflare-workers)), and `./scripts/workers-check.sh` in
[Testing](../development/testing.md#workers).
