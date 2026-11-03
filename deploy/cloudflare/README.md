# Rafi on Cloudflare Workers

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/sagikazarmark/rafi/tree/v0.1.0/deploy/cloudflare)

One installation of [Rafi](https://github.com/sagikazarmark/rafi), the verifiable raffle for in-person events,
on your own Cloudflare account: one Worker and one D1 database, with [Restate Cloud](https://cloud.restate.dev)
running the raffle engine and [Clerk](https://clerk.com) signing organizers in. Nothing here is compiled:
`npm run build` downloads a release of Rafi and puts your legal documents into it, and `npm run deploy` deploys it
with your configuration. The button does both from a repository of yours ([the quick way](#the-quick-way)); the
steps below do the same by hand.

The button deploys the template as it is at the release this README is from (`v0.1.0`), so the template and the
release it downloads always agree.

```
wrangler.toml       the Worker's shape, with your database ID and domain
legal/              your terms and privacy notice
.dev.vars.example   your secrets: what each is, and how to set them
package.json        the release it deploys (config.rafi_version), and the build and deploy scripts
worker/             what `npm run build` downloads (the release, with legal/ copied in); not tracked
```

## You'll need

- a Cloudflare account on the **paid** Workers plan (Workers, D1, Durable Objects): verifying a BLS signature and
  rendering the participant page don't fit the free plan's CPU time;
- a domain on that account (a zone), for the Worker and for Clerk's DNS records;
- a [Restate Cloud](https://cloud.restate.dev) account, and its CLI, `restate`
  ([install](https://docs.restate.dev/develop/local_dev#running-restate-server--cli-locally));
- a [Clerk](https://clerk.com) account;
- your legal documents (below), reviewed, and the name and email of the operator (you);
- [Node.js](https://nodejs.org) with npm (`npm install` brings the `wrangler` the release was checked with),
  `curl`, `tar`.

## Deploy

Copy this directory into a repository of yours (private is fine). Then, in this order (each step needs what the one
before it gave you):

1. **The domain.** Pick the origin organizers and phones will use, e.g. `https://raffle.example.org`. It is
   `RAFI_PUBLIC_URL` below, what Clerk allows, and the Worker's route. Put it into `routes` in
   [`wrangler.toml`](wrangler.toml) (`pattern = "raffle.example.org", custom_domain = true`).
2. **Clerk.** Create an application, then its **production instance** for your domain. Clerk lists DNS records
   (CNAMEs for its Frontend API, accounts portal and email) to add to your zone in Cloudflare (**DNS > Records**,
   proxy status *DNS only*); sign-in works once Clerk has verified them and issued its certificates, which can take
   a while. Note the production keys: `pk_live_…` and `sk_live_…`.
3. **Restate Cloud.** Create an environment (in the EU if personal data must stay there), then:

   ```sh
   restate cloud login
   restate cloud env configure      # pick the environment
   ```

   From the environment's page in the Restate Cloud console, note its ingress URL
   (`https://<env>.env.eu.restate.cloud:8080`), an API key with ingress access (create one), and the request
   identity's public key (`publickeyv1_…`), which the Worker checks every call from the engine against.
4. **The database.**

   ```sh
   npm install
   npx wrangler d1 create rafi --jurisdiction eu   # paste the database_id it prints into wrangler.toml
   ```

5. **Your legal documents**: [below](#your-legal-documents). The Worker refuses every request while one is still
   the template.
6. **The secrets.** `cp .dev.vars.example .dev.vars` and fill it in (`.dev.vars` is ignored by git): the URL, the
   Restate and Clerk values above, a proof-of-work key (`openssl rand -hex 32`), the operator and the documents'
   versions.
7. **Deploy.**

   ```sh
   npm run build                    # the release -> worker/, with legal/ copied in
   npx wrangler deploy              # the first time: creates the Worker, stops at the missing secrets
   npx wrangler secret bulk .dev.vars
   npm run deploy                   # the D1 migrations, then the deploy
   ```

   Open `https://<your domain>/healthz`: it answers `ok` and the version. Anything else answers `misconfigured`
   with the reason (`npx wrangler tail` has the whole of it).
8. **Register the raffle engine** with Restate, once the Worker answers:

   ```sh
   restate deployments register https://raffle.example.org/restate
   ```

Every later deploy is `npm run build && npm run deploy`. (`npm install` writes a `package-lock.json`; commit it,
it pins wrangler's own dependencies.)

### The quick way

The button copies this directory into a repository of yours, creates the database and the Durable Object, asks for
the secrets, and deploys from that repository on every push
([Workers Builds](https://developers.cloudflare.com/workers/ci-cd/builds/)), with `npm run build` as the build
command and `npm run deploy` as the deploy command, which it takes from `package.json` (check them on the setup
page). You still need steps 1 to 3 and 8. Two things differ:

- **its database is not in the EU**: the button creates one without a jurisdiction. If personal data must stay in
  the EU, create one with `--jurisdiction eu` as above, put its ID in `wrangler.toml`, push, and delete the
  button's;
- **its first deploy serves nothing**: the Worker refuses every request while `legal/` holds the templates. Fill
  them in in the repository it made (below) and push, which deploys.

## Your legal documents

The Worker serves your terms (with the data processing agreement organizers accept by using the service) and
your privacy notice (naming you and the subprocessors: Cloudflare, Restate Cloud, Clerk, drand) from
[`legal/terms.md`](legal/terms.md) and [`legal/privacy.md`](legal/privacy.md), which `npm run build` copies into
`worker/worker-assets/legal/`. They ship as engineering drafts marked **Template.**: fill in the bracketed parts,
have a lawyer review them, delete the marker. While a document still carries it, the Worker refuses every request,
saying why. Their versions are the `RAFI_TERMS_VERSION` and `RAFI_PRIVACY_VERSION` secrets (e.g. `2026-10`); a
changed document is a new version.

## Upgrading

Change `config.rafi_version` in [`package.json`](package.json) to the new release and push (Workers Builds then
builds and deploys it), or run `npm run build && npm run deploy`. A `RAFI_VERSION` build variable (in Workers
Builds, among the Worker's build settings; by hand, `RAFI_VERSION=<version> npm run build`) overrides the version
`package.json` names.

A release has one asset, `worker.tar.gz`. GitHub shows its SHA256 digest next to it on the release page (computed
on upload; the release workflow checks it is the one the build produced, and the repository's releases are
immutable, so the asset can't change under its tag once published). Its provenance is attested: GitHub built it
from the tag, in the release workflow. To check both (with the GitHub CLI):

```sh
curl -fsSLO https://github.com/sagikazarmark/rafi/releases/download/v<version>/worker.tar.gz
sha256sum worker.tar.gz                                     # the digest the release page shows
gh attestation verify worker.tar.gz -R sagikazarmark/rafi
```

`/healthz` answers `ok` and the version running, so you can tell which release a Worker is on.

The release notes say when `wrangler.toml`, `package.json` or `.dev.vars.example` changed shape, or when an
upgrade needs a step of yours; compare yours with the template at that release's tag then.

## From source

In the Rafi repository, `just cloudflare-artifact /path/to/your/copy/worker` builds what a release holds, into
this directory's `worker/`; then, here, put `legal/` in place and deploy without downloading:

```sh
mkdir -p worker/worker-assets/legal && cp legal/terms.md legal/privacy.md worker/worker-assets/legal/
npm run deploy
```

The full operations guide, with what to check after the first deploy:
[docs/operations/cloudflare.md](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md).
