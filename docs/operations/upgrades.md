# Upgrades

## The deploy freeze

**Don't deploy while a draw is pending or a room is live**: from the moment registration opens at the venue until
the raffle finishes and the thank-you screen is up. Check the admin's raffle list before you deploy: nothing
should be Open, and nothing Drawing.

A raffle that is merely *frozen* and waiting (an event that didn't happen, an organizer who walked away) doesn't
stop a deploy: see [What a deploy does to a raffle in flight](#what-a-deploy-does-to-a-raffle-in-flight).

What a deploy during an event breaks:

- On Workers, a deploy restarts the Durable Objects, which drops every phone's live connection. Phones reconnect
  and re-fetch, but the moment is lost.
- Natively, restarting `rafi` interrupts requests, and a random `RAFI_ALTCHA_KEY` (if you didn't set one)
  invalidates the challenges phones are solving.
- Restarting the engine (`rafi-restate` natively; on Workers it's the same deploy) while a draw waits for its round
  makes Restate retry the draw. The committed round is journaled, so a retry **cannot** pick another round; but
  the retry runs on the new code (see below), and until it succeeds the draw shows as stalled.

## What a deploy does to a raffle in flight

Restate pins each invocation to the deployment it started on and routes new invocations to the latest one. A
draw that sleeps until its round is an invocation in flight. Natively, if the old `rafi-restate` keeps running
until it drains, the draw finishes on the old code. **On Workers the old code is gone the moment the new one is
live**, so the draw replays on the new code. That replay is safe only if the new code journals the same steps in
the same order as the old (`crates/engine-restate/src/service.rs`, the `draw` handler). Releases keep that
sequence unless they say otherwise; when one changes it, its notes say so, and the deploy freeze is the safeguard.

The raffle's state in Restate, like its event log in the database and `verify.json`, is the JSON the
[draw algorithm](../reference/draw-algorithm.md#versions-and-compatibility) specifies. It is held from the freeze
until the raffle finishes, which for an abandoned raffle is until housekeeping asks the engine to finish it at the
retention ceiling, months later. So on a hosted installation there is always old state meeting new code, and what
keeps that safe is the compatibility rule there (additive only, defaults for new fields, a new algorithm version for
anything else), not the freeze.

## Database migrations

The app applies migrations on start: `migrations/sqlite` for SQLite, `migrations/postgres` for Postgres. On D1,
apply them with wrangler before deploying the Worker, which is what `deploy/cloudflare`'s `deploy.sh` does (the
release's migrations come with its artifact, in `rafi/migrations/sqlite`). An upgrade there is the two lines at
the top of `deploy.sh`, `RAFI_VERSION` and `RAFI_SHA256` (the tarball's hash, from the release page), then:

```sh
./deploy.sh        # the new release into rafi/, checked against the pin; wrangler d1 migrations apply DB --remote; wrangler deploy
```

Natively, deploy the app before (or together with) the engine. Until the schema is current, the engine's writes
fail and Restate retries them.

**Never run a build older than the one that wrote the database.** Nothing in the code checks (the Worker's
`/healthz` is a static `ok`): a rolled-back build meets columns it doesn't know, or event types it can't read,
and fails on the first request that touches them. Roll forward instead.

## Registering the raffle engine

Restate keeps a record of each *deployment*: a URI, and the services and handlers it discovered there. Register
without `force`:

```sh
# self-hosted Restate: its admin API
curl -X POST http://localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://<engine host>:9080"}'
```

On Restate Cloud, register the URL as a deployment in the console or the CLI. On Workers that URL is the
Worker's, with `/restate`: `https://<the Worker's hostname>/restate`.

What Restate does with that request, and what it means for you:

- **A new URI** is discovered and becomes the latest deployment of its services. New invocations go there.
- **A URI that is already registered** answers `200`, and Restate **keeps the services and handlers it already
  has for it**, whether or not the code behind the URI changed. There is no refusal and no warning. So after a
  release that changes the `Raffle` object's or `EngineInfo`'s handlers, re-registering the same URI does nothing,
  and the first invocation of a changed handler fails (a `404` from the ingress for a new handler, `unknown
  handler` from the endpoint for a removed one).
- **`force: true`** overwrites the deployment behind the URI. Don't: it "can lead in-flight invocations to an
  unrecoverable error state" (Restate's words), and the state it leaves behind is the kind nobody can fix.

So the rule is: **a changed engine is registered at a new URI.** Natively that is a new host or port for the new
`rafi-restate`; on Workers it is a new path, which a release that changes the handlers ships as a change to
`engine::PREFIX` in `apps/worker/src/engine.rs` (the Worker keeps accepting everything under `/restate/`, so
invocations pinned to the old URI still arrive). The release notes say when a registration is needed; a release
that doesn't change the handlers needs none.

After registering the new URI, leave the old deployment registered until it has drained:

```sh
restate deployments list
restate deployment describe <old id> --extra      # invocations still pinned to it
restate deployments remove <old id>               # once there are none
```

Never remove a deployment with `--force` while a draw is in flight on it.

## Order of operations

1. Check that no raffle is Open or Drawing.
2. Apply migrations (D1), or let the new app apply them on start (native).
3. Deploy the app (on Workers, that's the engine too).
4. Natively, deploy the engine. If the release changed the engine's handlers, register its new URI with Restate
   and drain the old one.
5. Run a test-mode draw.

## Data

The anonymous raffle (manifest and event log) is append-only, and it must stay verifiable after an upgrade. Each
manifest names its algorithm (`rafi-draw-v1`); readers accept every algorithm they ever shipped, verifiers fail on
one they don't know, and changing how winners are picked means a new algorithm version, not an edit to this one.
The rules are in the [draw algorithm's](../reference/draw-algorithm.md#versions-and-compatibility) compatibility
section.
