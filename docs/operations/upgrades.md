# Upgrades

## The deploy freeze

**Don't deploy while a raffle is live**, meaning registration is open, or the list is frozen and the draw hasn't
finished. Check the admin's raffle list before you deploy.

A deploy during a draw is safe for the result: Restate replays a draw in flight on the same round. It is still bad
for the room:

- On Workers, a deploy restarts the Durable Objects, which drops every phone's live connection. Phones reconnect and
  re-fetch, but the moment is lost.
- Natively, restarting `rafi` interrupts requests, and a random `RAFI_ALTCHA_KEY` (if you didn't set one)
  invalidates the challenges phones are solving.
- Restarting `rafi-restate` mid-draw makes Restate retry. The draw continues on its round, but may stall.

## Database migrations

The app applies migrations on start: `migrations/sqlite` for SQLite, `migrations/postgres` for Postgres. On D1,
apply them with wrangler before deploying the Workers:

```sh
(cd apps/worker && wrangler d1 migrations apply rafi --remote)
```

Deploy the app before (or together with) the engine. Until the schema is current, the engine's writes fail and
Restate retries them.

## Re-registering the raffle engine

Restate keeps a record of each deployment's services and handlers. **Register the engine again whenever its
handlers change** (the `Raffle` object or `EngineInfo` service). On Workers, register it for every release.

```sh
# self-hosted Restate
curl -X POST http://localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://<engine host>:9080", "force": true}'
```

On Restate Cloud, register the engine's URL as a new deployment.

## Order of operations

1. Check that no raffle is live.
2. Apply migrations (D1), or let the new app apply them on start (native).
3. Deploy the app.
4. Deploy the engine and re-register it with Restate.
5. Run a test-mode draw.

## Data

The anonymous raffle (manifest and event log) is append-only, and it must stay verifiable after an upgrade. Each
manifest names its algorithm (`rafi-draw-v1`), and verifiers reject any other. So changing how winners are picked
means a new algorithm version, not an edit to this one.
