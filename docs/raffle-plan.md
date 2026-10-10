# Raffle plan — the drawing becomes the raffle

Repo: /Users/mark/Projects/ventures/rafi (branch `raffle`). Crates in `crates/`.
Follows [drawing-plan.md](drawing-plan.md) and [drawing-review.md](drawing-review.md), and renames what they named.
Breaking changes are fine (see "Consequences"). Done (2026-10-10); see "What the implementation does differently" at
the end.

## Goal

The split between the domain and the app stays; its seam gets the right names:

- **The raffle** (`rafi-raffle`, was `rafi-drawing`): a manifest plus an engine runs the whole raffle, item by item,
  to its finish, and anyone can verify it. It knows nothing of the app.
- **The lifecycle** (`rafi-lifecycle`, was `rafi-raffle`): what the app adds around a raffle. Its setup and settings,
  registration (who may join, when), freezing the entrant list into the manifest, and the status the app keeps
  (`RaffleState`), which follows the raffle once it is drawn.

drawing-plan.md drew the line between "the raffle" (the app) and "the drawing" (raffle-free). The line was right, the
names were not:

- **The core was always a raffle.** It holds raffle rules: one win per person, `Resolution` (claimed / not here /
  declined), redraws. It is not a general selection engine.
- **There is one per raffle, with the raffle's ID** (`rafi_raffle::drawing::drawing_id` is the identity). The
  core is the raffle from its manifest on, not a second thing the raffle owns.
- **"Drawing" read oddly**: a gerund (`RaffleState::Drawing`), a picture, and the word the UI doesn't use ("Start the
  draw").

## Vocabulary

| Layer | Term | Meaning | Was |
|---|---|---|---|
| Raffle | **Raffle** | The aggregate: opened from a manifest, drawn item by item, ends at finish | `Drawing` |
| Raffle | **RaffleId** | The raffle's ID, the same one the app uses | `DrawingId` |
| Raffle | **Manifest** | Entrants, items, algorithm and beacon source; its hash is published | (unchanged; field `drawing` → `raffle`) |
| Raffle | **Item** | One copy that one entrant wins (`mug-1`, `mug-2`) | (unchanged) |
| Raffle | **Draw** | Selecting the winner of one item, committed to a future drand round | (unchanged) |
| Raffle | **Entrant** | Someone on the list, who can be drawn | (unchanged) |
| Lifecycle | **RaffleState** | The app's status of a raffle; `Drawing` and `Finished` follow the raffle | (unchanged; crate was `rafi-raffle`) |
| Lifecycle | **Prize** | What can be won, with a quantity; becomes one item per copy | (unchanged) |
| Engine | **RaffleEngine**, **RaffleNotifier**, **RaffleStore** | The engine contract | `DrawingEngine`, `DrawingNotifier`, `DrawStore` |

In prose: "the raffle engine" (was "the drawing engine"), "a raffle is drawn item by item, one draw per item".

## Decisions

- **Raffle for the aggregate.** The others lose: *draw* is already one item's draw, and "a draw has draws" makes
  `CompletedDraw` and `DrawInProgress` ambiguous; *lottery* and *sweepstakes* are legal categories (paid entry; US
  free promotion); *ballot* is a UK allocation scheme but a vote in the US; *giveaway* is vague. Regulators say "the
  sequence of draws", "each draw" for one prize (AGCO raffle rules), which is what the code already says.
- **Item stays Item; Prize stays in the lifecycle.** A prize has a quantity and expands into one item per copy
  (`items()`); a core "prize" would make one prize three prizes. "Raffle items" is ordinary raffle language.
- **Draw stays the per-item word; round stays drand's.** "Round" appears ~300 times as the beacon round (a draw
  commits to a round, a stalled draw keeps its round), and no raffle source uses it for one prize.
- **The rules crate is `rafi-lifecycle`.** It is `RaffleState` and the rules that hang on it: who may join when
  (`entrant`), what may change when (`settings`, `prizes_editable`), and starting the draw (`start`). Not
  `rafi-rules` (the raffle's published rules text), `rafi-policy` (`App { policy }`) or `rafi-event` (the raffle's
  `Event`).
- **`RaffleState` keeps its name, and `RaffleState::Drawing` its name and labels** ("Drawing" / "Sorsolás"). With no
  `Drawing` type left, it reads as "being drawn". drawing-plan.md's accepted gap (the aggregate exists from `Frozen`)
  stays.
- **The engine hosts don't depend on the lifecycle.** `restate-worker` and `restate-rafi-endpoint` use
  `rafi_raffle::drawing::raffle_id` and nothing else from it; with `RaffleId` it is `as_str()`. `drawing_id` and
  `raffle_id` go.
- **The Restate object is `Raffle`.** drawing-plan.md renamed it `Raffle` → `Drawing` so that raffle vocabulary
  wouldn't leak into the drawing; now the core *is* the raffle, so it goes back. `EngineInfo` is unchanged.
