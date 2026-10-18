# Events plan — the raffle's log becomes a table

Repo: /Users/mark/Projects/ventures/rafi. Crates in `crates/`, migrations in `migrations/sqlite/` (SQLite and D1) and
`migrations/postgres/`. Follows [drawing-review.md](drawing-review.md) (decision 4: a raffle is stored as its manifest
and its events) and [raffle-plan.md](raffle-plan.md), whose "Out of scope" listed storing only the events. Breaking
changes are fine: there are no real raffles yet (see "Consequences").

## Goal

The `raffles` row stops storing the raffle as JSON. Its event log becomes an append-only table, its completed draws a
table that SQL can query, and the manifest is stored once. The engines, the `Store` trait, Restate's state and the
published formats don't change.

Today the row holds four JSON columns, all written by `save_raffle` (`crates/store/src/lib.rs:120`) after every
change:

| Column | Holds | Read by |
|---|---|---|
| `log` | `{"manifest": …, "events": […]}`, rewritten whole on every save | every reader of the raffle, through `RaffleRow::drawn()` and `live::snapshot` |
| `log_version` | `1 + events.len()` | the save's newest-wins guard; `log IS NOT NULL` also means "opened" |
| `drawn_entrants` | JSON array of entrant IDs | the participant snapshot (`dialect::entrants_in`) |
| `winners` | JSON array of entrant IDs | retention (`dialect::entrants_in`) |

What's wrong with it:

- **The log is rewritten whole** on every change, though it only ever grows.
- **The manifest is stored twice**: in `raffles.manifest` (the exact string hashed and published) and inside `log`.
- **Two JSON arrays exist only for SQL**, read by the one dialect-specific query (`json_each` / `jsonb_array_elements_text`),
  with no constraint or index. They say per entrant what is really per draw: one entrant can win several prizes.
- **Nothing in the database enforces** that the log only grows.

## Vocabulary

| Term | Meaning |
|---|---|
| **Event** | One entry in the raffle's log (`rafi_raffle::Event`); a row of `raffle_events` |
| **seq** | An event's 1-based position in its raffle's log. Assigned by the domain (index + 1), never by the database |
| **Draw** | A completed draw (`rafi_raffle::CompletedDraw`): its item, its winner and where the winner stands; a row of `draws` |
| **Opened** | The engine has written the raffle at least once. An opened raffle may have no events |

## Decisions

- **The log is append-only, and the database enforces it.** The domain only ever pushes events
  (`crates/raffle/src/raffle.rs:388, 679`), so an older state is always a prefix of a newer one. Triggers refuse to
  update an event, delete one while its raffle exists, or insert one that leaves a gap.
- **`seq` comes from the domain.** It orders the log for replay and verification, and it identifies an event: content
  doesn't (a draw that stalls twice on a round records two identical `stalled` events; `k` is reused after an
  abandon). A database-assigned ID would give a retried event a new identity and duplicate it.
- **An event's `body` is its serde JSON**, the format `verify.json` publishes and Restate stores. A hand-written
  mapping of seven variants could drift, and the verifier compares whole events. `type` is copied out of it for
  querying.
- **No `log_version`.** The version is `MAX(seq)`: `MAX(seq) + 1 = Raffle::version()`, so the domain doesn't change.
  A column would be a second copy that could disagree.
- **`raffles.opened_at` says the engine opened the raffle.** Opening records no event, so an opened raffle can have no
  rows; `start::next` needs to tell it from an unopened one (today: `log IS NOT NULL`). Not an `opened` event: that
  would change the published log, the verifier, `tools/verify.py` and the test vectors.
- **`draws` replaces `drawn_entrants` and `winners`.** Rows come from `Raffle::draws()`, computed in Rust; SQL never
  works out the state machine (abandoned draws reusing `k`, voids, redraws). Its columns are what SQL reads, plus the
  item; everything else about a draw is in its events.
- **A claimed draw is a winner.** `winners()` reads item state, `draws()` the resolutions; they agree because the rules
  make a claim final: a draw is resolved at most once (`resolve` needs a drawn item, `raffle.rs:632`), a claimed item
  can't be withdrawn (`void` needs an available one, `raffle.rs:657`), and only a forfeit makes an item drawable
  again, under a new `k`. A test pins it.
