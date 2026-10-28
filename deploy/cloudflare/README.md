# Rafi on Cloudflare Workers

One installation of [Rafi](https://github.com/sagikazarmark/rafi), the verifiable raffle for in-person events,
on your own Cloudflare account: one Worker, one D1 database, with [Restate Cloud](https://cloud.restate.dev)
running the raffle engine and [Clerk](https://clerk.com) signing organizers in. Nothing here needs a Rust
toolchain: this directory fetches a release of Rafi and deploys it with your configuration.

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/sagikazarmark/rafi/tree/main/deploy/cloudflare)

## You'll need

- a Cloudflare account on the **paid** Workers plan (Workers, D1, Durable Objects): verifying a BLS signature and
  rendering the participant page don't fit the free plan's CPU time;
- a Restate Cloud environment: its ingress URL, an API key with ingress access, and its public identity key;
- a Clerk application: its publishable and secret keys. In Clerk, allow the origin your installation will have;
- your legal documents (below), reviewed, and the name and email of the operator.

## Deploy

**With the button.** Cloudflare copies this directory into a repository of yours, creates the database and the
Durable Object, asks for the values in [`.dev.vars.example`](.dev.vars.example) (what each is: `package.json`,
under `cloudflare.bindings`), then builds and deploys, and again on every push. The build command is
`npm run build`: it fetches the release `package.json` names (`rafi.version`) and checks it against the release's
checksums. The deploy command must be `npm run deploy` (the database migrations, then the deploy): the setup page
takes it from `package.json`; check it there, and under **Settings > Build** afterwards.

The first deploy serves the template documents, which the Worker refuses to run with: replace them (next
section), push, and Cloudflare deploys again. Then give the Worker your domain: `routes` in
[`wrangler.toml`](wrangler.toml) (or **Settings > Domains & Routes**). It must be the origin `RAFI_PUBLIC_URL`
names and Clerk allows.

**By hand.** Copy this directory, and in it:

```sh
npm ci                                          # wrangler
npx wrangler d1 create rafi --jurisdiction eu   # note the database_id; paste it into wrangler.toml
npm run build                                   # fetches the release into rafi/, puts legal/ in place
npx wrangler deploy                             # creates the Worker; stops at the missing secrets
cp .dev.vars.example .dev.vars                  # fill it in (it is ignored by git)
npx wrangler secret bulk .dev.vars
npm run deploy                                  # the D1 migrations, then the deploy
```

`npm run deploy` is every later deploy too: it applies the database migrations first.

**EU data residency.** The button creates the database without a jurisdiction. If personal data must stay in
the EU, create it yourself with `--jurisdiction eu` (as above), put its ID in `wrangler.toml`, push (or
`npm run deploy`); then delete the one the button made. The live channels' Durable Objects are in the EU
either way (`RAFI_DO_JURISDICTION` in `wrangler.toml`).

## Your legal documents

The Worker serves your terms (with the data processing agreement organizers accept by using the service) and
your privacy notice (naming you and the subprocessors: Cloudflare, Restate Cloud, Clerk, drand) from
[`legal/terms.md`](legal/terms.md) and [`legal/privacy.md`](legal/privacy.md). They ship as engineering drafts
marked **Template.**: fill in the bracketed parts, have a lawyer review them, delete the marker. A document that
still carries it makes the Worker refuse every request. Their versions are the `RAFI_TERMS_VERSION` and
`RAFI_PRIVACY_VERSION` secrets (e.g. `2026-10`); a changed document is a new version.

## Upgrading

Change `rafi.version` in [`package.json`](package.json) to the new release and deploy (push, or
`npm run build && npm run deploy`). The release notes say when `wrangler.toml` or `.dev.vars.example` changed
shape; compare yours with
[the template's](https://github.com/sagikazarmark/rafi/tree/main/deploy/cloudflare) then.

## From source

Build the Rafi repository's artifact and point this directory at it, instead of fetching a release:

```sh
./scripts/cloudflare-artifact.sh /path/to/your/copy/rafi     # in the Rafi repository
node build.mjs --local                                        # in your copy: the documents into place
npm run deploy
```

## What's in here

| File | |
|---|---|
| `wrangler.toml` | The Worker's shape, pointed at `rafi/`; your database ID and domain |
| `package.json` | The release this installation deploys (`rafi.version`); the `build` and `deploy` scripts; what each secret is |
| `build.mjs` | Fetches and verifies the release into `rafi/`; copies `legal/` among its static files |
| `.dev.vars.example` | The secrets, which Deploy to Cloudflare asks for |
| `legal/` | Your terms and privacy notice |
| `rafi/` | Not tracked: the release, as fetched |

The full operations guide, including what to check after the first deploy and how to run it locally:
[docs/operations/cloudflare.md](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md).
