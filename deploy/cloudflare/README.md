# Deploy Rafi on Cloudflare Workers

One installation of [Rafi](https://github.com/sagikazarmark/rafi), the verifiable raffle for in-person events, on
your own Cloudflare account. Nothing here is compiled: this repository holds your configuration and your legal
documents, and its GitHub Actions workflows deploy the Rafi release that `VERSION` names.

This page covers the tasks you'll come back to. The details, and the reasons behind them, are in Rafi's
[Cloudflare operations guide](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md).

## Prerequisites

- A **Cloudflare** account on the paid Workers plan, with a workers.dev subdomain. Optionally your domain on it as a
  zone, for the app's own address; without one the app is at its workers.dev address
  ([INSTALL.md, step 4](INSTALL.md#4-the-apps-address)).
- A **[Restate Cloud](https://cloud.restate.dev)** environment, which runs the raffle engine. Use one environment
  per installation.
- A **[Clerk](https://clerk.com)** application, which signs organizers in: its production instance on your domain,
  or, on workers.dev, its development instance (for trying Rafi or small, private use).
- A **GitHub** repository, created from this template, which runs the deploys.
- Your legal documents, reviewed by a lawyer, each naming its version at its top (`version:`; a changed document is
  a new version).
- On your machine: [`gh`](https://cli.github.com), [Node.js](https://nodejs.org) 22.18 or later, and `jq`.

## Installation

The first deploy is done once, step by step, in [INSTALL.md](INSTALL.md): from an empty repository to `/healthz`
answering. Two decisions there can't be changed later, so make them first:

- whether data stays in the EU ([Keeping data in the EU](INSTALL.md#keeping-data-in-the-eu));
- the installation's names, if the account will hold more than one ([Several
  installations](INSTALL.md#several-installations)).

## Upgrading

Read the release notes, then set `VERSION` to the new release and push:

```sh
echo 0.2.0 > VERSION
git commit -am "Rafi 0.2.0" && git push
```

With several installations in one repository, edit that installation's `VERSION.<environment>` instead.

Check the release notes for any changes in configuration, secrets, or workflows.
How to carry those over is in [Upgrading](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md#upgrading).

**Never go back to an older release.** Migrations only go forward. The deploy refuses an older release before it
touches anything; a bad release is fixed by the next one.

## Deploying

A push to `main` that changes `VERSION`, `legal/` or a `wrangler.*.toml` deploys.

To deploy by hand:

```sh
gh workflow run deploy.yaml
gh run watch
```

Don't deploy during an event, from when registration opens at the venue until the last raffle has finished. The
[runbook](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/runbook.md#before-a-deploy-is-anything-live)
has a query that lists live raffles. If one runs anyway:

- a draw in flight finishes on its own release's engine;
- phones and projectors lose their live connection only when the live Worker changes (the release notes say so,
  and so does any edit of `wrangler.live.toml`). They reconnect by themselves.

A failed run is finished by re-running its failed jobs (**Re-run failed jobs**), never undone.

## Cleanup

Each release leaves its engine Worker behind until the draws on it finish. The **Restate cleanup** workflow runs
daily and removes the ones Restate reports as drained. Run it by hand with:

```sh
gh workflow run restate-cleanup.yaml
```

Its run summary lists anything that needs you:

- **A paused invocation** keeps an old engine from draining. Look at why it stopped, then resume it on the latest
  engine or kill it:

  ```sh
  export RESTATE_ADMIN_URL=https://<env-id>.env.<region>.restate.cloud:9070 RESTATE_AUTH_TOKEN=<an Admin-role key>
  npx -y @restatedev/restate invocations describe <id>
  npx -y @restatedev/restate invocations resume <id> --deployment latest   # or: invocations kill <id>
  ```

- **A Worker left behind** is an engine Worker the cleanup couldn't delete, or one no Restate deployment of this
  installation refers to. The cleanup never deletes these itself. Once you're sure nothing needs it:

  ```sh
  CLOUDFLARE_API_TOKEN=<token> CLOUDFLARE_ACCOUNT_ID=<account ID> npx -y wrangler delete <worker>
  ```

GitHub disables scheduled workflows in a public repository after 60 days without activity: **Actions > Restate
cleanup > Enable workflow** turns it back on.

## Rotating secrets

Between events, one at a time, checking `/healthz` after each:

- **The app Worker's secrets** (listed in `.dev.vars.example`): `npx -y wrangler secret put <NAME> -c wrangler.app.toml`.
- **The workflows' secrets**: `gh secret set <NAME>`.
- **The engine's identity key**: add the new key next to the old one in `wrangler.engine.toml`, push, and remove the
  old one later.

The order and the details for each are in the runbook's
[Rotating secrets](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/runbook.md#rotating-secrets).

## Logs

- **Deploys and cleanups**: your repository's **Actions**. Each run's summary page reports what it did, and its
  **Artifacts** keep the JSON behind it for 14 days.
- **The Workers**: **Workers & Pages >** the Worker **> Logs** in the dashboard, or `npx -y wrangler tail <worker>`:
  `rafi-app`, `rafi-live`, and each release's `rafi-engine-v…`.
- **The raffle engine's invocations**: the Restate Cloud console.

## Troubleshooting

A failed run says why in the log of the step that failed. Most failures are a secret, a token permission or a
configuration value. The common ones and their fixes are in the guide's
[Troubleshooting](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md#troubleshooting);
what to do during an event is in the [runbook](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/runbook.md).
