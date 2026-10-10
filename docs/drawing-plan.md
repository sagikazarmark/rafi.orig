# Drawing domain plan — a raffle-free `rafi-drawing`

Repo: /Users/mark/Projects/ventures/rafi (branch `cloudflare`). Crates in `crates/`.
Follows [engine-plan.md](engine-plan.md), which made the draw engine a seam. Done (2026-10-09); see
"What the implementation does differently" at the end.

## Goal

Two parts of Rafi, named and separated:

- **The raffle**: what the organizer runs in the app. Prizes, registration, entrants joining,
  lifecycle, live screens.
- **The drawing**: what happens after the entrant list is frozen. A verifiable selection of
  winners from a fixed list, owned by the engine (Restate in production, the local engine in tests).

The drawing becomes its own crate that never mentions raffles: the compiler enforces the boundary.
Breaking changes are fine (see "Consequences").

## Vocabulary

| Context | Term | Meaning | Was |
|---|---|---|---|
| Raffle | **Raffle** | The event the organizer runs | — |
| Raffle | **Prize** | What can be won, with a quantity ("Mug ×2") | `item` |
| Raffle | **Participant** | Someone following a raffle on their phone; becomes an entrant by joining | "attendee" |
| Raffle | **Command** | A lifecycle request the rules can refuse (publish, open, freeze, …) | `raffle::Action` |
| Both | **Entrant** | Someone on the list, who can be drawn | (unchanged) |
| Drawing | **Drawing** | The aggregate: opened from a manifest, made of draws, ends at finish | `DrawState` |
| Drawing | **Manifest** | Entrants, items, algorithm and beacon source; its hash is published | `CommitDoc` |
| Drawing | **Item** | One copy that one entrant wins (`mug-1`, `mug-2`) | unit |
| Drawing | **Draw** | Selecting the winner for one item, committed to a future drand round | (unchanged) |
| Drawing | **Beacon source** | Where the randomness comes from: `BeaconSource::Drand(ChainHash)` | `drand_chain: String` |
| Drawing | **Manifest hash** | `SHA256(canonical manifest)`, published, mixed into every seed | `list_hash` |
| Drawing | **Command** | `Draw(DrawRequest)` or `Decide(Decision)` | `Command` (flat) |

Word rules:

- **Freeze** is the raffle's verb (the raffle freezes its entrant list). The drawing side **opens**
  a drawing from a manifest. `RaffleState::Frozen` stays.
- **Commit** means only a draw committing to a future drand round (the countdown). The manifest is
  not a "commit document" any more.
- **Beacon** (`drand::Beacon`, `BeaconOutcome`) stays a single round's output; the source is
  `BeaconSource`.
- **Action** is only the audit log's word: a recorded entry.
- "Drawing" over "draw" for the aggregate: the UI and event log already say "draw" for one
  selection, so *a drawing is made up of draws, one per item*. It is the more widely understood
  word for the event in American English; British English says "draw".

## Decisions

- **One manifest for handoff and verification.** The engine opens the drawing from the same bytes
  that are published; the drawing stores their hash; the seed mixes it in. A separate handoff type
  would need its own proof of equality.
- **`Command` stays data**, not one trait method per verb: the app's dry run against the
  projection and its refusal messages work on one value. It is split along the one real behavioral
  difference: a draw is *accepted* (settles later, progress in the projection), a decision is
  *applied*. The Restate handlers already split that way; the trait and the types now match them,
  which is also what the generated Restate ingress client needs (one typed call per handler).
- **`beacon` is a tagged value** (`{"drand": "<chain hash>"}`), so a non-drand source could be a new
  variant if every hosted drand network went away. Only the manifest field is generalized: there is
  still one variant, and events, the verifier, fetching, relays, lead and the mock chain stay
  drand-specific engine internals. This refines, not reverses, engine-plan.md's "drand stays domain
  vocabulary; no generic randomness source seam". Offline draws (server randomness, flagged
  unverifiable) stay the fallback when drand is down; they are a per-draw mode, not a source.
- **Pool rules and outcomes stay fixed** in `rafi-draw-v1` (one win per person, pass 2,
  claimed / not here / declined). No configurable policy in the manifest. A use that needs other
  rules gets another algorithm.
- **The Restate object follows the domain**: `Raffle` → `Drawing`, `RaffleEngine` →
  `DrawingEngine`, before the first deploy, while no live objects exist. Otherwise the generated
  client (`RaffleClient`) would be where raffle vocabulary leaks into the drawing.
