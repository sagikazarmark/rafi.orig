# Pre-release review

**Status: accepted, in progress (2026-10-26).** The review before the first stable release, and the remediation
it decided. Five passes over the code, each on its own axis: the Cloudflare Workers deployment, the privacy and
retention features against [privacy-and-retention.md](privacy-and-retention.md), dead code and stale references,
what becomes expensive to change once a database and a deployment exist, and security. Every finding is here,
with what to do about it, so the work can be ticketed and checked off. Findings that were raised and rejected are
in §Accepted risks and §Deferred, with the reasoning.

The premise: Cloudflare Workers is the production target (Docker images and native distribution come back
later); the privacy features are implemented completely; dead code and stale references go; and anything that is
cheaper to change before a release than after is changed now. "Cheaper now" means: the schema (`0001_init.sql`
is still edited in place), the public JSON, the draw algorithm's versioning, the admin API's wire shape, the
names of environment variables, routes and the Worker, the notice's wording, and the defaults the first
operators will build runbooks on.

## Decisions taken during the review

These settle questions the findings raised; the findings below assume them. Each was challenged a second time,
against primary sources (Restate's source, Cloudflare's documentation, the EDPB's guidelines, the built binaries);
where the challenge found the original reasoning wrong, the reasoning below is the corrected one and the decision
stood on it.

- **Legal acceptances go, for now.** The `legal_acceptances` table, `accept_terms`, the gate on `create` and
  `duplicate`, the admin's "I accept" screen, `rafi_store::legal`, `rafi_app::legal`, `Policy.terms_digest` and
  their tests are removed. `/terms` and `/privacy` stay as served documents with their versions; the generated
  notice keeps linking to the operator's notice. The terms template says that acceptance is by use, since nothing
  records it. A passive record (a `terms_in_force` audit row at creation, a line under the Create button) was
  offered and declined; the consequences are in §Accepted risks.
- **No schema version row, and no migration check.** Neither a `meta('schema_version')` row nor a `/healthz`
  comparison of the newest applied migration with the newest compiled-in one. The order of operations in
  `upgrades.md` (migrate, then deploy, never roll a Worker back past a migration) is the safeguard. `/healthz` is
  a static `ok` and does not detect a forgotten migration. See §Accepted risks.
- **Join codes become 6 characters and stay `NOT NULL`.** 31⁶ ≈ 887 M codes. A purged raffle keeps its code;
  nothing is recycled. Lengthening later is a code change with no migration (the column is `TEXT`, `normalize`
  strips dashes and case, old codes stay valid). What a hit yields, and the one free narrowing of the target set
  (purged raffles stop resolving at `/j/<code>`), are in SCH-4.
- **The engine is never registered with `force`, and the path stays `/restate`.** Restate's semantics, verified
  in its source (`crates/types/src/schema/metadata/updater/mod.rs`, `crates/admin/src/rest_api/deployments.rs`):
  registering a URI that already exists without `force` returns `200 Unchanged` **before any handler validation**,
  whether or not the handlers changed; `409` exists only on the legacy `/v1` and `/v2` admin paths. So there is no
  refusal to act as a signal. A changed engine must be registered at a **new URI**: natively a new host or port,
  on Workers a new path, which means changing `engine::PREFIX` in code at that time (versioning the path now, as
  `/restate/v<version>`, was offered and declined). The runbook therefore has to say it, and the operator has to
  know it. The docs stop claiming that a draw in flight survives a deploy: on Workers the old code is gone the
  moment the new one is live, so **the deploy freeze is the safeguard** for a draw in flight. No version tag is
  added to the engine's stored state (§Accepted risks: the reasoning is the compatibility rule, not the freeze).
- **Participant scripts keep their fixed names.** `/participant/rafi_participant.js`, `rafi_participant_bg.wasm`
  and `fallback.js` are not a public contract: only the server-rendered page links to them, and it is `no-store`.
  Workers Assets already revalidates every asset (`max-age=0, must-revalidate` plus an `ETag`, per Cloudflare's
  documentation); natively `ServeDir` sends `Last-Modified` and no `Cache-Control`, so browsers apply heuristic
  freshness and may keep an old script for days. What changes now is the native side: an explicit `Cache-Control:
  no-cache`, an `ETag`, and compression (API-7). Content-hashed names with `immutable` are a follow-up
  (§Deferred), with the design.

## Findings and remediation

Each finding has an ID for tickets and commits. **Size**: trivial (minutes), small (an hour or two), medium (a
day). **Why now**: what gets expensive after the release.

### Schema, while `0001_init.sql` is still edited in place

#### SCH-1 · `audit.actor` is the owner, never the user who acted — **done**

- **Where**: `migrations/sqlite/0001_init.sql` and `migrations/postgres/0001_init.sql` (`audit`);
  `crates/store/src/audit.rs:6`; the 16 `audit::record` call sites (`crates/app/src/draw.rs:131,179`,
  `raffles.rs:94,234,268,335,373`, `entrants.rs:267-445`, `reads.rs:184`) **and five raw `INSERT INTO audit`
  that bypass it**: `crates/store/src/retention.rs:41,136,152` (housekeeping) and `crates/store/src/raffles.rs:467,474`
  (`owner_leaves`, which should carry the user who pressed "Remove my data").
- **Why**: in a Clerk organization the owner is the organization, so two members pressing "Not here" or exporting
  the CSV are indistinguishable. The audit log is the organizer's accountability record and can't be backfilled.
  `Caller { owner, user }` already exists and is always populated, dev auth included
  (`crates/server/src/auth.rs:19-36`); only `accept_terms` used `user`.
- **Change**: `user_id TEXT NOT NULL DEFAULT ''` on `audit` in both dialects (valid and identical SQL in both;
  the default is a safety net once every writer sets it). The shape, since no use case sees `Caller` today and a
  request-local context would hide the dependency:
  `rafi_store::audit::Actor<'a> { owner: &'a str, user: &'a str }`, a `HOUSEKEEPING` constant,
  `audit::record(db, raffle_id, actor, action, detail)`; `From<&Caller> for Actor` in `rafi-app`; every use case
  takes `actor: Actor<'_>` instead of `owner: &str` (26 signatures) and calls `owned(app, actor.owner, id)`; the
  dispatch passes `c.into()` (31 arguments); `owner_leaves` takes the actor; the three housekeeping inserts write
  `HOUSEKEEPING` in both columns; the test fixtures gain an `ACTOR` constant. Record in `privacy-and-abuse.md`
  ("Leaving") that the account identifiers stay on raffles and in the audit log, as the operator's accountability
  record under Art. 6(1)(f), for the period the operator's notice names (`apps/worker/legal/privacy.md:49-50`
  already has the `[period]` placeholder). `remove_my_data` does not blank them (nor `raffles.owner_id`, today);
  enforcing the period is in §Deferred.
- **Size**: small (schema) + small-to-medium (mechanical plumbing: ~80 edits). **Why now**: a migration and a
  backfill that can't be done later.

#### SCH-2 · `prizes.id` is a global 6-character primary key with no collision handling — **done**

- **Where**: `0001_init.sql` (`prizes`, SQLite line 62, Postgres line 64); `crates/app/src/prizes.rs:41`
  (`ids::code(6).to_lowercase()`, one insert, no retry); `crates/app/src/raffles.rs:284-298` (`duplicate`
  generates the copies' IDs **outside** its retry loop at `:319`, so an intra-raffle collision retries eight times
  with the same IDs and fails as the misleading `Refusal::JoinCodes`, `:342`).
- **Why**: 31⁶ = 887 503 681 values, prizes of purged raffles are never deleted, and a collision is a unique
  violation that `crates/app/src/error.rs:31-34` maps to `Internal`, HTTP 500 on "Save prize". Cumulative
  collision probability: 1.4 % at 5 000 prizes, 50 % at ~35 000. Nothing references `prizes.id` by foreign key
  (`draws.item_id` has none), and every prize query is already `(id, raffle_id)`-scoped
  (`crates/store/src/prizes.rs:45,99,119`, `view.rs:33-37`); prize IDs appear in no URL.
- **Change**: `ids::id()` (128-bit) at `prizes.rs:41` and `raffles.rs:288`. No schema change, no retry loop, no
  table rebuild; item IDs become `<32 hex>-<n>`, still safe under `rsplit_once('-')`. The composite primary key
  was the alternative; it would additionally need a unique-violation retry in `upsert_prize` and in `duplicate`
  to honestly claim "no 500".
- **Size**: trivial. **Why now**: the IDs are in every manifest from the first raffle on.

#### SCH-3 · Remove `legal_acceptances` and the acceptance flow — **done**

- **Where** (everything that references it, so the ticket is complete):
  - schema: `0001_init.sql` (`legal_acceptances`, both dialects);
  - store: `crates/store/src/legal.rs`; `crates/store/src/lib.rs:17,55`; `conformance/mod.rs:221`,
    `conformance/domain.rs:2039-2070`;
  - app: `crates/app/src/legal.rs`; `crates/app/src/lib.rs:23,46,87-91` (`mod legal`, the re-exports,
    `Policy.terms_version`/`terms_digest`); `crates/app/src/error.rs:117,267` (`Refusal::TermsOutdated`); the gate
    in `crates/app/src/raffles.rs:57,280`;
  - api: `crates/api/src/admin.rs:30-34` (`get_terms`, `accept_terms`); `crates/api/src/lib.rs:339-342`
    (`Me.terms_accepted`), `:416-425` (`TermsDocument`);
  - server: `crates/server/src/admin.rs:96,101-112`; `crates/server/src/lib.rs:109-110` (sets the policy fields);
    `crates/server/src/config.rs:65-68` (`Documents::terms_digest`, whose only consumer this was);
  - web: `apps/web/src/ui/admin/mod.rs:166-192` (the gate branching) and `:249-295` (the screen);
  - Workers testbed: `apps/worker/evening.mjs:34-40` calls `get_terms`/`accept_terms`; `scripts/workers-check.sh`
    breaks unless it is updated;
  - tests: `crates/server/src/tests.rs:179-188` (the fixture helper) and its callers `:1354,1395,2835,3518`;
    `:1068-1105` (queries `legal_acceptances` inside `admin_api_needs_a_session_unless_dev_auth`); `:3073`;
    `:3210-3260` (`a_new_terms_version_is_accepted_again`); `:3359-3372` (inside
    `organization_tiers_gate_the_admin_api`); `crates/server/src/tests_restate.rs:98`;
    `crates/app/tests/use_cases.rs:44-54`;
  - `Access::Admin` keeps `remove_my_data` only (`crates/server/src/admin.rs:303`).
