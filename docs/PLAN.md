# Rafi: meetup raffle plan

Status: the current state, 2026-10-14 (revision 11). How it got here is in the
[revision history](#revision-history) and in the detailed plans, which are records of their time:
[engine-plan.md](engine-plan.md), [drawing-plan.md](drawing-plan.md), [drawing-review.md](drawing-review.md),
[raffle-plan.md](raffle-plan.md), [drand-crate.md](drand-crate.md), [workers-plan.md](workers-plan.md).

## 1. Summary

A live raffle for meetups.

- **Before the event:** the organizer sets up a raffle and its prizes in the admin (Clerk login).
- **During the meetup:** a **projector screen** shows the intro, a QR code and a live count. Attendees join on
  their phones with a **display name, email and consent to the rules**. There is no account. The organizer can
  **add people who have no phone**.
- **Starting the draw:** registration closes and the entrant list is frozen. The raffle is opened from a published
  **manifest**: the entrants (random IDs only), one item per prize copy, the algorithm and the beacon source.
- **Drawing each item:** the organizer picks an item and presses **Draw**. The raffle engine commits to a **drand
  round about 6 s in the future** and announces it (the countdown), waits durably for it, verifies the beacon's BLS
  signature and picks a winner. The projector and the winner's phone show it; everyone else sees "not this time".
- **No-show:** "Not here" (or "Declined") puts that person out, and the item is drawn again with a new round.
- **The end:** everyone sees "thanks for playing", and a public page lets anyone re-run every draw.

**Stack:** one app crate, `rafi`: the admin, projector and verification page are a Dioxus 0.7.10 single-page app;
the server is axum (natively, or on Cloudflare Workers) with participant pages rendered by `dioxus::ssr`. Phones
get a small WASM with a JavaScript fallback. The database is SQLite, PostgreSQL (self-hosting) or D1 (Workers).
Draws run in a Restate virtual object, `Raffle`, on Restate Cloud (EU) or a self-hosted Restate. Auth is Clerk
via `dioxus-clerk`. Production is planned on Cloudflare Workers; the native server stays for self-hosting (§11).

## 2. What a meetup raffle needs

What the first review found missing from the original idea: coming back without an account (§7); registration
open from the start, not only at the end (§3); no-shows, the most common live failure (§6); one win per person;
proving the winner at the stage without showing it on the projector (a private claim code, §7); perceived rigging
(a verifiable draw per item, §6); prizes with several copies, drawn in an order picked live; remote entries, spam
and bots (§9); GDPR and Hungarian gambling and tax rules (§10); sleeping phones (§8.6); a projector that never shows
emails or admin buttons; undo and live changes (void, decline); a rehearsal mode (test mode, fake entrants); the
after-event work (export, delivery, erasure); Hungarian name order, hence a **display name**; Hungarian and English.

## 3. Decisions

| Topic | Decision |
|---|---|
| Multiple wins | **One win per person.** Once drawn, an entrant leaves the pool whatever the outcome. If it runs dry, pass 2 draws from earlier winners who didn't forfeit (§6.2). |
| No-show | **Draw again; the no-show is out.** "Not here" and "Declined" forfeit the item, which is drawn again with a new round. Both are in the public log. |
| Fairness | **Verifiable, one draw per item** (§6). |
| Raffle engine | **Restate**, behind the `Engine` contract (§8.5). It sees anonymous IDs only. No fallback engine: `LocalEngine` is for tests and development. |
| drand chain | **The app decides** (`RAFI_DRAND_CHAIN`, quicknet); the engine refuses a mismatch. Release builds accept only public chains. |
| Live updates | On Workers a Durable Object per raffle pushes the participant view to phones; natively phones poll. The projector and admin poll the full view every 2.5 s (§8.6). |
| Auth | Clerk. A raffle belongs to the active organization, or else the user; every admin call checks it. |
| Registration | Opens at the start of the meetup; closes at "Start the draw", which freezes the list. |
| On the projector | The display name as typed (≤ 24 characters), after 5 s; one tap hides or renames it. |
| Item order | Picked live: nobody can know an outcome in advance. |
| Eligibility, age | In the rules. Minimum age 16 by default. |
| Retention | Non-winners' personal data is erased 30 days after the event; winners' after 90, or 30 once their prize is delivered. The anonymous raffle (manifest and log) is kept (§10). |
| Bot protection | Rafi's own proof-of-work (ALTCHA-compatible, self-hosted), one entry per email, rate limits per raffle and per email (never per IP), an entrant cap. No Turnstile. |
| Winner emails | Not in v1. |

## 4. The event, end to end

| When | Organizer (`/admin`) | Projector (`/s/{slug}`) | Attendee phone (`/j/{CODE}`) |
|---|---|---|---|
| Before | Create the raffle and its prizes (quantity, sponsor); title, rules, language. Optionally rehearse in **test mode** with fake entrants. | | |
| Meetup starts | Publish, then **Open registration**. | The prizes, the rules in short, "no phone? come to me", a QR code, the short URL and code, a live count and recent names. | Join form: display name, email, ☐ rules & privacy (required), ☐ emails about future meetups (optional). Then **"You're in!"** with a private claim code. |
| During | **Add entrants without a phone.** Hide or rename trolls; remove duplicates. | The count grows. | The page can be closed; reopening it restores the state (§7). |
| Start | **Start the draw:** closes registration, freezes the list, writes the manifest, opens the raffle. Pressing it again picks up where an attempt stopped. | "N entrants, M prizes". | "Draw starting!" |
| Each item | Pick the item, **Draw**. The admin shows the winner's **claim code**. Then **Handed over**, or **Not here** / **Declined** (drawn again). While a draw waits for its round only **Handed over** goes through. | **"Locking in drand round R…"**, a countdown, the **winner's name**. | Winner: **YOU WON** with the claim code, a chime, vibration on Android. Everyone else: "Not this time", with the prizes left. |
| End | **Finish**; **Export** the winners CSV; **End live updates.** | **Thanks for playing!**, winners, a link to the verification page. | Thanks screen; winners still see their prize. |
| After | Mark prizes delivered. Erasure runs on schedule, or by hand. | | `/v/{slug}` verifies every draw. |

## 5. Lifecycle

### Lifecycle (`rafi-lifecycle`)

```
Draft ──publish──▶ Ready ──open──▶ Open ◀──open── Closed
                     ▲               │ close ───────▶ │
                     └──unpublish────┘                │
        Open | Closed ──freeze──▶ Frozen   (the manifest is written; the engine opens the raffle)
        Frozen ──first draw──▶ Drawing ──the raffle finishes──▶ Finished ──purge / retention──▶ Purged
```

- Organizer commands go through `RaffleState::apply`; the app and the store keep no rules of their own. From the
  freeze until the raffle finishes, the lifecycle follows the raffle (`raffle::lifecycle`).
- **Starting the draw** is separate writes, each guarded by the state it expects and safe to repeat (D1 has no
  interactive transactions): freeze, read the fixed list, write the manifest (or unfreeze if nobody is left), open
  the raffle. `start::next` names the next step (`Freeze`, `WriteManifest`, `Resend`, `Started`). The app checks
  the engine's beacon source before anything is frozen.
- Joins, entrant and prize changes are guarded in SQL on the raffle's state: joins only while Open, the rest until
  the freeze. After it, items can only be **voided**. Only a winner who claimed can be marked delivered.

### Raffle (`rafi-raffle`, owned by the engine)

```
open(manifest)                                   (the same manifest again: no-op; another: refused)
 item: Available ──draw──▶ announced on round R ──beacon──▶ Drawn(entrant)
                                 │ no beacon
                                 └──▶ stalled, still pending on R: Draw again retries R; Abandon (logged) → Available
       Drawn ──claimed──▶ Claimed
       Drawn ──not here | declined──▶ Available (a forfeit: the entrant is out)
       Available ──void──▶ Voided
 finish ──▶ Finished (the engine forgets the raffle)
```

A raffle is stored as its manifest and its event log (`Announced`, `Drawn`, `Stalled`, `Abandoned`, `Resolved`,
`Voided`, `Finished`); everything else is rebuilt from them. Whether a command would go through now is the
raffle's own rule, `Raffle::admit`, which the app checks against the projection.

A phone's screen is a pure function (`rafi_live_client::screen`) of the participant view and its own entrant ID.

## 6. Verifiable per-item draw (`rafi-draw-v1`)

### 6.1 Why this works

- **The list is frozen and published before any randomness exists.** Nobody can add or remove people to steer an
  outcome.
- **Each draw commits to a drand round R that hasn't happened yet**, announced before its beacon exists. Nobody,
  the organizer and the server included, can know the result.
- **Restate makes the commitment final.** R is journaled before the durable sleep; a crash or redeploy replays to
  the same R and winner. A beacon that can't be had stalls the draw on R: a retry fetches R again, never a new round.
- **Every beacon is BLS-verified** against the committed chain, by the engine and by the browser verifier.
- **While a draw is pending, nothing may change its pool.**
- **The remaining levers are public:** "Not here", "Declined", "Void" and "Abandon" are in the log, and the
  affected person is in the room. For an abandoned draw the verifier shows whom its round would have picked.

### 6.2 Algorithm

```
open:   manifest      = canonical JSON {algo:"rafi-draw-v1", raffle, beacon:{drand:<chain hash>}, entrants, items:[{id,name}]}
                        entrants sorted, deduplicated, lowercase hex (random, 128-bit)
        manifest_hash = SHA256(manifest)                       — published at /v/{slug} before the first draw

draw k: R             = the first round at least 6 s (lead) after now — quicknet, journaled
        randomness    = SHA256(signature), the beacon BLS-verified
        pool_k        = pass 1: sorted(entrants − everyone drawn in pass 1, whatever the outcome)
                        pass 2, once pass 1 is exhausted: pass-1 winners who didn't forfeit, not yet drawn in pass 2
        seed_k        = SHA256("rafi-draw-v1" ‖ randomness ‖ manifest_hash ‖ BE_u64(k))
        rand_u64(c)   = BE_u64(SHA256(seed_k ‖ BE_u64(c))[0..8])
        uniform(n)    : limit = 2^64 − (2^64 mod n); c = 0; loop { x = rand_u64(c++); if x < limit return x mod n }
        winner_k      = pool_k[uniform(|pool_k|)]
```

- `k` counts every draw, no-show redraws included; retrying a stalled draw keeps its `k`.
- Only SHA-256: no `ctx.rand()` (outsiders can't verify it) and no `rand` crate (not stable across versions).
- **Chain:** quicknet `52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971`, genesis 1692803367,
  period 3 s; relays `api.drand.sh`, `api2`/`api3.drand.sh`, `drand.cloudflare.com`. The mock chain (`mock-drand`,
  development only) signs with a public key, and the views mark its raffles as not publicly verifiable.

### 6.3 Verification

`/v/{slug}` (data: `/api/r/{slug}/verify.json`) publishes the manifest (with display names until erasure, so people
can check they're in), every draw (`k`, item, R, signature, randomness, pass, pool size, index, winner, outcome) and
abandoned draws, and re-runs them in the browser (`rafi_raffle::verify_public`, WASM): the log is replayed through
the raffle's own transitions and every BLS signature is checked.

[`tools/verify.py`](../tools/verify.py) (Python standard library only) re-runs the algorithm independently; with
`--fetch` it compares each signature with the public relays' (it doesn't check BLS). CI checks the Rust code and
`verify.py` against the same vectors (`testdata/draw-vectors.json`, from real quicknet beacons).

### 6.4 Edge cases

- **More items than eligible entrants:** pass 2. **Nobody left:** the draw is refused; the item stays available.
- **drand outage:** the engine waits up to 60 s past the round. Then the draw **stalls** on R; retrying fetches R
  again. The only way out is an explicit, logged **abandon**. There is no offline draw (dropped in revision 6).
- **Restate outage:** there is no second engine (two engines owning a raffle could announce a draw on two rounds).
  The app refuses to start a draw while the engine is unreachable; a stalled draw waits for Restate.

## 7. Entrant identity and coming back

There's no account, but there is an **entry**: a public entrant ID (random, 128-bit; in the manifest), a private
**claim code**, and a secret token in an **HttpOnly, SameSite=Lax cookie** (Secure over HTTPS) for 3 days, of
which only the SHA-256 is stored.

| Situation | What happens |
|---|---|
| The phone locks, the tab is closed, the QR code is scanned again | Same cookie: `GET /j/{CODE}` renders the current state, claim code included. |
| The page comes back from the background | It re-fetches its participant view on `visibilitychange`, `pageshow`, `focus`, `online` and on reconnect; `v` orders views. |
| Another browser, private mode, cleared cookies | The **same email** finds the entry (keyed lowercased, without `+tag`, Gmail without dots). The page says it's safe. It is **not** moved to the new device, or anyone could take over an entry by typing an email. |
| No phone | The organizer adds them; they are confirmed by name in the admin. |

The projector is the source of truth: a fake "you won" screen lacks the winner's private claim code.

## 8. Architecture

### 8.1 Layers

```
rafi-raffle          the raffle: manifest, state machine and admission, rafi-draw-v1, verifier (pure; also WASM); the mock chain (`mock-chain`)
rafi-lifecycle       the raffle's lifecycle in the app: states, starting the draw, entrants, settings, prizes as items (pure)
rafi-engine          the engine contract: Engine, Store, Observer; DrandClient (`drand`), LocalEngine (`local`), the suite (`conformance`)
rafi-engine-restate  the `Raffle` object and `EngineInfo` service (`services()`); RestateClient, the app's Engine
rafi-store           all SQL (SQLite, Postgres, D1); save_raffle / RaffleWriter, the engine's Store; the phones' snapshot
rafi-live-client     what a phone sees and runs: ParticipantView, ParticipantChannel, screen(), Card (`card`), solving the proof-of-work, Follow, PageConfig
rafi-live-server     the servers' side: Notifier (an Observer that can end live updates; Polling), LiveUpdates, the view of a raffle as drawn, the publish rule, issuing and verifying the proof-of-work
rafi-model           the types the app's server and browser exchange
rafi-app-core        every use case, over App { db, chain, engine, live_updates, policy }; typed errors; RateLimit; no HTTP, no i18n
rafi                 the HTTP side (routes, participant pages, auth, retention) and the web UI
rafi-participant     the participant page's WASM and its JavaScript fallback
```

- The rules live in `rafi-raffle` and `rafi-lifecycle`; the store, the engines and the app ask them. The store's SQL
  state lists are generated from them.
- A manifest and an engine run a whole raffle: the engine knows nothing of the app (no lifecycle, no settings). A
  host passes the engine its live notifier as the `Observer` (on Workers; the native host passes none, since its
  clients poll), and the app's retention run dates erasure.
- `rafi-engine` depends on no live or Cloudflare crate, and nothing below the app pulls in Dioxus (`rafi-live-client`'s `card`
  is off). The phone builds only `rafi-live-client`: no `rafi-raffle`, drand or BLS.
- The server sends the admin what it may do (`RaffleAdmin.actions`, `AdminEntrant.deliverable`); the UI derives no
  rules.

### 8.2 Hosts

| | Native (self-hosting) | Cloudflare Workers (planned production) |
|---|---|---|
| App | `rafi --features server`; an hourly retention task; in-memory rate limits | `rafi --features worker`, the same routes; the Cron runs retention; rate-limiting bindings; hosts `RaffleRoom` |
| Database | SQLite or Postgres (`RAFI_DATABASE_URL`) | D1, EU jurisdiction |
| Raffle engine | `restate-rafi` (`crates/restate-rafi-endpoint`) on the same database | `rafi-restate-worker`, called by Restate Cloud over HTTPS (request-response, signed) |
| Live updates | none pushed: clients poll | `RaffleRoom`, a Durable Object per raffle (partyline) |
| Edge | Caddy | Cloudflare |

```
organizer, projector ──▶ app ──ingress──▶ Restate ──▶ Raffle (engine host)  ──▶ database (the record)
phones ────────────────▶ app ◀── database                      └──nudge──▶ RaffleRoom ──WebSocket──▶ phones (Workers)
```

### 8.3 Routes

| Path | What | Auth |
|---|---|---|
| `/admin/**`, `/s/{slug}`, `/v/{slug}` | the web app (static, single-page fallback) | the admin: Clerk |
| `POST /api/admin/{name}` | the admin's JSON API | Clerk session, owner checked |
| `GET`/`POST /j/{CODE}`, `GET /api/j/{CODE}/me` | the participant page, the join, the phone's own entry | entrant cookie |
| `/rules/{slug}`, `/privacy/{slug}` | rules, privacy notice | public |
| `/api/r/{slug}/view`, `/participant`, `/verify.json` | the full view, the participant view, the verification data | public; no emails, tokens or claim codes |
| `/api/altcha`, `/api/status` | a proof-of-work challenge; `live_raffles` for the deploy freeze | public |
| `/participant/` | the participant WASM and fallback | public |
| `/partyline/raffle/{id}` (Workers) | the raffle's live channel | public raffles that aren't over |

### 8.4 Who owns what

| Data | Owner |
|---|---|
| Raffles, prizes, entrants (personal data), consents, claim codes, token hashes, audit | the database. Personal data never leaves the app. |
| The manifest and its hash | the database, written before the raffle opens. |
| The raffle's log (manifest + event log, `raffles.log`) | the database, written by the engine after every change: the projection, and the record. The newest version wins. |
| The live raffle | the engine (Restate object state), from opening to finish. |
| Erasures | the database (`erasures`: entrant ID and time only); a restore must re-apply them. |
| The participant view | `RaffleRoom` (Workers), built from one snapshot (`rafi_cloudflare::participant_view`); destroyed when live updates end. |

### 8.5 The raffle engine

The app sees an engine only as `Engine` (`info`, `open`, `draw`, `decide`) and can't tell `LocalEngine` from
Restate. The contract is documented in `crates/engine/src/engine.rs` and checked by the conformance suite on both:
state is read from the projection; opening is idempotent per manifest; draws are accepted asynchronously (a refused
draw is silent, so the app checks `Raffle::admit` first) and decisions apply when the call returns; commands for one
raffle apply one at a time, so a decision sent during a draw queues behind it (past the client's deadline,
`RESTATE_TIMEOUT_SECS` = 90, the call returns `Pending` and the command still applies once); a command ID applies
once, and reused with another command answers like the first and applies nothing; the projection is the record (an
engine retries a failed write and applies nothing else meanwhile, an abandon included); after finish the engine
forgets the raffle, and callers must not open it again (past Restate's 1 h idempotency retention it would open,
unpublished); a `Observer` bounds its own time. `Unavailable` doesn't mean "not applied": retrying with the
same command ID is safe.

### 8.6 Live updates

- **Two views**, both carrying `v`, the raffle's version in the database (bumped by triggers). The **participant
  view** (state, the current draw, the last draw that wasn't a no-show, winners by entrant ID, prizes left) has no
  personal data beyond the last drawn entrant's public name. The **full view** (projector, admin) adds the count,
  recent names, prizes, every draw and the server's time.
- **Phones** follow the raffle as the server's `Follow` says: over its participant channel (the raffle's
  `RaffleRoom` on Workers), or polling `/api/r/{slug}/participant` natively. They accept only a newer `v`.
- **Each deployment's `LiveUpdates`** (`rafi-live-server`) pairs its `Notifier` with how clients follow a raffle, a
  function of that notifier and the raffle's ID (a Mercure topic would name the raffle): `LiveUpdates::polling()`
  natively, `DurableObjects` and `DurableObjects::follow` on Workers. Once live updates end, clients poll.
- **The projector and admin** poll the full view every 2.5 s while the raffle is live (accepting `v >=`, since recent
  names change with time); on Workers they reveal the winner from the participant channel too.
- **`RaffleRoom`** is nudged by the app (lifecycle, edits; not joins) and by the engine host after each raffle
  write. It reads the raffle in one D1 batch and publishes by `rafi_live_server::publish::decide`; a 15 s alarm repairs a
  lost nudge. **End live updates**, deleting or purging the raffle, or the Cron a day after it finished destroys it
  (final view, sockets closed with 4403, storage deleted).

| Term | Meaning | Not |
|---|---|---|
| **Live updates** | A raffle's changes pushed to screens and phones, until the organizer ends them | realtime, stream |
| **Participant view** | The slim, public state a phone's card renders from | phone view |
| **Participant channel** | The partyline channel carrying the latest participant view, one per raffle (`ParticipantChannel`) | feed, topic |
| **Follow** | How a client keeps its view fresh: poll, partyline or (reserved) SSE (`Follow`) | transport, subscription |
| **Notifier** | The server side: "this raffle changed", "end its live updates" | publisher |
| **Live channel** | A raffle's participant channel as hosted | hub |
| **Raffle room** | On Workers, the Durable Object hosting one raffle's live channel (`RaffleRoom`) | channel, hub |

"Follows the raffle" also says that `RaffleState` mirrors the raffle's progress (`follows_raffle`); that is the
lifecycle's sense, unrelated to `Follow`.

## 9. Abuse and remote entries

- **One entry per normalized email per raffle** (`UNIQUE(raffle_id, email_norm)`). No fingerprinting.
- **Proof-of-work** on the join form, verified with HMAC and redeemed once (a table, so it holds on Workers).
- **Rate limits** per raffle (600/min) and per raffle and email (5/min), never per IP: a venue shares one. On Workers
  they're bindings (soft, per location); the hard limit is the entrant cap in the database (`RAFI_MAX_ENTRANTS`,
  2000). Edge protection by IP is the platform's (Cloudflare, or Caddy).
- **Remote entries:** the join code works only while registration is open; the rules say "must be present to
  claim"; the organizer can remove entrants before the freeze; the no-show rule does the rest. Rotating QR codes were
  rejected (scan failures).
- **Display names:** a blocklist, hide or rename in one tap, a 5 s delay on the projector. Hidden names show as
  ★ ★ ★ (`rafi_live_client::public_name`).

## 10. Privacy and legal (Hungary / EU)

> Not legal advice. Items marked ⚖ need a one-time check with a Hungarian lawyer or tax adviser.

- **Gambling law (Szjtv.):** free entry means no stake, so this is **not gambling** and needs no notification to the
  SZTFH (the gift-draw rules were repealed on 2023-01-01). **Never sell entries or tie them to donations:** that
  makes it a "tombola", which must be notified.
- ⚖ **Personal income tax (SZJA):** a prize from a company is usually an "egyes meghatározott juttatás" (the giver
  pays 15% SZJA plus 13% social contribution tax on 1.18 × the value); from a private person it may be a gift,
  tax-free up to HUF 150k. Confirm who legally gives each prize.
- **Lawful basis:** Art. 6(1)(b) to run the draw and deliver prizes; 6(1)(f) for duplicate and abuse checks; 6(1)(a),
  a separate, optional, unticked consent, for emails about future meetups (Planet49, Grt. 6. §, NAIH-6372/2012). The
  consent version is stored with each entry.
- **Privacy notice** (`/privacy/{slug}`, Hungarian and English): controller, purposes and bases, processors
  (the host; Restate GmbH, DE, anonymous IDs only; Clerk, Inc., US under the DPF, organizers only), retention,
  rights, the NAIH. It names Hetzner as the host today; on Workers it must name Cloudflare, Inc. (US, DPF/SCCs) and
  say that D1 Time Travel keeps erased data up to 30 more days.
- **Cookies:** the entry and language cookies are strictly necessary: no banner, no analytics.
- **Minimisation:** a display name and an email (optional for entrants the organizer adds). Emails never appear on
  the projector, in either view, or in Restate.
- **Retention:** the hourly run dates a finished raffle's erasure (`RAFI_RETENTION_DAYS` /
  `RAFI_RETENTION_WINNERS_DAYS`, 30 / 90) the first time it sees it, then erases what is due: names, emails, tokens
  and claim codes go; entrant IDs and the raffle's log stay. Natively erasure is opt-in (`RAFI_RETENTION`); on Workers it
  is on unless switched off. The admin warns when the run is stale; the organizer can erase by hand.