- **The crate is `rafi-drawing`**, not `rafi-draw`: it matches the aggregate and isn't one letter
  away from `rafi-drand`.
- **`RaffleState::Drawing` keeps its name.** It means "at least one draw happened", while the
  aggregate already exists from `Frozen`. An accepted gap.

## Manifest

Changed in place; `algo` stays `rafi-draw-v1`:

```json
{
  "algo": "rafi-draw-v1",
  "drawing": "r_8f3k2m",
  "beacon": { "drand": "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971" },
  "entrants": ["0a1f9c…", "3c9e41…"],
  "items": [
    { "id": "mug-1", "name": "Mug" },
    { "id": "mug-2", "name": "Mug" },
    { "id": "book-1", "name": "Rust in Action" }
  ]
}
```

| Was | Now | Why |
|---|---|---|
| `raffle_id` | `drawing` | Binds the manifest to one drawing (whose ID equals the raffle's and is the Restate key). Not `id`: the manifest's own identity is its hash. |
| `drand_chain: "<hex>"` | `beacon: {"drand": "<hex>"}` | Tagged by kind of source. |
| `entrant_ids` | `entrants` | Still sorted, deduplicated, lowercase hex IDs. No names: the purge depends on it. |
| `units[].{unit_id, item_id, item_name}` | `items[].{id, name}` | The prize an item came from is raffle bookkeeping; the ID shows it. Names stay so the verify page says what was won. |

Unchanged: fixed field order, no floats, no maps except the one-key `beacon` tag, the exact
serialized string published beside its hash (verifiers never re-serialize), the seed formula and
sampling.

## Plan

Every step keeps the workspace tests, the engine conformance suite on both engines
(`engine-local/tests/conformance.rs`, `restate-rafi/tests/conformance.rs`) and the Restate e2e tests
green. One commit per step, except where noted.

### Step 1 — Move the drawing into `crates/drawing` (move only)

- New crate `rafi-drawing` (`crates/drawing`). `git mv` from `crates/core/src`: `commit.rs`,
  `drand.rs`, `draw.rs`, `state.rs`, `verify.rs`, `engine.rs`; move `Hash`, `sha256`,
  `hash_from_hex` from `lib.rs`. Dependencies as core's today (drand, serde, serde_json, sha2, hex,
  thiserror); no `rafi-*` dependency.
- `rafi-core` keeps `raffle.rs`, `codes.rs`, `email.rs`.
- Switch imports to `rafi_drawing` in `rafi-drand`, `rafi-platform`, `rafi-store`,
  `rafi-engine-local`, `restate-rafi`, `restate-rafi-client`, `rafi-cloudflare-check`, `rafi`
  (app, including the WASM verifier). `rafi-drand` depends on `rafi-drawing`, never the reverse.
- No renames, no behavior change, so git keeps the history.

### Step 2 — `error` module (move only)

- `DrawError` and `kind()` (`state.rs:160-216`) and `ErrorKind` with `status` / `from_status`
  (`engine.rs:51-87`) move to `drawing/src/error.rs`, re-exported at the crate root.

### Step 3 — Drawing vocabulary

Inside `rafi-drawing` and its callers:

- Modules: `state` → `drawing`, `commit` → `manifest`.
- Types: `DrawState` → `Drawing`, `CommitDoc` → `Manifest`, `UnitRef` → `manifest::ItemRef`,
  `Unit` → `Item`, `UnitStatus` → `ItemStatus`, `DrawError` → `Error`.
- Fields and functions: `unit_id` → `item_id` (commands, events, items), `list_hash` →
  `manifest_hash`, `freeze_or_check` → `open_or_check`, `freeze_key` → `open_key`, `units()` →
  `items()`.
- Engine: `DrawEngine` → `DrawingEngine`, `freeze` → `open` (trait, both engines, the Restate
  handler, the client, the conformance suite).
- Error variants:

  | Was | Now |
  |---|---|
  | `NotFrozen` / `AlreadyFrozen` | `NotOpen` / `AlreadyOpen` |
  | `InvalidCommit` | `InvalidManifest` |
  | `BadListHash` | `BadManifestHash` |
  | `UnknownUnit` / `UnitNotPending` / `UnitNotDrawn` | `UnknownItem` / `ItemNotPending` / `ItemNotDrawn` |
  | `ChainMismatch` / `WrongChain` | step 4 |
  | `NotACommand` | step 5 |

