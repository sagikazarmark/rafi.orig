# Upgrades

## The deploy freeze

**Don't deploy while a draw is pending or a room is live**: from the moment registration opens at the venue until
the raffle finishes and the thank-you screen is up. The admin shows each organizer only their raffles; before you
deploy, ask the database about everyone's ([the preflight](runbook.md#before-a-deploy-is-anything-live)): nothing
should be Open, Closed or Drawing. Nothing checks this for you: on Cloudflare a push that changes `VERSION`
deploys whenever it lands, so push between events.

A raffle that is merely *frozen* and waiting (an event that didn't happen, an organizer who walked away) doesn't
stop a deploy: see [What a deploy does to a raffle in flight](#what-a-deploy-does-to-a-raffle-in-flight).

What a deploy during an event breaks:

- On Workers, a release that changes the rooms Worker (its notes say so) redeploys it, which restarts the live
  channels' Durable Objects and drops every phone's connection. Phones reconnect and re-fetch, but the moment is
  lost. Other releases leave the rooms Worker alone and switch the app Worker to its new version.
- Natively, restarting `rafi` interrupts requests, and a random `RAFI_POW_KEY` (if you didn't set one)
  invalidates the challenges phones are solving.
- Natively, restarting the engine (`rafi-restate`) while a draw waits for its round makes Restate retry the draw.
  The committed round is journaled, so a retry **cannot** pick another round; but the retry runs on the new code
  (see below), and until it succeeds the draw shows as stalled. On Workers each release's engine is a Worker of
  its own, and the previous one keeps running what started on it.

## What a deploy does to a raffle in flight

Restate pins each invocation to the deployment it started on and routes new invocations to the latest one. A
draw that sleeps until its round is an invocation in flight.

- **On Workers** each release deploys its engine as a new Worker, registered as a new deployment, and leaves the
  previous release's engine Worker as it was: a draw in flight finishes on the code it started on, and the next
  call goes to the new engine. Once Restate reports the old deployment drained, the Restate cleanup workflow
  deletes its Worker and removes it from Restate ([below](#registering-the-raffle-engine)). While it drains, that
  old engine writes to the database the new release migrated and nudges the rooms Worker's live channels, so a
  release's migrations and its live channel stay compatible with the previous release's engine, and its notes say
  so when they can't.
- **Natively**, if the old `rafi-restate` keeps running until it drains, the draw finishes on the old code;
  stopped first, the draw replays on the new code. That replay is safe only if the new code journals the same steps
  in the same order as the old (`crates/engine-restate/src/service.rs`, the `draw` handler). Releases keep that
  sequence unless they say otherwise; when one changes it, its notes say so, and the deploy freeze is the
  safeguard.

The raffle's state in Restate, like its event log in the database and `verify.json`, is the JSON the
[draw algorithm](../reference/draw-algorithm.md#versions-and-compatibility) specifies. It is held from the freeze
until the raffle finishes, which for an abandoned raffle is until housekeeping asks the engine to finish it at the
retention ceiling, months later. So on a hosted installation there is always old state meeting new code, and what
keeps that safe is the compatibility rule there (additive only, defaults for new fields, a new algorithm version for
anything else), not the freeze.

## Database migrations

The app applies migrations on start: `migrations/sqlite` for SQLite, `migrations/postgres` for Postgres. On D1,
the deploy applies them (`wrangler d1 migrations apply DB --remote`, from the release's `migrations/`) before it
deploys any Worker. An upgrade there is the release named in the installation's `VERSION`, pushed: the Deploy
workflow deploys it ([Cloudflare Workers](cloudflare.md#upgrading)).

Natively, deploy the app before (or together with) the engine. Until the schema is current, the engine's writes
fail and Restate retries them.

**Never run a build older than the one that wrote the database.** Nothing in the code checks (`/healthz` says
which version runs, not which wrote the database): a rolled-back build meets columns it doesn't know, or event
types it can't read, and fails on the first request that touches them. On Cloudflare the deploy refuses a release
older than the one Restate runs in its first job, before touching anything, which catches a `VERSION` set back.
Roll forward instead ([Rolling back a deploy](runbook.md#rolling-back-a-deploy)).

## Registering the raffle engine

Restate keeps a record of each *deployment*: a URI, and the services and handlers it discovered there.

- **A new URI** is discovered and becomes the latest deployment of its services. New invocations go there.
- **A URI that is already registered** answers `200`, and Restate **keeps the services and handlers it already
  has for it**, whether or not the code behind the URI changed. There is no refusal and no warning. So after a
  release that changes the `Raffle` object's or `EngineInfo`'s handlers, re-registering the same URI does nothing,
  and the first invocation of a changed handler fails (a `404` from the ingress for a new handler, `unknown
  handler` from the endpoint for a removed one).
- **`force: true`** overwrites the deployment behind the URI. Don't: it "can lead in-flight invocations to an
  unrecoverable error state" (Restate's words), and the state it leaves behind is the kind nobody can fix.

So the rule is: **a changed engine is registered at a new URI, without `force`.**

### On Workers: automatic

Each release's engine is its own Worker, `rafi-engine-v<version, dashed>` (`rafi-engine-v0-1-0`), at its own
URI, `https://rafi-engine-v0-1-0.<subdomain>.workers.dev/restate`, which the deploy registers with Restate Cloud
(without `force`, and never over a newer release's engine) before it switches the app Worker to the new version.
Nothing is registered by hand, and every release is a new URI, so a release that changes the handlers needs nothing
of yours. A later deploy of the same release redeploys its engine Worker with the same code; no other deploy
touches it.

The Restate cleanup workflow (`restate-cleanup.yaml`, daily and by hand) looks at this installation's older engine
deployments:

- **drained** (no invocation pinned to it any more): its Worker deleted, then the deployment removed from Restate
  (`restate deployments remove`, never forced);
- **draining**: listed in the cleanup's log with the invocations still pinned to it, and deleted by a later cleanup
  once drained. A draw in flight usually drains it within minutes;
- an invocation that is **paused** (Restate stopped retrying it, as its retry policy says, and waits for a human)
  keeps its deployment draining. Find out why it failed (the engine Worker's log, `wrangler tail
  rafi-engine-<version>`, and `restate invocations describe <id>`), then either resume it on the latest
  deployment, if the new engine can carry it on:

  ```sh
  restate invocations resume <id> --deployment latest
  ```

  or kill it, which ends it where it stopped, without running the rest:

  ```sh
  restate invocations kill <id>
  ```

  The Restate CLI runs against your environment with `RESTATE_ADMIN_URL` and `RESTATE_AUTH_TOKEN` (an Admin-role
  key) set as in the workflows, or after `restate cloud login` and `restate cloud env configure`.

### Natively: by hand

Register the engine without `force`:

```sh
# self-hosted Restate: its admin API
curl -X POST http://localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://<engine host>:9080"}'
```

On Restate Cloud, register the engine's public URL as a deployment in the console or the CLI. A new
`rafi-restate` whose handlers changed runs at a new URI (a new host or port), registered beside the old one; the
release notes say when a registration is needed, and a release that doesn't change the handlers needs none.

After registering the new URI, leave the old deployment registered, and the old `rafi-restate` running, until it
has drained:

```sh
restate deployments list
restate deployment describe <old id> --extra      # invocations still pinned to it
restate deployments remove <old id>               # once there are none
```

Never remove a deployment with `--force` while a draw is in flight on it.

## Order of operations

On Cloudflare the Deploy workflow does it all, in this order: the D1 migrations; the rooms Worker, when it changed;
the app Worker's new version uploaded and the release's engine Worker deployed; the engine registered; the app's new
version deployed. The Restate cleanup deletes the drained engines later. Check that nothing is live before you push
([the preflight](runbook.md#before-a-deploy-is-anything-live)), and run a test-mode draw afterwards.

Natively:

1. Check that no raffle is live ([the preflight](runbook.md#before-a-deploy-is-anything-live)).
2. Deploy the app; it applies the migrations on start.
3. Deploy the engine. If the release changed the engine's handlers, start it at a new URI, register that with
   Restate and drain the old one.
4. Run a test-mode draw.

## Data

The anonymous raffle (manifest and event log) is append-only, and it must stay verifiable after an upgrade. Each
manifest names its algorithm (`rafi-draw-v1`); readers accept every algorithm they ever shipped, verifiers fail on
one they don't know, and changing how winners are picked means a new algorithm version, not an edit to this one.
The rules are in the [draw algorithm's](../reference/draw-algorithm.md#versions-and-compatibility) compatibility
section.
