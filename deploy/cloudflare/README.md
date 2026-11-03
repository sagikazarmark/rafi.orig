# Rafi on Cloudflare Workers

One installation of [Rafi](https://github.com/sagikazarmark/rafi), the verifiable raffle for in-person events, on
your own Cloudflare account: three Workers and one D1 database, with [Restate Cloud](https://cloud.restate.dev)
running the raffle engine's invocations and [Clerk](https://clerk.com) signing organizers in. Nothing here is
compiled. This repository is your configuration and your legal documents, and its GitHub Actions workflows deploy the
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
                                        deploy.sh runs them (npx -y); the lockfile pins them
.dev.vars.example                       the app Worker's secrets, and what each is (filled in as .dev.vars,
                                        git-ignored, never committed)
.github/workflows/deploy.yaml           the deploy, on a push to main that changes any of the above
.github/workflows/restate-cleanup.yaml  daily: removes the engines of earlier releases once drained
INSTALL.md                              the first deploy, once
README.md                               this page: everything after it
```

**The first deploy is [INSTALL.md](INSTALL.md)**, step by step, from an empty repository to `/healthz` answering.
Two things there are decided before it and stay: whether the data stays in the EU ([Keeping data in the
EU](INSTALL.md#keeping-data-in-the-eu)) and the installation's names ([Several
installations](INSTALL.md#several-installations), for a second one in the same account). The app Worker's
[placement](INSTALL.md#placement), a latency option, is explained there too, and can change at any deploy. This
page is what comes after, again and again. The reference behind both, with the reasons, is
[docs/operations/cloudflare.md](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md).

## Upgrading

Change `VERSION` to the new release and push:

```sh
echo 0.2.0 > VERSION
git commit -am "Rafi 0.2.0" && git push
```

That is the whole upgrade. With several installations in one repository, change that installation's
`VERSION.<environment>` instead ([Several installations](INSTALL.md#several-installations)).

Read the release notes first: they say when a configuration, `.dev.vars.example` or a secret changed, when the
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

## What a deploy does

A push to `main` that changes `VERSION` (or a `VERSION.<environment>`), `legal/` or one of the three
configurations deploys, and so does **Actions > Deploy > Run workflow**; one deploy or cleanup runs at a time. The
Deploy workflow's jobs, each on a fresh runner with the same verified release:

1. **Prepare the release**: downloads `worker.tar.gz` of the release the version file names from Rafi's releases,
   refuses it unless its attestation says Rafi's release workflow built it from that tag on GitHub's runners, reads
   the Workers' names from wrangler's dry run of each configuration, and checks it against Restate and Cloudflare
   (an older release than the one Restate runs is refused; so are secrets that don't work), before anything
   touches production. Nothing checks your configurations against the release or against each other;
2. **D1 migrations**: applies the release's migrations;
3. **Live Worker**: deploys `rafi-live` (the `RaffleRoom` Durable Objects), only when its code or
   `live.wrangler.toml` changed;
4. **App Worker (upload)** and **Engine Worker**, side by side: the app's new version uploaded, not serving yet
   (on the first deploy it is deployed directly), and this release's engine Worker, `rafi-engine-v0-1-0`, deployed
   at `https://rafi-engine-v0-1-0.<subdomain>.workers.dev/restate`;
5. **Register the engine, deploy the app**: registers the engine with Restate (refusing a release older than the
   one Restate runs once more, in case one was registered since), then deploys the app's new version with its
   routes and Cron.

A deploy doesn't check whether a raffle is live; you choose when to deploy. What it does to live raffles:

- **The live Worker** is redeployed only when its code or `live.wrangler.toml` changed (the release notes say when
  its code did; any edit of the file counts, a comment too). That restarts every live channel: phones and
  projectors lose their connection, reconnect and catch up, but the moment on the screen is lost.
- **The app** switches to the new version: requests go to it from then on.
- **A draw in flight** stays on the engine Worker it started on, its release's, until it finishes; new draws go to
  the new engine.

Still, don't deploy during an event: from when registration opens at the venue until the last raffle has
finished. The [runbook](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/runbook.md#before-a-deploy-is-anything-live)
has the query that lists every organizer's live raffles.

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

## Rotating keys and tokens

Between events, never during one, and one at a time, checking `/healthz` after each: the
[runbook's Rotating secrets](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/runbook.md#rotating-secrets)
has each, with its order. In short:

- **The app Worker's secrets** (`.dev.vars.example` lists them): `npx -y wrangler secret put <NAME> -c
  app.wrangler.toml` from your repository (`CLOUDFLARE_ENV=<name>` first for a Wrangler environment's); deploys keep
  it. Never `RAFI_POW_KEY` during an event.
- **The engine's identity keys** (`RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` in `engine.wrangler.toml`, a
  comma-separated list): add the new public key beside the old one and push; once the cleanup has removed any older
  engine still draining (it keeps the list it was deployed with, the old key alone), have Restate sign with the new
  key; later remove the old one and push again.
- **The workflows' secrets**: `gh secret set RESTATE_AUTH_TOKEN --repo <you>/<name>` with a new Admin-role key; the
  Cloudflare token rolled (`cf accounts tokens roll <ID> --force | jq -j . | gh secret set CLOUDFLARE_API_TOKEN
  --repo <you>/<name>`, which ends the old value at once, so between runs); `--env <name>` when they are in a GitHub
  environment.

## Logs

- **Deploys and cleanups**: your repository's **Actions**, one run each (`gh run list --repo <you>/<name>`, `gh
  run view <run-id> --log`), its summary page with the reports above. A failed run is GitHub's failure email.
- **The Workers**: each has its own logs in the dashboard (**Workers & Pages >** the Worker **> Logs**), or live
  with `npx -y wrangler tail <Worker>` from your repository: `rafi-app` (the app), `rafi-live` (the live channels) and each
  release's engine, `rafi-engine-v0-1-0` and so on. Only the application's own lines are kept, never the
  per-request records (they would hold client IPs and cookies).
- **Restate Cloud**'s console shows the invocations, including any stuck in retry.

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

## Troubleshooting

A failed run says why in its log, at the step that failed; most failures are a secret, a token's permission or a
configuration value, and a failed run is finished by running its failed jobs again (**Re-run failed jobs**), never
undone. The table of symptoms and fixes is in
[docs/operations/cloudflare.md](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/cloudflare.md#troubleshooting),
and what to do during an event (the engine down, a breach, a restore) in
[the runbook](https://github.com/sagikazarmark/rafi/blob/main/docs/operations/runbook.md).