- **Restate** gets anonymous IDs and the manifest only, keeps journals and idempotency keys for 1 h, and forgets a
  raffle at finish. It has no public DPA: ask, and list it in the notice.
- **On Workers:** D1 created with `--jurisdiction eu`, Durable Objects from an EU-jurisdiction namespace (fixed at
  creation); invocation logs off; no personal data in logs.

## 11. Hosting and operations

- **Production (planned): Cloudflare Workers.** The app Worker (with `RaffleRoom`), `rafi-restate-worker` and one
  D1 database ([workers-plan.md](workers-plan.md); the comments in `crates/app/wrangler.toml` and
  `crates/restate-worker/wrangler.toml`). Missing secrets stop a Worker; dev auth and mock drand are compiled out of
  release builds.
- **Self-hosting: the native server.** `deploy/compose.yaml` runs Caddy, the app and `restate-rafi` on one VM
  (SQLite on a shared volume, or Postgres), against Restate Cloud or, with `--profile self-hosted`, Restate on the
  VM. Development runs Restate in Docker (`compose.yaml`).
- **Restate:** register the engine again whenever its handlers change. On Workers the object uses its built-in
  options (journal and idempotency retention 1 h); natively they are configurable (`ServiceOptionsConfig`).
- **Deploy freeze:** not while a raffle is Open, Frozen or Drawing (`/api/status`). On Workers a deploy restarts the
  Durable Objects.
