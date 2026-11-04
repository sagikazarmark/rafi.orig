# First deploy

From nothing to one installation of [Rafi](https://github.com/sagikazarmark/rafi) on your Cloudflare account, with
`/healthz` answering. You do this once, in order. Each step says what you'll get and where it goes.

- After this page: the [README](README.md) (upgrading, deploying, the cleanup, secrets, logs).
- The details and the reasons: [docs/operations/cloudflare.md][guide], "the guide" below.

Two things can't be changed in place after the first deploy: whether the data stays in the EU
([Keeping data in the EU](#keeping-data-in-the-eu)), and the installation's names. This page deploys one
installation, named by its app Worker, `rafi-app`. A second one in the same account needs names of its own
([Several installations](#several-installations)).

## What you'll need

- [ ] A **Cloudflare account on the paid Workers plan**: verifying drand and rendering the participant page don't
  fit the free plan's CPU time.
- [ ] A **zone** (your domain) on that account.
- [ ] The account's **workers.dev subdomain**. Each release's engine Worker is served there, at
  `https://rafi-engine-v<version>.<subdomain>.workers.dev`. The dashboard's **Workers & Pages** shows it (**Your
  subdomain**), or offers one on your first visit. **Don't change it once deployed** ([why][guide-workers-dev]).
- [ ] The right to **create account API tokens**: a Super Administrator, or a member with API token provisioning.
- [ ] Accounts at [Restate Cloud](https://cloud.restate.dev), [Clerk](https://clerk.com) and GitHub.
- [ ] Your **legal documents**, reviewed by a lawyer (step 7), and the operator's (your) name and email.
- [ ] On your machine:
  - the [GitHub CLI](https://cli.github.com/manual/), `gh`, signed in to GitHub
    ([`gh auth login`](https://cli.github.com/manual/gh_auth_login));
  - [Node.js](https://nodejs.org) 22.18 or later (`npm`, `npx`);
  - `jq`, `openssl`, `curl` and `git`.

  wrangler and the Restate CLI come with your repository (its `package.json`): step 1 installs them, then logs
  wrangler in to your Cloudflare account ([`wrangler login`][wrangler-login]).

Below, `<you>/<name>` is your new repository, `example.org` your zone and `raffle.example.org` the app's hostname.
From step 1's `cd` on, every command runs in your repository's directory: `gh` takes the repository from
it, wrangler its configurations.

## What goes where

| Value | You get it in | It goes to |
|---|---|---|
| Account ID | step 1 (`npx wrangler whoami`) | GitHub secret `CLOUDFLARE_ACCOUNT_ID` (step 2) |
| API token | step 2 | GitHub secret `CLOUDFLARE_API_TOKEN` |
| Database ID | step 3 | `database_id` in `wrangler.app.toml`, `wrangler.engine.toml` and `wrangler.live.toml` |
| Hostname, `raffle.example.org` | step 4 | `routes` in `wrangler.app.toml` |
| Public URL, `https://raffle.example.org` | step 4 | app secret `RAFI_PUBLIC_URL` |
| Clerk's keys, `pk_live_…` and `sk_live_…` | step 5 | app secrets `CLERK_PUBLISHABLE_KEY` and `CLERK_SECRET_KEY` |
| Restate's ingress URL (`:8080`) | step 6 | app secret `RESTATE_INGRESS_URL` |
| Restate API key for the app | step 6 | app secret `RESTATE_INGRESS_API_KEY` |
| Restate's admin URL (`:9070`) | step 6 | GitHub secret `RESTATE_ADMIN_URL` |
| Restate API key with the Admin role | step 6 | GitHub secret `RESTATE_AUTH_TOKEN` |
| Restate's public key, `publickeyv1_…` | step 6 | `RESTATE_IDENTITY_KEYS` in `wrangler.engine.toml` |
| Terms and privacy notice | step 7 | `legal/terms.md` and `legal/privacy.md` |
| Proof-of-work key | step 8 | app secret `RAFI_POW_KEY` |
| Operator's name and email | you | app secrets `RAFI_OPERATOR_NAME` and `RAFI_OPERATOR_EMAIL` |

The files are committed. App secrets are set on the app Worker in step 8 and never committed. GitHub secrets are
set in steps 2 and 9.

## Keeping data in the EU

Optional. Decide before the first deploy: none of this can be changed in place afterwards. By default Cloudflare
places the database and the live channels where it sees fit. To keep them, and Restate's copy of the draws, in
the EU:

| What | How | When |
|---|---|---|
| The database | `npx wrangler d1 create rafi --jurisdiction eu` | Step 3. Fixed when it is created: moving it later is a new database and a [restore][runbook] |
| The live channels (Durable Objects) | Uncomment `RAFI_DO_JURISDICTION = "eu"` in all three `wrangler.*.toml`, and in every `[env.<name>.vars]` if you use [Wrangler environments](#one-repository-wrangler-environments) | Before the first deploy. Keep the three files the same: nothing compares them |
| Restate Cloud (the draws) | Create the environment in the EU region | Step 6 |

Not covered: the Workers run near each visitor, wherever that is, and [Clerk](https://clerk.com) (organizers'
accounts) is a US company, under the EU-U.S. Data Privacy Framework. Your privacy notice
([`legal/privacy.md`](legal/privacy.md)) names the hosting's location as a bracketed choice: keep the true one.
More, and Cloudflare's Regional Services: the guide's [Data residency][guide-residency].

### Placement

Optional, for latency, not data residency. Change it whenever you like. By default the app Worker handles each
request in the data center nearest the visitor, a round trip away from the database and from Restate Cloud. To run
it near them instead, uncomment `[placement]` in `wrangler.app.toml` and **one** of its two lines:

- `mode = "smart"`: Cloudflare moves it to a faster location, if its traffic shows one;
- `region = "aws:eu-central-1"`: near that cloud region (your Restate Cloud environment's, say).

What it moves and what it doesn't: the guide's [Data residency][guide-residency].

## 1. Create your repository

You'll get: your private repository, created from the [template repository][template] (Rafi's `deploy/cloudflare`
at 0.1.0), with `package-lock.json`; wrangler logged in; and your Cloudflare account's ID.

1. Create the repository from the template, and clone it:

   ```sh
   gh repo create <you>/<name> --private --template sagikazarmark/rafi-deploy-template --clone
   cd <name>
   ```

   Or in GitHub's web UI: on the [template repository][template], **Use this template > Create a new
   repository**, **Private**, then `gh repo clone <you>/<name>` and `cd <name>`.
2. Install wrangler and the Restate CLI, and commit the lockfile:

   ```sh
   npm install
   git add package-lock.json
   git commit -m "Lock wrangler and the Restate CLI"
   ```

   Every run, and yours, then uses the same wrangler and Restate CLI. **Don't push yet**: see below.
3. Log wrangler in to Cloudflare, and note your account's ID:

   ```sh
   npx wrangler login     # in the browser
   npx wrangler whoami    # your accounts, each with its Account ID
   ```

The repository's branch is `main`, as the template's: the workflows run there.

**The Deploy workflow has already run once.** Creating a repository from a template counts as a push for GitHub
Actions ([GitHub's `push` event][gh-push-event]), and that first commit adds `VERSION`, `legal/` and the
configurations, which Deploy runs on. Until step 9 has set its secrets, a run fails harmlessly in its first job, at
**Check it against Restate and Cloudflare**, before it touches anything (the daily Restate cleanup fails the same
way). Once the secrets are set, a run with the configuration unfinished goes as far as it gets: nothing looks for
the template's placeholders, so the zero database ID fails at the migrations, and an app Worker without its secrets
(step 8) is refused at its upload, after the live Worker and the engine are deployed. So:

- keep your commits local and push in step 10 (the steps in between need the repository on GitHub, not its
  content); or
- disable both workflows until step 10 (**Actions**, the workflow, **⋯ > Disable workflow**).

## 2. A Cloudflare API token

You'll get: the GitHub secrets `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.

The workflows deploy with an **account API token**. The account owns it, not you, so it keeps working whoever
leaves. Only a Super Administrator, or a member with API token provisioning, can create one, and it gets no
permission its creator lacks.

1. In the Cloudflare dashboard, **Manage account > Account API tokens > Create Token**, a custom token (Cloudflare's
   [Account API tokens][cf-account-tokens] and [Create API token][cf-create-token] have the steps). Name it after
   your repository, `rafi deploy (<you>/<name>)` say, and give it these permissions and nothing else
   ([what each is for][guide-token]):

   | Permission | Resources |
   |---|---|
   | Account · Workers Scripts · Edit | your account |
   | Account · D1 · Edit | your account |
   | Zone · Workers Routes · Edit | your zone (**Specific zone**, `example.org`) |

2. Cloudflare shows the token once. Hand it to GitHub, with step 1's account ID:

   ```sh
   gh secret set CLOUDFLARE_API_TOKEN                        # paste the token at the prompt
   gh secret set CLOUDFLARE_ACCOUNT_ID --body <account ID>
   ```

3. Optionally, check it. The answer is your workers.dev subdomain, `{"subdomain": "…"}`; an error if the token
   lacks Workers Scripts or the account has no subdomain.

   ```sh
   read -rs token    # paste the token again, then Enter (it isn't shown)
   curl -sS "https://api.cloudflare.com/client/v4/accounts/<account ID>/workers/subdomain" \
     -H "Authorization: Bearer $token" | jq '.result // .errors'
   unset token
   ```

Lost the value before it reached GitHub? **Roll** the token in **Account API tokens** for a new one, and run
`gh secret set CLOUDFLARE_API_TOKEN` again. Never save it in a file.

## 3. The database

You'll get: the D1 database, and its ID in the three Wrangler configurations.

wrangler is logged in since step 1. If your login has more than one account, first
`export CLOUDFLARE_ACCOUNT_ID=<account ID>`: wrangler then uses that one (here and in step 8).

```sh
npx wrangler d1 create rafi    # prints its database_id; for the EU, add --jurisdiction eu
```

In `wrangler.app.toml`, `wrangler.engine.toml` and `wrangler.live.toml`, set `database_id` to the ID it printed.

- All three Workers use this one database, so the three files carry the same ID. Nothing compares them: a file
  left on another database shows only in step 10's test-mode raffle.
- A [jurisdiction](#keeping-data-in-the-eu) can only be chosen here, when the database is created.

## 4. Your domain

You'll get: the app's hostname in `wrangler.app.toml`, and the value of `RAFI_PUBLIC_URL` (step 8).

1. Pick the hostname organizers and phones will use, on your zone, with no DNS record of its own yet. Each deploy
   attaches it to the app Worker as a custom domain: Cloudflare creates its DNS record and certificate. A
   workflow takes over an existing record or custom domain of that name without asking.
2. In `wrangler.app.toml`, uncomment `routes` and set your hostname:

   ```toml
   routes = [{ pattern = "raffle.example.org", custom_domain = true }]
   ```

3. `RAFI_PUBLIC_URL` is that origin with no trailing slash, `https://raffle.example.org`. QR codes and links use it,
   and Clerk must allow it. The app has no other address (`workers_dev` and preview URLs are off).

## 5. Clerk

You'll get: `CLERK_PUBLISHABLE_KEY` and `CLERK_SECRET_KEY` (step 8), and Clerk's DNS records in your zone.

1. In the [Clerk dashboard](https://dashboard.clerk.com), create an application, then its **production instance**
   for your domain.
2. Add each DNS record Clerk lists (CNAMEs for its Frontend API, the accounts portal and email) to your zone in
   Cloudflare (**DNS > Records**), with **Proxy status** *DNS only*.
3. Wait for Clerk to verify them and issue its certificates (**Domains** in Clerk shows the status). It can take a
   while; sign-in works after that.
4. The production instance's **API keys**: `pk_live_…` is `CLERK_PUBLISHABLE_KEY`, `sk_live_…` is
   `CLERK_SECRET_KEY`.

## 6. Restate Cloud

You'll get: the app's `RESTATE_INGRESS_URL` and `RESTATE_INGRESS_API_KEY` (step 8), the GitHub secrets
`RESTATE_ADMIN_URL` and `RESTATE_AUTH_TOKEN` (step 9), and the public key in `wrangler.engine.toml`.

In [Restate Cloud](https://cloud.restate.dev), create an environment for this installation, in the EU region if the
data stays there ([Keeping data in the EU](#keeping-data-in-the-eu); the region is part of every URL below). On
its **Developers** page:

| What | Where | Goes to |
|---|---|---|
| The ingress URL, `https://<env-id>.env.<region>.restate.cloud:8080` (`<env-id>` without `env_`) | the URLs | app secret `RESTATE_INGRESS_URL` |
| The admin URL, the same host on port 9070 | the URLs | GitHub secret `RESTATE_ADMIN_URL` |
| An API key for the app, with the narrowest role that can call the ingress; if none narrower than **Full** is listed, **Full** | **API Keys > Create API Key** | app secret `RESTATE_INGRESS_API_KEY` |
| An API key with the **Admin** role, for the workflows | **API Keys > Create API Key** | GitHub secret `RESTATE_AUTH_TOKEN` |
| The request identity's public key, `publickeyv1_…` | **Security > HTTP endpoints** | `wrangler.engine.toml` |

In `wrangler.engine.toml`, set `RESTATE_IDENTITY_KEYS` to the public key:

```toml
RESTATE_IDENTITY_KEYS = "publickeyv1_…"
```

- The public key is public, so it's committed. The engine refuses every request not signed with its private key.
- Two keys, two roles: the app only calls the engine through the ingress, while the workflows register each
  release's engine and remove drained ones ([the keys][guide-restate-keys]).
- Nothing is registered by hand: each deploy registers its release's engine with Restate.

## 7. Your legal documents

You'll get: `legal/terms.md` and `legal/privacy.md` as yours, each with its version.

The app serves your terms (with the data processing agreement organizers accept by using the service) and your
privacy notice from [`legal/terms.md`](legal/terms.md) and [`legal/privacy.md`](legal/privacy.md). They ship as
engineering drafts. For each:

1. Fill in the bracketed parts.
2. Have a lawyer review it.
3. Delete its **Template.** note. `grep -l '\*\*Template\.\*\*' legal/*.md` prints nothing once both are done.
4. Set its version, at the top, in the front matter (`---`, `version: 2026-11`, `---`). Change it whenever you
   change the document: a changed document is a new version. The pages show it, and every raffle's notice names
   the privacy notice's.

Until then the app Worker refuses every request, `/healthz` included, and says why: a document still marked
**Template.**, or one without its front matter or its version.

## 8. The app Worker's secrets

You'll get: the app Worker, `rafi-app`, holding only its secrets. The first deploy puts the app in it.

The deploy won't create the app Worker: its secrets come first (wrangler won't deploy it while one of
`[secrets] required` is missing). From your repository:

```sh
cp .dev.vars.example .dev.vars
openssl rand -hex 32          # for RAFI_POW_KEY
$EDITOR .dev.vars             # fill in every value (the table below)
npx wrangler secret bulk .dev.vars -c wrangler.app.toml    # creates rafi-app (it asks first)
rm .dev.vars
```

| Secret | Value |
|---|---|
| `RAFI_PUBLIC_URL` | The public origin, no trailing slash (step 4) |
| `RESTATE_INGRESS_URL` | Restate Cloud's ingress URL (step 6) |
| `RESTATE_INGRESS_API_KEY` | The app's Restate Cloud API key (step 6) |
| `RAFI_POW_KEY` | The proof-of-work key: random, 32 characters or more. Never rotate it during an event. |
| `CLERK_SECRET_KEY` | `sk_live_…` (step 5) |
| `CLERK_PUBLISHABLE_KEY` | `pk_live_…` (step 5): not secret (the web app gets it from `/config.js`), but yours |
| `RAFI_OPERATOR_NAME`, `RAFI_OPERATOR_EMAIL` | Who runs this installation (a legal name) and its privacy contact, named as the processor in every raffle's notice |

- **Never commit `.dev.vars`.** It's git-ignored; delete it once the secrets are set.
- One at a time, or a change later: `npx wrangler secret put <NAME> -c wrangler.app.toml` prompts for the
  value. Deploys keep the secrets.
- The optional settings (retention, the entrant cap, the minimum age and others, with their defaults) are in the
  [configuration reference](https://github.com/sagikazarmark/rafi/blob/main/docs/reference/configuration.md). The
  app's are secrets too; the engine's (drand) are vars in `wrangler.engine.toml`.

## 9. GitHub repository secrets

You'll get: the four secrets the workflows deploy with.

```sh
gh secret set RESTATE_ADMIN_URL --body "https://<env-id>.env.<region>.restate.cloud:9070"
gh secret set RESTATE_AUTH_TOKEN     # paste the Admin-role API key
gh secret list                       # these two, and step 2's CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID
```

- Without `--body`, `gh secret set` prompts for the value, so a secret stays out of your shell history.
- The app's own secrets are on its Worker (step 8), never here.
- If your account or organization restricts which actions run, allow `actions/checkout`, `actions/setup-node`,
  `actions/upload-artifact` and `actions/download-artifact`.
- To approve each deploy first: create a GitHub environment with required reviewers (**Settings >
  Environments**), uncomment `environment: production` on every job that has it in
  [`deploy.yaml`](.github/workflows/deploy.yaml) and [`restate-cleanup.yaml`](.github/workflows/restate-cleanup.yaml),
  and keep the secrets there (`gh secret set --env production …`) or in the repository.

## 10. Deploy

You'll get: the three Workers, the database's tables, and the release's engine registered with Restate.

1. Optionally, try your configuration with the release first. It needs no credentials and uploads nothing:

   ```sh
   gh release download v0.1.0 -R sagikazarmark/rafi -p worker.tar.gz
   mkdir -p release && tar -xzf worker.tar.gz -C release && rm worker.tar.gz
   release/deploy.sh dry-run
   ```

   wrangler validates each configuration (`release/` is git-ignored). It doesn't look for the template's
   placeholders, or compare the three files: they must agree on `database_id` and `RAFI_DO_JURISDICTION`, and
   name the live Worker as `script_name`, and the engine needs `RESTATE_IDENTITY_KEYS`
   ([what is checked when][guide-installation]).
2. If you disabled the workflows in step 1, enable both. Then push, and watch the run:

   ```sh
   git add -A
   git commit -m "Configure the installation"
   git push                                     # nothing to push? gh workflow run deploy.yaml
   gh run watch                                 # pick the Deploy run
   curl https://raffle.example.org/healthz      # ok 0.1.0
   ```

   `ok 0.1.0` means the app Worker has its secrets and documents. Anything missing answers `500 misconfigured`
   with the reason; the app Worker's logs have all of it.
3. Run a **test-mode raffle** end to end:
   1. sign in at `https://raffle.example.org`;
   2. create a raffle with **Test mode** on;
   3. open its participant page on a phone, and add fake entrants;
   4. freeze, and draw one item with the projector open.

   The phone follows along live, and the verification page shows **✅ All 1 draws check out** under a
   **🧪 TEST MODE** note. That has been through Clerk, the database, the live channels, Restate, the engine and
   real drand.

From then on, a push to `main` that changes `VERSION`, `legal/` or a `wrangler.*.toml` deploys, and so does
**Actions > Deploy > Run workflow**. One deploy or cleanup runs at a time. What each job does and reports: the
guide's [The workflows][guide-workflows]. The rest, from upgrading to the logs: the [README](README.md).

## Several installations

One Cloudflare account can hold several installations: for different organizations, say, or a second one to try a
release first. Each has its own:

- **names for its three Workers**, chosen before its first deploy. The app Worker's name *is* the installation
  (the `instance` the cleanup and the downgrade check go by): renamed later, it's another installation, and the
  engines registered under the old name are no longer this one's;
- **D1 database**, domain, Clerk production instance, app Worker secrets and
  [jurisdiction](#keeping-data-in-the-eu);
- **Restate Cloud environment**: two installations in one environment would take over each other's draws.

They can share the Cloudflare account, its workers.dev subdomain and the API token (with Workers Routes on each
installation's zone). Why: the guide's [Several installations][guide-several].

### A repository per installation

Steps 1 to 10 again, in a new repository. Before its first deploy, give it other names. For an installation called
`acme`:

| File | Key | Template | `acme` |
|---|---|---|---|
| `wrangler.app.toml` | `name` | `rafi-app` | `acme-app` |
| `wrangler.engine.toml` | `name` | `rafi-engine` | `acme-engine` |
| `wrangler.live.toml` | `name` | `rafi-live` | `acme-live` |
| `wrangler.app.toml`, `wrangler.engine.toml` | `script_name` (the live Worker's name) | `rafi-live` | `acme-live` |
| all three | `database_name` | `rafi` | `acme` |
| `wrangler.app.toml` | the four rate limits' `namespace_id`, so they don't share counters | `1001` to `1004` | `3001` to `3004` |

Its Workers are then `acme-app`, `acme-live` and `acme-engine-v0-1-0`, and its database `acme`
(`npx wrangler d1 create acme` in step 3). It has its own secrets, version, legal documents and workflows.

### One repository, Wrangler environments

The configurations' top level is one installation, and each `[env.<name>]` another
([Wrangler environments](https://developers.cloudflare.com/workers/wrangler/environments/)). They share the
repository's `legal/` and commits, so the same operator; installations with different operators or documents
take a repository each.

An environment's Worker is `<top-level name>-<environment>` unless it sets its own `name`. It inherits `main`, the
static files, `workers_dev` and `preview_urls`, the Cron, the routes, the observability settings and the live
Worker's Durable Object migrations. It does **not** inherit the vars, the bindings (the database, `RAFFLE_ROOM`,
the rate limits) or the required secrets: repeat those. Nothing checks them: a missing binding or var deploys (an
engine without its identity keys refuses Restate, so its registration fails; the test-mode raffle shows the rest).

For an environment `eu`, add to the end of each configuration:

`wrangler.app.toml` (with routes of its own: the inherited ones would claim the top level's domain twice):

```toml
[env.eu]
routes = [{ pattern = "raffle.example.eu", custom_domain = true }]

[[env.eu.d1_databases]]
binding = "DB"
database_name = "rafi-eu"
database_id = "<the eu database's ID>"
migrations_dir = "release/migrations"

[[env.eu.durable_objects.bindings]]
name = "RAFFLE_ROOM"
class_name = "RaffleRoom"
script_name = "rafi-live-eu"

# Namespaces of their own, so the two installations don't share counters.
[[env.eu.ratelimits]]
name = "JOIN_PER_RAFFLE_LIMITER"
namespace_id = "2001"
simple = { limit = 600, period = 60 }

[[env.eu.ratelimits]]
name = "JOIN_PER_EMAIL_LIMITER"
namespace_id = "2002"
simple = { limit = 5, period = 60 }

[[env.eu.ratelimits]]
name = "CONNECT_LIMITER"
namespace_id = "2003"
simple = { limit = 600, period = 60 }

[[env.eu.ratelimits]]
name = "ADD_LIMITER"
namespace_id = "2004"
simple = { limit = 20, period = 60 }

[env.eu.vars]
# RAFI_DO_JURISDICTION = "eu"   # this installation's choice, the same in all three files

[env.eu.secrets]
required = [
  "RAFI_PUBLIC_URL",
  "RESTATE_INGRESS_URL",
  "RESTATE_INGRESS_API_KEY",
  "RAFI_POW_KEY",
  "CLERK_SECRET_KEY",
  "CLERK_PUBLISHABLE_KEY",
  "RAFI_OPERATOR_NAME",
  "RAFI_OPERATOR_EMAIL",
]
```

`wrangler.engine.toml` (with the eu installation's Restate environment's public key):

```toml
[env.eu]

[[env.eu.d1_databases]]
binding = "DB"
database_name = "rafi-eu"
database_id = "<the eu database's ID>"

[[env.eu.durable_objects.bindings]]
name = "RAFFLE_ROOM"
class_name = "RaffleRoom"
script_name = "rafi-live-eu"

[env.eu.vars]
# RAFI_DO_JURISDICTION = "eu"   # this installation's choice, the same in all three files
RESTATE_IDENTITY_KEYS = "publickeyv1_<the eu Restate environment's>"
```

`wrangler.live.toml`:

```toml
[env.eu]

[[env.eu.d1_databases]]
binding = "DB"
database_name = "rafi-eu"
database_id = "<the eu database's ID>"

[[env.eu.durable_objects.bindings]]
name = "RAFFLE_ROOM"
class_name = "RaffleRoom"

[env.eu.vars]
# RAFI_DO_JURISDICTION = "eu"   # this installation's choice, the same in all three files
```

Its Workers are `rafi-app-eu`, `rafi-live-eu` and, for 0.1.0, `rafi-engine-eu-v0-1-0`. Then:

1. **Its version file**, optionally: `echo 0.1.0 > VERSION.eu`. Its workflows deploy the release `VERSION.eu`
   names, and `VERSION`'s without one, so with it the installations upgrade each on its own.
2. **Steps 3 to 8 for it**: its database (`npx wrangler d1 create rafi-eu`, with a
   [jurisdiction](#keeping-data-in-the-eu) if its data stays in the EU), the ID in the three `[env.eu]` sections;
   its domain; its Clerk instance; its Restate environment, the public key in `[env.eu.vars]`; and its app
   Worker's secrets, set with the environment chosen:

   ```sh
   CLOUDFLARE_ENV=eu npx wrangler secret bulk .dev.vars -c wrangler.app.toml   # creates rafi-app-eu
   ```

   `--env eu` does the same, for `secret put` too. Step 10's dry run takes it as well:
   `CLOUDFLARE_ENV=eu release/deploy.sh dry-run`.
3. **Its workflows**, copies of the two:

   ```sh
   cp .github/workflows/deploy.yaml .github/workflows/deploy-eu.yaml
   cp .github/workflows/restate-cleanup.yaml .github/workflows/restate-cleanup-eu.yaml
   ```

   In both copies:
   - set `CLOUDFLARE_ENV: "eu"` under `env:`;
   - rename the concurrency group, `group: rafi-eu`, so the two installations' runs don't wait for or replace
     each other's;
   - uncomment `environment:` on every job that has the line, naming a GitHub environment of the
     installation's: `environment: eu`;
   - optionally, rename the workflow (`name: Deploy (eu)`, `name: Restate cleanup (eu)`) to tell them apart under
     **Actions**.
4. **Its secrets**, in that GitHub environment: step 9's four, with its own Restate admin URL and key
   (`gh secret set RESTATE_AUTH_TOKEN --env eu`, and so on). A job with an environment takes
   its secrets over the repository's ones of the same name; a job without one gets the repository's. So name one
   on the top-level installation's jobs too (`environment: production`, holding its secrets): then neither ever
   runs with the other's.
5. **The paths each deploy runs on**:
   - in `deploy-eu.yaml`, `VERSION.eu` instead of `VERSION` and `"VERSION.*"` (keep `VERSION` while there is no
     `VERSION.eu`);
   - in `deploy.yaml`, drop `"VERSION.*"`, so that changing `VERSION.eu` doesn't deploy the top-level installation
     too.

   Both keep `legal/**` and `wrangler.*.toml`: a change there may be either's, and deploying the release an
   installation already runs is harmless (the same code again). Except for the live Worker: any edit of
   `wrangler.live.toml`, another installation's section included, redeploys every installation's live Worker on
   its next deploy, dropping its phones' connections. Edit it between events.

Then push: `Deploy (eu)` deploys the environment's Workers, and its cleanup touches only its engines.

[cf-account-tokens]: https://developers.cloudflare.com/fundamentals/api/get-started/account-owned-tokens/
[cf-create-token]: https://developers.cloudflare.com/fundamentals/api/get-started/create-token/
[gh-push-event]: https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#push
[guide]: https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md
[guide-installation]: https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md#what-an-installation-is
[guide-residency]: https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md#data-residency
[guide-restate-keys]: https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md#the-restate-keys
[guide-several]: https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md#several-installations
[guide-token]: https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md#the-api-token
[guide-workers-dev]: https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md#the-engine-on-workersdev
[guide-workflows]: https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md#the-workflows
[runbook]: https://github.com/sagikazarmark/rafi/blob/main/docs/operations/runbook.md
[template]: https://github.com/sagikazarmark/rafi-deploy-template
[wrangler-login]: https://developers.cloudflare.com/workers/wrangler/commands/general/#login