- **Resolutions stay final.** An undo would be a new lever: an organizer could redraw until the result suits them
  ([PLAN.md](PLAN.md) §6.1 counts the levers). Mistakes are guarded against in the app (step 4).
- **One `save_raffle(&Raffle)`, and it keeps its name.** The whole state makes the write idempotent and newest-wins
  for free, lets the store ask the domain (`lifecycle`, `draws()`), and keeps the write one atomic batch. Splitting it
  into per-draw or per-decision calls would mirror the event types in the engine contract (not every change is a
  draw). Not `record`: that is the domain's word for recording an event (`Raffle::record`).
- **The store reads before it writes.** It needs to know which events are new. Sending every event every time grows
  the batch with the log and runs into D1's limits; sending only the last one leans on the engine contract and would
  wedge a raffle if it broke. One extra round trip on a few writes per draw is tens of milliseconds, well inside the
  drand lead.
- **Readers keep reading the database.** It stays the live feed and the record; the engine is not read.
  `raffles.version` stays the live views' change counter (it covers more than events: names, prizes, settings).

## Schema

Migration `0010_raffle_events.sql`, and its Postgres equivalent. SQLite shown.

```sql
-- The raffle's event log (docs/events-plan.md): append-only, one row per
-- event, in the order the raffle recorded them.
CREATE TABLE raffle_events (
    raffle_id TEXT NOT NULL REFERENCES raffles (id) ON DELETE CASCADE,
    -- The event's position in the log, from 1: the domain's, never the database's.
    seq INTEGER NOT NULL CHECK (seq >= 1),
    -- The event's type, as in its body.
    type TEXT NOT NULL,
    -- The event as published (serde JSON of rafi_raffle::Event).
    body TEXT NOT NULL,
    -- When it was stored (unix seconds); not part of the published event.
    recorded_at INTEGER NOT NULL,
    PRIMARY KEY (raffle_id, seq)
);

CREATE TRIGGER raffle_events_no_update BEFORE UPDATE ON raffle_events
BEGIN
    SELECT RAISE(ABORT, 'raffle events are append-only');
END;

-- Deleting the raffle takes its events with it.
CREATE TRIGGER raffle_events_no_delete BEFORE DELETE ON raffle_events
WHEN EXISTS (SELECT 1 FROM raffles WHERE id = OLD.raffle_id)
BEGIN
    SELECT RAISE(ABORT, 'raffle events are append-only');
END;

-- A retried or stale event (seq already stored) passes, and the primary key
-- ignores it; only a gap is refused.
CREATE TRIGGER raffle_events_contiguous BEFORE INSERT ON raffle_events
WHEN NEW.seq > (SELECT COALESCE(MAX(seq), 0) FROM raffle_events WHERE raffle_id = NEW.raffle_id) + 1
BEGIN
    SELECT RAISE(ABORT, 'raffle events must be contiguous');
END;

-- Every completed draw and where its winner stands, as Raffle::draws() says.
CREATE TABLE draws (
    raffle_id TEXT NOT NULL REFERENCES raffles (id) ON DELETE CASCADE,
    k INTEGER NOT NULL,
    item_id TEXT NOT NULL,
    entrant_id TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('drawn', 'claimed', 'not_here', 'declined')),
    PRIMARY KEY (raffle_id, k)
);

-- Raffles opened before this migration can't be carried over (their draws are
-- worked out in Rust, not SQL): they are deleted, with their prizes and
-- entrants. There are no real raffles yet.
DELETE FROM raffles WHERE log IS NOT NULL;

ALTER TABLE raffles ADD COLUMN opened_at INTEGER;
ALTER TABLE raffles DROP COLUMN log;
ALTER TABLE raffles DROP COLUMN log_version;
ALTER TABLE raffles DROP COLUMN drawn_entrants;
ALTER TABLE raffles DROP COLUMN winners;
```

Postgres: `BIGINT` for the integers, the same `DELETE` before the columns change; plpgsql trigger functions, as in
`migrations/postgres/0002_workers.sql`. Update and delete share one function:

```sql
CREATE FUNCTION raffle_events_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' AND NOT EXISTS (SELECT 1 FROM raffles WHERE id = OLD.raffle_id) THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'raffle events are append-only';
END;
$$;

CREATE TRIGGER raffle_events_append_only BEFORE UPDATE OR DELETE ON raffle_events
FOR EACH ROW EXECUTE FUNCTION raffle_events_append_only();

CREATE FUNCTION raffle_events_contiguous() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.seq > (SELECT COALESCE(MAX(seq), 0) FROM raffle_events WHERE raffle_id = NEW.raffle_id) + 1 THEN
        RAISE EXCEPTION 'raffle events must be contiguous';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER raffle_events_contiguous BEFORE INSERT ON raffle_events
FOR EACH ROW EXECUTE FUNCTION raffle_events_contiguous();
```

Checked by hand (2026-10-17) on SQLite 3.53.3 and Postgres 18.6, with exactly these triggers:

- **A raffle deleted takes its events with it**: by the time the cascade deletes them, the raffle row is gone, so the
  delete trigger lets them through. Deleting a single event, or updating one, is refused.
- **Several events in one `INSERT`** pass the contiguity trigger: each row sees the rows the same statement inserted
  before it (1, 2, 3 in one statement). A retry overlapping stored events (2, 3, 4 after 1–3) adds only the new one;
  a gap is refused, and so is a multi-row insert containing one, which inserts nothing.
- **An event for a deleted raffle** fails its foreign key.
- **The guarded draws upsert** (`INSERT … SELECT … WHERE <guard> ON CONFLICT … DO UPDATE`) parses on both, applies when
  the guard holds, and a stale save (the guard fails) doesn't revert a newer outcome.

D1 runs SQLite with foreign keys enforced; `testbeds/store-d1` runs the same conformance scenarios there (step 2).

## Saving

`save_raffle(db, &Raffle) -> Result<bool>`, same signature and contract (`crates/engine/src/store.rs:9-16`): idempotent,
written only when newer and for the manifest the raffle was opened with; returns whether it wrote.

1. **Read** the raffle's `manifest_hash`, `opened_at` and `COALESCE(MAX(seq), 0)` as `stored`, in one query.
   - No row (deleted), or another manifest hash: `Ok(false)`.
   - `opened_at` set and `stored >= events.len()`: a retry or a stale save, `Ok(false)`.
2. **Write**, in one `Db::batch` (all or nothing), with `n = events.len()`:
   1. **The new events**, seq `stored + 1 ..= n`: `INSERT … VALUES (…), (…) ON CONFLICT DO NOTHING`, with
      `recorded_at = now()`. Five parameters per row, so at most 20 rows per statement (D1 binds at most 100); one
      row is the norm.
   2. **The draws the new events touch**: those whose `k` a new `drawn` or `resolved` event names, with their values
      from `Raffle::draws()`. One upsert each (`INSERT … SELECT … WHERE <guard> ON CONFLICT (raffle_id, k) DO UPDATE
      SET outcome = excluded.outcome`).
   3. **The row**: `state` (following the raffle, as today), `finished_at`, `opened_at = COALESCE(opened_at, now)`,
      `updated_at`, `WHERE id = ? AND manifest_hash = ? AND <guard>`.

   `<guard>` is `(SELECT COALESCE(MAX(seq), 0) FROM raffle_events WHERE raffle_id = ?) = n`. It is evaluated after the
   inserts, so a stale save that passed step 1 (a newer one landed in between) inserts nothing, touches no draw and
   leaves the row alone. Returns whether the row was updated.

On SQLite, an `INSERT … SELECT … ON CONFLICT` needs the `SELECT` to have a `WHERE` clause (a parsing ambiguity); the
guard provides it.

Why this is safe without the read and write being atomic: Restate runs one invocation per raffle at a time, and a
stale attempt carries the same events at the same positions as the newer state, so its inserts conflict. A raffle
deleted in between fails the batch on its foreign keys; the engine's retry then finds no row.

Opening (no events): step 1 finds `stored = 0` and no `opened_at`; the batch is only the row's update.

`StoredRaffle` (`lib.rs:153`), `encode`'s use for the arrays, `parse_raffle` and `dialect::entrants_in` go.
`entrants_in` is the dialect module's only function: `dialect` keeps the `Dialect` enum.

## Reading

- **A raffle is rebuilt** from `raffles.manifest` (parsed) and its events in `seq` order, with
  `Raffle::restore(&manifest, &events)`. A body that doesn't parse, or a log that doesn't replay, is an error, as
  today.
- **Any events read make a valid raffle on their own**: the manifest never changes once set, and the events only grow.
  So readers that only need the raffle (command checks) can read the row and its events separately.