- **Cost:** Restate Cloud's free tier, Clerk Hobby and the proof-of-work cost nothing (a draw is about 8 Restate
  actions, an estimate). Workers may need the paid plan, depending on BLS CPU time.

## 12. Status

**Verified, locally and in CI:**
- `rafi-raffle` and `rafi-lifecycle`: unit, property and vector tests, among them an exhaustive `start::next` table,
  a chi-square bias test of the selection, and properties that what `Raffle::admit` allows the engine can apply
  and that only claimed winners are deliverable. `tools/verify.py` agrees with the vectors.
- The engine conformance suite on `LocalEngine` and on the `Raffle` object against `restate-server` 1.7.13.
- The storage conformance suite on SQLite, an embedded Postgres 18, Postgres 16 (CI) and D1 under `wrangler dev`.
- The app: use-case tests (`crates/app-core/tests/use_cases.rs`), HTTP tests, and an end-to-end test of the app,
  `restate-rafi` and `restate-server` on one database.
- Workers, locally (`scripts/workers-check.sh`): D1, the live channel, the participant page in headless Chromium
  (the fallback too), a draw through Restate on the Workers endpoint, and a whole evening on the app Worker.
- Sizes, compressed: the app Worker 847 KB, the Restate Worker 914 KB, the participant WASM 263 KB (budget 280 KB).
- Earlier (2026-10-06, native): a run against real drand quicknet, re-checked with `verify.py --fetch`.

