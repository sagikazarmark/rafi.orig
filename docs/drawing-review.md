# Drawing review — domain language and architecture of `rafi-drawing`

Repo: /Users/mark/Projects/ventures/rafi (branch `cloudflare`). Reviewed 2026-10-09, after
[drawing-plan.md](drawing-plan.md) moved the drawing into its own crate. Three passes: the domain
language, the crate's internal structure, and its boundaries with the rest of the workspace.

## Findings

### Bugs and risks

1. **The verifier never validates the manifest.** `verify` parses it and checks that it is in
   canonical form, but never calls `Manifest::validate()`: a manifest naming `rafi-draw-v2` or
   listing an entrant twice replays under v1 and can report `ok()`. It also never compares the
   published `Event::Draw.index`. Engines validate on open, so honest logs are fine; the gap is
   exactly the doctored or foreign log a verifier exists for.
2. **The Workers e2e check is stale.** `crates/restate-worker/e2e.mjs` (run by
   `scripts/workers-check.sh`) still calls `/Raffle/{id}/freeze` and sends
   `{cmd:"draw", unit_id, offline}`; `crates/app/evening.mjs` likewise. The only end-to-end check
   of the Workers path fails as written.
3. **The aggregate's serde shape is the storage format.** Restate keeps `Json<Drawing>`; SQL stores
   it and queries its internals by JSON path (`items[].status = 'claimed'` for retention,
   `events[].type = 'draw'` for which names leave the database); reads swallow parse errors with
   `.ok()`. A field rename would orphan in-flight Restate objects, blank the projection silently,
   and make retention erase claimed winners on the non-winner schedule.
4. **`rafi-drand` depends on `rafi-drawing`** and uses nothing from it.
5. **`Drawing::entrant_outcomes` returns a `HashMap`**: nondeterministic iteration order out of a
   crate whose point is determinism.

### Domain language

What reads well and stays: Drawing, Manifest, manifest hash, Entrant, Item, `ItemRef`,
`BeaconSource`, round, randomness, seed, pass and pool, stall, abandon, void, finish, `Decision`,
`DrawRequest`, `Report`.