- **Readers that label content with `raffles.version`** read the events in the same `batch_rows` as the row, so a
  write can't land in between: `view::Full::read` gains the raffle (`Full.drawn: Option<Raffle>`), and
  `live::snapshot` reads `manifest`, `opened_at` and the events with the row.
- **`RaffleRow`** loses `log` and `log_version`, gains `opened_at`. `RaffleRow::drawn()` becomes
  `raffles::drawn(db, &RaffleRow) -> Result<Option<Raffle>>`: `None` until opened. An opened raffle without a
  manifest is an `Error::Decode`, never `None`: a stored raffle that doesn't load is an error (as `parse_raffle` says
  today). `Full.drawn` and `live::snapshot` follow the same rule.
- **`start::next(state, manifest.is_some(), opened)`** takes `opened_at.is_some()` (`crates/app/src/draw.rs:53`,
  `crates/app/src/reads.rs:31`).
- **The participant snapshot's names**: `e.id IN (SELECT entrant_id FROM draws WHERE raffle_id = ?1)`.
- **Retention's winners**: `entrants.id IN (SELECT entrant_id FROM draws WHERE raffle_id = r.id AND outcome =
  'claimed')`.

## Plan

Every step keeps green: the workspace tests, the engine conformance suite on both engines
(`crates/engine/tests/conformance.rs`, `crates/engine-restate/tests/conformance.rs`), the Restate e2e tests, the
storage conformance suite on SQLite and Postgres (`crates/store/tests/conformance.rs`, `RAFI_TEST_POSTGRES_URL`) and
D1 (`testbeds/store-d1`), the independent verifier (`tools/verify.py --vectors testdata/draw-vectors.json`, which
doesn't change) and the Workers clippy (wasm32). Run the Restate tests with
`RESTATE_SERVER_BIN=$(./scripts/restate-server.sh)`, and the Postgres suites against a throwaway server:

```shell
docker run -d --name rafi-pg -e POSTGRES_PASSWORD=rafi -p 5432:5432 postgres:18
export RAFI_TEST_POSTGRES_URL=postgres://postgres:rafi@localhost:5432/postgres
```

The end state also passes `scripts/workers-check.sh`. One commit per step.

### Step 1 — The domain says what the store needs

- `Event::kind() -> &'static str`: the event's `type`, with a test that it equals the serde tag for every variant.
- `Outcome::as_str()` (or its serde name) for the `draws.outcome` column.
- A test that the entrants of claimed draws are `winners()`, over the test vectors' scenario and the existing
  property tests' raffles.

### Step 2 — Store, migration and callers

One commit: the row's columns change, so the store and its callers move together.

- Migration `0010_raffle_events.sql` (both dialects), as above. No backfill: raffles opened before it are deleted.
- `save_raffle` as above; `RaffleWriter` unchanged.
- Reading as above: `raffles::drawn`, `Full.drawn`, `live::snapshot`, retention, `RaffleRow`.
- `dialect::entrants_in` goes; `dialect.rs` keeps only `Dialect`. Retention's `due` no longer differs by dialect: one
  `LazyLock`, not one per dialect.
- Callers: `crates/app/src/{draw,reads,view,raffles,entrants}.rs`, `crates/app/tests/use_cases.rs`, and the
  `drawn()` helper in `testbeds/cloudflare/src/lib.rs`.
- The server tests (`crates/server/src/tests.rs`, `tests_restate.rs`): `wait_for` and `wait_long` poll
  `view::Full::read` and hand the predicate a `&Full`, since reading the raffle is now async. `r.drawn().unwrap()`
  becomes `f.drawn`, `r.log.is_some()` becomes `f.raffle.opened_at.is_some()`. The tests read what the admin and
  the projector read; no test-only API.
