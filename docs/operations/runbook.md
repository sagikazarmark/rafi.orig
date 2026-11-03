# Runbook

What an operator does when something happens: before a deploy, during an event, after a restore, after a breach.
The commands are for Cloudflare Workers (`wrangler` is `npx -y wrangler` from your installation's repository, your
copy of `deploy/cloudflare`, its `package.json`'s after `npm install`); natively the same SQL runs against your
SQLite file or Postgres database, and the secrets are the app's environment. They use the template's names: the
database `rafi` (its `database_name`) and the Workers `rafi-app`, `rafi-live` and `rafi-engine-v…`; an
installation with names of its own, or one of several in a repository as a Wrangler environment (`rafi-eu`,
`rafi-app-eu`; `CLOUDFLARE_ENV=eu` for the `-c wrangler.app.toml` commands), uses its own. On Cloudflare the
deploys and the Restate cleanups are your repository's GitHub Actions runs (**Actions > Deploy**, **Actions >
Restate cleanup**): each run's log is that run's, step by step, its summary page says what it did, and its
artifacts hold the JSON `deploy.sh` read and wrote (`cleanup.json` among the cleanup's).

## Before a deploy: is anything live?

The [deploy freeze](upgrades.md#the-deploy-freeze) is about every organizer's raffles, not only yours, and the
admin shows each organizer only their own. Nothing asks for you: on Cloudflare a push that changes the version file
(or a configuration, or `legal/`) deploys whatever is live. So before you push, or before you run the workflow by
hand, ask the database:

```sh
wrangler d1 execute rafi --remote --command "
  SELECT id, title, state, event_date, owner_id FROM raffles
  WHERE state IN ('open', 'closed', 'drawing')
     OR (state IN ('ready', 'frozen') AND event_date >= date('now', '-1 day'))
  ORDER BY event_date"
```

Open, closed (registration over, the draw about to start) and drawing raffles are live; published or frozen ones
dated today or yesterday may be about to be. An empty answer is a safe deploy. Otherwise wait, or ask the
organizer (`contact_email` in the same row).

## Rotating secrets

Rotate between events, never during one, and one secret at a time, checking `/healthz` after each.

| Secret | How |
|---|---|
| `RAFI_POW_KEY` | `wrangler secret put RAFI_POW_KEY -c wrangler.app.toml` with a new `openssl rand -hex 32`. Challenges phones are solving at that moment fail once; they fetch a new one. Never during an event. |
| `CLERK_SECRET_KEY` | In Clerk, create a new secret key; `wrangler secret put CLERK_SECRET_KEY -c wrangler.app.toml`; check an organizer can sign in; then revoke the old key in Clerk. |
| `CLERK_PUBLISHABLE_KEY` | Changes only with the Clerk instance (a new domain): set it with the secret key, in the same deploy. Organizers sign in again. |
| `RAFI_ENGINE_API_KEY` | In Restate Cloud, create a new API key with the app's role ([Cloudflare Workers](cloudflare.md#the-restate-keys)); `wrangler secret put RAFI_ENGINE_API_KEY -c wrangler.app.toml`; run a test-mode draw; delete the old key. |
| `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` (the engine's, in `wrangler.engine.toml`, or its `[env.<name>.vars]`) | It takes a comma-separated list, for the overlap. Add the new public key next to the old one (`"publickeyv1_old,publickeyv1_new"`) and push: the deploy redeploys the current release's engine Worker in place with both keys (the same code at the same URL, so Restate sees nothing change). An older release's engine Worker still draining keeps the list it was deployed with, the old key alone: let the cleanup remove it first. Then have Restate sign with the new key, run a test-mode draw, and later remove the old key from the list and push. |
| `RESTATE_AUTH_TOKEN` (the workflows', a GitHub secret) | In Restate Cloud, create a new API key with the Admin role; `gh secret set RESTATE_AUTH_TOKEN` (`--env <name>` when the secrets are in a GitHub environment); run the Restate cleanup by hand (`gh workflow run restate-cleanup.yaml`: it lists Restate's deployments and removes only drained engines); delete the old key. |
| `CLOUDFLARE_API_TOKEN` (the workflows', a GitHub secret) | Roll it, which invalidates the old value at once, so between runs: `cf accounts tokens list` for its ID, then `cf accounts tokens roll <ID> --force \| jq -j . \| gh secret set CLOUDFLARE_API_TOKEN` (or the dashboard's **Account API tokens**, the token's **Roll**, and paste it into `gh secret set CLOUDFLARE_API_TOKEN`). Or create a new one with the same permissions ([Cloudflare Workers](cloudflare.md#the-api-token)), replace the secret and delete the old one. Then run the Deploy workflow by hand: it redeploys the same release. |
| `RAFI_OPERATOR_*`, `RAFI_*_VERSION` | Not secrets to rotate: changed when the operator or the documents change ([the operator's records](overview.md#the-operators-records)). |

Natively, change the environment and restart `rafi` (and `rafi-restate` for the engine's settings); the identity
keys belong to a self-hosted Restate only if you configured request signing.

## Rolling back a deploy

**Don't.** Never run a build older than the one that wrote the database
([Upgrades](upgrades.md#database-migrations)): the migrations only go forward, and a release's raffles may have
written event types an older build can't read. A bad release is fixed by the next one: report it, and deploy the
release that fixes it when it is out.

On Cloudflare, don't set the version file back: the deploy refuses a release older than the one Restate runs (its
downgrade check) in its first job, before touching anything, so such a run fails at **Prepare the release** with
nothing deployed. Don't roll a Worker back by hand either (`wrangler rollback`, the dashboard's **Deployments**):
the next deploy wouldn't know, the engine Restate runs would stay the newer release's, and an older live Worker
could refuse the newer Workers' requests. A failed deploy is finished by running its failed jobs again, not undone.
`/healthz` answers the version running. Natively the same holds: keep the newer binaries, and upgrade to the fix.

## When the raffle engine is down during an event

Restate Cloud (or your Restate) unreachable shows in the admin as an error on Start the draw or Draw ("The raffle
engine can't be reached"), or a draw that stays on its countdown. What the organizer can do, in order:

1. **Wait and retry.** Starting the draw is safe to retry: it picks up where it stopped. A draw in flight is
   journaled with its round and resumes when the engine is back, on the same round, so it can't pick anyone
   else.
2. **Draw the rest later.** Finish the evening without the remaining draws, tell the room when and where they will
   be drawn (the projector and the verification page show them when they happen), and draw them once the engine is
   back. Nothing expires before the raffle's erasure date.
3. **Draw by hand, and say so.** If the prizes must go tonight, draw them some other way in front of the room, say
   it isn't Rafi's verifiable draw, and withdraw those prizes in the console afterwards so the record doesn't
   claim them.

The operator checks the [Restate Cloud status](https://status.restate.dev), the engine Worker's log (`npx -y wrangler
tail rafi-engine-v<version, dashed>`: each release's engine is its own Worker), the app Worker's (`npx -y wrangler tail
rafi-app`) and, for a self-hosted Restate, `restate-server`'s.

## Backups and restore

**On Workers, D1 Time Travel is the backup**: it restores the database to any minute of the last 30 days on the paid
plan (7 on the free one). There is nothing to schedule, and nothing else to back up: Restate's state matters only
while a raffle is being drawn, and the database is the record.

```sh
wrangler d1 time-travel info rafi                                # the bookmark for now
wrangler d1 time-travel restore rafi --timestamp=<unix seconds>  # or --bookmark=<bookmark>
```

A restore brings back what was true then, including personal data erased since, people removed since (at their
request or not, with their whole entry), raffles deleted since (by their organizer, or as the organizer left, with
everyone in them), organizers' defaults since removed, names hidden since, people who took themselves off since
(their objection), and removals at a person's request recorded since. In this order:

1. **Before restoring, save what the restore would undo**, from the live database, as the statements that put it
   back: the erasure log, the raffles purged or whose organizer left, the raffles, entrants and organizers'
   defaults deleted, the removals at a person's request, and the hidden names and objections. Run it right before
   the restore (what changes in between is lost with the restore anyway):

   ```sh
   wrangler d1 execute rafi --remote --json --command "
     -- The erasure log.
     SELECT 'INSERT INTO erasures (raffle_id, entrant_id, at) SELECT ' || quote(raffle_id) || ', '
         || quote(entrant_id) || ', ' || at || ' WHERE NOT EXISTS (SELECT 1 FROM erasures WHERE raffle_id = '
         || quote(raffle_id) || ' AND entrant_id = ' || quote(entrant_id) || ');' AS sql
       FROM erasures;
     -- Raffles purged, or whose organizer left.
     SELECT 'UPDATE raffles SET purged_at = ' || quote(purged_at) || ', organizer_left_at = '
         || quote(organizer_left_at) || ' WHERE id = ' || quote(id) || ';' AS sql
       FROM raffles WHERE purged_at IS NOT NULL OR organizer_left_at IS NOT NULL;
     -- Raffles deleted, by their organizer or as they left, with everything in them.
     SELECT 'DELETE FROM raffles WHERE id = ' || quote(raffle_id) || ';' AS sql FROM audit WHERE action = 'delete';
     -- Entrants deleted: removed, an add undone, a raffle expired.
     SELECT 'DELETE FROM entrants WHERE raffle_id = ' || quote(r.id) || ' AND id NOT IN (' || quote('')
         || COALESCE((SELECT group_concat(', ' || quote(e.id), '') FROM entrants e WHERE e.raffle_id = r.id), '')
         || ');' AS sql
       FROM raffles r;
     -- The defaults of organizers who left.
     SELECT 'DELETE FROM owner_defaults WHERE owner_id NOT IN (' || quote('')
         || COALESCE((SELECT group_concat(', ' || quote(owner_id), '') FROM owner_defaults), '') || ');' AS sql;
     -- Removals at a person's request.
     SELECT 'INSERT INTO add_removals (raffle_id, email_norm, list_ref, removed_at) SELECT ' || quote(raffle_id)
         || ', ' || quote(email_norm) || ', ' || quote(list_ref) || ', ' || removed_at
         || ' WHERE EXISTS (SELECT 1 FROM raffles WHERE id = ' || quote(raffle_id) || ') ON CONFLICT DO NOTHING;' AS sql
       FROM add_removals;
     -- Hidden names, and who took themselves off.
     SELECT 'UPDATE entrants SET hidden = 1, objected_at = ' || quote(objected_at) || ' WHERE id = ' || quote(id)
         || ';' AS sql
       FROM entrants WHERE hidden = 1" | jq -r '.[].results[].sql' >put-back.sql
   ```

2. **Restore** (above).
3. **Put back what step 1 saved**: `wrangler d1 execute rafi --remote --file=put-back.sql`. Every statement can run
   twice, so run the file again if it stops halfway. Undone since, and back with the restore: a removal at a
   person's request the organizer lifted, and a name the organizer showed again; the organizer does it again. A
   raffle's `DELETE FROM entrants` lists everyone in it: with `RAFI_MAX_ENTRANTS` above about 2,700 it can pass
   D1's 100 KB per statement.
4. **Re-apply the erasures**: the SQL in [Cloudflare Workers](cloudflare.md#data-residency) (natively,
   `rafi reapply-erasures`). It erases everyone the log names and everyone in a purged raffle or one whose
   organizer left; the next hourly housekeeping erases whatever came due meanwhile, and whoever took themselves
   off a raffle that is over.
5. **Check the engine against the database** for raffles that were drawing: a raffle the restore took back to
   before a draw the engine has already made disagrees with it. The engine is the authority for the draw. Its next
   write for the raffle (the organizer's next decision, or Finish) brings the database's log up to its state, as
   long as the restored raffle is past its freeze (the write is for the entrant list it was frozen with); one taken
   back to before its freeze can't be caught up, so restore to a time after the freeze of every raffle being drawn,
   or don't restore during an event. List them, and ask their organizers to make their next decision:

   ```sh
   wrangler d1 execute rafi --remote --command "SELECT id, title, owner_id FROM raffles WHERE state IN ('frozen', 'drawing')"
   ```

6. **The live channels** catch up by themselves: the next change to a raffle publishes its view again, and phones
   and projectors poll. Nothing to do.

Self-hosting has the same steps with its own tools ([Self-hosting](self-hosting.md#backups)).

Rehearsed on 2026-11-02, against copies: `a_restore_puts_nobody_back` ([`crates/app/tests/restore.rs`](../../crates/app/tests/restore.rs))
runs these steps on SQLite files, with this page's queries and the Cloudflare page's SQL read from the pages, so CI
keeps them working; and once by hand, with `sqlite3`, `rafi reapply-erasures`, a local D1
(`wrangler d1 execute --local`), and on Postgres for step 1's query and its statements (a few rows). D1 caps the
terms of a compound `SELECT`: step 1's queries stay separate statements.

## A person asks for their data

Organizers are the controllers: an entrant asks the organizer, who answers from the admin (People tab, the CSV
export). When someone asks you, the operator, across raffles, find every raffle their email is in. Emails are
matched by a key: lowercased, without a `+tag`, and for Gmail without dots and with `googlemail.com` as
`gmail.com` (`Anna.K+meetup@GoogleMail.com` is `annak@gmail.com`). Compute the key by hand, then:

```sh
wrangler d1 execute rafi --remote --command "
  SELECT r.id AS raffle, r.title, r.organizer_name, r.contact_email, e.id AS entrant, e.display_name,
         e.email_raw, e.created_at, e.delivered_at, e.objected_at,
         (SELECT outcome FROM draws d WHERE d.raffle_id = e.raffle_id AND d.entrant_id = e.id
          ORDER BY k DESC LIMIT 1) AS drawn
  FROM entrants e JOIN raffles r ON r.id = e.raffle_id
  WHERE e.email_norm = 'annak@gmail.com'"
```

Removals at a person's request are kept by the same key, with no other data:
`SELECT raffle_id, removed_at FROM add_removals WHERE email_norm = 'annak@gmail.com'`. Pass the request to each
raffle's organizer (`contact_email`), who decides and acts; erasing on their behalf is the organizer's instruction
to give. Answer within the month the GDPR allows, and note what you did.

## Organizers who left: their account IDs

"Remove my data" keeps the account's identifiers (Clerk IDs: the owner, and the member who acted) on the
organizer's raffles and in the audit log, as your record of what was done. Your privacy notice names how long
(`legal/privacy.md`, Retention). Rafi doesn't delete them when that period ends
([#22](https://github.com/sagikazarmark/rafi.orig/issues/22)): you do, by running this on a schedule you can keep,
say monthly, with your period in days for `365`. It blanks the owner on every raffle whose organizer left more
than the period ago, and the owner and member on every audit row of that owner up to their leaving (an
organization that removed its data and carried on keeps its later rows). Running it again changes nothing; run it
again after a restore.

```sh
wrangler d1 execute rafi --remote --command "
  UPDATE raffles SET owner_id = ''
  WHERE owner_id <> '' AND organizer_left_at <= CAST(strftime('%s', 'now') AS INTEGER) - 365 * 86400;
  UPDATE audit SET actor = '', user_id = ''
  WHERE actor <> '' AND EXISTS (
    SELECT 1 FROM audit l
    WHERE l.detail = 'owner left' AND l.actor = audit.actor AND audit.at <= l.at
      AND l.at <= CAST(strftime('%s', 'now') AS INTEGER) - 365 * 86400)"
```

Natively, the same statements through `sqlite3 rafi.db`; on Postgres through `psql`, with
`EXTRACT(EPOCH FROM now())::bigint` for `CAST(strftime('%s', 'now') AS INTEGER)`. An organizer who deleted their
account without "Remove my data" first isn't covered: Rafi doesn't learn of it
([#31](https://github.com/sagikazarmark/rafi.orig/issues/31)), so they're among the requests you handle by hand.

## A personal-data breach

The terms promise organizers to be told without undue delay; they, as controllers, have 72 hours from becoming
aware of it to notify their supervisory authority (GDPR Art. 33), so your notice is what starts their clock.

1. **Contain**: rotate what leaked ([above](#rotating-secrets)); if an organizer account was taken over, have Clerk
   end its sessions.
2. **Find what was exposed and whose.** What exists: the database (entrants' names, emails, registration IDs, claim
   codes until erased), the Worker's log (application lines only, no personal data; `invocation_logs` are off),
   Clerk's and Cloudflare's own audit logs. The audit table records each organizer action with the raffle and the
   user: `SELECT * FROM audit WHERE at >= <unix seconds> ORDER BY at`.
3. **Find the controllers**: the organizers of the raffles affected.

   ```sh
   wrangler d1 execute rafi --remote --command "
     SELECT r.owner_id, r.organizer_name, r.contact_email, COUNT(DISTINCT r.id) AS raffles, COUNT(e.id) AS people
     FROM raffles r LEFT JOIN entrants e ON e.raffle_id = r.id AND e.erased = 0
     GROUP BY r.owner_id, r.organizer_name, r.contact_email"
   ```

4. **Tell each of them**, at their `contact_email`: what happened, when, which of their raffles and how many people,
   what is likely to follow, and what you did. Keep a record of the breach and of each notice, whatever its size.

## Where the logs are

- **Deploys and cleanups** (Cloudflare): your repository's **Actions**, one run each, every step printed (`gh run
  list`, `gh run view <run-id> --log`), and on each run's summary page what it did.
- **The Workers**: Workers Logs, one per Worker (the dashboard's **Workers & Pages >** the Worker **> Logs**, or live
  with `npx -y wrangler tail <Worker>`): `rafi-app` (the app and the hourly housekeeping), `rafi-live` (the live
  channels) and each release's engine, `rafi-engine-v0-1-0` and so on, whose log stays until the cleanup deletes
  the Worker. Only the application's own lines are kept: warnings and errors with raffle IDs and error text,
  never personal data, and no per-request records (`invocation_logs = false`).
- **Restate Cloud**: its console's invocations, and `restate invocations describe <id>`.
- **Natively**: `rafi`'s and `rafi-restate`'s standard output, and `restate-server`'s.

## Monitoring

Nothing in Rafi phones home, so watching it is yours:

- **An external check on `/healthz`** every few minutes, from any uptime service: anything but `ok …` is down or
  misconfigured.
- **Housekeeping**: the hourly Cron Trigger's runs (the Worker's **Settings > Triggers**, past events) fail visibly
  when a run fails, and the admin warns every organizer when no run has succeeded for two hours. Erasure dates are a
  promise to entrants, so look at a failed run the same day.
- **Errors**: Workers Logs keeps the application's warnings and errors, the app Worker's under `rafi-app`, the live
  channels' under `rafi-live` and the engine's under its `rafi-engine-v<version>` Worker. Watch for `request
  failed status=5…` lines and for `housekeeping` errors; a Workers Observability alert on errors, where your plan offers one, does it for
  you.
- **Restate Cloud**: its console shows invocations stuck in retry, which is a draw waiting for its round or a write
  that keeps failing.
- **Deploys and cleanups** (Cloudflare): each is a GitHub Actions run of your repository, and a failed one (a
  refusal, a failed registration, a Worker the cleanup couldn't delete) is GitHub's failure notice. The daily
  cleanup's summary lists the older engine deployments still draining and any paused invocation pinned to one,
  which waits for you ([Upgrades](upgrades.md#on-workers-automatic)), and any engine Worker left behind, for you to
  delete. In a public repository GitHub disables the scheduled cleanup after 60 days without activity: look at its
  runs now and then.

## What it costs

On Cloudflare, the Workers Paid plan (USD 5 a month at the time of writing) covers a 500-person evening with room to
spare: about 20,000 Worker requests (projector and admin polling for three hours, phones joining and connecting; the
static files are free), about 5 million D1 rows read (each poll of a 500-person raffle reads its entrants once), a
few thousand rows written, and one Durable Object holding the room's sockets for the evening. The plan includes
10 million requests, 25 billion rows read and 50 million rows written a month. Restate Cloud and Clerk have free
tiers that cover a few evenings a month. Prices and allowances change: check each provider's pricing page.