- **The `raffles` row stores the raffle as `log`.** `raffles.drawing` → `raffles.log`, `drawing_version` →
  `log_version`. `raffles.raffle` says nothing; the stored form is the manifest and the event log, the part that
  grows. `version` stays the app's row version (the live channel's).
- **`algo` stays `rafi-draw-v1`.** It names the draw algorithm, which doesn't change.

## Manifest

Changed in place:

```json
{
  "algo": "rafi-draw-v1",
  "raffle": "r_8f3k2m",
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
| `drawing` | `raffle` | Binds the manifest to one raffle (its ID is the app's and the Restate key). |

Unchanged: field order, the exact serialized string published beside its hash, the seed formula and sampling. The
hash covers the field name, so every manifest's hash changes.

## Plan

Every step keeps green: the workspace tests, the engine conformance suite on both engines
(`crates/engine/tests/conformance.rs`, `crates/engine-restate/tests/conformance.rs`), the Restate e2e tests
(`crates/engine-restate/tests/e2e.rs`, `cargo test -p rafi --features server`), the independent verifier
(`tools/verify.py --vectors testdata/draw-vectors.json`) and the Workers clippy (wasm32). Run the Restate tests with
`RESTATE_SERVER_BIN=$(./scripts/restate-server.sh)`; without it they pass by skipping. One commit per step.

Steps 1 and 2 must stay in this order: the lifecycle crate moves off the `rafi-raffle` name before the core takes it.

### Step 1 — `rafi-raffle` → `rafi-lifecycle` (move only)

- `git mv crates/raffle crates/lifecycle`; package `rafi-lifecycle`; workspace `Cargo.toml`.
- `rafi_raffle::` → `rafi_lifecycle::` in its seven dependents (app, app-core, model, restate-worker, live, store,
  restate-rafi-endpoint) and the `-p` list in `.github/workflows/ci.yaml`.
- The `drawing` module keeps its name until step 6. No renames, no behavior change.

### Step 2 — `rafi-drawing` → `rafi-raffle` (move only)

- `git mv crates/drawing crates/raffle`; package `rafi-raffle`; `src/drawing.rs` → `src/raffle.rs` (module only;
  the type stays `Drawing` until step 3).
- `rafi_drawing::` → `rafi_raffle::` everywhere (the engines, store, live, platform, drand, app incl. the WASM
  verifier, app-core, model, cloudflare-check, lifecycle); CI's `-p` list.

### Step 3 — The raffle's vocabulary

Inside `rafi-raffle` and its callers:

- Types: `Drawing` → `Raffle`, `DrawingId` → `RaffleId`, `StoredDrawing` → `StoredRaffle`; the `Error as
  DrawingError` aliases → `Error as RaffleError`.
- Variables and fields that hold the aggregate: `drawing` → `raffle`, `drawing_id` → `raffle_id`. Where `raffle`
  already names the app's row or ID (lifecycle, store, app-core), the row is `row`.
- Error messages ("the raffle is already open with another manifest", "the raffle is finished", "… draws from
  {raffle}, but the engine from {engine}"), crate docs (`lib.rs`, `Cargo.toml` description: "a verifiable raffle of
  a fixed set of items among a fixed list of entrants … knows nothing of the app"), test names
  (`a_raffle_restores_from_its_log`, …).
- Per-item names stay: `DrawRequest`, `PendingDraw`, `CompletedDraw`, `AbandonedDraw`, `DrawCheck`,
  `DrawInProgress`.

### Step 4 — Manifest field `drawing` → `raffle`

- `Manifest`'s field and its canonical JSON; `manifest.rs` tests; the `"drawing-vectors"` ID in
  `tests/vectors.rs` → `"raffle-vectors"`.
- Regenerate `testdata/draw-vectors.json` (`UPDATE_VECTORS=1`); `tools/verify.py` must agree without changes (it
  hashes the published string).

### Step 5 — The engine contract and Restate

- `rafi-engine`: `DrawingEngine` → `RaffleEngine`, `DrawingNotifier` → `RaffleNotifier`, `DrawStore` →
  `RaffleStore` (method `save_drawing` → `save_raffle`), `MemoryDrawStore` → `MemoryRaffleStore`; test names
  (`a_finished_raffle_is_forgotten`, …).
- `rafi-engine-restate`: the object `Drawing` → `Raffle` (`SERVICE_NAME`, `#[restate_sdk::object] name`),
  `DrawingIngressClient` → `RaffleIngressClient`, the call paths in errors (`Raffle/{id}/open`, …), the
  conformance test's name.
- `rafi-platform`: `Notify` as the engine's `RaffleNotifier`.

### Step 6 — The lifecycle meets the raffle

- `rafi_lifecycle::drawing` → `rafi_lifecycle::raffle`: prizes → items (`items`, `item_id`, `prize_id`), the
  `manifest`, and `lifecycle(&Raffle) -> RaffleState`.