**Not done (needs accounts):** the cutover (workers-plan M8: D1 with `--jurisdiction eu`, secrets, domains,
staging), the Restate Cloud deployment, Clerk with real keys, BLS CPU time on a deployed Worker, and the phone test
plan (iOS Safari, Android Chrome, locking during a draw, Wi-Fi off and on).

**Open risks:** Restate is on the critical path during the draw (the app checks it before starting; a stalled draw
is retried on its round or abandoned). BLS CPU time on Workers is unmeasured. A deploy or crash during an event
drops connections (clients re-fetch, Restate replays a draw in flight on the same R; hence the deploy freeze).
⚖ Prize tax treatment. iOS can't re-acquire the wake lock after backgrounding, or vibrate (cosmetic: the projector
is the source of truth).

**Phase 2:** winner emails (from a Restate handler), a magic link to move an entry to another device, voucher codes
shown only to the winner, sponsor-sharing consent, a Meetup.com RSVP cross-check, templates.

## Revision history

- **1–2** (2026-10-05). First drafts, on Cloudflare. Not kept.
- **3** (2026-10-05). No Cloudflare: one Dioxus fullstack server on SQLite with Litestream (Hetzner), the draw in a Restate
  `Raffle` virtual object reached through the SDK's tunnel, SSE from `tokio` watch channels.