| Problem | Today |
|---|---|
| "draw" means five things | the algorithm module, the `Draw` command, `Event::Draw` (only a draw's *last* step), and three `Drawn`s (a struct, an item status, an outcome). The `Draw` command emits `Announce`; the `Draw` event comes later. |
| Events read as commands | `Announce`, `Draw`, `Stall`, `Abandon`, `Resolve`, `Void`, `Finish`: bare verbs, the same as the decisions. `Abandon` is published as `"fail"`. |
| `Outcome` is two things | the winner's resolution *and* a state (`Outcome::Drawn`), which `resolve` refuses at runtime (`BadOutcome`). `BeaconOutcome` is a third "outcome". |
| "Pending" is two things | `ItemStatus::Pending` ("can be drawn"; its error already says "not available") and `Pending` ("a draw waiting for its beacon"). |
| "commit" drifts | The plan reserves it for a draw committing to a round; code also says "committed in the manifest", "committed to another beacon source", `BeaconMismatch { committed }`. `BeaconMismatch` and `WrongBeacon` are one concept under two names; fields named `beacon` hold a beacon *source*. |
| Winner vs entrant | `Item.entrant` holds the winner; "forfeit" means *not here* in one place and *not here or declined* in another. |
| Four verbs for one transition | `begin` (emits `Announce`), then `complete` / `conclude` / `settle`. |
| Smaller | "draw engine" in prose vs `DrawingEngine`; the `engine` module holds no engine; "chain" for the beacon source in the state machine; the drawing's ID is a bare `String`; test data says Prize, Ticket, "KCD Budapest ticket", sponsor, `u0`, `doc`, `"rafi-core-test"`. |

### Inside the crate

- **A module cycle**: `engine` imports `drawing::Outcome`; `drawing` imports
  `engine::{BeaconOutcome, Decision}`. The crate root doubles as the hashing module.
- **A shallow, leaky aggregate**: every field of `Drawing` is `pub`; the low-level transitions are
  public next to the engine protocol; `version += 1` is written by hand seven times
  (`version == 1 + events.len()` anyway); item fields are stored while `pool()` and friends
  re-derive from `events` (two sources of truth). The verifier reads `state.pending` and
  `state.events.last()` and needs a crate-private `complete_unverified`.
- **Illegal states are representable**: `Item { status, k: Option, entrant: Option }` (an
  `expect` and a property test police it), `finished: bool` beside `pending: Option`, a public
  `Drawing::open` that skips validation, and `manifest_hash` kept as hex and re-parsed on every
  draw (the only reason `BadManifestHash` exists).
- **`rafi-draw-v1` is split**: the pool rules are part of the algorithm but live in the aggregate;
  `draw.rs` only selects. `sha256(signature)` (the randomness) is written twice.
- **Domain errors carry transport**: `ErrorKind::status` / `from_status` (HTTP codes) and
  `Error::NotOpen` (raised only by the Restate object) live in the domain crate.
- **An incoherent API**: half the types are re-exported at the root, half reached through
  modules; `pub use drand` re-exports a crate whose features vary with the build.

Holds up: the crate is pure and builds for wasm32; the verifier replays through `Drawing` and
`draw::select`, so engine and verifier cannot disagree on the algorithm.

### Boundaries

- No cycles; `rafi-core` and `rafi-drawing` are independent.
- **The raffle ↔ drawing translation is in about seven places across app, store and core**:
  `rafi_core::prize` in plain strings, the app wrapping them into `ItemRef` and building the
  manifest, the view re-expanding prizes into items by hand, the store deriving raffle lifecycle
  strings from a `Drawing` and re-folding outcomes per draw.
- **The engine protocol is split**: its inputs in `rafi_drawing::engine`, the port
  (`DrawingEngine`, `DrawStore`, `EngineError`) in `rafi-platform`.
- The app reaches into `rafi_drawing::drand` for chain hashes and to decide "publicly
  verifiable"; engine-local says `raffle_id` and "the raffle's task ended".
- Stale docs: offline draws (removed in 7b8721f) in drawing-plan, engine-plan and PLAN.md;
  README says rafi-drawing does "drand math".

## Decisions

1. **Events in plain words, past tense**: `Announced`, `Stalled`, `Drawn`, `Abandoned`, `Resolved`,
   `Voided`, `Finished` (published as `announced`, …; no more `"fail"`). "Committed/revealed" is the
   cryptographer's word; "announced" is what the room sees (the countdown), and Drawing was chosen
   over draw for outsiders. "Commit" stays the docs' word: *an announced draw commits to a round*.
2. **`rafi-core` depends on `rafi-drawing`**, and one module there translates between the raffle
   and the drawing. A raffle uses a drawing; `rafi-drawing` still depends on no `rafi-*` crate, and
   every user of `rafi-core` already pulls `rafi-drawing` in.
3. **No engine crate.** The aggregate's inputs move into a `command` module of `rafi-drawing`; the
   port stays in `rafi-platform`, and the HTTP status mapping and `NotOpen` move there with it.
4. **A drawing is stored as its manifest and its events**, and rebuilt by applying the events: both
   are already published formats, pinned by the test vectors. The SQL projection gets the entrants
   it needs (drawn, winners) as columns written on save, and reads fail loudly.

## Plan

One commit per step; workspace tests, both engines' conformance suites and the app suite green
after each.

### Step 1 — Fixes

- The verifier validates the manifest and checks every published field of a draw (`index` too).
- `e2e.mjs` and `evening.mjs` speak the `Drawing` object's API.
- `rafi-drand` drops its `rafi-drawing` dependency.
- `entrant_outcomes` returns a `BTreeMap`.
- Stale docs follow (offline draws, README).

### Step 2 — Structure

- `hash` module: `Hash`, `sha256`, one `randomness(signature)`; `ManifestHash` and `DrawingId`
  types.
- `command` module (was `engine`): `Command`, `DrawRequest`, `Decision`, `BeaconFetch` (was
  `BeaconOutcome`). The cycle goes. `ErrorKind::status` / `from_status` and `NotOpen` move to
  `rafi-platform`.
- The pool rules move into the algorithm module (`algorithm`, was `draw`): one module is the whole
  of `rafi-draw-v1`.
- `Drawing` keeps its fields private, changes only by recording an event (one place applies an
  event to the state and bumps the version), and exposes read accessors. An item's state is one
  enum carrying its draw and winner; a resolution is `Resolution { Claimed, NotHere, Declined }`
  (`BadOutcome` goes). Engines open through `open_or_check` only.
- Public API: the modules are private, the consumer set is re-exported at the root, and only the
  drand types the API uses are re-exported. `BeaconSource` says whether it is publicly verifiable.

### Step 3 — Storage

- `Drawing` serializes as `{"manifest": …, "events": […]}` and deserializes by applying the
  events (Restate state, the SQL `drawing` column, the in-memory store).
- Migration 0004: `raffles.drawn_entrants` and `raffles.winners`, written by `save_drawing`;
  retention and the participant snapshot query those instead of JSON paths into the aggregate.
- A stored drawing that does not parse is an error, not "the engine hasn't confirmed yet".

### Step 4 — Vocabulary

- Events as in decision 1; the item statuses `Available`, `Drawn`, `Claimed`, `Voided`; `Pending`
  → `PendingDraw`; `Drawn` (the result) → `Selection`; `begin` → `announce`, `complete` → `draw`,
  `conclude` → `settle`; `BeaconMismatch` + `WrongBeacon` → `BeaconSourceMismatch` +
  `WrongBeaconSource`; `Item.winner`; "forfeit" defined (not here or declined) or gone; "drawing
  engine" in prose; neutral test data.
- The vectors are regenerated; `tools/verify.py`, the verify page and the spec follow.

### Step 5 — The raffle's side

- `rafi-core` depends on `rafi-drawing`; `rafi_core::drawing` holds the translation (prizes to
  items, the manifest, a prize from an item, the raffle lifecycle a drawing implies) and the app
  and store use it instead of their own copies.
- Engine-side names: `raffle_id` → `drawing_id`, "the raffle's task" → "the drawing's task".

## What the implementation does differently

Done (2026-10-10), one commit per step, in the planned order:

| Step | Commit |
|---|---|
| 1 | `rafi-drawing: the verifier checks the manifest and every published field of a draw` |
| 1 | `rafi: the end-to-end scripts and docs follow the Drawing object, without offline draws` |
| 2 | `rafi-drawing: a drawing changes only by recording events, behind a flat API` |
| 3 | `rafi: a drawing is stored as its manifest and event log` |
| 4 | `rafi-drawing: events in the past tense, available items, and winners` |
| 5 | `rafi-core: the raffle's side of the drawing in one module` |

- **The engine protocol's names moved in step 2**, since its signatures changed anyway: `begin` and
  `check_beacon` became one `announce(request, engine_beacon, round, round_time)` (the beacon check
  can't be forgotten, and announcing a pending item again re-enters it), and `conclude` became
  `settle(BeaconFetch)`. The strict transitions (`start`, `complete`, `stall`, `abandon`, `resolve`,
  `void`, `finish`) are crate-private; engines use `open_or_check`, `announce`, `settle` and
  `decide`, and the verifier replays a log through the transitions. The plan's `complete` → `draw`
  rename was dropped: the method is no longer public.
- **`Outcome` stays, as the read model**: where a draw's winner stands (`Drawn`, `Claimed`,
  `NotHere`, `Declined`), which the app shows. Decisions and events carry a `Resolution`; the field
  keeps its wire name `outcome` (`{"cmd":"resolve","item":…,"outcome":"claimed"}`).
- **The verifier compares whole events**: each published event must equal the one the drawing
  records when replaying it, which covers every field at once (including `index`). Stricter than
  before: hex must be the lowercase an engine writes.
- **`Drawing::restore(manifest, events)` came in step 2**, not 3: tests outside the crate needed a
  supported way to build a drawing with draws. A log naming a draw that holds no item is
  `Error::UnknownDraw`.
- **Draw events keep `entrant`** (the entrant drawn, who may still forfeit); items hold a `winner`.
- **The store**: `StoredDrawing` writes the drawing with its `drawn_entrants` and `winners`;
  `RaffleRow::drawing()` returns a `Result`. Migration 0004 converts stored drawings to the new
  shape and fills the columns; 0005 renames stored events to the past tense. Both were run by hand on
  SQLite and Postgres against rows in the old shape.
- **The conformance suite**: `a_finished_raffle_is_forgotten` is `a_finished_drawing_is_forgotten`;
  the "malformed command" refusal is now an invalid manifest (a decision can't be malformed any
  more).
- **`rafi_core::prize` became `rafi_core::drawing`** (items, the manifest, the lifecycle a drawing
  implies), typed in `ItemId` and `ItemRef`.

Left as they are:

- Engines notify clients with the drawing's ID, which is the raffle's (`Notifier` is per raffle);
  `rafi_core::drawing` documents it.
- `DrawingEngine` takes the drawing ID as `&str`, not `DrawingId`.

Before deploying: run migrations 0004, 0005 and 0006. A Restate `Drawing` object stored before this
change no longer loads (none exist before the first deploy). The Restate tests skip themselves
without a server: run them with `RESTATE_SERVER_BIN=$(./scripts/restate-server.sh)`.

## Second pass

A plain reading of the crate, component by component, found three names for one completed draw and a
few words that said less than they meant. Decided:

- **`DrawRequest` stays.** "Next draw" would be wrong when a stalled draw is re-entered (the same
  draw, not the next), and the organizer picks the item, not the drawing; "request" says that a draw
  is accepted and settles later.
- **`Decision` stays**, and names the split: a drawing's inputs are *draws*, decided by the beacon,
  and *decisions*, made by the organizer. (`Ruling`, `Action`, `OrganizerCommand` and
  `Adjudication` were weighed.)
- **One `CompletedDraw`**: what `settle` returns (was `Selection`), what the history lists, and what
  the pool rules read (was the private `PastDraw`). A draw's own types are a `draw` module:
  `PendingDraw`, `CompletedDraw`, `Outcome`.
- **`AbandonedDraw`** (was `CommittedRound`: every draw commits to a round),
  **`Report::beacons_verified`** (was `chain_known`), **`ManifestItem`** (was `ItemRef`, which read
  as a reference).
- **Decisions are tagged `"type"`**, like events (was `"cmd"`, from when draws and decisions were one
  tagged command), and the resolution's field is **`resolution`** (was `outcome`) in the decision
  and the `resolved` event. Migration 0006 renames it in stored drawings.

| Commit |
|---|
| `rafi-drawing: one type for a completed draw, and the verifier's words` |
| `rafi-drawing: decisions are tagged by type, and a resolution is a resolution` |