- `drawing_id` / `raffle_id` go: callers use `RaffleId::new(id)` and `id.as_str()` (the hosts hand
  `Notify::new` `RaffleId::as_str`, which says which raffle's live channel to ping).
- `RaffleState::follows_drawing` → `follows_raffle`; tests (`the_lifecycle_follows_the_raffle`,
  `commands_leave_drawing_and_finishing_to_the_raffle`).
- `restate-worker` and `restate-rafi-endpoint` drop `rafi-lifecycle` from their dependencies.

### Step 7 — Store and SQL

- Migration `0008_raffle_log.sql` (SQLite/D1 and `migrations/postgres`): `raffles.drawing` → `log`,
  `drawing_version` → `log_version`.
- `rafi-store`: `save_drawing` → `save_raffle`, `DrawWriter` → `RaffleWriter`, `parse_drawing` → `parse_raffle`,
  `RaffleRow.drawing` / `drawing_version` → `log` / `log_version`, `RaffleRow::drawing()` → `raffle()`, the live
  snapshot's `drawing` → `raffle`; the SQL in `rafi-cloudflare-check` and `rafi-store-d1-check`.
- The engine's write says what it writes: "the raffle's log, after every change".

### Step 8 — App, app-core and live

- Local names and imports; `drawing_refusal` → `raffle_refusal`. `Refusal::NotDrawing` stays (it is the
  `Drawing` state).
- Copy: "the drawing engine" → "the raffle engine" (app-core's errors, the admin's retry button, the privacy notice
  in `server/participant.rs`). The `drawing` state label stays.

### Step 9 — Docs

- `PLAN.md` (vocabulary, §8.1 layers, the engine section), `README.md`, `crates/engine-restate/README.md`.
- A historical-record note at the top of drawing-plan.md, drawing-review.md, engine-plan.md, workers-plan.md and
  drand-crate.md: names are of their time; `rafi-drawing` is now `rafi-raffle`, the old `rafi-raffle` is
  `rafi-lifecycle`.

## Consequences

- **Frozen raffles stop verifying.** Their manifests name `drawing`, and a renamed field means a new hash that no
  longer matches the published one. Accepted; development databases with frozen raffles are reset (a stored log in
  the old shape fails to load, as `a_drawing_that_does_not_load_is_an_error` expects).
- **Restate**: the deployment is registered again under the new object name. Live `Drawing` objects would have to
  be drained first; before the first deploy there are none.
- **History**: `crates/raffle` holds a different crate before and after steps 1–2. `git log --follow` on
  `crates/raffle/src/raffle.rs` leads back to `drawing.rs`; the old rules' history is under `crates/lifecycle`.

## Out of scope

- Renaming `RaffleState` or its `Drawing` state.
- Storing only the events in `raffles.log` (the manifest is in it and in `raffles.manifest`).
- Renaming `rafi-engine`, `rafi-engine-restate`, or `algo`.
- Changing what any rule does.

## What the implementation does differently

One commit per step, in the plan's order:

| Step | Commit |
|---|---|
| 1 | `f8c973f` rafi-lifecycle: the raffle's rules move off the rafi-raffle name |
| 2 | `3c63a9b` rafi-raffle: the drawing crate takes the raffle's name |
| 3 | `f762169` rafi-raffle: the drawing is a Raffle |
| 4 | `621d66e` rafi-raffle: the manifest names its raffle |
| 5 | `9140526` rafi-engine: the raffle engine, and the Restate object is Raffle |
| 6 | `c8b2230` rafi-lifecycle: the lifecycle follows the raffle |
| 7 | `add0522` rafi-store: the raffles row stores the raffle as its log |
| 8 | `91386e0` rafi: the app says raffle engine |
| 9 | docs: the raffle is the raffle, everywhere |

- **Step 3 renames the callers' types only**; their variables and prose follow in the engine, lifecycle, store and app
  steps. `rafi-live` had its own `raffle::Raffle` (what a participant view is built from): it is `raffle::Input`, its
  `drawing` field `raffle`.
- **Step 5 takes more along:** `save_drawing` → `save_raffle` everywhere, the store's own function included (planned
  for step 7); the endpoint's policy section `[services.drawing]` → `[services.raffle]`;
  `DatabaseConfig::draw_store` → `raffle_store`; the Worker's e2e script calls `/Raffle/...`.
- **Step 6:** app-core imported `rafi-lifecycle` as `raffle`, which would read `raffle::raffle::items`; it imports it
  as `lifecycle`. The engine hosts drop their direct `rafi-lifecycle` dependency, but still build it through
  `rafi-store`, whose engine write sets the lifecycle state the raffle leads to.
- **Step 7: `RaffleRow::drawing()` is `drawn()`, not `raffle()`.** Callers hold the row as `raffle`, and
  `raffle.raffle()` says nothing; `drawn` is "the raffle as drawn, once opened". The live `Snapshot`'s field and
  app-core's locals are `drawn` too. The row's columns are `log` / `log_version` as planned.
- **Step 8:** `Refusal::Drawing` (the raffle's rules refuse a command) is `Refusal::Raffle`.
- **Checked on every backend:** every step passed the CI's test job locally (with the Restate tests against
  `restate-server` and the storage suite on a local Postgres); the end state also passed `scripts/workers-check.sh`
  (D1, the live channel, a draw through Restate on the Worker, an evening on the app Worker).