- Messages and docs in the crate never say "raffle" (e.g. "the drawing is finished",
  "unknown item mug-1").
- Event JSON follows (`unit_id` → `item_id`): `tools/verify.py` and the verify page change in
  this step.
- The app's `refusal()` (`app/src/server/service.rs:504`) follows the variant renames.

### Step 4 — Manifest format and `BeaconSource`

- `Manifest` gets the new shape above; `BeaconSource` (`#[serde(rename_all = "snake_case")]`,
  externally tagged) with one variant, `Drand(ChainHash)`.
- `Drawing.drand_chain: String` → `beacon: BeaconSource`; `EngineInfo.chain` → `beacon`. The
  checks become typed equality instead of `eq_ignore_ascii_case` on strings (`state.rs:475`,
  `check_chain`). `ChainMismatch` → `BeaconMismatch`, `WrongChain` → `WrongBeacon`. The
  `quicknet_chain` serde default for old stored states goes away.
- The app resolves `RAFI_DRAND_CHAIN` to a `BeaconSource::Drand` and checks it against
  `EngineInfo.beacon` before freezing (as today with the chain). The mock chain is still
  `Drand(<mock hash>)` and still reported as not publicly verifiable.
- `tools/verify.py`, the verify page ("manifest hash"), and the spec in `docs/PLAN.md`
  (`commit_doc` / `list_hash`) follow.

### Step 5 — Draw and decide

- `rafi-drawing`: `DrawRequest { item_id, offline }`, `Decision { Resolve, Void, Abandon, Finish }`,
  `Command { Draw(DrawRequest), Decide(Decision) }`. `Drawing::apply(&Command)` →
  `Drawing::decide(&Decision)`. `NotACommand` goes away: a decision cannot be a draw.
- `DrawingEngine`: `info`, `open`, `draw(drawing_id, &DrawRequest, command_id)`,
  `decide(drawing_id, &Decision, command_id)`.
- Restate: object `Raffle` → `Drawing`, service `RaffleEngine` → `DrawingEngine`; handlers `open`,
  `draw(Json<DrawRequest>)`, `decide(Json<Decision>)`, `get`. The `draw` handler no longer rejects
  other commands; the client no longer routes by command. Register the deployment again; README
  follows.
- Local engine: its queue holds `Command`s as today.
- App: the dry run and refusal messages keep working on `Command`; `send` does the one `match`
  (`Draw` → `engine.draw`, `Decide` → `engine.decide`).
- Check whether `Command` still needs serde (step 6 stops serializing it into the audit log). If
  not, drop the derive and the wire-format test in `engine.rs`.

### Step 6 — Audit verbs

- A drawing command is recorded as its own verb with a short subject instead of
  `action = "command"` with JSON (`app/src/server/service.rs:539`): `draw` / `mug-1`, `resolve` /
  `mug-1 claimed`, `void` / `mug-2`, `abandon` / `mug-1`, `finish` / `""`. This matches every
  other audit row ("create", "freeze", "add_manual").

### Step 7 — App: prizes and the stored drawing

- Migration `0003` (SQLite and Postgres): table `items` → `prizes`; columns `raffles.commit_doc` →
  `manifest`, `list_hash` → `manifest_hash`, `draw_state` → `drawing`.
- `rafi-store` and the app follow: `upsert_item` / `delete_item` → `upsert_prize` /
  `delete_prize`, the prize forms, model types. The UI already says "prize".

### Step 8 — `raffle::Action` → `raffle::Command`

- `crates/core/src/raffle.rs` and `lifecycle()` (`app/src/server/service.rs:327`).

### Step 9 — "Attendee" → "participant" (independent)

- The leftover meetup wording: `crates/attendee` → `crates/participant` (`rafi-participant`),
  `app/src/server/attendee.rs`, UI text ("the attendee page"), the entrants' store module and
  live crate mentions. The phone-facing view is already the participant view
  (`/api/r/{slug}/participant`).

## Consequences

- **Published raffles stop verifying.** Their manifests and events are in the old shape, and they
  can't be converted: a new shape means a new hash, which no longer matches the published one.
  Accepted (breaking changes are fine). Development databases with frozen raffles are reset.
- **Restate**: the deployment is registered again under the new names. Before the first deploy no
  live `Raffle` objects exist; after it, a rename would mean draining every live drawing first.

## Out of scope

