# Rafi on Cloudflare Workers

One installation of [Rafi](https://github.com/sagikazarmark/rafi), the verifiable raffle for in-person events, on
your own Cloudflare account: three Workers and one D1 database, with [Restate Cloud](https://cloud.restate.dev)
running the raffle engine's invocations and [Clerk](https://clerk.com) signing organizers in. Nothing here is
compiled. This directory is your configuration and your legal documents, and its GitHub Actions workflows deploy the
Rafi release that `VERSION` names, as Rafi's release workflow built and attested it.

```
VERSION                                 the Rafi release you deploy (0.1.0)
app.wrangler.toml                       the app Worker, rafi-app (web app, API, phones' sockets, hourly
                                        housekeeping): your database ID and domain
engine.wrangler.toml                    the raffle engine Worker, one per release (rafi-engine-v0-1-0), which
                                        Restate calls: your database ID and your Restate environment's public key
live.wrangler.toml                      the live Worker, rafi-live (the raffles' live channels): your database ID
legal/                                  your terms and privacy notice
package.json, package-lock.json        wrangler and the Restate CLI: the workflows install them (npm ci),
                                        deploy.sh runs them (npx -y); the lockfile, from step 1, pins them
.prod.vars.example                      the app Worker's secrets, and what each is
.github/workflows/deploy.yaml           the deploy, on a push to main that changes any of the above
.github/workflows/restate-cleanup.yaml  daily: removes the engines of earlier releases once drained
```

This page is the first deploy, step by step and in order: each step says what it produces and where that goes. The
reference behind it, with the reasons, is
[docs/operations/cloudflare.md](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md).
It deploys one installation, named by its app Worker (`rafi-app`); a second one in the same Cloudflare account takes
names of its own ([Several installations](#several-installations)).
Whether its data stays in the EU is decided before its first deploy: [Keeping data in the
EU](#keeping-data-in-the-eu).

## What you'll need

- **A Cloudflare account on the paid Workers plan** (Workers, D1, Durable Objects): verifying a drand signature
  and rendering the participant page don't fit the free plan's CPU time.
- **A zone (your domain) on that account**, for the app's hostname and for Clerk's DNS records.
- **The account's workers.dev subdomain.** Each release's engine Worker is served at
  `https://rafi-engine-v<version>.<subdomain>.workers.dev`, where Restate calls it. In the dashboard, **Workers &
  Pages** shows it next to **Your subdomain**; an account without one is offered one on its first visit there. With
  the API token of step 2 you can read it (`GET /accounts/{account_id}/workers/subdomain`, shown there) or register
  one (`PUT` to the same path with `{"subdomain": "<name>"}`); `cf` has no command for it. A deploy without one stops
  before deploying the engine, naming the dashboard page. **Don't change it once deployed**: Restate calls every
  registered engine at an address that contains it.
- **A [Restate Cloud](https://cloud.restate.dev) account**, a **[Clerk](https://clerk.com) account** and a **GitHub
  account**.
- Your legal documents, reviewed (step 7), and the operator's name and email (you).
- Tools on your machine:
  - [`gh`](https://cli.github.com), signed in (`gh auth login`);
  - [Node.js](https://nodejs.org) 22.18 or later, for `npm` and `npx` (wrangler and the Restate CLI, the versions
    `package.json` and its lockfile name, installed in step 1) and `cf`;
  - [`cf`](https://developers.cloudflare.com/cf/), Cloudflare's CLI (npm `cf`, in beta: commands can change, so
    this page pins `1.0.0-beta.27` and gives the raw API calls too): `npm install --global cf@1.0.0-beta.27`;
  - `jq` (`deploy.sh` reads every JSON answer with it; GitHub's runners have it), `openssl`, `curl`, `git`;
  - the [Restate CLI](https://docs.restate.dev/installation) comes with `package.json` (`npx -y
    @restatedev/restate`), for what the cleanup leaves to you (a paused invocation).

Below, `<you>/<name>` is your new repository, `example.org` your zone, and `raffle.example.org` the app's hostname.

## Keeping data in the EU

Optional, and decided before the first deploy: neither the database's nor the live channels' location can be
changed afterwards in place. By default Cloudflare places the database and the Durable Objects where it sees fit.
To keep them, and Restate's copy of the draws, in the EU:

- **The database**: in step 3, create it with a jurisdiction, `npx -y wrangler d1 create rafi --jurisdiction eu`.
  A D1 database's jurisdiction is fixed when it is created: moving one later is a new database and its data copied
  over (the [runbook's restore](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/runbook.md)).
- **The live channels' Durable Objects**: uncomment `RAFI_DO_JURISDICTION = "eu"` in all three configurations
  (`app.wrangler.toml`, `engine.wrangler.toml`, `live.wrangler.toml`), and in every `[env.<name>.vars]` section
  if you use [Wrangler environments](#one-repository-wrangler-environments) (environments don't inherit vars),
  before the first deploy. The app and the engine create each raffle's object in that jurisdiction; an object stays
  where it was created, so changing the setting later moves the live channels to new objects, the old ones left
  behind. The deploy doesn't compare the three files: keep them the same.
- **Restate Cloud**: in step 6, create the environment in the EU region (the region is part of its URLs). Restate
  keeps the engine's invocations and their journals: the draws.

What it doesn't cover: the Workers run wherever the visitor is (Cloudflare's network handles a request in a data
center near it, outside the EU for a visitor outside it, with what the request reads from the database), and
[Clerk](https://clerk.com), which holds the organizers' accounts, is a US company (under the EU-U.S. Data Privacy
Framework). drand, the public randomness, gets no personal data. Your privacy notice
([`legal/privacy.md`](legal/privacy.md)) names the hosting's location as one of its bracketed choices: keep the one
that's true.

## 1. Create your repository

Produces: a private repository of yours whose root is this directory, at the release's tag, and the tools'
lockfile, `package-lock.json`.

**From a terminal**:

```sh
gh repo create <you>/<name> --private --clone
cd <name>
# deploy/cloudflare at the tag v0.1.0, at the repository's root, .github/ and the dotfiles included (GNU tar):
gh api repos/sagikazarmark/rafi/tarball/v0.1.0 | tar -xz --strip-components=3 --wildcards '*/deploy/cloudflare/*'
# on macOS, whose tar matches patterns by default and refuses --wildcards:
#   gh api repos/sagikazarmark/rafi/tarball/v0.1.0 | tar -xz --strip-components=3 '*/deploy/cloudflare/*'
npm install --no-audit --no-fund    # wrangler and the Restate CLI into node_modules/ (git-ignored); writes package-lock.json
git add -A
git commit -m "Rafi 0.1.0"
git branch -M main
git push -u origin main
```

The workflows run on `main` (and a scheduled one only on the default branch), so the branch is `main`. Commit
`package-lock.json`: the workflows install from it (`npm ci`), so every run, and yours, uses the same wrangler and
Restate CLI (without it they install the latest `package.json` allows, `npm install`). The commands below run them
from the repository with `npx -y`, which takes the installed ones.

**From GitHub's web UI**: create the repository at [github.com/new](https://github.com/new) (private, with no
README, `.gitignore` or license), download the release's source (`Source code (zip)` on
[the release page](https://github.com/sagikazarmark/rafi/releases/tag/v0.1.0)), and on the new repository's page
choose **uploading an existing file** and drop in the *contents* of its `deploy/cloudflare` folder. The two workflow
files and `.gitignore` are hidden files (on macOS, ⇧⌘. shows them in Finder): check that the commit lists
`.github/workflows/deploy.yaml`, `.github/workflows/restate-cleanup.yaml` and `.gitignore`, and add any that's
missing with **Add file > Create new file**, typing its path and pasting its content. Commit to `main`. Steps 3 and
8 need a terminal anyway, so clone it there (`gh repo clone <you>/<name>`), run `npm install --no-audit --no-fund`
and commit `package-lock.json`.

**The first push starts the Deploy workflow**, which fails, harmlessly: its first job downloads and verifies the
release, then stops at **Check it against this repository**, which refuses the placeholders still in the
configurations (the zero database ID, the identity key), before any step that uses your secrets or touches
Cloudflare or Restate. The daily Restate cleanup stops at the same check until then. To skip those failed runs,
keep your commits local and push at step 10 (the steps in between need the repository to exist on GitHub, not its content),
or disable both workflows (**Actions**, the workflow, **⋯ > Disable workflow**) and enable them at step 10.

## 2. A Cloudflare API token

Produces: the GitHub secrets `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.

The workflows deploy with an **account API token** (owned by the account, not by you, so it keeps working whoever
leaves) with exactly:

| Permission (API name) | Resource | What it's for |
|---|---|---|
| Account · Workers Scripts · Edit (`Workers Scripts Write`) | your account | Uploading and deploying the three Workers (code, static files, Durable Object class and migrations, Cron, custom domain, workers.dev on or off), reading their deployments and versions, the workers.dev subdomain, listing the account's Workers and deleting a drained engine Worker (the cleanup) |
| Account · D1 · Edit (`D1 Write`) | your account | The migrations |
| Zone · Workers Routes · Edit (`Workers Routes Write`) | your zone | The app Worker's hostname on your zone |

Nothing else (not Workers Tail, Account Settings, Memberships or User Details: with the account ID given, wrangler
doesn't look your user up). Creating account tokens takes a Super Administrator, or a member with API token
provisioning, and a token gets no permission its creator lacks.

Work in a scratch directory, so nothing of this lands in your repository:

```sh
cd "$(mktemp -d)"
cf auth login                                                # in the browser; cf keeps its own login
cf accounts list | jq -r '.[] | "\(.id)  \(.name)"'          # your account's ID
export CLOUDFLARE_ACCOUNT_ID=<your account's ID>             # cf and wrangler use it from here on
cf zones list --name example.org | jq -r '.[] | "\(.id)  \(.name)"'
export ZONE_ID=<your zone's ID>
cf accounts tokens permission-groups list > permission-groups.json
jq -r '.[] | select(.name == "Workers Scripts Write" or .name == "D1 Write" or .name == "Workers Routes Write")
           | "\(.id)  \(.name)"' permission-groups.json
```

Save the token's policies as `token.json`, with your account's ID, your zone's ID and the three permission groups'
IDs from the last command:

```json
[
  {
    "effect": "allow",
    "resources": { "com.cloudflare.api.account.<ACCOUNT_ID>": "*" },
    "permission_groups": [
      { "id": "<ID of Workers Scripts Write>", "name": "Workers Scripts Write" },
      { "id": "<ID of D1 Write>", "name": "D1 Write" }
    ]
  },
  {
    "effect": "allow",
    "resources": { "com.cloudflare.api.account.zone.<ZONE_ID>": "*" },
    "permission_groups": [
      { "id": "<ID of Workers Routes Write>", "name": "Workers Routes Write" }
    ]
  }
]
```

or have `jq` write it from the lookups:

```sh
jq --arg account "$CLOUDFLARE_ACCOUNT_ID" --arg zone "$ZONE_ID" '
  def group($name): (map(select(.name == $name)) | if length == 1 then .[0] else error("no group \($name)") end)
                    | {id, name};
  [ { effect: "allow", resources: { ("com.cloudflare.api.account." + $account): "*" },
      permission_groups: [group("Workers Scripts Write"), group("D1 Write")] },
    { effect: "allow", resources: { ("com.cloudflare.api.account.zone." + $zone): "*" },
      permission_groups: [group("Workers Routes Write")] } ]' permission-groups.json > token.json
```

Create the token and hand its value straight to GitHub. Cloudflare shows it once; here it goes into a shell
variable, the repository's secret and one check, and never to a file or your screen:

```sh
token="$(cf accounts tokens create --name "rafi deploy (<you>/<name>)" --policies @token.json | jq -j '.value // empty')"
test -n "$token" && printf '%s' "$token" | gh secret set CLOUDFLARE_API_TOKEN --repo <you>/<name>
gh secret set CLOUDFLARE_ACCOUNT_ID --repo <you>/<name> --body "$CLOUDFLARE_ACCOUNT_ID"
# the check: the token works, and the account's workers.dev subdomain ({"subdomain": "..."}; an error if there is none)
curl -sS "https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/subdomain" \
  -H "Authorization: Bearer $token" | jq '.result // .errors'
unset token
rm permission-groups.json token.json
```

`token.json` holds IDs only, no secret. If you saved `cf accounts tokens create`'s output anywhere, delete it: it
contains the token. Lost the value before it reached GitHub? `cf accounts tokens list` shows the token's ID, and
`cf accounts tokens roll <ID> --force | jq -j . | gh secret set CLOUDFLARE_API_TOKEN --repo <you>/<name>` gives it
a new value (piped, `cf` can't ask, so `--force` confirms; without it, it prints `Aborted.` and changes nothing).

**Without `cf`**: in the dashboard, **Manage account > Account API tokens > Create Token > Create Custom Token**,
with the three permissions above and your zone; then `gh secret set CLOUDFLARE_API_TOKEN --repo <you>/<name>` and
paste the value at its prompt. Or the same API calls `cf` makes (List Permission Groups `GET
/accounts/{account_id}/tokens/permission_groups`, List Zones `GET /zones?name=`, Create Token `POST
/accounts/{account_id}/tokens`), with a token of yours that has Account · Account API Tokens · Edit (and Zone ·
Zone · Read for the lookup) in `CLOUDFLARE_API_TOKEN`:

```sh
api=https://api.cloudflare.com/client/v4
curl -sS "$api/accounts/$CLOUDFLARE_ACCOUNT_ID/tokens/permission_groups" -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" \
  | jq '.result' > permission-groups.json                    # then token.json as above
curl -sS "$api/zones?name=example.org" -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" | jq -r '.result[0].id'
curl -sS -X POST "$api/accounts/$CLOUDFLARE_ACCOUNT_ID/tokens" -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" \
  -H 'Content-Type: application/json' \
  --data "$(jq -n --slurpfile policies token.json '{name: "rafi deploy", policies: $policies[0]}')" \
  | jq -j '.result.value // empty' | gh secret set CLOUDFLARE_API_TOKEN --repo <you>/<name>
```

## 3. The database

Produces: the D1 database, and its ID in all three Wrangler configurations.

From your repository (with `CLOUDFLARE_ACCOUNT_ID` still set, so wrangler picks that account):

```sh
npx -y wrangler login
npx -y wrangler d1 create rafi    # prints the database_id; for the EU, read "Keeping data in the EU" first
id=<the database_id it printed>
sed -i.bak "s/00000000-0000-0000-0000-000000000000/$id/" app.wrangler.toml engine.wrangler.toml live.wrangler.toml
rm app.wrangler.toml.bak engine.wrangler.toml.bak live.wrangler.toml.bak
```

All three Workers use the one database (the app reads and writes it, the engine writes the draws, the live
channels read their raffle), so the three files carry the same `database_id`. The deploy doesn't compare them: a
file left on another database shows in the first test-mode raffle (step 10). A [jurisdiction](#keeping-data-in-the-eu)
can only be chosen here, when the database is created.

## 4. Your domain

Produces: the app's hostname in `app.wrangler.toml`, and the value of `RAFI_PUBLIC_URL` (step 8).

Pick the hostname organizers and phones will use, on your zone, with no DNS record of its own yet: each deploy
attaches it to the app Worker as a custom domain (Cloudflare creates its DNS record and certificate), and from a
workflow it takes over an existing record or custom domain of that name without asking. Uncomment `routes` in
[`app.wrangler.toml`](app.wrangler.toml) with it:

```sh
sed -i.bak 's|^# routes = .*|routes = [{ pattern = "raffle.example.org", custom_domain = true }]|' app.wrangler.toml
rm app.wrangler.toml.bak
```

`RAFI_PUBLIC_URL` is that origin with no trailing slash, `https://raffle.example.org`: QR codes and links use it,
and Clerk must allow it. The app has no other address (`workers_dev` and preview URLs are off).

## 5. Clerk

Produces: `CLERK_PUBLISHABLE_KEY` and `CLERK_SECRET_KEY` (step 8), and Clerk's DNS records in your zone.

In the [Clerk dashboard](https://dashboard.clerk.com), create an application, then its **production instance** for
your domain. Clerk lists DNS records (CNAMEs for its Frontend API, the accounts portal and email): add each to your
zone in Cloudflare (**DNS > Records**), with **Proxy status** *DNS only*. Sign-in works once Clerk has verified
them and issued its certificates, which can take a while (**Domains** in Clerk shows the status). The production
instance's **API keys** are `pk_live_…` (publishable) and `sk_live_…` (secret).

## 6. Restate Cloud

Produces: the app's `RAFI_ENGINE_INGRESS_URL` and `RAFI_ENGINE_API_KEY` (step 8), the GitHub secrets
`RESTATE_ADMIN_URL` and `RESTATE_AUTH_TOKEN` (step 9), and the identity key in `engine.wrangler.toml`.

In [Restate Cloud](https://cloud.restate.dev), create an environment, in the EU region if the data must stay there
([Keeping data in the EU](#keeping-data-in-the-eu); the region is part of every URL below). On its **Developers** page:

- **The ingress URL**, `https://<env-id>.env.<region>.restate.cloud:8080` (`<env-id>` is the environment's ID
  without `env_`): the app Worker's `RAFI_ENGINE_INGRESS_URL`.
- **An API key for the app** (**API Keys > Create API Key**): the app only invokes the engine's services through
  the ingress, never the admin API, so give it the narrowest role your console offers that can call the ingress
  (Restate's guides name **Admin** for deploying and **Full** for tunnels; if no role narrower than **Full** is
  listed, use **Full**): the app Worker's `RAFI_ENGINE_API_KEY`.
- **An API key with the Admin role**, for the workflows, which register each release's engine, list the
  deployments, ask which invocations are still pinned to one, and remove drained ones: the GitHub secret
  `RESTATE_AUTH_TOKEN`.
- **The admin URL**, the same host on port 9070, `https://<env-id>.env.<region>.restate.cloud:9070`: the GitHub
  secret `RESTATE_ADMIN_URL`.
- **The request identity's public key** (**Security > HTTP endpoints**, `publickeyv1_…`): the engine refuses
  every request not signed with the matching private key. It is public, so it goes into
  [`engine.wrangler.toml`](engine.wrangler.toml), committed:

  ```sh
  key=<the publickeyv1_... key>
  sed -i.bak "s/^RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS = .*/RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS = \"$key\"/" engine.wrangler.toml
  rm engine.wrangler.toml.bak
  ```

Nothing is registered by hand: each deploy registers its release's engine Worker with Restate.

## 7. Your legal documents

Produces: `legal/terms.md` and `legal/privacy.md` as yours, and their versions (step 8).

The app serves your terms (with the data processing agreement organizers accept by using the service) and your
privacy notice (naming you and the subprocessors: Cloudflare, Restate Cloud, Clerk, drand) from
[`legal/terms.md`](legal/terms.md) and [`legal/privacy.md`](legal/privacy.md), which each deploy copies into its
static files. They ship as engineering drafts marked **Template.**: fill in the bracketed parts, have a lawyer review
them, and delete the marker. While a document still carries it, the app Worker refuses every request, `/healthz`
included, saying why. `grep -l '\*\*Template\.\*\*' legal/*.md` prints nothing once both are done. Their versions
(e.g. `2026-11`) are the secrets `RAFI_TERMS_VERSION` and `RAFI_PRIVACY_VERSION`; a changed document is a new
version.

## 8. The app Worker's secrets

Produces: the app Worker, `rafi-app`, holding only its secrets; the first deploy puts the app in it.

The deploy refuses to create the app Worker itself: its secrets come first (wrangler won't deploy it while one of
`[secrets] required` is missing). From your repository:

```sh
cp .prod.vars.example .prod.vars
openssl rand -hex 32          # for RAFI_POW_KEY
$EDITOR .prod.vars            # fill in every value
npx -y wrangler secret bulk .prod.vars -c app.wrangler.toml
rm .prod.vars
```

Wrangler sees no Worker `rafi-app` yet and creates it with the secrets (it asks first when run in a terminal). One
at a time instead: `npx -y wrangler secret put <NAME> -c app.wrangler.toml`, which prompts for the value.
Later changes are `secret put` too; deploys keep the secrets. `.prod.vars` is in `.gitignore`: never commit it, and
delete it once the secrets are set.

| Secret | What it is |
|---|---|
| `RAFI_PUBLIC_URL` | The public origin, no trailing slash (step 4) |
| `RAFI_ENGINE_INGRESS_URL` | Restate Cloud's ingress URL (step 6) |
| `RAFI_ENGINE_API_KEY` | The app's Restate Cloud API key (step 6) |
| `RAFI_POW_KEY` | The proof-of-work key: random, 32 characters or more. Never rotate it during an event. |
| `CLERK_SECRET_KEY` | `sk_live_…` (step 5) |
| `CLERK_PUBLISHABLE_KEY` | `pk_live_…` (step 5): not secret (the web app gets it from `/config.js`), but yours |
| `RAFI_OPERATOR_NAME`, `RAFI_OPERATOR_EMAIL` | Who runs this installation (a legal name) and its privacy contact, named as the processor in every raffle's notice |
| `RAFI_TERMS_VERSION`, `RAFI_PRIVACY_VERSION` | The legal documents' versions (step 7) |

The optional settings (retention, the entrant cap, the minimum age and the others, with their defaults) are in the
[configuration reference](https://github.com/sagikazarmark/rafi/blob/main/docs/reference/configuration.md): the
app's are secrets too when you change them; the engine's (drand) are vars in `engine.wrangler.toml`.

## 9. GitHub repository secrets

Produces: what the workflows deploy with.

Without `--body`, `gh secret set` reads the value from a prompt (or standard input), so it stays out of your shell
history; the account ID and the admin URL aren't secret, so `--body` is fine for them:

```sh
gh secret set CLOUDFLARE_API_TOKEN --repo <you>/<name>         # done in step 2; else paste the token
gh secret set CLOUDFLARE_ACCOUNT_ID --repo <you>/<name> --body "<your account's ID>"   # done in step 2
gh secret set RESTATE_ADMIN_URL --repo <you>/<name> --body "https://<env-id>.env.<region>.restate.cloud:9070"
gh secret set RESTATE_AUTH_TOKEN --repo <you>/<name>           # paste the Admin-role API key
gh secret list --repo <you>/<name>                             # the four names
```

The app's own secrets are on its Worker (step 8), never here. If your account or organization restricts which
actions run, allow `actions/checkout`, `actions/setup-node`, `actions/upload-artifact` and
`actions/download-artifact`. To approve each deploy first, create an environment with required reviewers
(**Settings > Environments**), uncomment `environment: production` on every job in
[`deploy.yaml`](.github/workflows/deploy.yaml) that has it (and in
[`restate-cleanup.yaml`](.github/workflows/restate-cleanup.yaml)), and keep the secrets there (`gh secret set
--env production …`) or in the repository.

## 10. Deploy

Produces: the three Workers, the database's tables, and the release's engine registered with Restate.

Optionally, check your configuration against the release first, with no credentials (the check the deploy's first
job runs):

```sh
gh release download v0.1.0 -R sagikazarmark/rafi -p worker.tar.gz
mkdir -p release && tar -xzf worker.tar.gz -C release && rm worker.tar.gz
release/deploy.sh check
```

`deploy.sh` needs `jq`, and runs wrangler as `npx -y wrangler` from your repository: `package.json`'s, installed in
step 1 (`WRANGLER=<command>` names another). It checks the release against `VERSION`, refuses the template's
placeholders (a plain search for the zero database ID and the `publickeyv1_…` key), and reads the three Workers'
names from `wrangler deploy --dry-run` of each configuration, which also checks that each deploys. It doesn't compare
the configurations with each other: the three must agree on the `database_id` and `RAFI_DO_JURISDICTION`, the app's
and the engine's `RAFFLE_ROOM` name the live Worker as `script_name`, and the engine needs its identity keys (in
every environment, with [several installations](#one-repository-wrangler-environments)). A mismatch deploys: an
engine without its keys refuses Restate, so its registration (the last job) fails, and the first test-mode raffle
below shows the rest (the phone doesn't follow the draw, or the draw doesn't land). `release/` is git-ignored.
Then commit and push, and watch the run:

```sh
git add -A
git commit -m "Configure the installation"
git push                                       # or, with nothing to push: gh workflow run deploy.yaml --repo <you>/<name>
gh run watch --repo <you>/<name>               # pick the Deploy run
curl https://raffle.example.org/healthz        # ok 0.1.0
```

The Deploy workflow's jobs, each on a fresh runner with the same verified release:

1. **Prepare the release**: downloads `worker.tar.gz` of `v<VERSION>` from Rafi's releases, refuses it unless its
   attestation says Rafi's release workflow built it from that tag on GitHub's runners, and checks it against your
   repository (`VERSION`, the placeholders, the Workers' names from wrangler's dry runs), then against Restate and
   Cloudflare (an older release than the one Restate runs is refused; so are secrets that don't work), before
   anything touches production;
2. **D1 migrations**: applies the release's migrations;
3. **Live Worker**: deploys `rafi-live` (the `RaffleRoom` Durable Objects), only when its code or
   `live.wrangler.toml` changed;
4. **App Worker (upload)** and **Engine Worker**, side by side: the app's new version uploaded, not serving yet
   (on the first deploy it is deployed directly), and this release's engine Worker, `rafi-engine-v0-1-0`, deployed
   at `https://rafi-engine-v0-1-0.<subdomain>.workers.dev/restate`;
5. **Register the engine, deploy the app**: registers the engine with Restate (refusing a release older than the
   one Restate runs once more, in case one was registered since), then deploys the app's new version with its
   routes and Cron.

`/healthz` answering `ok 0.1.0` means the app Worker has its secrets and documents; anything missing answers `500
misconfigured` with the reason (the app Worker's logs have the whole of it). Then run a **test-mode raffle** end to
end: sign in at `https://raffle.example.org`, create a raffle with **Test mode** on, open its participant page on a
phone, add fake entrants, freeze, and draw one item with the projector open. The phone follows along live (the
live Worker), and the verification page shows **✅ All 1 draws check out** under a **🧪 TEST MODE** note. That has
been through Clerk, the database, the live channels, Restate, the engine and real drand.

From then on a push to `main` that changes `VERSION`, `legal/` or one of the three configurations deploys; so does
**Actions > Deploy > Run workflow**. One deploy or cleanup runs at a time.

## What each run reports

Every run's summary page (**Actions**, the run) shows what its jobs did, as `deploy.sh` reports it; the job logs
have every step.

- **Deploy**: the live Worker deployed, with its fingerprint tag, or unchanged and left alone (phones kept
  connected); the app Worker's version uploaded (its ID), or, on a first install, deployed directly; this release's
  engine Worker and its address; its Restate deployment ID (registered now, or Restate had it already); the app's
  version deployed at 100 %, with its routes and Cron.
- **Restate cleanup**, in four lists:
  - **Removed**: each drained deployment removed from Restate, and its Worker deleted (or already gone); and
    **Not removed**, when Restate refused one (its Worker untouched; the next cleanup tries again);
  - **Still draining**: each older deployment with its active and paused invocation counts and the paused ones'
    IDs, and the commands to resume or kill a paused one ([Cleanup](#cleanup));
  - **Workers left behind**: an engine Worker whose deletion failed after its deployment was removed, and every
    `rafi-engine-v…` Worker on the account that no Restate deployment refers to, with the `wrangler delete` command
    to remove one by hand. The cleanup never deletes these itself;
  - **Not this installation's**: deployments in the Restate environment with other metadata, never touched.

Each job also keeps the JSON `deploy.sh` read and wrote as an artifact of the run (on the run's page under
**Artifacts**, for 14 days, or `gh run download <run-id> --repo <you>/<name>`), a directory per subcommand:
wrangler's output of each call (`wrangler-<what>.ndjson`, its `WRANGLER_OUTPUT_FILE_PATH` file), Restate's services
and deployments, the registration's answer, the account's Workers, and the cleanup's summary, `cleanup.json` (what
was removed and deleted, what still drains, what was left behind, the leftovers and what isn't this
installation's). Never a token: `deploy.sh` hands those to `curl` on its standard input only. The Deploy run's are
named `deploy-json-<job>-<attempt>`, the cleanup's `cleanup-json-<job>-<attempt>`; outside GitHub,
`RAFI_ARTIFACTS_DIR=<dir>` keeps them the same way.

## Upgrading

Change `VERSION` to the new release and push:

```sh
echo 0.2.0 > VERSION
git commit -am "Rafi 0.2.0" && git push
```

That is the whole upgrade. With several installations in one repository, change that installation's
`VERSION.<environment>` instead ([Several installations](#several-installations)).

Read the release notes first: they say when a configuration, `.prod.vars.example` or a secret changed, when the
live Worker changed (that deploy drops the live connections), when an upgrade needs a step of yours, and when the
template's workflows or configurations changed. Nothing checks your workflows against the release, so when the
notes say the workflows changed, copy the two from this directory at that release's tag before pushing (and, with
several installations in one repository, redo each copy's edits):

```sh
for f in deploy.yaml restate-cleanup.yaml; do
  gh api -H 'Accept: application/vnd.github.raw' \
    "repos/sagikazarmark/rafi/contents/deploy/cloudflare/.github/workflows/$f?ref=v0.2.0" > ".github/workflows/$f"
done
```

A change to a configuration is yours to make: compare yours with the template's at the new tag (`gh api -H
'Accept: application/vnd.github.raw' "repos/sagikazarmark/rafi/contents/deploy/cloudflare/app.wrangler.toml?ref=v0.2.0"`,
and the other two), keeping your names, IDs, domain and keys.

wrangler and the Restate CLI move apart from Rafi's releases, when you choose: `npm update --no-audit --no-fund`,
then commit `package-lock.json` (the release notes say when a release needs newer ones).

**Never go back to an older release.** Migrations only go forward, so an older release meets a schema it doesn't
know. The deploy refuses a release older than the one Restate runs in its first job, before touching anything: a
`VERSION` set back fails at **Prepare the release** with nothing deployed. A bad release is fixed by the next one.

## What a deploy does to live raffles

A deploy doesn't check whether a raffle is live; you choose when to deploy. What happens:

- **The live Worker** is redeployed only when its code or `live.wrangler.toml` changed (the release notes say when
  its code did; any edit of the file counts, a comment too). That restarts every live channel: phones and projectors lose their connection, reconnect and
  catch up, but the moment on the screen is lost.
- **The app** switches to the new version: requests go to it from then on.
- **A draw in flight** stays on the engine Worker it started on, its release's, until it finishes; new draws go to
  the new engine.

Still, don't deploy during an event: from when registration opens at the venue until the last raffle has
finished. The [runbook](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/runbook.md#before-a-deploy-is-anything-live)
has the query that lists every organizer's live raffles.

## Cleanup

Each release leaves its engine Worker behind, serving what started on it. The **Restate cleanup** workflow runs
daily at 04:37 UTC, and by hand (**Actions > Restate cleanup > Run workflow**, or `gh workflow run
restate-cleanup.yaml --repo <you>/<name>`). It lists Restate's deployments and the account's Workers, and stops
before changing anything if either listing fails. It acts only on this installation's engine deployments, those
the deploy registered with `app` `rafi`, this installation's app Worker's name as `instance` and this account in
their metadata:

- **drained** (nothing pinned to it any more): first removes the deployment from Restate (`restate deployments
  remove`, never forced: Restate refuses one that isn't drained, and then the Worker is left alone for the next
  run), then deletes its Worker (already gone is fine). A Worker whose deletion fails is reported as left behind,
  and the run fails;
- **draining**: lists the invocations still pinned to it, and leaves it for a later run;
- **paused** invocations (Restate stopped retrying one and waits for a human) keep their deployment draining
  until you resume one on the latest deployment, if the new engine can carry it on, or kill it:

  ```sh
  export RESTATE_ADMIN_URL=https://<env-id>.env.<region>.restate.cloud:9070 RESTATE_AUTH_TOKEN=<an Admin-role key>
  npx -y @restatedev/restate invocations describe <id>    # why it stopped (from your repository)
  npx -y @restatedev/restate invocations resume <id> --deployment latest
  npx -y @restatedev/restate invocations kill <id>        # or end it where it stopped
  ```

- the **active** one (the latest): kept;
- deployments that aren't this installation's: listed, never touched.

It also lists, and never deletes, the account's `rafi-engine-v…` Workers (this installation's engine name, then
`-v`) that no Restate deployment refers to: one left behind by a failed deletion, or an earlier release's that a
deploy stopped before registering (the current release's is left out: its deploy registers it). Once you're sure
nothing needs it, delete it by hand:

```sh
CLOUDFLARE_API_TOKEN=<a token with Workers Scripts Write> CLOUDFLARE_ACCOUNT_ID=<your account's ID> \
  npx -y wrangler delete rafi-engine-v0-1-0
```

GitHub runs scheduled workflows only on the default branch (keep it `main`), and in a public repository disables
them after 60 days without repository activity (**Actions > Restate cleanup > Enable workflow** turns it back
on). Deploys and cleanups share one concurrency group and never cancel a running one, but GitHub keeps only one
run waiting: a cleanup queued behind a running deploy replaces another deploy waiting there, which then shows as
cancelled. Run that deploy again.

## Several installations

One Cloudflare account can hold several installations: for different organizations, say, or a second one for
trying a release first. An installation is its app Worker's name: unique in the account, it is the `instance` in
Restate's deployment metadata, which the downgrade check and the cleanup go by. So each installation needs, of its
own:

- **names for its three Workers** (the engine's name is the base of each release's engine Worker, `<name>-v0-1-0`),
  chosen before its first deploy: renaming the app Worker later makes another installation of it, and the engines
  registered under the old name are no longer this one's to the cleanup or the downgrade check;
- **its own D1 database** (`wrangler d1 create <name>`), its domain and Clerk production instance, and its app
  Worker's secrets; and its own choice of [jurisdiction](#keeping-data-in-the-eu);
- **its own Restate Cloud environment**: Restate sends a service's new invocations to its latest deployment, and
  every Rafi engine registers the same services (`Raffle`, `EngineInfo`), so two installations in one environment
  would take over each other's draws.

The Cloudflare account, its workers.dev subdomain and the API token can be shared (the token needs Workers Routes
on each installation's zone). There are two ways to lay them out.

### A repository per installation

Steps 1 to 10 again, in a new repository, with other names in the three configurations before the first deploy:
each file's `name`, the live Worker's name as `script_name` in the app's and the engine's, and the database's
`database_name`; and the rate limits' `namespace_id`s, so the installations don't share counters. For an
installation called `acme`:

```sh
sed -i.bak 's/^name = "rafi-app"$/name = "acme-app"/' app.wrangler.toml
sed -i.bak 's/^name = "rafi-engine"$/name = "acme-engine"/' engine.wrangler.toml
sed -i.bak 's/"rafi-live"$/"acme-live"/' app.wrangler.toml engine.wrangler.toml live.wrangler.toml
sed -i.bak 's/^database_name = "rafi"$/database_name = "acme"/' app.wrangler.toml engine.wrangler.toml live.wrangler.toml
sed -i.bak 's/^namespace_id = "100\([1-4]\)"$/namespace_id = "300\1"/' app.wrangler.toml
rm app.wrangler.toml.bak engine.wrangler.toml.bak live.wrangler.toml.bak
```

Its Workers are then `acme-app`, `acme-live` and `acme-engine-v0-1-0`, and its database `acme` (`wrangler d1 create
acme` in step 3). Everything else is as above: the repository has its own secrets, version,
legal documents and workflows.

### One repository, Wrangler environments

One repository can hold several installations as [Wrangler
environments](https://developers.cloudflare.com/workers/wrangler/environments/): the configurations' top level is
one installation, each `[env.<name>]` another. They share the repository's `legal/` (so the same operator's
documents) and its commits; installations with different operators or documents take a repository each.

An environment's Worker is named `<top-level name>-<environment>` unless it sets a `name` of its own. It inherits
`main`, the static files, `workers_dev` and `preview_urls`, the Cron, the routes, the observability settings and
the live Worker's Durable Object migrations, but **not** the vars, the bindings (the database, `RAFFLE_ROOM`, the
rate limits) or the required secrets: repeat those under `[env.<name>]`. The deploy doesn't check that: a missing
binding or var deploys (an engine without its identity keys refuses Restate, so its registration fails; the first
test-mode raffle shows the rest). For an environment `eu`, add to the end of each configuration:

`app.wrangler.toml` (the routes too: inherited, the top level's domain would be claimed twice):

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
  "RAFI_ENGINE_INGRESS_URL",
  "RAFI_ENGINE_API_KEY",
  "RAFI_POW_KEY",
  "CLERK_SECRET_KEY",
  "CLERK_PUBLISHABLE_KEY",
  "RAFI_OPERATOR_NAME",
  "RAFI_OPERATOR_EMAIL",
  "RAFI_TERMS_VERSION",
  "RAFI_PRIVACY_VERSION",
]
```

`engine.wrangler.toml` (the eu installation's Restate environment's public key):

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
RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS = "publickeyv1_<the eu Restate environment's>"
```

`live.wrangler.toml`:

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

Its Workers are `rafi-app-eu`, `rafi-live-eu` and, for 0.1.0, `rafi-engine-eu-v0-1-0`. Then, for the environment:

1. **Its version file**, optionally: `echo 0.1.0 > VERSION.eu`. With `CLOUDFLARE_ENV=eu`, the deploy reads
   `VERSION.eu` when it exists and `VERSION` otherwise, so with one the installations upgrade each on its own.
2. **Steps 3 to 8 for it**: its database (`npx -y wrangler d1 create rafi-eu`, with a
   [jurisdiction](#keeping-data-in-the-eu) if this installation keeps its data in the EU; the ID into the three
   `[env.eu]` sections), its domain, its Clerk instance, its Restate environment (the public key
   into `[env.eu.vars]`), and its app Worker's secrets, set with the environment chosen:

   ```sh
   CLOUDFLARE_ENV=eu npx -y wrangler secret bulk .prod.vars -c app.wrangler.toml   # creates rafi-app-eu
   ```

   (`--env eu` instead of `CLOUDFLARE_ENV=eu` does the same; so for `secret put`.) The checks of step 10 take it
   too: `CLOUDFLARE_ENV=eu release/deploy.sh check`.
3. **Its workflows**, copies of the two:

   ```sh
   cp .github/workflows/deploy.yaml .github/workflows/deploy-eu.yaml
   cp .github/workflows/restate-cleanup.yaml .github/workflows/restate-cleanup-eu.yaml
   ```

   In both copies, set `CLOUDFLARE_ENV: "eu"` under `env:`, rename the concurrency group (`group: rafi-eu`), so
   the two installations' runs don't wait for or replace each other's, and uncomment `environment:` on every job
   that has the line, naming a GitHub environment of the installation's (`environment: eu`). Optionally rename
   the workflows (`name: Deploy (eu)`, `name: Restate cleanup (eu)`), to tell them apart under **Actions**.
4. **Its secrets**, in that GitHub environment: the four of step 9, its own Restate admin URL and key among them
   (`gh secret set RESTATE_AUTH_TOKEN --env eu --repo <you>/<name>`, and so on). A job's environment secrets take
   the place of the repository's of the same name, and a job without an environment gets the repository's: name
   one on the top-level installation's jobs too (`environment: production`, holding its secrets), so neither ever
   runs with the other's.
5. **The paths** each deploy runs on: in `deploy-eu.yaml`, `VERSION.eu` instead of `VERSION` and `"VERSION.*"`
   (keep `VERSION` while there is no `VERSION.eu`); in `deploy.yaml`, drop `"VERSION.*"`, so that changing
   `VERSION.eu` doesn't run the top-level installation's deploy too. Both keep `legal/**` and `*.wrangler.toml`: a
   change there may be either's, and deploying the release an installation already runs again is harmless (the
   same code again), but for the live Worker: any edit of `live.wrangler.toml`, another installation's section
   included, redeploys every installation's live Worker on its next deploy, dropping its phones' connections, so
   edit it between events.

Then push: `Deploy (eu)` deploys the environment's Workers, and its cleanup touches only its engines.

## Verifying a release by hand

A release has one asset, `worker.tar.gz`. Its release page shows its SHA256 digest, Rafi's releases are immutable
(neither the asset nor the tag changes once published), and its build provenance is attested. Every deploy checks
the attestation; by hand, anywhere outside your repository:

```sh
gh release download v0.1.0 -R sagikazarmark/rafi -p worker.tar.gz
sha256sum worker.tar.gz                     # the digest on the release page (shasum -a 256 on macOS)
gh attestation verify worker.tar.gz -R sagikazarmark/rafi \
  --signer-workflow github.com/sagikazarmark/rafi/.github/workflows/release.yaml \
  --source-ref refs/tags/v0.1.0 --deny-self-hosted-runners
```

## Logs

- **Deploys and cleanups**: your repository's **Actions**, one run each (`gh run list --repo <you>/<name>`, `gh
  run view <run-id> --log`), its summary page with the reports above. A failed run is GitHub's failure email.
- **The Workers**: each has its own logs in the dashboard (**Workers & Pages >** the Worker **> Logs**), or live
  with `npx -y wrangler tail <Worker>` from your repository: `rafi-app` (the app), `rafi-live` (the live channels) and each
  release's engine, `rafi-engine-v0-1-0` and so on. Only the application's own lines are kept, never the
  per-request records (they would hold client IPs and cookies).
- **Restate Cloud**'s console shows the invocations, including any stuck in retry.

More, with the reasons and what to do when something goes wrong:
[Cloudflare Workers](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md) and
[the runbook](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/runbook.md).
