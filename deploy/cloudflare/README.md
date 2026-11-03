# Rafi on Cloudflare Workers

One installation of [Rafi](https://github.com/sagikazarmark/rafi), the verifiable raffle for in-person events, on
your own Cloudflare account: three Workers and one D1 database, with [Restate Cloud](https://cloud.restate.dev)
running the raffle engine's invocations and [Clerk](https://clerk.com) signing organizers in. Nothing here is
compiled. This directory is your configuration and your legal documents, and its GitHub Actions workflows deploy the
Rafi release that `VERSION` names, as Rafi's release workflow built and attested it.

```
VERSION                                 the Rafi release you deploy (0.1.0)
wrangler.toml                           the app Worker (web app, API, phones' sockets, hourly housekeeping):
                                        your database ID and domain
engine.wrangler.toml                    the raffle engine Worker, one per release, which Restate calls:
                                        your database ID and your Restate environment's public key
rooms.wrangler.toml                     the rooms Worker (the raffles' live channels): your database ID
legal/                                  your terms and privacy notice
.prod.vars.example                      the app Worker's secrets, and what each is
.github/workflows/deploy.yaml           the deploy, on a push to main that changes any of the above
.github/workflows/restate-cleanup.yaml  daily: deletes the engine Workers of earlier releases once drained
```

This page is the first deploy, step by step and in order: each step says what it produces and where that goes. The
reference behind it, with the reasons, is
[docs/operations/cloudflare.md](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md).

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
  - [Node.js](https://nodejs.org) 22.18 or later, for `npx wrangler@4.148.0` (the version the deploy uses) and
    `cf`;
  - [`cf`](https://developers.cloudflare.com/cf/), Cloudflare's CLI (npm `cf`, in beta: commands can change, so
    this page pins `1.0.0-beta.27` and gives the raw API calls too): `npm install --global cf@1.0.0-beta.27`;
  - `jq`, `openssl`, `curl`, `git`;
  - optionally the [Restate CLI](https://docs.restate.dev/installation) (`npx --yes @restatedev/restate@1.8.1`),
    for what the cleanup leaves to you (a paused invocation).

Below, `<you>/<name>` is your new repository, `example.org` your zone, and `raffle.example.org` the app's hostname.

## 1. Create your repository

Produces: a private repository of yours whose root is this directory, at the release's tag.

**From a terminal**:

```sh
gh repo create <you>/<name> --private --clone
cd <name>
# deploy/cloudflare at the tag v0.1.0, at the repository's root, .github/ and the dotfiles included (GNU tar):
gh api repos/sagikazarmark/rafi/tarball/v0.1.0 | tar -xz --strip-components=3 --wildcards '*/deploy/cloudflare/*'
# on macOS, whose tar matches patterns by default and refuses --wildcards:
#   gh api repos/sagikazarmark/rafi/tarball/v0.1.0 | tar -xz --strip-components=3 '*/deploy/cloudflare/*'
git add -A
git commit -m "Rafi 0.1.0"
git branch -M main
git push -u origin main
```

The workflows run on `main` (and a scheduled one only on the default branch), so the branch is `main`.

**From GitHub's web UI**: create the repository at [github.com/new](https://github.com/new) (private, with no
README, `.gitignore` or license), download the release's source (`Source code (zip)` on
[the release page](https://github.com/sagikazarmark/rafi/releases/tag/v0.1.0)), and on the new repository's page
choose **uploading an existing file** and drop in the *contents* of its `deploy/cloudflare` folder. The two workflow
files and `.gitignore` are hidden files (on macOS, ⇧⌘. shows them in Finder): check that the commit lists
`.github/workflows/deploy.yaml`, `.github/workflows/restate-cleanup.yaml` and `.gitignore`, and add any that's
missing with **Add file > Create new file**, typing its path and pasting its content. Commit to `main`. Steps 3 and
8 need a terminal anyway, so clone it there (`gh repo clone <you>/<name>`).

**The first push starts the Deploy workflow**, which fails, harmlessly: its first job downloads and verifies the
release, then stops at **Check it against this repository**, which refuses the placeholders still in the
configurations (the zero database ID, the identity key), before any step that uses your secrets or touches
Cloudflare or Restate. The daily Restate cleanup stops at the same check until then. To skip those failed runs, keep your
commits local and push at step 10 (the steps in between need the repository to exist on GitHub, not its content),
or disable both workflows (**Actions**, the workflow, **⋯ > Disable workflow**) and enable them at step 10.

## 2. A Cloudflare API token

Produces: the GitHub secrets `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.

The workflows deploy with an **account API token** (owned by the account, not by you, so it keeps working whoever
leaves) with exactly:

| Permission (API name) | Resource | What it's for |
|---|---|---|
| Account · Workers Scripts · Edit (`Workers Scripts Write`) | your account | Uploading and deploying the three Workers (code, static files, Durable Object class and migrations, Cron, custom domain, workers.dev on or off), reading their deployments and versions, the workers.dev subdomain, deleting a drained engine Worker |
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
npx --yes wrangler@4.148.0 login
npx --yes wrangler@4.148.0 d1 create rafi --jurisdiction eu    # prints the database_id
id=<the database_id it printed>
sed -i.bak "s/00000000-0000-0000-0000-000000000000/$id/" wrangler.toml engine.wrangler.toml rooms.wrangler.toml
rm wrangler.toml.bak engine.wrangler.toml.bak rooms.wrangler.toml.bak
```

All three Workers use the one database (the app reads and writes it, the engine writes the draws, the live
channels read their raffle), so the deploy refuses configurations whose `database_id`s differ. `--jurisdiction eu`
keeps it in the EU, as `RAFI_DO_JURISDICTION = "eu"` in the three files keeps the Durable Objects; a jurisdiction
can't be changed later.

## 4. Your domain

Produces: the app's hostname in `wrangler.toml`, and the value of `RAFI_PUBLIC_URL` (step 8).

Pick the hostname organizers and phones will use, on your zone, with no DNS record of its own yet: each deploy
attaches it to the app Worker as a custom domain (Cloudflare creates its DNS record and certificate), and from a
workflow it takes over an existing record or custom domain of that name without asking. Uncomment `routes` in
[`wrangler.toml`](wrangler.toml) with it:

```sh
sed -i.bak 's|^# routes = .*|routes = [{ pattern = "raffle.example.org", custom_domain = true }]|' wrangler.toml
rm wrangler.toml.bak
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
(the region is part of every URL below). On its **Developers** page:

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

Produces: the app Worker, `rafi`, holding only its secrets; the first deploy puts the app in it.

The deploy refuses to create the app Worker itself: its secrets come first (wrangler won't deploy it while one of
`[secrets] required` is missing). From your repository:

```sh
cp .prod.vars.example .prod.vars
openssl rand -hex 32          # for RAFI_POW_KEY
$EDITOR .prod.vars            # fill in every value
npx --yes wrangler@4.148.0 secret bulk .prod.vars -c wrangler.toml
rm .prod.vars
```

Wrangler sees no Worker `rafi` yet and creates it with the secrets (it asks first when run in a terminal). One at a
time instead: `npx --yes wrangler@4.148.0 secret put <NAME> -c wrangler.toml`, which prompts for the value. Later
changes are `secret put` too; deploys keep the secrets. `.prod.vars` is in `.gitignore`: never commit it, and
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
RAFI_DEPLOY_VERSION=1 release/deploy.sh check
```

(`release/` is git-ignored.) Then commit and push, and watch the run:

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
   repository (`VERSION`, the three configurations, the deploy interface);
2. **D1 migrations**: applies the release's migrations;
3. **Rooms Worker**: deploys `rafi-rooms` (the `RaffleRoom` Durable Objects), only when its code or
   `rooms.wrangler.toml` changed;
4. **App Worker (upload)** and **Engine Worker**, side by side: the app's new version uploaded, not serving yet
   (on the first deploy it is deployed directly), and this release's engine Worker, `rafi-engine-v0-1-0`, deployed
   at `https://rafi-engine-v0-1-0.<subdomain>.workers.dev/restate`;
5. **Register the engine, deploy the app**: registers the engine with Restate (refusing a release older than the
   one Restate runs), then deploys the app's new version with its routes and Cron.

`/healthz` answering `ok 0.1.0` means the app Worker has its secrets and documents; anything missing answers `500
misconfigured` with the reason (the app Worker's logs have the whole of it). Then run a **test-mode raffle** end to
end: sign in at `https://raffle.example.org`, create a raffle with **Test mode** on, open its participant page on a
phone, add fake entrants, freeze, and draw one item with the projector open. The phone follows along live (the
rooms Worker), and the verification page shows **✅ All 1 draws check out** under a **🧪 TEST MODE** note. That has
been through Clerk, the database, the live channels, Restate, the engine and real drand.

From then on a push to `main` that changes `VERSION`, `legal/` or one of the three configurations deploys; so does
**Actions > Deploy > Run workflow**. One deploy or cleanup runs at a time.

## Upgrading

Change `VERSION` to the new release and push:

```sh
echo 0.2.0 > VERSION
git commit -am "Rafi 0.2.0" && git push
```

Read the release notes first: they say when a configuration, `.prod.vars.example` or a secret changed, or when an
upgrade needs a step of yours. When a release's notes say its **`DEPLOY_VERSION`** changed (the deploy's check says
so too, and stops), copy the two workflows from this directory at that release's tag before pushing:

```sh
for f in deploy.yaml restate-cleanup.yaml; do
  gh api -H 'Accept: application/vnd.github.raw' \
    "repos/sagikazarmark/rafi/contents/deploy/cloudflare/.github/workflows/$f?ref=v0.2.0" > ".github/workflows/$f"
done
```

**Never go back to an older release.** Migrations only go forward, so an older release meets a schema it doesn't
know. The deploy refuses to register an engine older than the one Restate runs, but only in its last job, after the
earlier ones have redeployed the older rooms Worker (when it differs). A bad release is fixed by the next one.

## What a deploy does to live raffles

A deploy doesn't check whether a raffle is live; you choose when to deploy. What happens:

- **The rooms Worker** is redeployed only when its code or `rooms.wrangler.toml` changed (the release notes say
  so). That restarts every live channel: phones and projectors lose their connection, reconnect and catch up, but
  the moment on the screen is lost.
- **The app** switches to the new version: requests go to it from then on.
- **A draw in flight** stays on the engine Worker it started on, its release's, until it finishes; new draws go to
  the new engine.

Still, don't deploy during an event: from when registration opens at the venue until the last raffle has
finished. The [runbook](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/runbook.md#before-a-deploy-is-anything-live)
has the query that lists every organizer's live raffles.

## Cleanup

Each release leaves its engine Worker behind, serving what started on it. The **Restate cleanup** workflow runs
daily at 04:37 UTC, and by hand (**Actions > Restate cleanup > Run workflow**, or `gh workflow run
restate-cleanup.yaml --repo <you>/<name>`). It looks at this installation's engine deployments in Restate (those
the deploy registered, with `rafi`, this repository and this account in their metadata):

- **drained** (nothing pinned to it any more): deletes its Worker, then removes the deployment from Restate;
- **draining**: lists the invocations still pinned to it, and leaves it for a later run;
- **paused** invocations (Restate stopped retrying one and waits for a human) keep their deployment draining
  until you resume one on the latest deployment, if the new engine can carry it on, or kill it:

  ```sh
  export RESTATE_ADMIN_URL=https://<env-id>.env.<region>.restate.cloud:9070 RESTATE_AUTH_TOKEN=<an Admin-role key>
  npx --yes @restatedev/restate@1.8.1 invocations describe <id>    # why it stopped
  npx --yes @restatedev/restate@1.8.1 invocations resume <id> --deployment latest
  npx --yes @restatedev/restate@1.8.1 invocations kill <id>        # or end it where it stopped
  ```

- the **active** one (the latest): kept;
- deployments that aren't this installation's: listed, never touched.

GitHub runs scheduled workflows only on the default branch (keep it `main`), and in a public repository disables
them after 60 days without repository activity (**Actions > Restate cleanup > Enable workflow** turns it back
on). Deploys and cleanups share one concurrency group and never cancel a running one, but GitHub keeps only one
run waiting: a cleanup queued behind a running deploy replaces another deploy waiting there, which then shows as
cancelled. Run that deploy again.

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
  run view <run-id> --log`). A failed run is GitHub's failure email.
- **The Workers**: each has its own logs in the dashboard (**Workers & Pages >** the Worker **> Logs**), or live
  with `npx --yes wrangler@4.148.0 tail <Worker>`: `rafi` (the app), `rafi-rooms` (the live channels) and each
  release's engine, `rafi-engine-v0-1-0` and so on. Only the application's own lines are kept, never the
  per-request records (they would hold client IPs and cookies).
- **Restate Cloud**'s console shows the invocations, including any stuck in retry.

More, with the reasons and what to do when something goes wrong:
[Cloudflare Workers](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md) and
[the runbook](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/runbook.md).