- One engine method per verb.
- Configurable pool rules or outcomes.
- A non-drand beacon source (only the tagged field leaves room for one).
- Moving to the generated Restate ingress client (step 5 prepares for it).
- Renaming the `restate-rafi*` crates.
- Renaming `rafi-core` to `rafi-raffle` (it only holds the raffle's lifecycle, codes and email
  normalization after step 1; can follow later). Done later: join codes moved into the app, and
  the crate is `rafi-raffle`. Email normalization moved too, then back into `rafi-raffle` as entrant
  identity (`entrant::Email`).

## What the implementation does differently

One commit per step, in this order (step 7 moved up, step 3 split in two):

| Step | Commit |
|---|---|
| 1 | `rafi-drawing: the drawing moves out of rafi-core` |
| 2 | `rafi-drawing: refusals in one error module` |
| 7 | `rafi: the raffle's items are prizes` — before step 3, so "item" never means two things in the app |
| 3 | `rafi-drawing: the modules are drawing and manifest` (files only), then `rafi-drawing: the drawing's vocabulary` |
| 4 | `rafi-drawing: the manifest names a beacon source` |
| 5 | `rafi: draws and decisions, and the Restate Drawing object` |
| 6 | `rafi: the audit log records commands by their verb` |
| 8 | `rafi-core: a raffle's lifecycle requests are commands` |
| 9 | `rafi: attendees are participants` |

Beyond the plan:

- **Items lost their prize reference in step 3**, not 4: renaming unit → item collided with the old `item_id`
  (the prize). Which prize an item came from is now `rafi_core::prize` (`item_id(prize, n)`, `prize_id(item)`,
  `items(prizes)`), the raffle's bookkeeping; the expansion left `rafi-drawing`.
- **Names the plan didn't fix:** `manifest::hash` (the free function), `Manifest::hash_hex`, the drawing's
  `id` (was `raffle_id`), `rafi_store::raffles::set_manifest` (was `commit`), the app's `write_manifest`,
  `DrawStore::save_drawing`, the app API's `draw_item` / `resolve_item` / `void_item`, and the Restate
  endpoint's policy key `services.drawing` (was `services.raffle`). Outside the drawing crate its error is
  imported as `Error as DrawingError`; inside the Restate object module the domain `Drawing` is `State`.
- **The migration (step 7) also renames** `raffles.draw_version` → `drawing_version`.
- **`Command` has no serde** any more (step 5): `DrawRequest` and `Decision` are the wire format; `Command` is
  only the app's and the tests' way to hold either (`From` both).
- **The verify report** names the `beacon` source instead of a chain string; the verify page and
  `verify.py` say "manifest hash".
- **The test vectors** were regenerated in steps 3 and 4 (the manifest changed shape twice); `verify.py`
  agrees with them.
- **Afterwards, offline draws were dropped** (`7b8721f`), superseding the decision that they stay the
  fallback: they were the one way to settle a draw nobody can verify. `DrawRequest` is just the item
  (step 5's `offline` flag is gone), as are `BeaconOutcome::Offline`, `DrawMode` and the `Draw` event's
  `mode`. There is no fallback when drand is down: a stalled draw stays pending on its round, a retry
  fetches the same round, and the only way out is to abandon it (logged; the verifier's
  `Report::abandoned` shows whom the round would have picked). The vectors lost their `mode` fields.
- **Afterwards, IDs got types:** `EntrantId` and `ItemId` (`rafi_drawing::id`), and the fields holding them
  dropped the suffix, in Rust and JSON alike: `Item.entrant`, `Event::Draw { item, entrant }`,
  `DrawRequest { item }`, `Decision::Resolve { item, .. }`. The type (or, in JSON, a string value) says it
  is an ID; SQL columns keep `_id` (`erasures.entrant_id`). The vectors were regenerated once more: only
  the event keys changed, the manifest and its hash did not.

Known, not caused by this work: the Restate conformance scenario `Lead/a_deadline_leaves_the_command_pending`
fails intermittently (it did before step 1), a 300 ms deadline racing the draw for the object's lock.

Before deploying: run migration 0003; register the Restate deployment again (the object is `Drawing`, the
service `DrawingEngine`; a drawing still open in a `Raffle` object is orphaned, so finish open raffles
first); `RAFI_ATTENDEE_DIR` is now `RAFI_PARTICIPANT_DIR` (the image sets it); rebuild the participant page
(`./scripts/participant.sh`). Raffles published before this change no longer verify.