- **4** (2026-10-06). `restate-rafi`, a separate service writing draw state into the app's database (the app polled it
  every 250 ms; ingress over plain HTTP); Litestream dropped; one VM with `deploy/compose.yaml`. 2026-10-07: every
  beacon BLS-verified, a missing beacon stalls the draw on its round ([drand-crate.md](drand-crate.md)).
- **5** (2026-10-09). The draw engine is a seam, with its rules in `rafi-core`, a local engine for tests and a conformance
  suite; the app decides the drand chain; the "local draw" fallback is dropped. (2026-10-08: the stateless server,
  the admin as a web app, all SQL in `rafi-store`, Postgres; [workers-plan.md](workers-plan.md).)
- **6** (2026-10-09). The drawing is its own domain, `rafi-drawing`, opened from a manifest; prizes become items; the
  Restate object is `Drawing`. Offline draws dropped (`7b8721f`).
- **7** (2026-10-10). The engine contract in its own crate, `rafi-engine`; the Restate engine is `rafi-engine-restate`,
  with the SDK's generated client; `restate-rafi-client` and `raffles.engine` gone.
- **8** (2026-10-10). `rafi-core` is `rafi-raffle` (`746f3a0`; now `rafi-lifecycle`) and holds all the raffle's rules;
  the engine carries no app policy; Workers planned for production, the native server kept for self-hosting.
