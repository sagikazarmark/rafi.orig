# Rafi on Cloudflare Workers

One installation of [Rafi](https://github.com/sagikazarmark/rafi), the verifiable raffle for in-person events,
on your own Cloudflare account: one Worker and one D1 database, with [Restate Cloud](https://cloud.restate.dev)
running the raffle engine and [Clerk](https://clerk.com) signing organizers in. Nothing here is built: `deploy.sh`
fetches a release of Rafi and deploys it with your configuration.

```
wrangler.toml       the Worker's shape, with your database ID and domain
legal/              your terms and privacy notice
.dev.vars.example   your secrets: what each is, and how to set them
deploy.sh           fetch the release it names, put legal/ in place, migrate, deploy
```

## You'll need

- a Cloudflare account on the **paid** Workers plan (Workers, D1, Durable Objects): verifying a BLS signature and
  rendering the participant page don't fit the free plan's CPU time;
- a Restate Cloud environment: its ingress URL, an API key with ingress access, and its public identity key;
- a Clerk application: its publishable and secret keys. In Clerk, allow the origin your installation will have;
- your legal documents (below), reviewed, and the name and email of the operator;
- `wrangler` ([install](https://developers.cloudflare.com/workers/wrangler/install-and-update/)), `curl`, `tar`.

## Deploy

Copy this directory into a repository of yours (private is fine), then:

```sh
wrangler d1 create rafi --jurisdiction eu   # paste the database_id it prints into wrangler.toml
./deploy.sh fetch                           # the release -> rafi/, checked against its checksums
wrangler deploy                             # creates the Worker; stops at the missing secrets
cp .dev.vars.example .dev.vars              # fill it in (.dev.vars is ignored by git)
wrangler secret bulk .dev.vars
./deploy.sh deploy                          # legal/ into place, the D1 migrations, the deploy
```

Then give the Worker your domain: `routes` in [`wrangler.toml`](wrangler.toml), or **Settings > Domains &
Routes**. It must be the origin `RAFI_PUBLIC_URL` names and Clerk allows. Every later deploy is `./deploy.sh`.

**The quick way.** The button copies this directory into a repository of yours, creates the database and the
Durable Object, asks for the secrets, and deploys from that repository on every push
([Workers Builds](https://developers.cloudflare.com/workers/ci-cd/builds/)); `package.json` is there so it picks
`./deploy.sh fetch` as the build command and `./deploy.sh deploy` as the deploy command (check them on the setup
page). Its database has no jurisdiction: if personal data must stay in the EU, create one with `--jurisdiction eu`
as above, put its ID in `wrangler.toml`, push, and delete the button's.

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/sagikazarmark/rafi/tree/main/deploy/cloudflare)

## Your legal documents

The Worker serves your terms (with the data processing agreement organizers accept by using the service) and
your privacy notice (naming you and the subprocessors: Cloudflare, Restate Cloud, Clerk, drand) from
[`legal/terms.md`](legal/terms.md) and [`legal/privacy.md`](legal/privacy.md). They ship as engineering drafts
marked **Template.**: fill in the bracketed parts, have a lawyer review them, delete the marker. A document that
still carries it makes the Worker refuse every request. Their versions are the `RAFI_TERMS_VERSION` and
`RAFI_PRIVACY_VERSION` secrets (e.g. `2026-10`); a changed document is a new version.

## Upgrading

Change `RAFI_VERSION` at the top of [`deploy.sh`](deploy.sh) and deploy. The release notes say when
`wrangler.toml` or `.dev.vars.example` changed shape; compare yours with
[the template's](https://github.com/sagikazarmark/rafi/tree/main/deploy/cloudflare) then.

## From source

In the Rafi repository, `./scripts/cloudflare-artifact.sh /path/to/your/copy/rafi` builds what a release holds,
into this directory's `rafi/`; then `./deploy.sh deploy` here.

The full operations guide, with what to check after the first deploy:
[docs/operations/cloudflare.md](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md).