- **Docs**: `privacy-and-abuse.md:78,82` ("Organizers accept the operator's terms … the acceptance, its version and
  a digest of the text are recorded, and a new version is accepted again" goes); `running-a-raffle.md:71-72`;
  `routes.md:40-41`; `glossary.md:12,14`; `self-hosting.md:86-93`; `apps/worker/legal/terms.md` says acceptance is
  by use and, per the EDPB (§Accepted risks), that a change to the processing agreement is notified directly and
  needs the organizer's approval, while other changes take effect when published with a new version;
  `privacy-and-retention.md` gets a line that the acceptance record was removed before release.
- **Keep** (used independently): `/terms`, `/privacy` and the `pages::legal` renderer, `Config.terms_version`/
  `privacy_version`, `Documents`, `RAFI_DOCS_DIR`, `RAFI_TERMS_VERSION`, `RAFI_PRIVACY_VERSION`, the template
  refusal (`config.rs:250-256`), `apps/worker/legal/` and its `include_str!`, the operator named in the notice,
  `sha2`/`hex` in `rafi-server` (used elsewhere).
- **Size**: small. **Why now**: a decision taken; removing it later would be a migration that drops evidence.

#### SCH-4 · Join codes: 6 characters, and purged raffles stop resolving — **done**

- **Where**: `crates/app/src/raffles.rs:85,326` (`ids::code(4)`); `crates/app/src/public.rs:17-25`
  (`raffle_by_join_code` filters on `is_public()`, which is every state but `Draft`, `crates/lifecycle/src/lib.rs:196-198`);
  the admin badge `apps/web/src/ui/admin/header.rs:262`; the projector and the join page show the code.
- **Why**: 31⁴ ≈ 923 k codes, global `UNIQUE`, never recycled; `/j/<code>` is a 404/200 oracle with no per-IP
  limit by design. What a hit yields, **before any proof-of-work**: the join page with the slug in it
  (`pages/participant.rs:159-160,287-291`), and from the slug `/rules/<slug>` (organizer name, contact email,
  prizes with sponsors, `pages/text.rs:164-171`), `/privacy/<slug>` (the privacy contact) and `/api/r/<slug>/view`
  (`join_code`, the entrant count, the last 20 display names, `crates/app/src/view.rs:50-56,176-183`). The
  proof-of-work gates only the join itself. And the discoverable set is every non-Draft raffle **ever**, purged
  ones included, since a purged raffle keeps its code: at 1 000 lifetime raffles the expected cost of one hit is
  about 887 k requests at 31⁶, the figure this finding calls unacceptable at 31⁴ today. A per-code-miss limiter
  isn't viable (random codes have no prefix to bucket on, per-IP is excluded, a global miss limiter is a
  one-request denial of service against everyone typing a code). The column stays `NOT NULL` (decided).
- **Change**: `code(6)`, rendered `XXX-XXX` on the projector, the join page and the admin badge
  (`codes::normalize` already drops dashes). **`/j/<code>` returns 404 for `Purged` raffles** (a filter in
  `public::raffle_by_join_code`; the page would show nothing anyway: the token hash is erased, so no entry is ever
  found), which shrinks the discoverable set from "every raffle ever" to "raffles inside their hold period, about
  100 days". Document the length in `glossary.md`. Eight characters (`XXXX-XXXX`) is the knob if an installation
  ever runs into thousands of raffles per hold period.
- **Size**: trivial. **Why now**: the code is on every projector and QR; changing it after people have learned the
  shape is confusing, changing it now is free.

#### SCH-5 · `consent_version` is an undocumented compound string — **done** (documented, not split)

- **Where**: `crates/app/src/entrants.rs:17` (`"2026-10-21.2"`), `crates/app/src/lib.rs:100-102`
  (`"{CONSENT_VERSION}/age{min_age}"`), stored per entrant for good.
- **Why**: it is legal evidence of what the entrant was shown, and its grammar lives in two source lines.
- **Change**: either document the grammar (`<form version>/age<min age>`) in `privacy-and-abuse.md` and in the
  schema comment, or split into `consent_version TEXT` and `consent_min_age INTEGER`. The split is cleaner for
  the operational query "who was asked for 18"; the documentation is enough if that query never comes.
- **Size**: trivial (document) or small (split). **Why now**: a column split is a migration with a parse.

#### SCH-6 · Housekeeping scans every non-erased entrant several times an hour — **done**

- **Where**: `crates/store/src/retention.rs:67-80` (`due()`) and `:107-130` (`erase_due`). `EXPLAIN QUERY PLAN`
  on the actual schema: `SCAN entrants`, the whole table, twice per idle run and five times when something is
  due. The `raffles_erase_at` partial index (`0001_init.sql:59`) exists but these queries are entrant-driven.
- **Why**: D1 bills rows read; the table only grows (purged raffles keep their rows).
- **Change**: prepend `entrants.raffle_id IN (SELECT id FROM raffles WHERE state IN ({frozen}) AND erase_at <= ?1)
  AND` to `due()` and keep the rest unchanged. The plan becomes `SEARCH entrants USING INDEX entrants_raffle` plus
  one `SCAN raffles`, for the `SELECT` and the `UPDATE` alike, with no new index, no `UPDATE … FROM`, no CTE, and
  the same SQL on SQLite, D1 and Postgres; the per-entrant clauses (delivered, the hold, `NOT IN` the claimed
  draws) stay in the correlated `EXISTS`, so the semantics are identical. **Do not add `purged_at IS NULL`**:
  `frozen_or_later` includes `Purged` on purpose (`crates/lifecycle/src/lib.rs:183-188`), so a purged raffle
  restored from a backup with un-erased entrants is still erased at `erase_at`; the clause would exempt it until
  PRV-4's tool is run. A partial index on `entrants (raffle_id) WHERE erased = 0` alone doesn't help (it still
  scans every live entrant). While here: `raffles_owner` becomes `(owner_id, created_at, id)` and
  `list_by_owner`'s `ORDER BY` gets `, id DESC` as a tiebreak (that also fixes CI-4).
- **Size**: small. **Why now**: a query shape and an index are cheap on an empty database and a surprise on the
  first D1 bill.

### Public contracts: JSON, the algorithm, the engine, the admin API

#### API-1 · The algorithm identifier is checked by equality on every restore; the verifiers disagree on unknown events — **done** (the rule and the Python fix; no dispatch code)

- **Where**: `crates/raffle/src/manifest.rs:88` (`if self.algo != ALGO`), called by `Raffle::open`
  (`raffle.rs:390-393`), which `Raffle::restore` calls (`raffle.rs:400-407`), which `rafi_store::raffles::rebuild`
  calls (`crates/store/src/raffles.rs:132-145`); `verify.rs:123-125` reports it as an error; `algorithm.rs:36`
  seeds with the `ALGO` constant rather than the manifest's string; `tools/verify.py:95` has the same equality
  check, and `:130-189` has **no `else` branch**: unknown event types, and `finished`, are silently skipped.
- **Why**: two cases. *Upgrade* (a v1 raffle read by v2 code): entirely v2's problem and fully solvable then,
  because every stored manifest already carries `"algo":"rafi-draw-v1"` and every event its `type`; the data is
  self-describing. *Rollback* (a v2 raffle read by v1 code): `rebuild` fails on the algorithm **and** on any
  unknown event `type`, so the admin page, the projector, `verify.json` and even `delete` are 500 for that raffle;
  no code shipped now can prevent that, and the "never roll back past a migration" rule already covers it. The
  refactor originally proposed here (a known set, dispatch by `algo`) is a behavioural no-op while one algorithm
  exists. What is *not* a no-op is the verifiers' disagreement: a verifier that skips what it can't interpret
  vouches for a log it didn't check (a renamed `voided` or `finished`, or a pass-1-only `resolved`, goes
  unnoticed). Reject is right.
- **Change**: no change to `Manifest::validate` or `select` now. Write into `draw-algorithm.md`: (a) a reader
  dispatches on the manifest's `algo` and keeps every algorithm it ever shipped; opening a *new* raffle requires
  the current one; (b) a verifier **fails** on an unknown `algo` or an unknown event `type`, never skips. Fix
  `tools/verify.py`: an explicit `finished` branch and `else: fail("unknown event type")`. Optional: the web
  verifier (`apps/web/src/ui/verify.rs:36-41`) reports an unknown event as a verification error rather than
  "hasn't been frozen yet, or doesn't exist". The rollback case goes to §Accepted risks.
- **Size**: trivial. **Why now**: the forward-compatibility rule is the contract; the dispatch code is v2's.

#### API-2 · The engine's deploy story — **done**

- **Where**: `docs/operations/upgrades.md:5-8` ("Don't deploy while a raffle is live, meaning registration is
  open, or the list is frozen…"; "A deploy during a draw is safe for the result") and `:35-39`,
  `docs/operations/self-hosting.md:77-78` (`"force": true`); `scripts/workers-check.sh:95` (the bare
  `/deployments` path); `crates/engine-restate/src/service.rs:145` (`open` writes the state), `:213-215`
  (`decide(Finish)` clears it), `:161-201` (the `draw` handler's journaled steps); `apps/worker/src/engine.rs:35-41`
  (`PREFIX`, `owns`).
- **Why**: three things the docs get wrong, verified against Restate's source (`crates/types/src/schema/metadata/updater/mod.rs`,
  `crates/admin/src/rest_api/deployments.rs`) and its versioning guide.
  1. `force` overwrites the deployment behind a URI and "can lead in-flight invocations to an unrecoverable error
     state"; it also implies `breaking`. Without `force`, registering an existing URI returns `200 Unchanged`
     **before any handler validation**, whichever handlers changed (`409` only on the legacy `/v1`,`/v2` paths).
     So a no-`force` re-register of `https://host/restate` after a handler change silently keeps the **old**
     schema, and the failure appears at invocation time (ingress 404 for a new handler, `UnknownServiceHandler`
     for a removed one).
  2. Restate routes new invocations to the latest deployment and pins in-flight ones to theirs; virtual object
     state persists across deployments (it is keyed by service and key, not by deployment). Replay of a draw in
     flight is only safe if the old code keeps serving the old deployment until it drains, and the sequence of
     `ctx.run` steps is unchanged. On Workers neither holds across a deploy.
  3. The engine holds a raffle's state from `open` until `decide(Finish)`, and a frozen raffle the organizer
     walked away from is only asked to finish at the ceiling: `erase_at` plus the winner hold, **about 97 days by
     default**, up to 730. A hosted installation with many organizers will essentially always have a raffle Open
     or Frozen, so "don't deploy while a raffle is frozen" is unachievable there, and operators will deploy with
     old state in Restate. The freeze protects the live room and a draw in flight (minutes around a draw); it does
     not, and cannot, protect data shapes (months). That protection is a compatibility rule (§Accepted risks).
- **Change** (decided: docs only, the path stays `/restate`): never `force`. Register without it; `upgrades.md`
  and `self-hosting.md` say plainly that Restate answers 200 and keeps what it has, so **a changed engine (any
  change to the `Raffle` object's or `EngineInfo`'s handlers) must be registered at a new URI**: natively a new
  host or port; on Workers a new path, which means changing `engine::PREFIX` in that release (`owns()` keeps
  accepting `/restate/*`, so invocations pinned to the old URI still arrive). Keep the previous deployment
  registered until `restate deployment describe <id> --extra` shows no invocations, then remove it; never
  `DELETE …?force=true` while a draw is in flight. Rewrite the deploy freeze to what it protects: no deploy while
  a draw is pending or a room is live (minutes), not "while any raffle is frozen" (months). State the
  compatibility rule for `Event` and `Manifest` JSON (§Accepted risks) in `draw-algorithm.md` and `upgrades.md`,
  and add a test that deserializes a committed fixture of every v1 event shape (`testdata/draw-vectors.json`
  carries events; assert the round trip). No version tag on `StoredRaffle` (decided). Document the `Raffle.get`
  handler (`service.rs:219-225`) or remove it: a handler name is part of the deployment's contract. SEC-2's rule
  also closes a leak here: deleting a frozen raffle today never tells the engine, so its Restate state is
  orphaned for good.
- **Size**: small (docs, one test). **Why now**: the first registered deployment makes it operational.

#### API-3 · Phones reject unknown enum variants — **done**

- **Where**: `crates/live-client/src/view.rs:26-44` (`Phase`, externally tagged, unit-only, `rename_all`), no
  `#[serde(other)]`; `:55-60` (`is_live()` is a `matches!` over five named states); `crates/live-client/src/page.rs`
  (`PageConfig.page_state`, parsed by a stale module from a page the new server rendered);
  `crates/live-client/src/follow.rs:12-23` (`Follow`, internally tagged, the same exposure);
  `apps/participant/src/net.rs:21` and `config.rs:10` swallow parse errors with `.ok()`;
  `apps/participant/src/app.rs:164,173` (the poll loop refetches and the channel subscribes only while
  `is_live()`). `DrawOutcome` (`view.rs:115-123`) is **not** on the wire to phones: it is the server-side input to
  `ParticipantView::new`; the view carries `Winner.claimed: bool`.
- **Why**: a phone on an old module freezes on its last view when the server adds a state. `fallback.js` is
  tolerant; the WebAssembly isn't. `#[serde(other)]` alone would be self-defeating: an `Unknown` phase is not
  `is_live()`, so the module would stop polling, exactly the freeze this prevents.
- **Change**: `#[serde(other)] Unknown` on `Phase` (confirmed to work with `rename_all` on a unit-only enum) and
  on `Follow` (confirmed on an internally tagged enum); `is_live()` returns `true` and `is_over()` `false` for
  `Unknown`; `screen()` shows the waiting card. Nothing on `DrawOutcome`.
- **Size**: trivial. **Why now**: every phone that ever loads the module before the first change.

#### API-4 · `verify.json` has no format version, no caching headers, and reads inconsistently — **done**

- **Where**: `crates/api/src/lib.rs:310-324` (`VerifyData`, no `v`); `crates/app/src/reads.rs:191,200,216` (three
  separate reads); `crates/server/src/public.rs:67-76` (no `Cache-Control`, no `ETag`).
  `view::Full::read` (`crates/store/src/view.rs:28-45`) already reads raffle, prizes, entrants and events in one
  `batch_rows`, keyed by ID.
- **Why**: the manifest's `algo` and the events' `type` tags cover the inner shapes, but the envelope has no
  version; `names` and `events` can straddle a write; and a payload that is immutable once the raffle finishes,
  and that carries display names erased later, is served with no caching instruction at all. `batch_rows` is a
  snapshot on every backend (one transaction on SQLite and on Postgres under `serializable`; `D1Database.batch`
  is one transaction), pinned by `query.batch_rows_snapshot` and `query.batch_rows_under_writes`. Erasure bumps
  the raffle's `version` (`entrants_version_update` on SQLite, `entrants_version` on Postgres fire on the `ERASED`
  update), and so do event appends (`write_raffle` always updates the `raffles` row in the same batch), so an
  `ETag` from the version never serves an erased name.