- Test fixtures write the raffle as the app and the engine do:
  - wherever a fixture sets `manifest_hash`, it also sets `manifest` (`manifest.canonical_json()`): `frozen()` in
    `crates/store/src/conformance/domain.rs`, the setup in `crates/store/src/live.rs` (tests), `freeze()` in
    `testbeds/cloudflare/src/lib.rs` (`prepare()` already does);
  - fixtures that stand in for the engine (`store()` in the `live.rs` tests, the retention scenario, the Cloudflare
    testbed's `freeze()`) call `save_raffle` instead of writing columns;
  - `live.rs`'s "a log that does not load" test stores an event whose body doesn't parse (inserted directly, with
    `opened_at` set).
- `testbeds/store-d1` has no setup SQL to change: its `fresh()` deletes `raffles`, whose foreign keys take
  `raffle_events` and `draws` with it. Don't add `raffle_events` to its list before `raffles`: the delete trigger
  refuses it.
- Storage conformance scenarios (`crates/store/src/conformance/domain.rs`):
  - a raffle read back equals the raffle saved, after each change of a full evening: the vectors' scenario,
    `include_str!` of `testdata/draw-vectors.json` (it builds for wasm32, so D1 runs it too), saved one prefix of its
    events at a time, on a row holding its manifest and hash;
  - an opened raffle with no events reads as opened; an opened raffle without a manifest is an error;
  - a repeated save writes nothing; a stale save (older state, after a newer one) inserts nothing, leaves the draws
    and the row alone, and returns `false`; a save for another manifest is refused;
  - an event can't be updated or deleted; an event after a gap is refused; deleting the raffle deletes its events;
  - several new events in one save are stored in order;
  - the draws rows equal `Raffle::draws()` after every save;
  - retention keeps a winner of two prizes on the winner schedule, and the snapshot shows the names of every drawn
    entrant and nobody else (as today, against `draws`).

### Step 3 — Docs

- [PLAN.md](PLAN.md): §8.4 (`raffles.log` → `raffle_events` and `draws`), and wherever the stored log is described.
- `crates/store/src/lib.rs` (module docs, `save_raffle`), `crates/engine/src/store.rs` ("a raffle, after every
  change" stays true), `crates/raffle/src/raffle.rs` (`StoredRaffle`'s doc: Restate still stores
  `{manifest, events}`).
- A note in [workers-plan.md](workers-plan.md) where it describes `dialect::entrants_in` (line 455).

### Step 4 — Confirm what can't be undone (app, independent of steps 1–3)

The admin draw console resolves and withdraws in one click (`apps/web/src/ui/admin/draw.rs:210-215`); Finish and
Abandon already use `Confirm` (`apps/web/src/ui/admin/mod.rs:105`).

- **"Not here", "Declined" and "Withdraw" ask first**, naming the person and the prize. Names only, never a pronoun
  (a display name says nothing about it):
  - "Anna isn't here? Anna is out of the raffle, and the Book goes back to be drawn again."
  - "Anna declined? Anna is out of the raffle, and the Book goes back to be drawn again."
  - "Withdraw the Mug? It won't be drawn."
- **One dialog for all three**: a click stores a `Pending` (resolve this item this way, or withdraw this item) in a
  signal and opens one `Confirm` (`open: Signal<bool>`, unchanged), whose title, body and action come from it;
  confirming sends that command.
- **"Handed over" stays one click.** It is the common path, once per prize, live; a mistaken claim takes nothing from
  anyone (the prize stays with the person drawn, who can still collect it).

## Consequences

- **Development databases lose their opened raffles.** Migration 0010 deletes every raffle opened before it (with its
  prizes and entrants) rather than leave it looking opened in `state` but not in `opened_at`. A raffle Restate still
  holds then finds no row: its saves return `false`, which the engine already handles. There are no real raffles.
- **Restate is unaffected**: its state stays `Json<Raffle>` (`{manifest, events}`), and no object changes shape.
- **Published formats are unchanged**: the manifest string, its hash, and each event in `verify.json`.
- **Future renames of events** (like migrations 0005 and 0006) rewrite `body` and `type`, and must drop and recreate
  `raffle_events_no_update` around it.

## Out of scope

- Reading live state from the engine, or splitting `Store::save_raffle` into several calls.
- Watching `raffle_events` for live updates: `raffles.version` already orders every change screens show.
- Sending only the last event per save (one round trip instead of two); worth it only if save latency ever matters.
- Undoing a resolution or a withdrawal.
- Checking an event's identity, not only its position, when saving. The guard compares `MAX(seq)` with the state's
  length; a state whose earlier events differ from the stored ones (two engines owning one raffle, Restate's state
  lost) would be appended or partly applied. Restate's attempts always carry a prefix of the newer state, so this
  needs a broken engine contract; checking it would turn that silent divergence into a raffle that can't be saved.
- Changing what any rule does, or the `Event` and `Manifest` formats.