- **9** (2026-10-10). `rafi-engine` free of `rafi-platform` and Dioxus (`Notify` to `rafi-platform`, the publish rule to
  `rafi_live::publish`, `Card` behind `card`); the engine contract tidied and stated (reused command IDs, the
  projection as the record, no reopening, notifier deadlines); `rafi-raffle` gains settings normalization,
  `may_join`, `fakes`, `deliverable` (claimed winners only); `Start::Commit` is `Start::WriteManifest`; new crates
  `rafi-model` and `rafi-app-core`; entrant patches all-or-nothing, entrant and prize writes guarded on the raffle's
  state, the admin recovers a raffle frozen without a manifest. This document rewritten as the current state.
- **10** (2026-10-10). The drawing is the raffle ([raffle-plan.md](raffle-plan.md)): `rafi-drawing` is `rafi-raffle`
  (`Raffle`, `RaffleId`; the manifest names its `raffle`), the app's rules are `rafi-lifecycle`, the engine contract is
  `RaffleEngine` / `RaffleStore` / `RaffleNotifier`, the Restate object is `Raffle` again, and the row stores it as
  `raffles.log`. Draws and items keep their names.
- **11** (2026-10-14). `rafi-platform` dissolved: the engine contract is `Engine` / `Store` / `Observer`; `rafi-live`
  split into `rafi-live-client` (the phone's) and `rafi-live-server`, whose `Notifier` is an `Observer` that can also
  end live updates, so a deployment hands its notifier to the engine as is (`Notify` gone); `RateLimit` is
  `rafi-app-core`'s, its Workers bindings the app Worker's; `LiveUpdates` moved to `rafi-live-server`.

## Appendix: key sources

- Dioxus 0.7: https://dioxuslabs.com/learn/0.7/ · dioxus-clerk: https://github.com/sagikazarmark/dioxus-clerk · partyline: https://github.com/sagikazarmark/partyline
- Restate Rust SDK: https://docs.rs/restate-sdk/latest/restate_sdk/ · retention: https://docs.restate.dev/services/configuration · pricing: https://restate.dev/pricing
- drand: https://docs.drand.love/developer/API-v2/drand-http-api/ · https://crates.io/crates/drand · rand reproducibility: https://rust-random.github.io/book/crate-reprod.html
- SZTFH (gift draws, tombola): https://sztfh.hu/tevekenysegek/szerencsejatek-felugyelet/ajandeksorsolas/ · https://sztfh.hu/tevekenysegek/szerencsejatek-felugyelet/tombola/
- EDPB on Art. 6(1)(b): https://www.edpb.europa.eu/sites/default/files/files/file1/edpb_guidelines-art_6-1-b-adopted_after_public_consultation_en.pdf · NAIH-6372/2012: https://www.naih.hu/files/6372_2012_hatarozat.pdf
- Wake Lock: https://developer.mozilla.org/en-US/docs/Web/API/Screen_Wake_Lock_API · WebKit bug 254545: https://bugs.webkit.org/show_bug.cgi?id=254545