- **Change**: `"format": 1` at the top level; `verify_data` resolves slug → ID and reads through `Full::read`
  (one batch); `VerifyData` gains `v: u64` like `RaffleView`; `ETag` from the version alone, with
  `Cache-Control: no-cache` (revalidate).
- **Size**: small. **Why now**: `tools/verify.py` and anyone else's verifier read this shape.

#### API-5 · Admin API: no error code, inconsistent function names — **done**

- **Where**: `crates/api/src/admin.rs:88-92` (`ApiError { message }`, English only); `:29-83` (`deliver_all` vs
  `purge_raffle`/`finish_raffle`, `remove_entrant` vs `delete_prize`/`delete_raffle`, `lifecycle` as a bare noun,
  `end_live_updates`/`export_winners`/`save_prize`/`add_entrant` vs the use cases `end_live`/`export_csv`/
  `upsert_prize`/`add_manual`).
- **Why**: the web app is served from the same deploy, so skew is only during a rollout; but both sides are
  generated from one macro, so renaming is free today and a wire break tomorrow.
- **Change**: add `code: &'static str` (the `Refusal` variant in snake_case) next to `message`; settle the names:
  `deliver_all_prizes`, `delete_entrant`, `set_lifecycle` (or `transition`), `export_winners_csv`, and align each
  with its use case. Collapse the three "not finished" refusals (`NotOver`, `NotFinishedForDelivery`,
  `NotPurgeable`, `crates/app/src/error.rs:111,157-160`) into one with the action named.
- **Size**: small. **Why now**: one macro edit now; a versioned API later.

#### API-6 · The verifier re-serialises the manifest and fails on a mismatch — **done** (the escaping is pinned; the check stays an error)

