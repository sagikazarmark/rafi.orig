# Cloudflare Workers

Rafi on Cloudflare: three Workers and one D1 database, with Restate Cloud running the raffle engine's invocations
and Clerk signing organizers in. This is the production deployment. An installation is a repository of the
operator's, a copy of [`deploy/cloudflare/`](../../deploy/cloudflare/), whose GitHub Actions workflows deploy the
Rafi release its `VERSION` names (or several installations in one repository, as Wrangler environments: [Several
installations](#several-installations)). The first deploy, step by step, is that directory's
[INSTALL.md](../../deploy/cloudflare/INSTALL.md), and its day-to-day running (upgrading, the run reports, the
cleanup, rotating keys, logs) its [README](../../deploy/cloudflare/README.md); this page is the reference behind
both: what runs where, what the deploy does and why, the permissions it needs, and what to do when something goes
wrong.

Cloudflare is one deployment, not an assumption of the code. The app and the engine are written against small
interfaces (the engine's `Store` and `Observer`, the live `Notifier`, the app's `RateLimit`), and `rafi-cloudflare`
implements them over Workers bindings.

## The three Workers

| | App | Engine | Live |
|---|---|---|---|
| Worker (the template's names) | `rafi-app` | `rafi-engine-v<version, dashed>` (`rafi-engine-v0-1-0`), one per release | `rafi-live` |
| Built from | `apps/worker-app` | `apps/worker-engine` | `apps/worker-live` |
| Configuration | `app.wrangler.toml` | `engine.wrangler.toml` (named `rafi-engine` there) | `live.wrangler.toml` |
| Address | your domain only (a custom domain; no workers.dev, no preview URLs) | `https://<it>.<subdomain>.workers.dev/restate` | none |
| Serves | the web app and its API, the static files, phones' live-channel sockets (gated, then handed to the raffle's object), `/healthz`; 404 under `/restate` | the Restate endpoint (`Raffle`, `EngineInfo`) under `/restate`; 404 elsewhere | 404 to every request: its objects are reached through the bindings |
| Bindings | `ASSETS`, `DB`, `RAFFLE_ROOM` (`script_name = "rafi-live"`), four rate limiters, Cron `0 * * * *` | `DB`, `RAFFLE_ROOM` (`script_name = "rafi-live"`) | `DB`, `RAFFLE_ROOM` (its own class `RaffleRoom`, migration `v1`) |
| Vars | optionally `RAFI_DO_JURISDICTION` | `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS`; optionally `RAFI_DO_JURISDICTION`, `RAFI_DRAND_RELAYS`, `RAFI_DRAND_LEAD_SECS`, `RAFI_DRAND_TIMEOUT_SECS` | optionally `RAFI_DO_JURISDICTION` |
| Secrets | the installation's ([below](#the-app-workers-secrets)) | none | none |

All three bind the one D1 database (the same `database_id`) and say the same `RAFI_DO_JURISDICTION` (unset by
default, the plain namespace; `eu` keeps the objects in the EU: [Data residency](#data-residency)). The names are
the template's; an installation may choose its own, and a second installation in the same account must
([Several installations](#several-installations)). Every variable, and which Worker reads it, is in the
[configuration reference](../reference/configuration.md#workers-only). Locally the three are one Worker,
`apps/worker` (`rafi-worker`), with the engine under `/restate` and the class beside the app: see
[Local development](#local-development).

### Why the engine is a Worker per release

Restate treats a deployment as immutable: it pins each invocation to the deployment it started on, sends new ones
to the latest, and, re-registering a URI it knows, keeps the handlers it discovered there the first time
([Upgrades](upgrades.md#registering-the-raffle-engine)). A draw sleeping until its drand round is an invocation in
flight, and it must finish on the code that started it. So each release's engine is a Worker of its own at an
address of its own, registered as a new deployment, and only ever running that release's code: a later deploy of
the same release redeploys it with the same code (with `engine.wrangler.toml`'s vars as they are then), and a
deploy of another release never touches it. The previous release's engine keeps serving what is pinned to it until
Restate reports it drained; the [cleanup](#the-restate-cleanup) then removes it.

While an older engine drains, it writes to the database the newer release migrated and nudges the live Worker's
objects, so a release's migrations stay compatible with the previous release's engine, and its notes say so when
they can't ([Releasing](../development/releasing.md#before-the-tag)).

### Why the live Worker is separate

The raffles' live channels are `RaffleRoom` Durable Objects, one per raffle, holding phones' sockets. Deploying the
Worker that defines a Durable Object class restarts its objects, which drops every connection; and a new class or
migration needs `wrangler deploy` (a version upload refuses a pending Durable Object migration). In a Worker of its
own, the class is deployed only when its code or `live.wrangler.toml` changed (the deploy compares a
fingerprint, below), so most releases keep every phone connected, and the app Worker, which has no class, is uploaded as a
version and switched to only once the engine is registered.

The objects' requests (`/nudge`, `/destroy`, the forwarded socket upgrade) are an interface between Workers
deployed apart: a new live Worker accepts the previous app's and engine's requests, so a change to it takes two
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
`worker.tar.gz`, built and attested by `.github/workflows/release.yaml`, holds `VERSION`, `app/` (the app Worker
and its static files), `engine/`, `live/`, `migrations/` (the D1 migrations) and `deploy.sh`
(`scripts/cloudflare-deploy.sh`), so a release deploys with its own logic ([Releasing](../development/releasing.md)).
The installation is the operator's repository, a copy of `deploy/cloudflare` at a release's tag:

- `VERSION`: the release it deploys (`VERSION.<environment>` for an installation that is a Wrangler environment,
  when that file exists);
- `app.wrangler.toml`, `engine.wrangler.toml`, `live.wrangler.toml`: the three Workers' shapes as this repository
  ships them (`apps/worker-app/wrangler.toml` and the others; `scripts/workers-check.sh` checks they agree),
  pointed at `release/`, with the operator's database ID, domain and Restate public key;
- `legal/terms.md` and `legal/privacy.md`, which the deploy copies into the app's static files;
- `package.json` (wrangler and the Restate CLI, `"wrangler": "^4"` and `"@restatedev/restate": "^1"`) and the
  `package-lock.json` the operator commits after `npm install`, which pins them: the workflows install from it
  (`npm ci`; `npm install` without it) and `deploy.sh` runs them with `npx -y`, which takes the installed ones;
- `.dev.vars.example`: the app Worker's secrets, filled in as `.dev.vars` and set on the Worker once
  (`wrangler secret bulk .dev.vars`), never committed (`.dev.vars` is git-ignored, as are `release/` and
  `node_modules/`; wrangler also reads `.dev.vars` for `wrangler dev`, which an installation doesn't run);
- `.github/workflows/deploy.yaml` and `restate-cleanup.yaml`;
- `INSTALL.md` (the first deploy) and `README.md` (the rest).

**The installation is its app Worker's name**, as wrangler's dry run of `app.wrangler.toml` reports it. Unique
in the account, it is the `instance` in the metadata each engine is registered with in Restate (`app` `rafi`,
`instance`, `version`, `cloudflare_account`, `worker`), and the downgrade check and the cleanup act only on
deployments whose metadata names this instance and this account.

`deploy.sh` doesn't parse the configurations, and checks nothing of the installation's. The version it deploys is
the release's own `VERSION` (the engine Worker's name and Restate's metadata are made of it): nothing compares it
with the installation's version file, which only tells the workflows which release to download. Nothing checks
that the release or the installation has its files (a missing one fails where wrangler meets it), and nothing looks
for the template's placeholders (the zero `database_id` fails at the migrations; the `publickeyv1_…` key deploys
an engine that refuses Restate, so its registration fails). It reads the Workers' names from wrangler itself:
`wrangler deploy --dry-run -c <file>` of each, for the Wrangler environment `CLOUDFLARE_ENV` names (unset or empty:
the top level), with `WRANGLER_OUTPUT_FILE_PATH`, whose `deploy` entry carries the `worker_name`. A dry run
validates the file as a deploy would, uploading nothing: one wrangler refuses (an environment it lacks, a
malformed file) stops the subcommand. Nor does anything check that the three files agree, which the operator keeps
true: the same `database_id` and `RAFI_DO_JURISDICTION`, the app's and the engine's `RAFFLE_ROOM` with the live
Worker's name as `script_name`, and `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` on the engine in every environment
(environments don't inherit vars). A mismatch deploys: an engine without its keys refuses Restate, so the
registration fails; the rest shows in the first test-mode raffle after an install (the phone doesn't follow the
draw, or the draw doesn't land). What `deploy.sh` does check before a deploy changes anything is
`check-remote`'s: the credentials work, and the release isn't older than the one Restate runs.

## Several installations

One account can hold several installations. Each has names of its own for its three Workers (so its own instance,
and its own engine Workers, `<engine name>-v…`), its own D1 database, domain, Clerk instance and app secrets, and
its own Restate environment: every Rafi engine registers the same services (`Raffle`, `EngineInfo`), and Restate
sends a service's new invocations to its latest deployment, so two installations in one environment would take
over each other's draws. The account, its workers.dev subdomain and the API token can be shared.

- **A repository each**: a copy of `deploy/cloudflare` per installation, with other names in its configurations.
  Nothing else differs.
- **One repository, Wrangler environments**: the configurations' top level is one installation, each
  `[env.<name>]` (in all three files) another, chosen by `CLOUDFLARE_ENV`, which `deploy.sh` reads and passes on
  to every wrangler call. An environment's Worker is `<top-level name>-<environment>` unless it names itself
  (`rafi-app-eu`, `rafi-live-eu`, `rafi-engine-eu-v0-1-0`); it inherits `main`, the static files, `workers_dev`,
  `preview_urls`, the Cron, the routes and the migrations, but not the vars, the bindings or the required
  secrets, which it repeats (with routes and rate-limit namespaces of its own). Its version file is
  `VERSION.<environment>` when that exists, `VERSION` otherwise. Each installation has its own copies of the two
  workflows, with `CLOUDFLARE_ENV` set, a concurrency group of its own, its jobs in a GitHub environment holding
  its secrets, and its push paths narrowed to its version file. The installations share `legal/`.

INSTALL.md's [Several installations](../../deploy/cloudflare/INSTALL.md#several-installations) has both, step
by step, with a whole `[env.eu]`.

## The workflows

Both are the installation's own, thin, and name the Rafi repository once (`RAFI_REPOSITORY`). Their environment:

| | |
|---|---|
| `RAFI_REPOSITORY` | `sagikazarmark/rafi`: where releases come from, and whose `release.yaml` must have attested them |
| `CLOUDFLARE_ENV` | Empty: the configurations' top level is the installation. In a copy for an installation that is a Wrangler environment, its name ([above](#several-installations)) |

Their secrets: `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` ([the token](#the-api-token)),
`RESTATE_ADMIN_URL` (`https://<env-id>.env.<region>.restate.cloud:9070`) and `RESTATE_AUTH_TOKEN` (an Admin-role
API key, [below](#the-restate-keys)). Each job gets only the ones its subcommand uses. The jobs need `contents:
read` only (the release and its attestation are public), run on `ubuntu-latest` with Node.js 24 and `jq`, install
wrangler and the Restate CLI from the installation's `package.json` first (`npm ci` with a committed
`package-lock.json`, `npm install --no-audit --no-fund` otherwise; `deploy.sh` runs them as `npx -y wrangler` and
`npx -y @restatedev/restate`), and can each name a GitHub environment (`environment: production`, commented out)
for required reviewers or its secrets.

**`deploy.yaml`** runs on a push to `main` that changes `VERSION`, `VERSION.*`, `legal/**` or a `*.wrangler.toml`,
and by hand (**Actions > Deploy > Run workflow**, `gh workflow run deploy.yaml`). Its jobs, each on a fresh runner:

| Job | Runs | Needs |
|---|---|---|
| **Prepare the release** | reads the version file (the release to download); `gh release download v<version>` of `worker.tar.gz`; `gh attestation verify` with `--signer-workflow github.com/<RAFI_REPOSITORY>/.github/workflows/release.yaml`, `--source-ref refs/tags/v<version>` and `--deny-self-hosted-runners`; unpacks it into `release/`; `deploy.sh check-remote`; hands the tarball on as an artifact with its sha256 | Restate, Cloudflare |
| **D1 migrations** | `deploy.sh migrate` | Cloudflare |
| **Live Worker** | `deploy.sh deploy-live` | Cloudflare |
| **App Worker (upload)** | `deploy.sh upload-app`; outputs `app_version_id` or `app_promoted` | Cloudflare |
| **Engine Worker** (beside the app's) | `deploy.sh deploy-engine`; outputs `engine_worker`, `engine_url` | Cloudflare |
| **Register the engine, deploy the app** | `deploy.sh register-engine`, then `deploy.sh promote-app` | Restate, the account ID; Cloudflare |

Every later job checks the artifact against prepare's sha256 before unpacking it, so all of them deploy the bytes
prepare verified. The order is the point: everything that can refuse the deploy first (a release without Rafi's
attestation, a configuration wrangler's dry run refuses, a release older than the one Restate runs, credentials
that don't work), before anything touches production; the migrations before any code that needs them, the class before the Workers binding it, and the
engine registered before the app that calls it switches over, so the app never calls a handler the engine Restate
runs lacks.

**`restate-cleanup.yaml`** runs daily at 04:37 UTC and by hand: a prepare job that downloads and verifies the
release as deploy's does and runs nothing of it, then `deploy.sh cleanup` ([below](#the-restate-cleanup)).

The two share a concurrency group (`rafi` in the template; one per installation when a repository holds several)
without cancelling: one deploy or cleanup at a time, a run waiting for the one in progress. GitHub keeps one waiting
run per group, so a newer run replaces a waiting one, which shows as cancelled (a scheduled cleanup queued behind a
running deploy can cancel another deploy waiting there): run it again. Scheduled workflows run only on the default
branch, and in a public repository GitHub disables them after 60 days without repository activity.

Each run's log in the installation's **Actions** is the deploy's log, each step printed, and its summary page holds
the subcommands' reports: the workflows point `deploy.sh`'s `RAFI_OUTPUT_FILE` at `$GITHUB_OUTPUT` (the jobs'
outputs) and its `RAFI_REPORT_FILE` at `$GITHUB_STEP_SUMMARY`. What the reports say is in the README's
[What each run reports](../../deploy/cloudflare/README.md#what-each-run-reports). Each job also sets
`RAFI_ARTIFACTS_DIR` (`rafi-artifacts/` in the workspace) and uploads it whatever happened (`if: always()`), as
the run's artifact `deploy-json-<job>-<attempt>` (`cleanup-json-<job>-<attempt>`), kept 14 days: the JSON
`deploy.sh` read and wrote ([below](#what-each-subcommand-does)).

## What each subcommand does

`release/deploy.sh <subcommand>`, from the installation's root, with the release unpacked into `release/`. Its
header ([`scripts/cloudflare-deploy.sh`](../../scripts/cloudflare-deploy.sh)) is the reference. Every subcommand
first reads the release's `VERSION` and, but for `migrate`, the Workers' names from `wrangler deploy --dry-run` of
each configuration for `CLOUDFLARE_ENV` ([above](#what-an-installation-is)); in short:

| Subcommand | Does | Outputs |
|---|---|---|
| `check-remote` | Before anything is deployed, the downgrade check `register-engine` makes (below): a release older than the one Restate runs is refused; nothing registered (a first install) passes. Then the token reads the account's workers.dev subdomain. Restate unreachable, or a key or token that doesn't work, fails it. Reads only. | |
| `migrate` | `wrangler d1 migrations apply DB --remote` (the app's configuration) | |
| `deploy-live` | Fingerprints the live Worker (sha256 over its bundle, `live.wrangler.toml`'s bytes and `CLOUDFLARE_ENV`, so any edit of the file, a comment or another environment's section too, redeploys it; the tag `live-<32 hex>`) and reads what the Worker serves (its latest deployment's versions and their `workers/tag` annotations). One version at 100 % with this tag: nothing deployed, the live channels keep their connections. Anything else (no Worker, another tag, a gradual deployment): `wrangler deploy --tag <it>`, which applies the class's migrations. | |
| `upload-app` | Copies `legal/` into the app's static files. Refuses an app Worker that doesn't exist (its secrets come first). If a release has run on it (a version tagged `v<version>`, deployed or among its recent versions): `wrangler versions upload --tag v<version>`, not serving yet. Otherwise, a first install: `wrangler deploy`. | `app_version_id`, or `app_promoted=true` |
| `deploy-engine` | Looks up the account's workers.dev subdomain, then `wrangler deploy --name <engine name>-v<version> --tag v<version>` (`rafi-engine-v0-1-0`), every time (the same release is the same code). | `engine_worker`, `engine_url` |
| `register-engine` | Refuses a downgrade (again, after `check-remote`, for a release registered since): if the deployment Restate routes new `Raffle` and `EngineInfo` invocations to is this installation's (its metadata) and of a newer release, it stops. Then `POST /deployments` of the engine's URL, `"force": false`, with the metadata `app` (`rafi`), `instance` (the app Worker's name), `version`, `cloudflare_account` and `worker`; 200 (Restate had it) and 201 succeed, a 5xx or no answer is retried five times (a Worker deployed a moment ago), anything else fails. | |
| `promote-app` | After a first install, nothing. Otherwise `wrangler versions deploy <id>@100%`, then `wrangler triggers deploy` (the custom domain and the Cron, which a version upload doesn't apply). | |
| `cleanup` | [Below](#the-restate-cleanup). | |
| `dry-run` | `legal/` copied in, then the three `wrangler deploy --dry-run` that read the names. No credentials, nothing remote. | |

Outputs are appended to `RAFI_OUTPUT_FILE` as `key=value` lines when it is set, and printed otherwise; the later
subcommands take them as `RAFI_APP_VERSION_ID`, `RAFI_APP_PROMOTED`, `RAFI_ENGINE_WORKER` and `RAFI_ENGINE_URL`.
`deploy-live`, `upload-app`, `deploy-engine`, `register-engine`, `promote-app` and `cleanup` print a Markdown
report of what they did, and append it to `RAFI_REPORT_FILE` when it is set. Every wrangler call gets
`CLOUDFLARE_ENV`. `deploy.sh` installs nothing: wrangler is `npx -y wrangler` and the Restate CLI `npx -y
@restatedev/restate` (`package.json`'s once installed; the latest when none is), or the commands `WRANGLER` and
`RESTATE` name. It reads every JSON answer with `jq`, and stops at once without it. With `RAFI_ARTIFACTS_DIR` set,
it keeps that JSON in `<dir>/<subcommand>/` (replaced on each run): wrangler's ND-JSON output of every call
(`wrangler-<what>.ndjson`: the dry runs, the deploys, the upload, the migrations), Restate's services and
deployments (`restate-services.json`, `restate-deployments.json`, the CLI's `restate-deployments-list.json`), the
registration's answer (`restate-register.json`), the account's Workers (`cloudflare-workers.json`) and the
cleanup's summary (`cleanup.json`: `removed`, `deleted`, `not_removed`, `draining` with the paused invocations,
`kept`, `left_behind`, `leftover`, `not_ours`, `failed`). The exit status is 0, 1 on a failure or refusal, 2 on a
usage error. Tokens reach `curl` only on its standard input, so none is among the artifacts.

Every subcommand can run again, and a failed run is retried by running it again (**Re-run failed jobs**). What a
failed run leaves: migrations apply forward and stay; a live Worker deployed stays (a re-run skips it); an
uploaded app version not deployed is harmless; an engine Worker deployed but not registered serves nothing until a
re-run registers it; a registered engine stays the latest deployment even if the app's switch then failed (the
previous app calls the same handlers unless the release notes say otherwise), and a re-run finishes the switch.

## The Restate cleanup

`deploy.sh cleanup` lists Restate's deployments twice, with their metadata (the admin API's `GET /deployments`) and
with their status (`restate deployments list --extra --json`), and the account's Workers (`GET
/accounts/{account_id}/workers/scripts`), and stops before removing or deleting anything if any listing fails. It
touches only this installation's deployments: metadata `app` `rafi`, `instance` the app Worker's name,
`cloudflare_account` the account, `worker` named `<engine name>-v…`, and its address that Worker's workers.dev one.
For each:

- **Drained**: first removes the deployment from Restate (`restate deployments remove`, never forced: Restate
  refuses one that isn't drained, and a refusal leaves its Worker untouched for the next run), then deletes its
  Worker (`DELETE /accounts/{account_id}/workers/scripts/<worker>`; 404, already gone, is fine). A deletion that
  fails leaves the Worker behind, reported; the cleanup carries on with the others and fails at the end. The
  deployment goes first so that Restate never holds a deployment whose Worker is gone.
- **Draining**: lists the invocations still pinned to it (`/query`), with its active and paused counts. A
  **paused** one (Restate stopped retrying it, as its retry policy says, and waits for a human) keeps it draining
  until someone resumes it on the latest deployment or kills it ([Upgrades](upgrades.md#on-workers-automatic)).
- **Active**: kept.

Deployments that aren't this installation's (one registered by hand, or with other metadata) are listed and never
touched. The account's `<engine name>-v<version>` Workers that no Restate deployment refers to (one left behind
by a failed deletion, or one a failed deploy never registered; this release's own excepted, which its deploy
registers) are reported with the command to delete them by hand (`npx -y wrangler delete <worker>`), and never
deleted by the cleanup: nothing tells it whether one is still needed.

The report (the run's summary) lists what was removed and deleted, what Restate refused to remove, what still
drains (with the paused invocations' IDs and the `restate invocations resume <id> --deployment latest` and `restate
invocations kill <id>` commands), the Workers left behind, and the deployments that aren't this installation's.
`cleanup.json`, among the run's artifacts, has the same in JSON.

**Don't rename the app Worker**: its name is the installation's `instance`, so the engines registered under the old
one become another installation's to the cleanup (listed, never touched) and to the downgrade check (no check).
Remove those yourself once Restate reports them drained (`restate deployments remove <id>`, then `npx -y wrangler
delete <worker>`). Renaming or transferring the repository changes nothing.

## The API token

An account API token (owned by the account rather than a user, so it survives its creator leaving; the README's
[step 2](../../deploy/cloudflare/INSTALL.md#2-a-cloudflare-api-token) creates it with `cf` or by hand), for the
account and the installation's zone (several installations in one account can share one, with Workers Routes on
each one's zone):

| Permission (dashboard · API name) | What uses it |
|---|---|
| Account · Workers Scripts · Edit (`Workers Scripts Write`) | Uploading and deploying the three Workers (`PUT …/workers/scripts/{name}`, versions and deployments), their secrets, Cron (`…/schedules`), settings, workers.dev on or off (`…/subdomain`), the Durable Object class's migrations (part of the upload), the app's custom domain; reading deployments and versions (`deploy-live`, `upload-app`); the account's workers.dev subdomain (`GET /accounts/{id}/workers/subdomain`); listing the account's Workers (`GET /accounts/{id}/workers/scripts`, the cleanup) and deleting a drained engine Worker (`DELETE …/workers/scripts/{name}`) |
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
  guides say for registering: `check-remote` and `register-engine` read `GET /services` and `GET /deployments`,
  and `register-engine` calls `POST /deployments`; `cleanup` reads `GET /deployments`, `POST /query` (the pinned
  invocations), and runs the Restate CLI's `deployments list` and `deployments remove` (with `RESTATE_ADMIN_URL`
  and `RESTATE_AUTH_TOKEN` in its environment).

The engine's `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` is the environment's request identity public key
(`publickeyv1_…`, **Developers > Security > HTTP endpoints** in Restate Cloud), not a key to call Restate with.

## The app Worker's secrets

The installation's values are the app Worker's secrets, set once before the first deploy (`wrangler secret bulk
.dev.vars -c app.wrangler.toml`, which creates the Worker holding only them; `upload-app` refuses a Worker that
doesn't exist) and changed with `wrangler secret put <NAME> -c app.wrangler.toml`; deploys keep them. For an
installation that is a Wrangler environment, either with `CLOUDFLARE_ENV=<environment>` (or `--env`).
`app.wrangler.toml` lists them under `[secrets] required` (an environment, under `[env.<name>.secrets]`), and
wrangler refuses to deploy the app while one is missing. What each is:
[`.dev.vars.example`](../../deploy/cloudflare/.dev.vars.example) and the
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

Change `VERSION` (an installation that is a Wrangler environment: its `VERSION.<environment>`, when it has one)
and push, between events; never to an older release ([Upgrades](upgrades.md#database-migrations)): the migrations
only go forward, and the deploy refuses a release older than the one Restate runs in its first job
(`check-remote`), before touching anything. The release notes say when a configuration, `.dev.vars.example` or a
secret changed shape, when the live Worker changed (that deploy drops the live connections), and when the
template's workflows changed.

The workflows drive the release's `deploy.sh` (its subcommands, their environment and outputs), and nothing checks
an installation's copies against the release: a release that changes how `deploy.sh` is driven says so in its
notes, and the operator copies `deploy.yaml` and `restate-cleanup.yaml` from `deploy/cloudflare/.github/workflows`
at that tag before pushing `VERSION` (redoing each copy's edits, with several installations in one repository). A
change to the template's configurations is the operator's to carry over, keeping their names, IDs and keys.

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

- the token's permissions cover every step (the custom domain, the workers.dev subdomain lookup, the account's
  Workers listed and a drained engine Worker deleted included), and the Admin key every Restate call (registering,
  listing, `/query`, removing);
- the workflows read the release and its attestation with the repository's own `GITHUB_TOKEN`, and the reports
  land on the run's summary page;
- the API shapes `deploy.sh` reads (deployments, versions and their annotations, the subdomain, the account's
  Workers) are as wrangler 4.148.0's source and Cloudflare's API reference have them, and the wrangler and Restate
  CLI `package.json` installs write the JSON it reads (the dry run's `deploy` entry, `deployments list --json`);
- the app Worker reaches Restate Cloud's ingress on its port (`:8080`): `workerd` allows any port, production has
  port rules for proxied hosts; Restate Cloud's `:443` alias is the fallback;
- the startup time `wrangler deploy` reports stays under the 400 ms limit for each Worker;
- the Durable Objects' jurisdiction, when `RAFI_DO_JURISDICTION` sets one (`ns.jurisdiction("eu")`), is reached
  only in production (workerd has none, so local development leaves it unset): from the app (a phone's first
  connect) and from the engine (the first draw's nudge), both through their bindings to the live Worker's class;
- Restate Cloud calls the engine's `/restate/*` on workers.dev over HTTP/2 (locally HTTP/1.1 only);
- the paid Workers plan: BLS verification and server-side rendering don't fit the free plan's CPU time;
- a second release's deploy registers its engine, and once the first engine has drained, the cleanup removes it
  from Restate and then deletes its Worker; a deploy that doesn't change the live Worker keeps phones connected;
- a second installation in the same account, as a Wrangler environment, deploys beside the first without touching
  its Workers, and each one's cleanup leaves the other's deployments alone.

## From source

Instead of a release, build one here into your installation's `release/` and run its `deploy.sh` there, in the
workflows' order, with their environment (`CLOUDFLARE_ENV` too, for an installation that is a Wrangler
environment):

```sh
RAFI_VERSION=<version> just cloudflare-artifact /path/to/your/installation/release
cd /path/to/your/installation
export CLOUDFLARE_API_TOKEN=… CLOUDFLARE_ACCOUNT_ID=…
RESTATE_ADMIN_URL=… RESTATE_AUTH_TOKEN=… release/deploy.sh check-remote
release/deploy.sh migrate
release/deploy.sh deploy-live
release/deploy.sh upload-app        # prints app_version_id=… (or app_promoted=true)
release/deploy.sh deploy-engine     # prints engine_worker=… and engine_url=…
RESTATE_ADMIN_URL=… RESTATE_AUTH_TOKEN=… RAFI_ENGINE_WORKER=… RAFI_ENGINE_URL=… release/deploy.sh register-engine
RAFI_APP_VERSION_ID=… release/deploy.sh promote-app    # or RAFI_APP_PROMOTED=true
```

The installation is the app Worker's name in your configuration, the same whether the workflows or you run it, so
the cleanup and the downgrade check see one installation. Run `npm install` in the installation first (its
`package.json`'s wrangler and Restate CLI, which `deploy.sh` runs with `npx -y`), or, in the devenv shell,
`WRANGLER=wrangler` uses its wrangler; `deploy.sh` needs `jq` either way. `RAFI_ARTIFACTS_DIR=<dir>` keeps the
JSON, as the workflows do.
`just cloudflare-artifact` is what the release workflow packs. The version names the engine Worker,
so a build of your own deploys an engine of its own; the rule against older builds holds the same.

## Data residency

By default the database and the Durable Objects are wherever Cloudflare places them. Keeping them in the EU is
optional and a first-deployment decision (INSTALL.md's [Keeping data in the
EU](../../deploy/cloudflare/INSTALL.md#keeping-data-in-the-eu)):

- **D1** created with `wrangler d1 create <name> --jurisdiction eu`: a database's jurisdiction is fixed when it
  is created, so a later move is a new database and a restore into it.
- **Durable Objects** from an EU-jurisdiction namespace: `RAFI_DO_JURISDICTION = "eu"` in all three configurations
  and in every `[env.<name>.vars]`, before the first deploy. Unset or empty, the code uses the plain namespace;
  set, `ns.jurisdiction(<it>)` (`crates/cloudflare/src/gate.rs`). An object's jurisdiction is fixed when its ID is
  created, so changing the setting later moves the live channels to new objects, the old ones left behind; and
  nothing compares the three files, so a mismatch shows as phones that don't follow the draw.
- **Restate Cloud**: an environment in the EU region, which keeps the engine's invocations and journals.

Not covered by any of it: the Workers run near each visitor, wherever that is, and handle the data they read
there; Clerk (organizers' accounts) is a US company, under the EU-U.S. Data Privacy Framework. The privacy
template's provider table has the hosting's location and Restate's region as bracketed choices. Keeping where
Workers run inside a region is Cloudflare's
[Regional Services](https://developers.cloudflare.com/data-localization/regional-services/), an Enterprise add-on
(a hostname's requests decrypted and handled, Workers scripts among what it lists, only in the chosen region's data
centers); Rafi neither needs nor configures it, and what it would cover of these Workers (the engine is on
workers.dev, not the installation's zone) is Cloudflare's to say.

**Placement** is a latency option, not a residency control: `app.wrangler.toml` has a commented-out
`[placement]` block with its two choices, which exclude each other, `mode = "smart"` (Cloudflare picks a faster
location from the Worker's own traffic) and `region = "aws:eu-central-1"` (the data center nearest that cloud
region; Restate Cloud's, say), so the app's request handling runs near the D1 database and Restate Cloud instead of
near each visitor ([INSTALL.md, Placement](../../deploy/cloudflare/INSTALL.md#placement)). From
[Cloudflare's placement documentation](https://developers.cloudflare.com/workers/configuration/placement/): Workers
run on Cloudflare's global network, not inside cloud provider regions; placement only affects the fetch handler
(not the app's hourly Cron); static assets are always served from the location nearest the visitor; and at very
high request volumes Cloudflare may run instances across a more distributed area. The live Worker has nothing to
set (Durable Objects are placed automatically), and neither has the engine: only Restate Cloud calls it, so it
already runs near Restate.

Either way:

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
| Any job but the migrations, at once: `wrangler deploy --dry-run -c <file> … failed (above)` | wrangler refuses that configuration (its error is above it in the log): a malformed file, or, with `CLOUDFLARE_ENV` set, an environment the file has no `[env.<name>]` for (add it to all three, or fix the name). |
| … `reported no Worker's name` | wrangler's dry run wrote no `deploy` entry with a Worker's name into its ND-JSON output: a wrangler too old for it, or a name that isn't a Worker's (a-z, 0-9, -). The `wrangler-dry-run-<worker>.ndjson` artifact has what it wrote. |
| **Read the version file**: `… not a Rafi release` | The version file (`VERSION`, or `VERSION.<environment>`) isn't a version (`x.y.z`, or `x.y.z-pre.release`): fix it and push. Nothing else compares it with the release: `deploy.sh` deploys the release it came in. |
| **D1 migrations** fails on the database `00000000-0000-0000-0000-000000000000` | A configuration still has the template's zero `database_id` (nothing looks for it): [INSTALL.md step 3](../../deploy/cloudflare/INSTALL.md#3-the-database), the ID in all three files. |
| Any job, at once: `jq isn't installed` | `deploy.sh` reads every JSON answer with `jq` (GitHub's runners have it): install it where you run it. |
| **Install wrangler and the Restate CLI** fails | `npm ci` refuses a `package-lock.json` that doesn't match `package.json`: run `npm install` and commit the lockfile. Otherwise npm couldn't reach its registry: run it again. |
| Prepare fails at **Download the release** or **Verify its provenance** | No release `v<VERSION>` (a typo, or a version not released yet), or a tarball that Rafi's release workflow didn't build from that tag on GitHub's runners: never work around this. |
| **App Worker (upload)**: `the app Worker rafi-app doesn't exist on the account` | Its secrets come first: [INSTALL.md step 8](../../deploy/cloudflare/INSTALL.md#8-the-app-workers-secrets) (with `CLOUDFLARE_ENV=<name>` for an environment's Worker). |
| **App Worker**: wrangler names missing secrets | One of `[secrets] required` isn't set: `wrangler secret put <NAME> -c app.wrangler.toml`. |
| **Engine Worker**: `the account's workers.dev subdomain: HTTP …` | The account has no workers.dev subdomain (register it on the dashboard's **Workers & Pages** page), or the token lacks Workers Scripts. |
| Any Cloudflare step: `Authentication error` (10000) or 403 | The token is missing a permission or the zone (above), expired, or rolled: replace the `CLOUDFLARE_API_TOKEN` secret. |
| **Prepare the release** at **Check it against Restate and Cloudflare**: `refusing to deploy an older release (a downgrade)` | The version file is older than the release Restate runs; nothing was deployed. Set it to that release or later; never roll back. |
| **Prepare the release** at **Check it against Restate and Cloudflare**: `no answer from …`, 401 or 403 | `RESTATE_ADMIN_URL` isn't the environment's admin API (`:9070`), `RESTATE_AUTH_TOKEN` isn't an Admin-role key of it, or the Cloudflare token or account ID is wrong (the workers.dev subdomain lookup): fix the secret, run it again. |
| **Register the engine**: `refusing to register an older engine (a downgrade)` | A newer release was registered after this run's first job checked: set the version file to that release or later; never roll back. |
| **Register the engine**: 401 or 403 | `RESTATE_AUTH_TOKEN` isn't an Admin-role key of this environment, or `RESTATE_ADMIN_URL` is another environment's (or the ingress, `:8080`). |
| **Register the engine**: Restate couldn't discover the engine (at once, or after five attempts) | Restate couldn't reach the engine (workers.dev not serving it yet), or the engine refused the discovery because the identity key in `engine.wrangler.toml` isn't this environment's (still the template's `publickeyv1_…`, say: nothing looks for it; or, in a Wrangler environment, its `[env.<name>.vars]` has none: vars aren't inherited): check the engine Worker's logs, fix, re-run the failed jobs. |
| A deploy shows as cancelled | A newer run replaced it while it waited (the concurrency group): run it again. |
| `/healthz` answers `500 misconfigured` | Missing secrets or documents, or a document still a template: the reason is in the answer when it names no value, and in the app Worker's logs. |
| The admin says the raffle engine can't be reached | `RAFI_ENGINE_INGRESS_URL` or `RAFI_ENGINE_API_KEY` wrong, or Restate Cloud down: the [runbook](runbook.md#when-the-raffle-engine-is-down-during-an-event). |
| Phones don't update live | The app's logs (`nudging … failed`), the live Worker's logs; `RAFFLE_ROOM` bound with `script_name` the live Worker's name; the same `database_id` and `RAFI_DO_JURISDICTION` in all three configurations (nothing compares them: the app and the engine in different jurisdictions reach different objects). |
| The cleanup reports a deployment draining for days | A paused invocation: resume or kill it ([Upgrades](upgrades.md#on-workers-automatic)). |
| The cleanup fails: `… couldn't be listed: nothing removed or deleted`, or `listing Restate's deployments: …` | A listing failed (the token, the Restate key or admin URL, or either service down), so it touched nothing: fix the secret, or run it again later. |
| The cleanup fails `in part`: **Not removed** in its report | Restate refused to remove a deployment it had reported drained (its answer is in the job's log): its Worker is untouched, and the next run tries again. |
| The cleanup fails `in part`: **Workers left behind** | A deployment was removed but deleting its Worker failed (the token, or Cloudflare): nothing refers to that Worker any more, so later runs only report it. Delete it by hand with the command in the report. |
| The cleanup reports a Worker left behind that no deployment refers to | An engine Worker of this installation's name that Restate doesn't know: one a failed deletion left, or an earlier deploy deployed and never registered. Delete it by hand once you're sure nothing needs it (`npx -y wrangler delete <worker>`). |
| The cleanup doesn't run | Scheduled runs happen on the default branch only, and a public repository's are disabled after 60 days without activity: enable it again (**Actions > Restate cleanup**). |

Logs: each deploy and cleanup is a run in the installation's **Actions**, its summary page with the reports; each
Worker has its own Workers Logs (the dashboard's **Workers & Pages >** the Worker **> Logs**, or `npx -y wrangler
tail <Worker>`): `rafi-app`, `rafi-live`, and each release's `rafi-engine-v…` (an installation's own names, if it chose
them). [Troubleshooting](troubleshooting.md) has the rest.

## Local development

`devenv --profile workers up` (see [Getting started](../development/getting-started.md#profiles), or
[without devenv](../development/getting-started.md#cloudflare-workers)), and `./scripts/workers-check.sh` in
[Testing](../development/testing.md#workers).
