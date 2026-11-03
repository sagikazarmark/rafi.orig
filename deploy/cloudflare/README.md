# Rafi on Cloudflare Workers

One installation of [Rafi](https://github.com/sagikazarmark/rafi), the verifiable raffle for in-person events, on
your own Cloudflare account: two Workers and one D1 database, with [Restate Cloud](https://cloud.restate.dev) running
the raffle engine's invocations and [Clerk](https://clerk.com) signing organizers in. Nothing here is compiled: this
directory is your configuration and your legal documents, and a GitHub Actions workflow deploys a release of Rafi
with them on every push.

```
wrangler.toml                  the app Worker (the web app, its API, the live channels, the hourly housekeeping):
                               your database ID and domain
engine.wrangler.toml           the raffle engine Worker, one per release, which Restate calls: your database ID and
                               your Restate environment's public key
legal/                         your terms and privacy notice
.dev.vars.example              the app Worker's secrets: what each is
.github/workflows/deploy.yml   the deploy: which release, and your repository's secrets passed to it
```

## You'll need

- a Cloudflare account on the **paid** Workers plan (Workers, D1, Durable Objects): verifying a BLS signature and
  rendering the participant page don't fit the free plan's CPU time;
- a domain on that account (a zone), for the app Worker and for Clerk's DNS records;
- the account's **workers.dev subdomain** registered (the dashboard's **Workers & Pages** offers it on a first
  visit; a deploy without one stops before changing anything, naming the page): each release's engine Worker is
  served there, where Restate calls it;
- a [Restate Cloud](https://cloud.restate.dev) environment, and an **API key with the Admin role** for the deploy;
- a [Clerk](https://clerk.com) application;
- a GitHub account: your copy of this directory is a repository of yours, and its Actions deploy it;
- your legal documents (below), reviewed, and the name and email of the operator (you);
- for the one-time steps below, [Node.js](https://nodejs.org) 22 or later, for `npx wrangler`.

## Set up

In this order (each step needs what the one before it gave you):

1. **Your repository.** Copy this directory, `.github/` included, into a new repository of yours (private is
   fine), as its root. Its workflow runs on every push to `main`; until the steps below are done it stops at its
   first checks, having changed nothing.
2. **The domain.** Pick the origin organizers and phones will use, e.g. `https://raffle.example.org`. It is
   `RAFI_PUBLIC_URL` below, what Clerk allows, and the app Worker's route: put it into `routes` in
   [`wrangler.toml`](wrangler.toml) (`pattern = "raffle.example.org", custom_domain = true`).
3. **Clerk.** Create an application, then its **production instance** for your domain. Clerk lists DNS records
   (CNAMEs for its Frontend API, accounts portal and email) to add to your zone in Cloudflare (**DNS > Records**,
   proxy status *DNS only*); sign-in works once Clerk has verified them and issued its certificates, which can take
   a while. Note the production keys: `pk_live_…` and `sk_live_…`.
4. **Restate Cloud.** Create an environment (in the EU if personal data must stay there). From its page in the
   Restate Cloud console, note:
   - the **ingress URL** (`https://<env>.env.<region>.restate.cloud:8080`) and an API key with ingress access
     (create one): the app Worker's `RAFI_ENGINE_INGRESS_URL` and `RAFI_ENGINE_API_KEY`;
   - the **admin URL** (`https://<env>.env.<region>.restate.cloud:9070`) and an API key with the **Admin** role
     (create another): the deploy's `RESTATE_ADMIN_URL` and `RESTATE_AUTH_TOKEN`, with which it registers each
     release's engine and removes the drained ones;
   - the request identity's **public key** (`publickeyv1_…`): put it into
     [`engine.wrangler.toml`](engine.wrangler.toml) as `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS`. The engine refuses
     every request not signed with it.
5. **The database.**

   ```sh
   npx wrangler@4.148.0 login
   npx wrangler@4.148.0 d1 create rafi --jurisdiction eu
   ```

   Put the `database_id` it prints into **both** `wrangler.toml` and `engine.wrangler.toml`: the engine writes the
   draws to the app's database.
6. **Your legal documents**: [below](#your-legal-documents). The app Worker refuses every request while one is
   still the template.
7. **The app Worker's secrets.** `cp .dev.vars.example .dev.vars` and fill it in (`.dev.vars` is ignored by git):
   the URL, the Restate ingress and Clerk values above, a proof-of-work key (`openssl rand -hex 32`), the operator
   and the documents' versions. Then either
   - put the whole file's content into a GitHub secret, `RAFI_SECRETS` (next step): every deploy sets them, the
     first one included. A line you delete later isn't removed from the Worker (`npx wrangler secret delete`
     does); or
   - set them yourself, once, before the first deploy: `npx wrangler@4.148.0 secret bulk .dev.vars` (in your
     repository, next to `wrangler.toml`). It creates the app Worker holding only the secrets; the first deploy
     puts the app in it. Change one later with `npx wrangler secret put <NAME>`.

   Without either, the first deploy stops at the app Worker, naming the missing secrets (after applying the
   migrations, which is harmless); set them and run it again.
8. **A Cloudflare API token** (**Manage account > Account API tokens > Create token**, a custom token) for your
   account and your domain's zone, with:
   - Account · **Workers Scripts** · Edit: both Workers (their code, static files, secrets, Cron, Durable Object,
     workers.dev address), the app's custom domain, the workers.dev subdomain looked up, the account's Workers
     listed, a drained engine Worker deleted;
   - Account · **D1** · Edit: "is anything live?" and the migrations;
   - Zone · **Workers Routes** · Edit, for your domain's zone: the app Worker's route.

   Cloudflare's **Edit Cloudflare Workers** template plus Account · D1 · Edit is a superset of these.
9. **GitHub's secrets.** In your repository, **Settings > Secrets and variables > Actions**, add:

   | Secret | |
   |---|---|
   | `CLOUDFLARE_API_TOKEN` | the token above |
   | `CLOUDFLARE_ACCOUNT_ID` | your account's ID (the dashboard's account home, or `npx wrangler whoami`) |
   | `RESTATE_ADMIN_URL` | the admin URL, `https://<env>.env.<region>.restate.cloud:9070` |
   | `RESTATE_AUTH_TOKEN` | the Admin-role API key |
   | `RAFI_SECRETS` | optional: your `.dev.vars`'s content (step 7) |

   If your account or organization restricts which actions run, allow `sagikazarmark/rafi`'s workflows,
   `actions/checkout` and `actions/setup-node`. To approve each deploy first, create an environment (**Settings >
   Environments**, with required reviewers), uncomment `environment:` in
   [`.github/workflows/deploy.yml`](.github/workflows/deploy.yml), and keep the secrets there or in the repository.
10. **Deploy**: commit and push to `main` (or **Actions > Deploy > Run workflow**). The run's log is the deploy's.
    Then open `https://<your domain>/healthz`: it answers `ok 0.1.0`. Anything else answers `misconfigured` with
    the reason (`npx wrangler tail rafi` has the whole of it).

From then on every push to `main` deploys, but one that changes only Markdown outside `legal/`.

## What a deploy does

The workflow is the Rafi repository's
[`deploy-cloudflare.yml`](https://github.com/sagikazarmark/rafi/blob/main/.github/workflows/deploy-cloudflare.yml),
as it is at the release's tag. It downloads the release's one asset, `worker.tar.gz`, checks its provenance (that Rafi's
release workflow built it from that tag, on GitHub's runners), unpacks it into `release/` and runs the release's own
`release/deploy.sh`, which, printing each step:

1. checks the release and your two configurations, and that they agree (the database, the app's name, the
   jurisdiction; no placeholders left);
2. copies `legal/` into the app's static files;
3. checks the account's workers.dev subdomain and that Restate's admin API answers, before changing anything;
4. asks the database **whether any raffle is live** (open, closed, drawing, or published or frozen and dated today or
   yesterday), and stops, listing them, if one is: a deploy restarts the live channels. Run it by hand with
   **deploy_while_live** to deploy anyway;
5. applies the D1 migrations;
6. deploys this release's **engine Worker**, `rafi-engine-<version>` (`rafi-engine-0-1-0`), and registers
   `https://rafi-engine-0-1-0.<your subdomain>.workers.dev/restate` with Restate, then deploys the **app Worker**
   (on the first deploy, the app first). A registered engine Worker is never deployed again: Restate finishes each
   invocation on the deployment it started on, so that code stays as it was. A re-run deploys only the app;
7. prunes: every older engine Worker whose Restate deployment is **drained** (nothing pinned to it any more) is
   removed from Restate, then deleted. One still draining is listed with the invocations pinned to it and pruned
   by a later deploy. A **paused** invocation (Restate stopped retrying it and waits for a human) keeps its
   engine Worker until you resume it on the latest deployment or kill it, with the Restate CLI against your
   environment (`restate invocations resume <id> --deployment latest`, or `restate invocations kill <id>`).

One deploy runs at a time; a push during one waits for it.

## Your legal documents

The app Worker serves your terms (with the data processing agreement organizers accept by using the service) and
your privacy notice (naming you and the subprocessors: Cloudflare, Restate Cloud, Clerk, drand) from
[`legal/terms.md`](legal/terms.md) and [`legal/privacy.md`](legal/privacy.md), which each deploy copies into its
static files. They ship as engineering drafts marked **Template.**: fill in the bracketed parts, have a lawyer
review them, delete the marker. While a document still carries it, the Worker refuses every request, saying why.
Their versions are the `RAFI_TERMS_VERSION` and `RAFI_PRIVACY_VERSION` secrets (e.g. `2026-10`); a changed document
is a new version.

## Upgrading

Change the release in [`.github/workflows/deploy.yml`](.github/workflows/deploy.yml) in **both** places, the
`uses: …/deploy-cloudflare.yml@v<version>` and `version: "<version>"`, to the new release, and push. Never to an
older release: a release older than the one that wrote the database meets a schema it doesn't know. Deploy
between events (the deploy stops while one is live).

The release notes say when `wrangler.toml`, `engine.wrangler.toml`, `.dev.vars.example` or the workflow changed
shape, or when an upgrade needs a step of yours; compare yours with this directory at that release's tag then.
`/healthz` answers `ok` and the version running.

## Checking a release by hand

A release has one asset, `worker.tar.gz`. GitHub shows its SHA256 digest next to it on the release page, and the
repository's releases are immutable, so neither the asset nor its tag can change once published. Its provenance is
attested; the deploy checks the attestation the same way:

```sh
gh release download v<version> -R sagikazarmark/rafi -p worker.tar.gz
sha256sum worker.tar.gz                    # the digest the release page shows
gh attestation verify worker.tar.gz -R sagikazarmark/rafi \
  --signer-workflow github.com/sagikazarmark/rafi/.github/workflows/release.yaml \
  --source-ref refs/tags/v<version> --deny-self-hosted-runners
```

The full operations guide, with the two Workers, the token, and what to check after the first deploy:
[docs/operations/cloudflare.md](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md).