- **Where**: `crates/raffle/src/verify.rs:118-122`; `docs/reference/draw-algorithm.md:36` ("verifiers must not
  re-serialize").
- **Why**: prize names are arbitrary text; if `serde_json`'s escaping ever differs between the engine's version and
  the verifier's (U+2028/2029 are the classic case), a legitimate published manifest fails verification.
- **Change**: either pin the escaping rules in the spec and keep the check, or report "not canonical" as a warning
  that doesn't flip `ok()`. The Python verifier must agree.
- **Size**: trivial. **Why now**: the spec is the contract; the first published manifest freezes it.

#### API-7 · Participant scripts: explicit caching, and compression natively — **done**

- **Where**: `crates/server/src/pages/participant.rs:82-84` (`LOADER`), `crates/server/src/lib.rs:166`
  (`ServeDir`), `apps/worker/wrangler.toml:33` (`run_worker_first` lists `/participant/altcha-worker.js` only, so
  the three scripts are served by Workers Assets), `scripts/participant.sh`, `scripts/app-assets.sh`.
- **Why**: the two targets differ. Workers Assets sends `Cache-Control: public, max-age=0, must-revalidate` and
  an `ETag` by default (Cloudflare, Static Assets › Headers), so it never caches heuristically; `no-cache` there
  is the default restated. tower-http 0.6's `ServeDir` sends `Last-Modified` and honours `If-Modified-Since` but
  emits **no `ETag` and no `Cache-Control`** (grep of the crate), so browsers apply heuristic freshness (about a
  tenth of the file's age) and a module built a month ago can be served stale for days after a deploy. And
  natively nothing compresses: no `CompressionLayer`, no `precompressed_*` in `crates/server` or `apps/rafi`, so
  the module (280 KB gzipped is the budget; raw is larger) crosses venue wifi uncompressed. Cloudflare compresses
  at the edge. `?v=` is not a cheap alternative: the wasm-bindgen glue fetches `rafi_participant_bg.wasm` relative
  to `import.meta.url`, so a query on the import never reaches the wasm, and a long `max-age` would let new JS
  initialise an old module. That is the hashed-name design in §Deferred. The names stay fixed (decided).
- **Change**: natively, `Cache-Control: no-cache` plus an `ETag` on `/participant/*` (a header layer on the
  `ServeDir`, or a small handler), and compression: `ServeDir::precompressed_br().precompressed_gzip()` with
  `participant.sh` writing `.br` and `.gz` beside the files (tower-http needs both), or a `CompressionLayer` on
  the nest. On Workers, a `_headers` file that `app-assets.sh` writes into `worker-assets/`, stating the same
  `no-cache` explicitly (the file also carries SEC-6's headers and, later, `immutable` for the dx bundle's hashed
  `/assets/*`); it must not match `/participant/altcha-worker.js`, which is a route (Cloudflare: `_headers` does
  not apply to responses the Worker generates under `run_worker_first`), and that route gets the header from the
  native layer since `routes()` is shared. Cost: two conditional requests per page load, answered 304; identical
  to today on Workers.
- **Size**: small.

### Configuration names, before the first operator writes them down

#### CFG-1 · Three environment prefixes, and a collision waiting to happen — **done**

- **Where**: the app reads `RESTATE_INGRESS_URL`, `RESTATE_API_KEY`, `RESTATE_TIMEOUT_SECS`
  (`crates/server/src/config.rs:179-188`); the Worker's engine reads `RESTATE_IDENTITY_KEYS`
  (`apps/worker/src/engine.rs:64`); the native engine reads `RAFI_RESTATE_*` through figment with `__` nesting
  into a `deny_unknown_fields` struct whose top-level keys are `endpoint`, `services`, `drand`, `database`
  (`apps/restate/src/config.rs:17,20,91`), and the unprefixed `CONFIG_FILE` (`apps/restate/src/main.rs:27`);
  everything else is `RAFI_*`.
- **Why**: `RESTATE_*` is the namespace `restate-server` itself reads (`scripts/workers-check.sh:93-96` runs both in
  one environment). And any `RAFI_RESTATE_<x>` that isn't one of the engine's four sections kills the native
  engine at startup; verified on the built binary: `RAFI_RESTATE_CONFIG=… rafi-restate` → `unknown field: found
  'config', expected one of endpoint, services, drand, database`, and the same for `RAFI_RESTATE_IDENTITY_KEYS`.
  So the first draft of this finding, which proposed exactly those two names, would have shipped the collision it
  warned about.
- **Change**: the engine keeps `RAFI_RESTATE_*` (its binary is `rafi-restate`). The app's become
  `RAFI_ENGINE_INGRESS_URL`, `RAFI_ENGINE_API_KEY`, `RAFI_ENGINE_TIMEOUT_SECS`: "engine" is the app's own word
  (`app.engine()`, `rafi-engine`, `EngineInfo`), and "ingress" stays because the wrong URL to paste is the
  engine's own endpoint (`:9080`, `/restate`) and the name says which one is meant. The Worker's identity keys
  become **`RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS`**: the native engine's name for the same setting
  (`configuration.md:87`), a valid figment key, one name for one concept. The config file: keep `CONFIG_FILE`'s
  role but take it out of the collision by adding `.ignore(&["config"])` to the figment `Env` provider and naming
  it `RAFI_RESTATE_CONFIG` (one line and a test), or pick a name outside the prefix. `RAFI_DRAND_*` stays as is on
  the Worker (the app reads it in the same process). Update `configuration.md`, `cloudflare.md`,
  `self-hosting.md`, `wrangler.toml`, `devenv.nix`, `scripts/workers-check.sh`, the tests. Also:
  `RAFI_DO_JURISDICTION` treats empty as "none" (`crates/cloudflare/src/gate.rs:30-34`), contradicting
  `configuration.md:11` ("empty values count as unset everywhere"): use a literal `none`. And
  `RAFI_RETENTION_DAYS_MAX` vs `RAFI_RETENTION_WINNERS_DAYS` have the qualifier on different sides;
  `RAFI_RETENTION_MAX_DAYS` reads like the other.
- **Size**: medium (mechanical, many files). **Why now**: every operator's env file and runbook.

#### CFG-2 · Operator-specific values live in tracked files — **done**

- **Where**: `apps/worker/wrangler.toml:38` (`database_id` placeholder), `:79-89` (`RAFI_PUBLIC_URL`,
  `RESTATE_INGRESS_URL`, operator name and email, document versions); `apps/worker/legal/{terms,privacy}.md`
  (`include_str!` at `apps/worker/src/lib.rs:86-89`); `docs/operations/cloudflare.md:36,44-50` tells the operator
  to edit them.
- **Why**: every operator deploys from a dirty tree or a fork, and every upgrade is a merge against those files.
  A hand-maintained `wrangler.<site>.toml` would recreate the merge problem one file over. Wrangler's own
  mechanics (its configuration reference): `vars` are non-inheritable and the config's `[vars]` overwrite
  dashboard vars on deploy unless `keep_vars = true`; secrets are never deleted by a deploy; `[secrets] required
  = [...]` is validated in local development and on deploy; `assets.directory` and `migrations_dir` resolve
  relative to the config file; automatic D1 provisioning writes IDs back into the tracked file and has no
  `--jurisdiction`, so it is unusable for an EU database. The Worker reads vars and secrets through one `lookup`,
  so every operator value can be a secret. `include_str!(concat!(env!("RAFI_LEGAL_DIR"), …))` works under
  `worker-build`, but the path must be absolute, `env!` fails the build when unset, and without
  `cargo:rerun-if-changed` an edited document outside the tree is **not** rebuilt: a stale legal text would ship
  silently.
- **Change**: (1) the tracked `wrangler.toml` carries the production *shape* only (bindings, assets, crons, the
  Durable Object migration, rate limits, observability), `[secrets] required = [RAFI_PUBLIC_URL,
  RAFI_OPERATOR_NAME, RAFI_OPERATOR_EMAIL, RAFI_TERMS_VERSION, RAFI_PRIVACY_VERSION, RAFI_ENGINE_INGRESS_URL,
  RAFI_ENGINE_API_KEY, RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS, RAFI_ALTCHA_KEY, CLERK_SECRET_KEY]`, no operator
  `[vars]`, and `[env.dev]` as today; a deploy then fails closed on a missing value before the first request.
  (2) Operator values go in with `wrangler secret bulk site.json` (gitignored); the custom domain stays in the
  dashboard (no `routes` key, as today). (3) The one unavoidable per-site binding value, `database_id`, goes
  into a gitignored `wrangler.<site>.toml` that a script **generates** from the tracked file by substitution,
  never hand-edited, regenerated on every upgrade, in the same directory so relative paths hold; `wrangler deploy
  -c apps/worker/wrangler.<site>.toml`; `workers-check.sh` dry-runs the generator's output. (4) The legal
  documents: a `build.rs` reads `RAFI_LEGAL_DIR` (default `$CARGO_MANIFEST_DIR/legal`), canonicalises it, and
  emits `cargo:rustc-env` plus `rerun-if-env-changed` and `rerun-if-changed` for both files.
- **Size**: small. **Why now**: the first real deployment's config is the one that gets merged forever.

#### CFG-3 · The Worker's name — **done**

- **Where**: `apps/worker/wrangler.toml:14` (`name = "rafi"`); the crate is `rafi-worker`; `README.md:71`,
  `docs/reference/crates.md:56`, `glossary.md:76` call the deployable `rafi-worker`.
- **Change**: one story, e.g. "the Worker `rafi`, built from `apps/worker` (`rafi-worker`)", applied to the docs.
  `wrangler tail rafi` ends up in runbooks.
- **Size**: trivial.

#### CFG-4 · Numeric variables parse inconsistently — **done**

- **Where**: `RAFI_MAX_ENTRANTS`, `RAFI_RETENTION_*`, `RAFI_MIN_AGE` refuse a typo (`crates/server/src/config.rs:157-163`,
  through `misparsed`); `RESTATE_TIMEOUT_SECS` (`:185-187`), `RAFI_ALTCHA_COST` (`:194-196`) and
  `RAFI_DRAND_LEAD_SECS`/`RAFI_DRAND_TIMEOUT_SECS` (`crates/engine/src/drand.rs:75-79`) silently take the default.
  `configuration.md:45-47` promises the refusal for all.
- **Change**: route them all through `misparsed`; `DrandConfig::from_env` returns what it couldn't parse.
- **Size**: trivial.

### Fail-closed defaults and security

#### SEC-1 · Production is blind: no `tracing` subscriber on Workers — **done**

- **Where**: `apps/worker/src/lib.rs` installs none (only `apps/rafi/src/main.rs:14` and
  `apps/restate/src/main.rs:35` do); `[observability] enabled = false` (`wrangler.toml:73-74`). Every
  `tracing::error!` behind an admin 500 (`crates/server/src/admin.rs:60,64`, `public.rs:28-82`,
  `participant.rs:85,186`, `crates/app/src/entrants.rs:134,197`) and every housekeeping sub-step `warn!`
  (`crates/app/src/housekeeping.rs:73,96,111,146,155,161`, `owners.rs:29`, `raffles.rs:271`,
  `crates/engine-restate/src/service.rs:73,110`) is a no-op in production. `troubleshooting.md:59-61` points the
  operator at `wrangler tail`; it does show the three existing `console_error!` lines (`wrangler tail` is
  independent of `[observability]`), and nothing of `tracing`, because no subscriber is installed.
- **Change**: in `cached()`, once per isolate, a `tracing` layer writing to the console, installed with
  `try_init()` (ignore "already set"). `tracing-web 0.1`'s `MakeWebConsoleWriter` uses only `web_sys::console`
  and works on Workers (no `window` needed); its `performance_layer` does not (it asserts the Performance mark
  API, which workerd lacks); `.with_ansi(false).without_time()` are mandatory (`SystemTime::now()` panics on
  wasm32). A hand-rolled `MakeWriter` over `worker::console_*!` is the same size; either is fine. Filter with
  `LevelFilter::WARN`, not `EnvFilter` (its `env-filter` feature pulls regex into the module). Wrap the layer in
  `restate_sdk::filter::ReplayAwareFilter` as `apps/restate/src/main.rs:42` does: the engine's handlers replay on
  Workers too and would log every warning twice. The existing events carry raffle IDs and error text, no
  personal data. Then `[observability] enabled = true` with `[observability.logs] invocation_logs = false`
  (Cloudflare, Workers Logs: "Invocation logs can be disabled in wrangler by adding the `invocation_logs = false`
  configuration"; `console.*` lines are a separate feature and stay), which keeps the application's own lines
  (the hourly housekeeping report) and drops the per-request records with IPs that the comment objects to.
  Before writing it into `privacy-and-abuse.md`, check in the dashboard whether a custom log line still carries
  the invocation's `$workers.event.request` metadata (headers, IP); the documentation doesn't settle it, and if it
  does, `head_sampling_rate` and the line's content are the controls. `cloudflare.md` says that `wrangler tail`
  works with observability off and shows request metadata live (transient, operator-initiated).
- **Size**: small. **Why now**: the first live failure.

#### SEC-2 · A raffle that has drawn can be deleted, and the public record with it — **done**

- **Where**: `crates/lifecycle/src/raffle.rs:82-84` (`deletable` refuses only `Drawing` with a pending beacon);
  `crates/app/src/raffles.rs:261-274`; `ON DELETE CASCADE` on `prizes`, `entrants`, `raffle_events`, `draws`;
  `crates/lifecycle/src/lib.rs:111-117` (`unfreeze` is "not a command (organizers can't unfreeze)") and its only
  caller `crates/app/src/draw.rs:110-116` (the freeze that found nobody to draw from); `crates/raffle/src/raffle.rs:680-689`
  (`finish` is allowed with zero draws); `apps/web/src/ui/admin/mod.rs:218-241` (the raffle list has no filter or
  archive; "Data erased" raffles are listed forever).
- **Why**: freeze, publish the manifest hash on `/v/<slug>`, draw, dislike the winner, delete, duplicate, draw
  again: `/v/<slug>` and `verify.json` are 404, the `Abandoned`/`Drawn` evidence is gone, only the operator's audit
  row survives. `remove_my_data` deliberately keeps frozen raffles verifiable (`crates/store/src/raffles.rs:445-531`);
  `delete_raffle` contradicts it and the "live, verifiable" promise. Deleting a frozen raffle also never tells
  the engine, so its Restate state is orphaned (API-2). **But the trigger must be the first draw, not the
  manifest**: the attack needs a draw to have happened, while an accidental freeze (the wrong raffle, a rehearsal
  without test mode, three entrants) has **no exit other than Delete**: `unfreeze` is not reachable by an
  organizer, so "no delete once a manifest exists" would leave Finish (with zero draws) then Purge as the only
  way out, two irreversible steps, a "Data erased" raffle in the list forever and a `/v/<slug>` showing a hash, an
  empty name map and a lone `Finished` event. Deleting a frozen raffle before any draw is harmless to
  verifiability: its entrants go with it, so a "fixed" list means the whole room re-registers under a new code;
  nothing can be swapped silently.
- **Change**: `deletable` (which already receives `Option<&Raffle>`) refuses once **any draw has been announced**
  (`raffle.events()` contains an `Announced`: the raffle is `Drawing`, or `Finished` with at least one draw);
  the refusal says "This raffle has drawn prizes; its record stays verifiable. Finish it, then erase personal
  data." A frozen raffle with no draw stays deletable: the way out of an accidental freeze. `delete` of a frozen
  raffle also asks the engine to forget it (or SEC-2 documents the orphan). The admin list groups or hides "Data
  erased" raffles (a toggle, hidden by default); without it, "purge instead of delete" makes the list grow for
  good. `running-a-raffle.md` and `privacy-and-abuse.md` say so.
- **Size**: small. **Why now**: organizers will have learned the other behaviour.

#### SEC-3 · `RESTATE_API_KEY` is not fail-closed — **done**

- **Where**: `crates/server/src/config.rs:184` (`unwrap_or_default()`); `crates/engine-restate/src/client.rs:85`
  skips auth on empty; `apps/worker/src/lib.rs:26-27` and `cloudflare.md:71-72` claim every secret fails closed.
- **Change**: `anyhow::bail!` in the Worker's `app()` when empty and not a `dev` build; `configuration.md:29`
  says "required on Workers".
- **Size**: trivial.

#### SEC-4 · Native dev auth has no build gate — **done**

- **Where**: `crates/server/src/config.rs:164` (`flag("RAFI_DEV_AUTH")`), `:242-244` (`validate` returns early, so
  no operator identity or documents are required either); `apps/rafi/src/main.rs:26-27,39-43` (warn, then no Clerk
  layer). The Worker bails without `--features dev` (`apps/worker/src/lib.rs:90-93`).
- **Why**: `devenv.nix:194` sets `RAFI_DEV_AUTH=1`; copied to a public host, every `POST /api/admin/*` runs as
  owner `dev` with `Access::Admin`.
- **Change**: mirror the Worker: a `dev` feature on `rafi` gates `RAFI_DEV_AUTH`; devenv builds with it.
- **Size**: small.

#### SEC-5 · The native engine fails open, and the ingress isn't on the "never expose" list — **done**

- **Where**: `rafi-restate` listens on `0.0.0.0:9080` (`restate-config`'s `EndpointConfig::default`) and runs
  without identity keys with a warning (`apps/restate/src/main.rs:48-52`); `self-hosting.md:139` names `:9080` and
  `:9070` but not the ingress `:8080`.
- **Why**: a client reaching `:9080` that speaks the Restate protocol *supplies the object's state*; `project()`
  (`crates/engine-restate/src/service.rs:67-88`) and `save_raffle` (`crates/store/src/lib.rs:121-152`) then append
  whatever events it is handed after checking only the manifest hash. A reachable ingress lets anyone
  `POST /Raffle/<public raffle id>/decide` or `/draw`, bypassing `owned()`.
- **Change**: default `endpoint.listener` to `127.0.0.1:9080`; refuse to start with empty `identity_keys` unless
  `endpoint.insecure = true`; add the ingress to the line in `self-hosting.md`.
- **Size**: trivial.

#### SEC-6 · No security headers anywhere — **done**

- **Where**: no `Content-Security-Policy`, `X-Frame-Options`, `X-Content-Type-Options`, `Referrer-Policy` or
  `Strict-Transport-Security` in `crates/server`, `apps/worker` or `crates/cloudflare`; `crates/server/src/lib.rs:126-132`
  has no header layer; the SPA shell is served by Workers Assets with no `_headers`.
- **Why**: clickjacking of `/admin/r/<id>` (raffle IDs are public in `/api/r/<slug>/view`) is blocked only by
  Clerk's cookie attribute, a third party's default. The join URL contains the code, so a click on the organizer's
  `privacy_url` leaks it in the referrer. Whether `/s/<slug>` and `/v/<slug>` may be framed has to be decided
  before anyone embeds them.
- **Change**: `SetResponseHeaderLayer` in `routes()`: `X-Content-Type-Options: nosniff`, `Referrer-Policy:
  strict-origin-when-cross-origin`, `Content-Security-Policy: frame-ancestors 'none'` (and `X-Frame-Options:
  DENY`) on `/admin`, `/j/*`, `/api/*`; a `_headers` file for the SPA shell; HSTS at the edge, documented. Decision
  proposed: the projector and the verification page may be framed (`frame-ancestors *`), nothing else. Admin API
  error responses get `Cache-Control: no-store` like the successes (`crates/server/src/admin.rs:45,71-73`). A full
  CSP is in §Deferred.
- **Size**: small.

#### SEC-7 · Clerk: no authorized party; admin API: no content-type check — **done**

- **Where**: `apps/worker/src/lib.rs:106` (`ClerkAuthLayerConfig::new(key)`), `apps/rafi/src/main.rs:42`
  (`from_env()`), never `add_authorized_party`; `crates/server/src/admin.rs:35-48` reads raw `Bytes` with no
  `Content-Type`/`Sec-Fetch-Site`/`Origin` check; the SPA sends the session as the `__session` cookie
  (`apps/web/src/api.rs:44-57`), and `Args` structs don't `deny_unknown_fields`, so a `<form enctype=text/plain>`
  body parses.
- **Why**: CSRF on the admin API rests entirely on Clerk's `SameSite=Lax`.
- **Change**: `.add_authorized_party(cfg.public_url)` on both targets; 415 unless `Content-Type:
  application/json` (the SPA always sends it). Later: `Authorization: Bearer` from Clerk.js.
- **Size**: trivial.

#### SEC-8 · `RAFI_ALTCHA_KEY` has no minimum strength — **done**

- **Where**: `crates/server/src/config.rs:170-171`; `altcha-server` refuses only an empty key.
- **Why**: challenges are public (`/api/altcha`) and HMAC-signed, so a short key is crackable offline and the
  proof-of-work is forged. The rate limits and the entrant cap still hold.
- **Change**: `validate()` requires ≥ 32 characters outside dev; `configuration.md` says so.
- **Size**: trivial.

#### SEC-9 · The rate-limit key carries the plaintext email into the platform's limiter — **done**

- **Where**: `crates/app/src/entrants.rs:117-122` builds `"{raffle_id}:{normalized email}"`;
  `apps/worker/src/ratelimit.rs:35` hands it to the Cloudflare binding.
- **Change**: SHA-256 the key in `entrants.rs`, so native and Workers both get it; the key only needs stability.
- **Size**: trivial.

#### SEC-10 · Display names admit bidi, zero-width and filler characters — **done**

- **Where**: `crates/lifecycle/src/entrant.rs:66-77` rejects `is_control` (Cc) only and collapses whitespace with
  `split_whitespace()` (so U+2028/2029, `Zl`/`Zp`, are already handled). U+202E (RLO), U+2066–2069, U+200B/200D,
  U+FEFF, U+00AD (all `Cf`) pass. So do the Hangul fillers U+115F, U+1160, U+3164, U+FFA0, which are **`Lo` and
  `is_alphanumeric()`**, so neither "reject `Cf`" nor "require an alphanumeric" would catch a name made of them;
  and U+2800 (Braille blank, `So`). `rafi-lifecycle` depends on `rafi-raffle` and `serde` only; it is compiled
  into the Worker and the admin SPA but **not** into the phone module (`apps/participant/Cargo.toml`).
- **Why**: visually empty names, "Anna" vs "Anna\u200B" on the projector and the verification page, a reversed
  "name → prize" line, a blocklist bypass (`fu\u200Bck`).
- **Change**: reject `Cc` (already) and every `Default_Ignorable_Code_Point` with a hand-written range table in
  `entrant.rs` (about twenty stable ranges: U+00AD, 034F, 061C, 115F–1160, 17B4–17B5, 180B–180F, 200B–200F,
  202A–202E, 2060–206F, 3164, FE00–FE0F, FEFF, FFA0, FFF0–FFF8, 1BCA0–1BCA3, 1D173–1D17A, E0000–E0FFF), which
  covers the bidi and zero-width set **and** the fillers; add U+2800; require `chars().any(char::is_alphanumeric)`.
  No NFKC and no new dependency: once ignorable code points are rejected outright the `fu\u200Bck` bypass is gone,
  and folding fullwidth letters for a 22-word best-effort blocklist that "f u c k" already defeats isn't worth
  `unicode-normalization` on wasm32. Apply the same predicate in `settings::title` (`settings.rs:301`) for prize
  and organizer names.
- **Size**: small. **Why now**: changes what the projector accepts; easier before real data.

#### SEC-11 · Smaller hardening — **done**

- `crates/cloudflare/src/gate.rs:94-110`: the socket gate reads D1 (`live::snapshot`) for any `raffle_id` before
  rate limiting, and the limiter key is the attacker-chosen ID. Shape-check the ID (32 hex) and limit first.
  `gate.rs:106-108`: a limiter error propagates with `?` and the browser sees 1006 and retries; treat it like
  `ratelimit.rs:37-40` (allow and log). (trivial)
- `crates/engine-restate/src/service.rs:184-189` writes `format!("draw failed: {e}")` into the raffle, and
  `crates/app/src/view.rs:141-150` surfaces it as `last_failure` in the public `/view`. Map to a fixed public
  phrase; keep the detail in logs. (trivial)
- No cap on prizes (`crates/app/src/prizes.rs`; quantity ≤ 500 each); `insert_manual` and `insert_fakes`
  (`crates/store/src/entrants.rs:204-227,242-281`) don't check `max_entrants`. Cap total items in `start::ready`
  and apply the entrant cap in every insert. (small)
- Operator Markdown (`crates/server/src/pages/text.rs:313-324`) escapes raw HTML but passes `javascript:` hrefs;
  operator-trusted, still a few lines to filter `Event::Start(Tag::Link)` to `https`, `http`, `mailto`, `#`.
  (trivial)
- The cookie `rafi_e_<raffle_id>` has `Path=/` (set at `crates/server/src/participant.rs:240-243`). It is read in
  exactly one function, `cookie()` (`participant.rs:58-67`), reached only from `page` and `submit` on `/j/{code}`;
  `/api/r/<slug>/participant` reads no headers, the phone's claim code comes from `#rafi-config` in the page, and
  nothing in `crates/cloudflare`, `apps/worker` or the live client reads cookies. `Path=/j/` keeps it off every
  other request and out of any proxy log, **and off the Durable Object**: today the socket gate forwards the
  browser's headers verbatim to the `RaffleRoom` (`crates/cloudflare/src/gate.rs:112-118`), so the entry token
  rides along on `GET /partyline/raffle/{id}`. Old `Path=/` cookies keep matching; with both present the
  longer path is sent first, so the new one wins. (trivial)
- The email-membership oracle ("This email is already entered", `crates/app/src/entrants.rs:193-195,219-224`,
  `pages/participant.rs:259-262`) is **structural, not a matter of copy**: a successful join increments the
  public `entrant_count` and, after five seconds, adds the name to `recent_names` in `/api/r/<slug>/view`
  (`crates/app/src/view.rs:50-56,176-183`), so anyone with the join code can read the count, post a probe, and
  read it again. A non-committal message would protect nobody. Bounded by the join code, one proof-of-work per
  probe, 5/min/email, and only while registration is open; for someone in the room the fact is visible anyway.
  `privacy-and-abuse.md:101` and §Accepted risks say why. (trivial, docs)

### Privacy and retention: what the audit found incomplete

#### PRV-1 · The generated notice is wrong about hosting — **done**

- **Where**: `crates/server/src/pages/text.rs:259`: "… hosting, the raffle engine, organizer sign-in and the public
  randomness network, none of which receives your email."
- **Why**: the hosting provider stores the database.
- **Change**: "… the raffle engine, organizer sign-in and the public randomness network receive no personal data;
  the hosting provider stores the raffle's data on the operator's behalf." No consent version bump: the notice is
  shown, not accepted.
- **Size**: trivial.

#### PRV-2 · Portability, third-country transfers, residue and backups are missing from the notice — **done**

- **Where**: `crates/server/src/pages/text.rs:206-288`, checked line by line against Art. 13. Present: controller
  and contact (`:218-228`), purposes with their legal bases 6(1)(b) and 6(1)(f) (`:233-234`), the legitimate
  interest named (`:234`), recipients (`:238-239,259`), retention (`:245-254`), the right to complain to a
  supervisory authority (`:267`), and that the email is required to enter (`:235`, thin). Missing: **13(1)(f),
  third-country transfers**, which applies (Cloudflare, Inc. and Clerk are US processors; EU-pinned D1 is a
  storage location, not a transfer waiver; the operator template has the DPF/SCC paragraph at
  `apps/worker/legal/privacy.md:35-37` but the entrant is never pointed to it for this item); **13(2)(b)
  portability** (`:267`); the residue and the backup caveat the plan's own §Further notes says go in the notice.
  Not required but worth one line: the draw is automated (Art. 22 is the question every reader has), and a free
  meetup prize is not a decision with "legal or similarly significant effects".
- **Why**: Art. 13 is what the legal review before the hosted launch checks the generated notice against; these
  are the items it will find.
- **Change**:
  - `:259` (with PRV-1): "{op} ({email}) runs this service and processes the raffle's data on the organizer's
    behalf, as its processor, under a data processing agreement. The raffle engine, organizer sign-in and the
    public randomness network receive no personal data; the hosting provider stores the raffle's data on the
    operator's behalf. Where a provider is outside the EU/EEA, the service's privacy notice (version {v}) names
    it and the safeguard that covers the transfer (an adequacy decision, the Data Privacy Framework or standard
    contractual clauses) and how to obtain a copy." The generated notice can't hard-code jurisdictions (every
    self-hoster differs); pointing at the operator's document is 13(1)(f)'s "where they have been made available"
    branch. Add to the template how to obtain a copy of the clauses.
  - `:267`: "… restriction, or object to processing, and to receive the data you gave us in a portable form, by
    contacting the controller …"
  - after `:254`: "After erasure, the anonymous entry keeps when it joined, how it joined, whether a prize was
    delivered and whether its name was hidden; backups and the hosting provider's point-in-time recovery may hold
    erased data a while longer (see the service's notice)."
  - `:235`: "Your email address is required to enter: without it you can't take part. A display name is optional."
  - optional, under "What we collect and why": "The draw is automated and random, from public randomness anyone
    can check; it makes no decision with legal or similarly significant effects on you."
- **Size**: trivial.

#### PRV-3 · Joins are accepted after the erasure date on an already-open raffle — **done**

- **Where**: `crates/store/src/entrants.rs:155-159` (the INSERT guards on `state` only);
  `crates/lifecycle/src/entrant.rs:278-286` (`may_join`); `Command::Open` has the guard
  (`crates/app/src/raffles.rs:365-368`); `crates/lifecycle/src/settings.rs:317-319` (`retention_days = 0` is
  allowed, so `erase_at` can be the local midnight that ends the event day); `apps/web/src/ui/admin/mod.rs:509-513`
  (the admin's warning names the escapes: move the date until someone joins, or duplicate).
- **Why**: for up to an hour, until `expire_due` runs, an entrant sees "erased on <a past date>", joins, and is
  deleted. Refuse-then-expire is the right order: expiry *deletes* entrants, so letting housekeeping act at once
  would be worse than an hour's grace in which the organizer can still freeze (a frozen raffle doesn't expire).
  The lock is real and stays: date, period and `erase_at` are fixed by one guarded `UPDATE` once a real entrant
  exists, and mid-event the only escape is a duplicate under a new code. The one live trap: with a period of 0
  (or 1 and yesterday's date), an event that runs past midnight loses its entrants within the hour if unfrozen,
  and if frozen but not yet drawn, everyone is a non-winner at the hourly erase and the draw proceeds on nameless
  IDs. PRV-3 doesn't create that trap; it makes it visible ("Registration is closed" on phones at midnight).
- **Change**: `AND erase_at > ?now` in the INSERT; `JoinRefusal::Closed` from `may_join` given `past_erase_date`
  ("Registration is closed.", no new copy). Name the trap: when `Open` is pressed with less than 24 h to
  `erase_at`, the admin says "Erasure is at the end of today ({local date}): freeze and finish before midnight",
  and the settings form warns when the period is 0. Don't touch the lock.
- **Size**: trivial.

#### PRV-4 · The restore tool relies on the `erasures` table being restored too — **done**

- **Where**: `crates/store/src/retention.rs:195-211` (`reapply_purges` flips `state` only);
  `docs/operations/cloudflare.md:101-108` (the hand-written D1 SQL).
- **Why**: a raffle purged by hand before its date, restored from a backup without the `erasures` rows, keeps
  personal data until `erase_at`.
- **Change**: `reapply_purges` also runs `UPDATE entrants SET {ERASED} WHERE erased = 0 AND raffle_id IN (SELECT id
  FROM raffles WHERE purged_at IS NOT NULL OR organizer_left_at IS NOT NULL)` and logs each to `erasures`; mirror
  it in the D1 SQL. A purged raffle has nobody un-erased by definition, so it is idempotent.
- **Size**: small.

#### PRV-5 · A frozen raffle without a manifest never leaves limbo — **done**

- **Where**: `crates/lifecycle/src/lib.rs:123-125` (`expires` = `!frozen_or_later`);
  `crates/store/src/retention.rs:85-90` (`at_ceiling` = `Finished` only); `crates/app/src/housekeeping.rs:92-115`.
- **Why**: an interrupted freeze the organizer abandons is erased at `erase_at` (fine) but stays `Frozen` forever
  and `finish_at_ceiling` nudges the engine about it every hour.
- **Change**: `state = 'frozen' AND manifest IS NULL` expires like a never-frozen raffle; a drawable raffle the
  engine refuses to finish is purged at the ceiling.
- **Size**: small.

#### PRV-6 · Audit gaps and a misleading refusal — **done**

- Settings changes aren't audited (`crates/app/src/raffles.rs:181-239` audits only `remove_fakes`): the
  controller's name and contacts can change silently. Record `settings` with the changed field *names*. (trivial)
- Housekeeping's engine `Finish` (`crates/app/src/housekeeping.rs:103-113`) writes no audit row. Record
  `auto_finish`, actor `housekeeping`. (trivial)
- `crates/app/src/error.rs:261-263` (`NotFinishedForDelivery`): "Prizes can be marked delivered once the raffle has
  finished" is false for the per-entrant action, which works from `claimed` on; only "All prizes delivered" is
  gated. Folded into API-5's refusal cleanup. (trivial)
- Individual "Mark delivered" is audited as a generic `patch_entrant` (`entrants.rs:348`); name it. (trivial)

#### PRV-7 · Docs that under-report — **done**

- `privacy-and-abuse.md:62-64`: the residue list omits `consent_version`.
- `privacy-and-abuse.md:84-87` ("Leaving"): the account identifier stays on raffles and in the audit log (the
  template `apps/worker/legal/privacy.md:24-26` says it; the explanation page doesn't).
- `privacy-and-abuse.md:43`: registration can't *open* after the date; say an already-open raffle stops taking
  joins (PRV-3) and expires within the hour.
- `configuration.md:38-39`: `RAFI_RETENTION_WINNERS_DAYS` is capped at 365 and `RAFI_MIN_AGE` at 30
  (`crates/server/src/config.rs:239,323`, `crates/lifecycle/src/settings.rs:61-66`); neither cap is documented.
- `apps/worker/legal/privacy.md:37` says "Workers and D1 in the EU jurisdiction"; only D1 and the Durable Objects
  are pinned, the Worker runs where the user is. Fix the template.
- `pages/participant.rs:261` "the organizer will confirm you by email" and `apps/worker/legal/terms.md:28` "we
  may remind you" promise mail nothing sends. Soften.
- Self-hosting: the reverse proxy's access logs hold client IPs (and cookies, unless told not to); the app logs
  neither. Say so in `self-hosting.md`.
- Offboarding blanks five fields (`crates/store/src/raffles.rs:492-496`) but not `title`, `description`, `rules`
  or a prize's `sponsor`, which may name people. Document that they stay, or blank `description` and `rules` too.

### Build, CI and the production path

#### CI-1 · The production build and configuration are never exercised

- **Where**: `scripts/workers-check.sh:84` and `.github/workflows/ci.yaml:89-106` build the Worker only with
  `--features dev`; nothing runs `wrangler deploy --dry-run`; nothing fires the `scheduled` handler
  (`apps/worker/src/lib.rs:169-188`); `scripts/app-assets.sh:14` bakes whatever `CLERK_PUBLISHABLE_KEY` is in the
  environment, and an empty one compiles an admin with no sign-in (`apps/web/src/ui/mod.rs:42-45,245`).
- **Why**: the first time `[assets]`, `[[migrations]]`, `[[ratelimits]]` and the fail-closed path are validated by
  wrangler is the real deploy; the cron is what keeps the "erased on <date>" promise and is never run; a forgotten
  export ships an admin whose every call is 401 with no sign-in button.
- **Change**: `workers-check.sh` gains: `worker-build --release` + `wrangler deploy --dry-run --outdir` of the
  top-level config; the top-level env under `wrangler dev` asserting `/healthz` → 500 `misconfigured`; a
  `GET /__scheduled?cron=0+*+*+*+*` in the dev run asserting `whoami.retention_stale === false` afterwards; a gzip
  size budget for the Worker like `participant.sh:45-49`. `app-assets.sh` refuses an empty key unless `--dev` is
  passed and prints the mode it built.
- **Size**: small.

#### CI-2 · `cargo deny` is red on `main`

- **Where**: RUSTSEC-2026-0381/0383 (webpki-roots, webpki-root-certs), 0382 (rustls-webpki), 0385 (ctutils), 0386
  (tokio, unsound), 0387 (crypto-bigint 0.5.5 and 0.7.5), 0391/0398/0403 (zstd, zstd-safe), 0410 (zstd-sys);
  crypto-bigint 0.7.5 is yanked.
- **Change**: `cargo update` fixes all but two; scoped ignores with reasons for crypto-bigint 0.5.5 (via
  `jsonwebtoken 10` ← `restate-sdk-shared-core`, upstream) and zstd 0.13 / zstd-safe 7 (via
  `liboliphaunt-native-bindings` ← `oliphaunt`, test-only, visible because `deny.toml` sets `all-features`).
- **Size**: small.

#### CI-3 · The MSRV is false

- **Where**: `Cargo.toml:9` `rust-version = "1.92"`; `sqlx 0.9` declares `1.94.0`, `oliphaunt` `1.93`. With
  `resolver = "3"` (MSRV-aware) `cargo update` already refuses to move those crates.
- **Change**: `1.94`.
- **Size**: trivial.

#### CI-4 · A flaky Postgres conformance assertion — **done** (with SCH-6)

- **Where**: `crates/store/src/conformance/domain.rs:1965` (`left.purged == ["r1", "r3"]`) depends on
  `rows_by_owner`'s `ORDER BY created_at DESC` (`crates/store/src/raffles.rs:227`); `created_at` is unix seconds
  in both dialects, the scenario inserts r1…r4 within one second (`:115-131,1876-1935`), and `force_state` then
  updates r1 and r3 (`:1890,1920`), which on Postgres moves heap tuples, so ties come back in arbitrary order.
  Failed in CI run 40262291679 and is why Dependabot #19 is red.
- **Change**: `, id DESC` as a tiebreak in the query (also SCH-6: an index `(owner_id, created_at, id)` scanned
  backwards yields exactly that), and sort before comparing, as `expired` does at `:1657`.
- **Size**: trivial.

#### CI-5 · Pinning and small CI gaps

- `Cargo.toml:44-46`: `restate-sdk` is patched by `branch = "wasm"`; `worker-build` doesn't pass `--locked`. Pin
  `rev =`; keep tracking restatedev/sdk-rust#128. (trivial)
- `.github/workflows/ci.yaml:115-116`: `actions/checkout@v7` and `EmbarkStudios/cargo-deny-action@v2` are the only
  unpinned actions. Pin by SHA. (trivial)
- `ci.yaml:70-75`: clippy skips `rafi-restate --features mock-drand`. Add it. (trivial)
- `crates/app/Cargo.toml:14`: `getrandom 0.3` → `0.4` (uuid already uses 0.4; Dependabot #19). Check the wasm
  backend feature name. (small)
- No deploy automation: nothing applies remote D1 migrations or deploys. Either a `workflow_dispatch` deploy
  (migrations → `wrangler deploy` → remind to re-register with Restate) or an explicit statement that deploys are
  manual with CI-1's dry-run as the gate. (small)
- `scheduled` (`apps/worker/src/lib.rs:171-176`) returns normally when `cached()` fails, so the Cron Trigger
  history shows green; after SEC-1 the error is at least visible. `finish_at_ceiling`
  (`crates/app/src/housekeeping.rs:92-115`) loops serially with a 90 s timer per `decide`; cap the count per run
  so a few stuck raffles can't push the cron past its wall clock. (trivial)

#### CI-6 · To verify on the first real deploy (can't be checked locally)

- Outbound `fetch` from the Worker to Restate Cloud's ingress on `:8080` (`crates/engine-restate/src/client.rs:38-40`);
  workerd allows any port, production Cloudflare has port rules for proxied hosts.
- `wrangler deploy`'s reported startup time for a 4.85 MB module (400 ms limit).
- `ns.jurisdiction("eu")` (`crates/cloudflare/src/gate.rs:43-47`) is reached only in production; the test-mode
  raffle exercises it.
- Restate Cloud → Worker over HTTP/2 (locally HTTP/1.1 only).
- A zone-level Bot Fight Mode or WAF rule challenging `POST /restate/*`; add a skip rule for the path.
- The paid plan is a certainty, not a "may be needed" (`cloudflare.md:21`): BLS verification and server-side
  rendering won't fit 10 ms of CPU.

### Cleanup: dead code, stale references, hygiene

All trivial unless marked. The sweep found no `TODO`/`FIXME`, no commented-out code, no unused dependency, no
declared-but-unused feature, no residue of the removed marketing opt-in, `RAFI_RETENTION` flag,
`apps/restate-worker`, `script_name` bindings, `/prepare`, `rafi:d1-migrations`, `scripts/restate-server.sh` or
the `room` feature; `cargo build` and `cargo clippy` are warning-free natively and on wasm32.

| ID | What | Where | Change |
|---|---|---|---|
| CLN-1 | `cargo-watch` package, unused (devenv implements `watch` itself) | `devenv.nix:96` | remove |
| CLN-2 | Empty `[dependencies]` | `apps/worker/Cargo.toml:21` | remove |
| CLN-3 | No-op test tail `let c = select(…); let _ = c;` | `crates/raffle/src/algorithm.rs:163-166` | `assert_ne!` or drop |
| CLN-4 | YAML config pulls the deprecated `serde_yaml` and the unmaintained `unsafe-libyaml` (the RUSTSEC-2024-0449 ignore); no `.yaml`/`.yml` is passed to `rafi-restate` anywhere (devenv uses env vars, the docs and tests use TOML) | `apps/restate/Cargo.toml:24,40` (both `figment` entries carry `"yaml"`), `src/config.rs:1,87-88` (and the error string), `src/main.rs:26`, `README.md:29`, `configuration.md:80`, `deny.toml:42-46` | drop the `yaml` feature; TOML and JSON stay; remove the ignore (small) |
| CLN-5 | `tokio` `features = ["full"]` for `spawn`, `time::interval` and `#[tokio::test]` | `crates/server/Cargo.toml:38` | `["rt", "rt-multi-thread", "time", "macros"]` |
| CLN-6 | `publish = false` missing (their path deps carry no `version`, so they couldn't publish anyway) | `crates/lifecycle/Cargo.toml`, `crates/raffle/Cargo.toml` | add |
| CLN-7 | README example `DrandClient::new(DrandConfig::default())?` fails at runtime since 70a9a00 (`new` is mock-only) | `crates/engine-restate/README.md:37` | `DrandClient::builder(cfg).http_transport(ReqwestTransport::default()).build()?` |
| CLN-8 | "Handed over" residue | `crates/server/src/tests.rs:1551,1554`, `apps/web/src/ui/admin/mod.rs:520` | rename the test; "Everything delivered?" |
| CLN-9 | "draw engine" / "drawing engine" (glossary: raffle engine) | `apps/restate/Cargo.toml:7`, `crates/store/Cargo.toml:7`, `testbeds/cloudflare/scenario.mjs:3` | reword |
| CLN-10 | `remove_my_data` doc says "Draft raffles are deleted"; it deletes every never-frozen raffle | `crates/api/src/admin.rs:82` | fix |
| CLN-11 | "(see README)" pointers to content now in `docs/development/getting-started.md` | `apps/worker/wrangler.toml:95`, `apps/web/Dioxus.toml:13` | repoint |
| CLN-12 | `dx serve` proxy list lacks `/terms` (linked from the admin) and `/healthz` | `apps/web/Dioxus.toml:15-28` | add both |
| CLN-13 | Routes reference omits the Worker's `/restate`, `/restate/*` | `docs/reference/routes.md:51-55` | add a row under "Workers only" |
| CLN-14 | Crates reference: `rafi-engine`'s feature list omits `mock-drand`; `docs/plans` missing from "Everything else" | `docs/reference/crates.md:41,61-73` | add |
| CLN-15 | `privacy-and-retention.md` is implemented but still under `docs/plans` | `README.md:115-116`, `docs/plans/marketing-opt-in.md:3-5`, `docs/history/retention.md:6-7` | move to `docs/history/` with the Historical banner, keep its Follow-ups, repoint the three links (small) |
| CLN-16 | Status text "not deployed yet", "no packaged images" | `README.md:81-87`, `docs/operations/overview.md:63`, `docs/operations/cloudflare.md:4` | decide at tag time; update together |
| CLN-17 | Dangling `[vars]` comment describing retention variables that aren't listed | `apps/worker/wrangler.toml:89-93` | list them commented out, or trim |
| CLN-18 | Doc link names `rafi_app::housekeeping::run`; the code calls `server::service::housekeeping::run` | `apps/worker/src/lib.rs:15` vs `:178` | align |
| CLN-19 | "attendee's cookie" in SQL comments (glossary: entrant) | `migrations/sqlite/0001_init.sql:85`, `migrations/postgres/0001_init.sql:87` | reword |
| CLN-20 | D1 testbed: package `rafi-store-d1-check`, database `rafi-store-check` | `testbeds/store-d1/wrangler.toml:14`, `scripts/workers-check.sh:66` | align |
| CLN-21 | CSV export has no UTF-8 BOM (Excel on Windows mangles accented names); the formula guard rewrites a name starting with `-` | `crates/app/src/reads.rs:151-181` | prepend `\u{FEFF}`; document the format in `routes.md` |
| CLN-22 | `rafi_store::legal::list` has no production caller | `crates/store/src/legal.rs:64` | goes with SCH-3 |
| CLN-23 | Local artefacts: `testbeds/cloudflare/site/attendee/` (ignored, untracked); stale Dependabot PR #11 (old `/crates/web` path) | local, GitHub | `rm -rf`; close |
| CLN-24 | No `CHANGELOG.md` | repo root | add before the first tag (small) |

## Order of work

1. **Schema and the removal** (SCH-1 … SCH-6), while `0001_init.sql` is still the only migration. SCH-3 first:
   it deletes code the others would otherwise touch. **Done 2026-10-26**: SQLite, embedded Postgres and D1
   conformance, the HTTP suite, the Restate end-to-end test and the Workers evening all pass. CI-4 (the flaky
   assertion) went with SCH-6.
2. **Contracts** (API-1 … API-7): the forward-compatibility rule and the Python verifier's `else`, the engine
   docs and the event-shape fixture test, the phone enums, `verify.json`, the admin API names and error code, the
   verifier's canonical check, the participant caching and compression. **Done 2026-10-27.** Two corrections
   found while doing it: tower-http 0.7 (the version `ServeDir` runs on) does send an `ETag`, so natively only
   `Cache-Control` and compression were missing; `remove_entrant` keeps its name, since "remove" (deleted before
   the freeze, erased after it) is the right word and `delete` would be wrong half the time.
3. **Names** (CFG-1 … CFG-4): one commit, every file, with the docs. **Done 2026-10-27**; `wrangler deploy
   --dry-run` of the production shape passes.
4. **Fail-closed and security** (SEC-1 … SEC-11): the subscriber first, so the rest is observable; then the
   delete rule, the secrets, the headers, the names. **Done 2026-10-27.** Two things found while doing it: the
   SDK's `EndpointConfig` has no `insecure` flag, so the native engine has its own `EndpointSettings` section
   (loopback default, keys or an explicit `insecure`); and a stalled draw's reason is in the *public* event log,
   so the fixed phrases are written by the engines, not mapped at `/view`.
5. **Privacy** (PRV-1 … PRV-7): wording, the join guard, the restore tool, limbo, the audit rows, the docs.
   **Done 2026-10-27.** The join guard made the HTTP tests' clock technique (backdate through the settings API,
   then join) impossible by design, which is the point; the one test that needed it now joins first and shifts
   the clock in the database, as the conformance suite does.
6. **CI** (CI-1 … CI-5): so what was fixed stays fixed; `cargo deny`, the MSRV and the flaky test can go in any
   earlier step's PR.
7. **Cleanup** (CLN-*) as one sweep, and the status texts at tag time.

Each step ends with `cargo test --workspace`, `cargo clippy` on both targets, and `scripts/workers-check.sh`.

## Accepted risks

Raised by the review, considered, and not done, with the reasoning. Each was challenged a second time; where the
challenge changed the reasoning, the corrected reasoning is here.

- **No `schema_version` row, no migration check.** A rolled-back Worker against a newer D1, a Worker deployed
  before its D1 migration was applied, or the lazy-connecting native engine against an older schema, isn't
  detected by the code. What happens: the first query against a missing column fails, so that request is a 500,
  the cron logs an error (visible in `wrangler tail`, stored only if SEC-1's observability is on) and the engine's
  `save_raffle` fails until Restate's retries meet the migrated schema. `/healthz` is a static `ok` and does not
  catch it, although `cloudflare.md` and CI-1 use it as the post-deploy check. The order of operations in
  `upgrades.md` (migrate, deploy, never roll a Worker back past a migration; natively the app before the engine)
  is the safeguard, and sqlx's refusal to run on a database with unknown applied migrations protects the native
  app. Wrangler's `d1_migrations` and sqlx's `_sqlx_migrations` already record what was applied, so a
  `meta('schema_version')` row would duplicate them; a `/healthz` comparison of the newest applied migration with
  the newest compiled-in one (one read per probe, no new table) was offered and declined. Revisit when the second
  migration ships.
- **Legal acceptance is by use, with no record.** Removing the acceptance record means the processing agreement
  binds through the terms' "by using" clause, not a click, and nothing records which version was in force when an
  organizer created a raffle. What the EDPB says (Guidelines 07/2020 on controller and processor, v2.0): Art.
  28(9) demands the agreement "in writing, including in electronic form", which a published document is; §101
  recommends signatures to avoid "difficulties in demonstrating that the contract … is actually in force", a
  recommendation about evidence, not validity; §110 explicitly contemplates a processing agreement inside
  standard terms accepted by making use of the service, **and** says that "any proposed modification, by a
  processor, of data processing agreements included in standard terms and conditions should be directly notified
  to and approved by the controller … The mere publication of these modifications on the processor's website is
  not compliant with Article 28." So: defensible for the initial agreement, not for changes to it. Consequences
  recorded here: (1) the terms template's change clause says that changes to the processing agreement are
  notified directly and need the organizer's approval, and that other changes take effect when published with a
  new version; the acceptance gate returns before the first such change; (2) any `Manager` who presses Create
  binds the organization, where the gate let only an `Admin` accept, which the legal review should see named;
  (3) a passive record (a `terms_in_force` audit row with version and digest at creation, no schema, no gate, and
  a line under the Create button) was offered and declined, so the interim has no evidence trail and can't be
  backfilled when the feature "comes back whole". The legal review before the hosted launch
  (privacy-and-retention.md, §Further notes) decides whether this holds for v1.
- **Join codes are enumerable.** 31⁶ codes, no per-IP limit by design. A hit yields the raffle's public pages
  **before any proof-of-work**: the join page, and through its slug the rules (organizer name and contact, prizes
  and sponsors), the privacy notice (the privacy contact) and `/api/r/<slug>/view` (the entrant count and the
  last 20 display names). Joining still costs a proof-of-work and an email. With SCH-4's filter, purged raffles
  don't resolve, so the discoverable set is the raffles inside their hold period (about 100 days), not every
  raffle ever. Edge protection is the platform's. Lengthen in code if an installation ever runs into thousands of
  raffles per hold period.
- **A draw in flight does not survive a deploy; a changed engine is not refused.** On Workers the old code is
  replaced; the deploy freeze (scoped to a pending draw or a live room) is the rule, and the docs say so plainly
  instead of promising replay. Restate answers `200 Unchanged` to a no-`force` re-registration of an existing URI
  whatever the handlers now are, so a forgotten new URI after a handler change is noticed at invocation time, not
  at registration; the runbook says it, and versioning the path (`/restate/v<version>`) was offered and declined.
- **The engine's stored state carries no version tag, and the shape of `Event` and `Manifest` JSON is a
  forever contract.** The same JSON is in `raffle_events.body`, in `verify.json`, in published logs and in Restate
  (`StoredRaffle { manifest, events }`), read by the same types everywhere. The engine's window is not "freeze to
  finish" in practice: an abandoned frozen raffle is asked to finish at the ceiling, about 97 days by default, so
  old state in Restate meets new code at every deploy of a hosted installation. A version field on
  `StoredRaffle` would not help: `TryFrom<StoredRaffle>` runs `restore` after `manifest` and `events` are already
  deserialized, so any shape break fails before the field is consulted. What protects the data is a
  **compatibility rule**, written into `draw-algorithm.md` and `upgrades.md` (API-2): additive only; every new
  field carries `#[serde(default)]`; a field or a variant is never renamed or removed; a new `Event` variant or a
  semantic change is a new `algo` version; an `Event`-shape change is treated like a migration (no Worker
  rollback past it). The failure mode if the rule is broken: `ctx.get` fails with a non-terminal error, Restate
  retries forever, the object is wedged (no draw, no finish, housekeeping's `Finish` fails hourly). The manifest's
  `algo` is the version tag of the published formats.
- **A raffle written by a newer version is unreadable by an older one.** The rollback case of API-1: a v2
  raffle (a new `algo`, or a new event `type`) read by v1 code fails in `rebuild`, so its admin page, projector,
  `verify.json` and `delete` are 500 until the newer code is back. No v1 code shipped now can prevent it; "never
  roll back past a migration" covers it; an "drawn by a newer version" screen instead of a 500 is a nicety for
  later.
- **The email-membership oracle** ("This email is already entered") stays. It is structural: a join is observable
  in the public entrant count and the recent names, so whether an email is already in can be learned by anyone
  with the join code who solves a proof-of-work, at five attempts per minute per email, while registration is
  open. The page says so plainly rather than pretending otherwise.
- **The `/view` endpoint carries `join_code` and display names** to anyone with the slug, by design: the slug is
  the projector's URL.
- **Audit identifiers outlive the account.** `audit.actor` (and, after SCH-1, `audit.user_id`) and
  `raffles.owner_id` are Clerk identifiers kept after `remove_my_data`, as the operator's accountability record
  under Art. 6(1)(f) (and Art. 17(3)(e)); the operator's notice names the period
  (`apps/worker/legal/privacy.md:49-50`). Nothing enforces that period yet (§Deferred).

## Deferred

Worth doing, not before the release.

- **Content-hashed participant scripts with `immutable`.** Design: `scripts/participant.sh` hashes the wasm,
  rewrites the reference in `rafi_participant.js` (the glue resolves `rafi_participant_bg.wasm` relative to
  `import.meta.url`, which is why a `?v=` on the import alone can't work), hashes the JS and `fallback.js`,
  writes `manifest.json`; `rafi-server` reads it at startup natively (`Dirs::participant`) and the Worker fetches
  it once per isolate through the `ASSETS` binding; `LOADER` becomes a formatted string; `_headers` sets
  `immutable` on the hashed names. `/participant/altcha-worker.js` stays a fixed route (a Web Worker URL).
- **Enforcing the audit identifiers' retention period.** A housekeeping sweep that blanks `actor` and `user_id`
  on audit rows of raffles whose `organizer_left_at` is older than the period the operator's notice names, so the
  stated period is enforced rather than aspirational. Not on `remove_my_data` itself: the row is the operator's
  defence record.
- **A full Content-Security-Policy.** Inline `HEAD_SCRIPT` and `LOADER`, Dioxus `document::eval` and the SPA's
  inline styles need nonces or hashes; a day's work with the Dioxus build.
- **The `tracing`/`console` layer growing into request IDs.** The router has one place for a `TraceLayer`
  (`crates/server/src/lib.rs:126-132`) and the admin API one error mapper (`admin.rs:51-69`), so nothing structural
  is needed now.
- **A `Clock` on `App`/`Policy`.** `ids::now()` is called directly in ~15 places; tests shift data instead. Not a
  contract.
- **Rate-limit budgets in two places** (`crates/app/src/ratelimit.rs:25-30` and the `[[ratelimits]]` bindings).
  Document that `wrangler.toml` is authoritative on Workers, or move the native budgets into `Policy`.
- **Smart Placement** (`[placement] mode = "smart"`): D1 is EU-pinned and the participant page makes several
  round trips; measure first.
- **Caching the axum `Router` in `App`** (`apps/worker/src/lib.rs:158-162` rebuilds it per request).
- **A Worker path for `reapply-erasures`**, or generating `cloudflare.md`'s D1 SQL from `rafi_store`, so the two
  can't drift.
- **`DrandClient::wait_for` uses `std::time::SystemTime`** (`crates/engine/src/drand.rs:256`); only `LocalEngine`
  calls it, but it is compiled on wasm32. Gate it or use `web_time`.
- **Clerk deletion webhook** (`user.deleted`, `organization.deleted` → `remove_my_data`): the terms and the admin
  say to remove data first; the webhook needs Svix verification and an ID mapping.
- **`RAFI_ALTCHA_*`, `/api/altcha`, `altcha_used`, `/participant/altcha-worker.js`** bake a library's name into
  env, route and table names. Renaming is cosmetic; not before release.
- **Clerk on Workers (JWKS through `tower-worker-fetch`)** is checked only by hand with `--profile clerk`; CI runs
  dev auth. A CI secret for a test instance would close it.
- **A notifier or live-channel conformance suite** comparable to the store's and the engine's. One real
  `Notifier` exists; `testbeds/cloudflare` covers it.
- **A "drawn by a newer version" screen** instead of a 500 when an older build reads a raffle a newer one wrote
  (the rollback case in §Accepted risks).

## Checked and found solid

So nobody re-audits them.

- **Authorization**: every raffle-scoped admin call goes through `owned()` (`crates/app/src/lib.rs:126-132`) or
  checks `owner_id` after `Full::read`; prizes and entrants are keyed `(id, raffle_id)` in SQL; nothing is looked up
  by a sub-ID alone; tiers (`Admin`, `Manager`, member) gate what the plan says. The Clerk layer fails closed (401
  on none, 503 on an unavailable JWKS), RS256 only, `kid`-pinned, `typ=JWT`, `sid` required, 5 s skew, Bearer
  preferred over the cookie. The Worker refuses to build without `CLERK_SECRET_KEY`, `RAFI_ALTCHA_KEY`,
  `RESTATE_INGRESS_URL`, `RESTATE_IDENTITY_KEYS` (non-dev), or with `RAFI_DEV_AUTH` without `dev`.
- **The join**: ALTCHA signatures cover every parameter, 20-minute TTL, single redemption by primary-key insert
  (race-safe across instances), payload ≤ 8 KiB, rate limits before the proof-of-work; the cookie is `HttpOnly;
  SameSite=Lax; Secure` over HTTPS, 3 days, 128-bit token, SHA-256 at rest, lookup by `(raffle_id, token_hash)`;
  `AlreadyIn` sets no cookie and shows no claim code; claim codes are unique per raffle and shown only to the
  cookie's holder and the admin. No state-changing GET anywhere.
- **No personal data** in `/view`, `/participant`, `verify.json`, the engine's manifest and state, the
  `RaffleRoom`'s cached view, `erasures`, `audit.detail`, or any `tracing`/`console` line (tests
  `public_view_has_no_personal_data`, `participant_view_has_no_personal_data`). The `ERASED` write blanks
  `display_name`, `email_raw`, `email_norm`, `token_hash`, `claim_code`; what survives names no one. The marketing
  opt-in is gone everywhere but the plans and a negative test.
- **SQL**: every user value is bound; `format!` interpolates only enum constants, `ERASED` and placeholder
  numbers; the Postgres `?`→`$N` rewrite runs on SQL text only; D1 binds arrays and respects the 100-parameter
  limit. SQLite and Postgres triggers are equivalent (version bump, append-only events with the cascade-delete
  exception, contiguity); booleans and seconds consistent; `erase_at` as a stored generated column works on
  SQLite ≥ 3.31, D1 and Postgres ≥ 12 and is exercised by the D1 testbed.
- **XSS**: dioxus-ssr escapes text and attributes; `#rafi-config` escapes `</` and `<!--`; `fallback.js` uses
  `textContent`; `privacy_url` must be absolute `https://`; the organizer's rules and privacy text render as plain
  paragraphs; `dangerous_inner_html` only for operator terms and the self-generated QR SVG.
- **Verifiability**: `algo` in the manifest acts as a domain-separation tag; canonical serialization is fixed field
  order with no maps or floats and the exact string is hashed; the drand chain hash is committed; `k` reuse after
  abandon and rejection sampling are specified and tested (chi-square, vectors); the verifier replays through the
  raffle's own transitions and compares byte-for-byte; `set_manifest` only when `NULL`; prizes, entrants and test
  mode lock at the freeze in the same statement; unfreeze only before a manifest exists; `Abandon` is logged with
  the would-be round; `test_mode` is in `verify.json`; `tools/verify.py` matches every current shape and runs on
  the shared vectors in CI.
- **Engine contract**: `open_or_check` refuses a different manifest or chain; the app pre-checks the chain via
  `EngineInfo`; `open-<manifest_hash>` is a stable idempotency key; command IDs are UUIDv4 per click; the
  committed round is journaled before the sleep; the engine writes only `raffles` (guarded columns),
  `raffle_events` and `draws`.
- **Workers**: D1 `DB` with `migrations_dir` shared with sqlx; `RAFFLE_ROOM` with `new_sqlite_classes`; three
  `[[ratelimits]]` matching the code; cron `0 * * * *`; `workers_dev` and `preview_urls` off; `[env.dev]` repeats
  the non-inheritable bindings; `run_worker_first` covers every server route; no stale `script_name` or
  `restate-worker` reference outside `docs/history`. wasm hygiene: getrandom backends enabled for 0.2/0.3/0.4,
  clocks through `js_sys::Date` and `web-time`, the `Instant` limiter is `cfg(not(wasm32))`, tokio on wasm is
  `sync`/`time`/`rt` only with no runtime started, the ingress calls race a `gloo-timers` timeout, drand goes
  through `TowerTransport`. Nothing fetches at isolate start. Panics re-instantiate through `worker-build`'s shim.
- **`RaffleRoom`**: hibernation API via partyline, alarm only while sockets are open, one storage row, `destroy`
  publishes the final view, closes 4403, `delete_alarm` + `delete_all`; `destroyed` re-checked after each await;
  `/nudge` and `/destroy` unreachable from the public gate; `x-partyline-tags` stripped; `MAX_SOCKETS` 2000, 1 MiB
  view cap under the 2 MB row limit; stale objects destroyed by housekeeping and the app paths; exercised by
  `testbeds/cloudflare/scenario.mjs` and `evening.mjs`.
- **Cron parity**: `scheduled` runs the same `housekeeping::run` as native (expiry, erasure, finish at ceiling,
  altcha purge, channel ending, `record_ok`); the retention statements are in the D1 conformance suite.
- **Housekeeping and the three clocks**: expire, erase non-winners, erase everyone at the ceiling, purge and end
  channels, ask the engine to finish, record success; the winner definition is the entrant of a `claimed` draw;
  delivered winners are on the non-winner clock; idempotent, set-based; `retention_stale` after two hours from the
  first raffle; erasure on request follows the date; `Open` refused without organizer and contact; `All prizes
  delivered` gated on the finish; export audited with its count; "Remove my data" is one guarded batch.
- **Hygiene**: 0 build and clippy warnings natively and on the CI's wasm32 matrix; every declared feature is enabled
  by some dependent and every `cfg(feature)` names a declared one; every dependency is referenced; `edition` and
  `rust-version` are workspace-inherited everywhere; `.env` is ignored and untracked; 203 relative Markdown links
  (with anchors) all resolve; `docs/reference/configuration.md` names every variable the code reads;
  `docs/reference/routes.md` matches `crates/server` except CLN-13; JSON is `snake_case` throughout; all five
  `#[ignore]` tests run in CI with `--include-ignored`; both `testdata` fixtures are referenced; the `docs/history`
  notes all carry the Historical banner.
- **Identity, slugs, dates**: the owner key is Clerk's own `org_…`/`user_…` (or `dev`), nothing bakes its format
  into the schema or URLs; slugs always carry a random suffix so `/s/admin` can't collide; join-code lookup
  normalizes and redirects to the canonical code from the database; entrant IDs are 128-bit hex validated sorted,
  unique and lowercase in the manifest; item IDs are safe under `rsplit_once('-')`; the browser supplies the
  end-of-day instant, bounded ±14 h, the server stores the civil date and the instant separately and never uses
  local time; every English string sits in `CardStrings`, `Date::long`, `pages/text.rs` or `JoinError`, so a
  locale is mechanical later.
