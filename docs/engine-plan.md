# Rafi reorganization plan — draw engine as a seam (v3)

Repo: /Users/mark/Projects/ventures/rafi (branch `cloudflare`). Crates in `crates/`.
v2 incorporated the round-1 reviews; v3 incorporates rounds 2–3. FINAL: signed off by the
architect, engineer, domain-expert and test/platform reviewers. See "Changes from v2" at the end.

## Goal (from the owner)

Restate is an implementation detail of "the draw engine". Business logic lives centrally, reusable
by Restate and a local runner, so validating the app and the business logic gets much easier.
drand possibly an implementation detail. Components: app (admin + frontend), possibly a separate
participant app, the Restate implementation, store implementations, glue.

## Current state (verified by reviewers)

- `rafi-core`: pure domain — `DrawState` state machine (`state.rs`), rafi-draw-v1 (`draw.rs`),
  `CommitDoc` (`commit.rs`), verifier (`verify.rs`, also WASM), `Command` (`engine.rs`), drand
  vocabulary + round rule (`drand.rs`).
- The draw protocol around it exists three times: `restate-rafi/src/service.rs` (real), 
  `app/src/server/fake_engine.rs` (HTTP imitation of the ingress; already diverges: 409 vs 400 codes,
  drops the per-raffle lock while awaiting the beacon), `app/src/server/service.rs:433-502` (app
  pre-validation; also misses "void the pending unit", which today comes back as a 500).
- App↔engine seam is HTTP: concrete `RestateClient` (`app/src/server/engine.rs`), stringly-typed
  handler paths; every engine refusal is flattened into `anyhow` → `AppError::Internal` (500).
- The in-process engine was removed in `231cc13` (stateless server). `docs/PLAN.md` §6 still promises
  an organizer-facing "local draw" fallback; `core/src/state.rs:3-4` and `core/src/engine.rs:1-3`
  still mention a "local fallback engine".
- App uses drand config for: `CommitDoc.drand_chain` (`service.rs` `commit()`), the mock label
  (`view.rs:190` `is_mock()` → `model.rs:45`), the mock-only-in-dev guards (`main.rs:15`,
  `worker.rs:39`), features `mock-drand`/`worker-dev`.
- Owner constraint: Restate for draw durability; Durable Objects only for WS fan-out.

## Decisions

- **drand stays domain vocabulary.** The chain is part of the commitment; events record rounds; the
  verifier (and `tools/verify.py`) key on the chain. No generic "randomness source" seam (one
  verifiable adapter = hypothetical seam). Fetching/relays/lead/mock remain engine internals.
- **The organizer's app decides the chain, never the engine.** The engine checks; it does not choose.
- **The local runner is never a production engine.** The "local draw" fallback in PLAN.md §6 is
  dropped explicitly (two engines owning one raffle = round-shopping risk; Workers can't sleep a draw;
  conflicts with "Restate for draws"). Offline draws (server randomness, flagged unverifiable) were
  kept as the organizer's fallback when drand is unavailable; they were dropped later (see
  "Implementation status").

## Plan

### Step A — Protocol rules into `rafi-core` (pure)

New/changed in `rafi-core`:
- `DrawError` new variants: `NotFrozen` (→ NotFound), `AlreadyFrozen` (different list → Conflict),
  `ChainMismatch { committed, engine }` (→ Conflict). `WrongChain` keeps its meaning (beacon from
  another chain; ends in a stall).
- `ErrorKind { NotFound, BadRequest, Conflict, Invariant }` via `DrawError::kind()`. `WrongRound`,
  `WrongChain`, `BadListHash` are `Invariant` (engine bug / stall path), not caller errors. One table,
  used by every adapter to encode and by the client to decode.
- `CommitDoc::validate()`: `algo == ALGO`; ≥1 entrant and ≥1 unit; entrant IDs sorted, deduplicated,
  lowercase (re-serialized list hash == published hash). Today the non-empty checks exist only in
  the app.
- Unit expansion (items × qty → `UnitRef` with id `{item}-{n}`, now `service.rs` `units()`) moves
  next to `CommitDoc` — it is part of what gets committed.
- `CommitDoc::freeze_key()` (idempotency key from the list hash; today built inside the client).
- `DrawState::check_chain(engine_chain) -> Result<(), DrawError>` — used on freeze AND on every draw.
- `DrawState::freeze_or_check(existing: Option<&DrawState>, doc, engine_chain) -> Result<Option<DrawState>, DrawError>`:
  validate doc, check chain, same list → `None` (no-op), different list → `AlreadyFrozen`.
  The object-key ↔ `raffle_id` check stays in the Restate adapter (it is Restate's key).
- `DrawState::conclude(BeaconOutcome) -> Result<Option<Drawn>, DrawError>` (named to avoid the
  existing private `settle(pending, randomness, signature, mode)` in `state.rs`) with
  `BeaconOutcome = Verified(VerifiedBeacon) | Offline(Hash) | Unavailable(String) | Cancelled(String)`:
  complete, complete offline, or stall (an invalid beacon stalls, never abandons; the pending draw
  keeps its round). Adapters only do I/O: journal clock → `begin(unit, round, round_time)` → project
  the Announce → sleep/fetch or produce offline randomness → `conclude` → project.
- `DrawState::apply(cmd) -> Result<Applied, DrawError>` for Resolve/Void/Abandon/Finish; `Draw` is
  rejected (BadRequest); `Applied` tells the adapter the raffle finished (so it can clear state).
- `DrawState::stalled() -> bool` (pending draw whose latest event is its `Stall`): a read-only helper
  for the app's dry run. Engines need no special case: core's "refuse while `pending` is set" plus
  serialization already yields "refused only when stalled".
- Contract after finish: the engine forgets the raffle; further commands → `NotFrozen`. A freeze
  replayed after finish (beyond the idempotency window) re-creates engine state but must never
  change the published projection: the store's highest-version guard ensures it, pinned by a
  scenario (tombstone in Restate state is the stricter alternative; not needed now).
- No full sans-IO step machine: Restate must journal each effect, so an effect-per-arm interpreter
  adds indirection without removing code. `conclude` holds every decision.
- App pre-validation (`service.rs` `command`) becomes a dry run (`begin(unit,0,0)` / `apply`) on a
  clone of the projected state, plus one `(Command, DrawError) → friendly message` table in the app
  (keeps copy like "not here / declined could change who it can pick"). Fixes the void-pending 500.
  The dry run's dummy round never leaves the clone. While a draw is **actively awaiting its beacon**
  (pending, not stalled) the app accepts only a re-draw of the same unit and `Resolve(Claimed)`;
  abandon, void, finish and pool-changing resolves are offered only once the draw has settled or
  stalled (matches the UI, which shows retry/offline/abandon on a stall). This refusal is the app's;
  the engine itself serializes instead (see the contract).
- `ErrorKind::Invariant` maps to 500 and is logged as an engine bug, never shown as a conflict. The
  Restate service sends it as a terminal 500 and the client decodes 500 back to `Invariant`; the
  core table covers all four kinds in both directions.
- Update the stale "local fallback engine" comments.
- Optional nit (separate commit): rename `Event::Fail` → `Event::Abandon` with
  `#[serde(rename = "fail")]` (published logs and `tools/verify.py` depend on `"fail"`).

Restate service and `fake_engine.rs` switch to these functions in the same step (green).

### Step B — `DrawEngine` port in `rafi-platform`

In `crates/platform` next to `DrawStore`/`Notifier`, same `BoxFuture` conventions (no cycle: the app,
the client crate, the local runner and the conformance suite all depend on platform; platform
already depends on core, adds no BLS weight to `rafi-live`):

```rust
pub trait DrawEngine: Send + Sync + 'static {
    fn info<'a>(&'a self) -> BoxFuture<'a, Result<EngineInfo, EngineError>>; // replaces healthy()
    fn freeze<'a>(&'a self, doc: &'a CommitDoc) -> BoxFuture<'a, Result<(), EngineError>>;
    fn command<'a>(&'a self, raffle_id: &'a str, cmd: &'a Command, command_id: &'a str)
        -> BoxFuture<'a, Result<(), EngineError>>;
}
pub struct EngineInfo { pub chain: ChainHash }
pub enum EngineError { Rejected { kind: ErrorKind, message: String }, Pending, Unavailable(anyhow::Error) }
```

Documented contract:
- **Freeze** is synchronous and idempotent per `CommitDoc::freeze_key()`.
- **Draw is accepted asynchronously**: `command(Draw)` returns when accepted, not when settled; a
  draw refused inside the engine is silent (only the projection shows progress). Hence the app's
  pre-validation is load-bearing, not cosmetic.
- **Serialization**: commands for one raffle apply one at a time; a command sent while a draw awaits
  its beacon is queued and applies after the draw settles (virtual-object semantics), against the
  settled state. The engine therefore refuses pool-changing commands only while a *stalled* draw is
  pending; during an active wait it queues them.
- **Blocking bound**: a synchronous command (freeze, resolve/void/abandon/finish) may wait up to
  `lead_secs + timeout_secs` (+ margin) behind an in-flight draw. If the adapter's own deadline
  elapses first it returns `EngineError::Pending` ("accepted; outcome via the projection"), not
  `Unavailable`; the app answers 202 and the UI follows the projection. Retrying is safe (same
  `command_id` → applied once). The Restate client's native timeout is set above the bound.
- **`Unavailable` does not mean "not applied"**: the engine may still apply the command later;
  retrying with the same `command_id` is safe.
- **Idempotency**: same `command_id` within the retention window (Restate: 1h) → applied once.
- **State is read from the projection** (`DrawStore`), never from the engine. `RestateClient::get`
  (test-only today) goes away.
- `AppState.restate: Option<RestateClient>` → `engine: Option<Arc<dyn DrawEngine>>`, injected through
  `AppState::from_parts` (callers: `worker.rs:58`, `tests.rs:50`, `tests.rs:1530`) instead of built
  from `cfg.restate`;
  `service::send` maps `Rejected{kind}` to 404/409/400/500, `Pending` to 202, `Unavailable` to 503.

Landing: trait + `RestateClient` implementing it while still in the app; tests still use the HTTP fake.

### Step C — Local runner crate `rafi-engine-local` (native, test/dev only)

- Native-only (`cfg(not(target_arch = "wasm32"))`), tokio: per-raffle `tokio::Mutex` **held across
  the beacon wait**, `spawn` for async draws, dedup by `command_id`, writes via `DrawStore`
  (`rafi_store::DrawWriter` in app tests), notifies via `Notifier`.
  **Arrival order is preserved**: `command(Draw)` acquires the per-raffle lock
  (`Arc<tokio::Mutex<()>>::lock_owned().await`) and records the `command_id` *before* returning, then
  moves the guard into the spawned task, so a command sent right after an accepted draw always runs
  after it (as Restate's per-key queue does).
- Uses `DrandConfig::validate` and `rafi_core::drand::target_round` — no ad-hoc clock math
  (`fake_engine.rs:192` reading `ids::now()` is not carried over). Its clock and randomness are
  injected (the app's `ids::*` is app-internal).
- Only ever a **dev-dependency** (app tests, conformance suite). No production crate depends on it,
  so it cannot ship in a release build.
- Replace `fake_engine.rs`: switch the `app_with_live` helper (`tests.rs:39-50`), delete the fake.
  Expected churn: setup helper, `unreachable_drand_app`, `draws_need_restate`, `stalled_draw_keeps_its_round`
  (asserts "handed over doesn't wait", which real Restate never does — rewrite to the serialization
  contract). The app-side "Claimed during pending" rule (`service.rs:470`) is documented as UX only.
- drand fault modes are a **local fake relay** (an HTTP test fixture serving the mock chain) selected
  through the configurable relay URLs, not code inside `rafi-drand`: `restate_drand` takes the
  external `drand::Client`, so only the relay is a seam both engines share. `Withheld` (fails
  immediately; stalls take ~1s instead of lead + timeout) and `Forged` (serves a beacon from another
  chain or round; it will most likely fail BLS verification inside the client and stall as
  "unavailable", which is all scenario 3 asserts).

### Step D — Restate client crate `restate-rafi-client`

- Tiny crate: reqwest + core + platform; implements `DrawEngine` over the ingress (URL building,
  `/send` for draws, `idempotency-key`, bearer auth, `{"message"}` + status → `ErrorKind` decoding via
  the core table). Exports handler-name constants; `restate-rafi` tests assert them.
- Not a feature of `restate-rafi` (its restate-sdk/restate-drand/rafi-drand/getrandom deps are
  non-optional; the SDK must stay out of the app's Worker bundle).
- App depends on it `optional` under `server` (the app also builds as the browser bundle).
- `restate-rafi` gains a keyless `RaffleEngine` service with an `info` handler (`{chain}`), replacing
  the `/restate/health` probe. Both `restate-rafi-endpoint` and `restate-worker/src/lib.rs` bind it.
  Deploy checklist: deploy the engine, re-register the Restate (Cloud) deployment so the new service
  is discovered, then deploy the app that calls it.

### Step E — Engine conformance suite

In `rafi-engine-local` behind a `conformance` feature (mirrors `rafi-store::conformance`):

```rust
trait EngineHarness { fn name(&self) -> &str; fn capabilities(&self) -> Capabilities;
    async fn fresh(&self, drand: DrandProfile) -> Result<Fixture, String>; }
struct Fixture { engine: Arc<dyn DrawEngine>, store: MemoryDrawStore, notifier: Arc<Recording> }
enum DrandProfile { Instant, Lead(u64), Withheld, Forged }
```

- Harnesses: local runner (always, in `rafi-engine-local`'s own tests); Restate in
  `restate-rafi/tests` (dev-depends on `rafi-engine-local[conformance]`, `restate-rafi-client` and
  `restate-e2e-harness`; keeps `rafi-engine-local` free of Restate deps), **driven through
  `restate-rafi-client`** so the client is exercised by every scenario. One `restate-server` per
  **(DrandProfile, store behaviour)** pair — `Raffle::new` fixes drand, store and notifier, and a
  second deployment of the same `Raffle`/`RaffleEngine` services on one server takes over new
  invocations — shared by the scenarios that use that pair and isolated by unique raffle keys
  (about five servers: Instant, Lead, Withheld, Forged, Instant+flaky store).
- Scenarios observe the projection (poll, like `projected()` in `restate-rafi/tests/e2e.rs`), never
  return values for draws. Must cover at least:
  1. the Announce is in the store before the relay is asked for the round;
  2. relay withholding → stall; re-draw after the clock moved fetches the **same** round;
  3. a beacon from the wrong chain or wrong round (`Forged`) → stall;
  4. offline draw records the replaced round and is flagged unverifiable;
  5. abandon emits the public abandon (`fail`) event;
  6. pool-changing resolve/void/finish refused while a **stalled** draw is pending (no invocation in
     flight);
  7. a command sent while a draw awaits its beacon applies after it settles, against the settled
     state (`Lead(1–2s)`); variant: with an adapter deadline shorter than the wait it returns
     `Pending`, and the command still applies exactly once;
  8. under `Lead(≥ period)`, the Announce's `round_time` is after the draw was requested (any chain);
     (that `restate_drand::commit_round` and `rafi_core::drand::target_round` pick the same round for
     a given time is a unit test with an injected clock, not a scenario: Restate's clock is real);
  9. same `command_id` twice → applied once; freeze with the same list → no-op; different list → Conflict;
     chain mismatch → Conflict;
  10. each `DrawError` surfaces with its `ErrorKind` through the port;
  11. a refused draw produces no new projection (silent), asserted with a bounded wait plus a
      follow-up command as a barrier, not a bare sleep;
  12. after finish, commands → NotFound; a freeze replayed after finish never changes the published
      projection;
  13. a flaky `DrawStore` (fails first N writes) still projects every version (at-least-once), gated
      by a capability if the local runner doesn't retry;
  14. every scenario's final event log passes `rafi_core::verify::verify`.
- Restate-only checks (journal step names, cancel → stall, replay) stay in `restate-rafi/tests/e2e.rs`
  or behind `Capabilities`.
- Keep `app/src/server/tests_restate.rs` as the single app↔Restate end-to-end test (projection into
  SQLite/Postgres via `DrawWriter`); don't fold it into the suite.
- Runs in the existing `test` job. Optional later: a third harness against `restate-worker` under
  `wrangler dev` (already wired in `workers-check.sh`).

### Step F — Chain policy; app drops `rafi-drand`

- App config: `RAFI_DRAND_CHAIN` (hash string). Policy in the app: release builds accept only public
  chains known to `rafi_core::drand::chains::by_hash`; the mock chain only in dev builds (the existing
  `worker-dev`/`mock-drand` guard pattern is kept). In dev builds `RAFI_DRAND_CHAIN=mock` (and the
  existing `RAFI_DRAND_MOCK=1`, kept as an alias) resolve to the mock chain's hash: `mock-drand`
  enables an optional `rafi-drand` dependency (`mock`) used only for `mock_chain()`'s hash.
  Release builds reject both the literal `mock` alias and the mock hash (same guard).
- The core refusals (`resolve`/`void`/`finish` while `pending` is set) stay as the backstop even
  though serialization means they only fire for a stalled draw in practice.
- `start_draw`: `engine.info()` (same round-trip as today's health check) → if `info.chain` ≠ the
  configured chain, refuse **before** freezing/committing, with a clear error. The engine still
  refuses mismatches at freeze and at every draw.
- Mock label per raffle from the committed chain: parse `doc.drand_chain` as `ChainHash`, then
  `chains::by_hash(..).is_none()`; a parse failure counts as an unknown (unverifiable) chain
  (`view.rs:190`), not from config.
- `Config::for_tests` sets the mock chain's hash via the dev-dependency `rafi_drand::mock_chain()`.
- `rafi-drand` leaves the app's release dependency graph (dev-dependency + optional under `mock-drand`).

### Step G — Participant split (deferred, with triggers)

Not now: bundles are 847 KB / 914 KB compressed (`workers-plan.md:440`), nothing is deployed.
Triggers: hitting the Worker size limit, phone cold-start latency, or wanting Clerk isolated from the
public surface. When triggered:
- Split by **audience and data class**: public PII-free (projector `/s/*`, phone, `/v/*` verify) vs
  organizer (admin, emails, every entrant's claim code; only a winner's own phone sees its own code).
  The PII-free snapshot builder is one shared function whichever Worker serves it.
- Participant routes: `/j/*`, `/api/j/*`, `/rules/*`, `/privacy/*`, `/api/r/*/participant`,
  `/api/altcha`, `/partyline/*`, plus attendee assets in its own `[assets]`.
- Prerequisite: extract `i18n`, `icons`, `style`, the participant `model` types and `service::join`
  into a shared crate. The join-only-while-open guard stays in `rafi-store` (`entrants.rs:133`).
- Routing: both Workers have `workers_dev=false`; a Custom Domain binds the whole hostname, so a path
  split needs zone Routes (verify precedence) or a front Worker.
- `RaffleChannel` DO stays defined in `rafi`; participant Worker binds it with `script_name`
  (per env), so `rafi` deploys first. `JOIN_*` and `CONNECT_LIMITER` rate limits move with it.
- Separate fail-closed config (ALTCHA key, no Clerk/Restate). D1 has no per-table permissions:
  least privilege is enforced in code: the participant crate gets a store API whose types expose no
  email or claim-code queries.
- Native/Docker: one binary still mounts both routers.

### Step H — Store

Already split by feature/target with a conformance suite; no change.

## Sequencing (each step lands green)

A (core + new variants; Restate service, fake, app pre-check use them) → B (port in platform;
`RestateClient` implements it in-app; `AppState` switched) → C (local runner; delete fake) →
D (client crate + `RaffleEngine/info`) → E (conformance suite) → F (chain policy, drop `rafi-drand`
from app; after D is deployed) → docs (PLAN.md §6 fallback removed) → G only on trigger.

## Changes from v1

- Port moved from the app to `rafi-platform` (v1 created a crate cycle); typed `EngineError`; written contract
  (async draw, serialization, idempotency, state from projection).
- Core cut extended to the draw path (`settle`, `check_chain`), doc validation, unit expansion,
  `freeze_key`, new error variants and `Invariant` kind; explicit "no sans-IO machine".
- Engine no longer *decides* the chain (v1 §4 was a trust regression): app config + policy, engine
  `info()` used only as a pre-freeze check; mock label from the committed chain.
- Restate client is its own crate, not a `restate-rafi` feature.
- Local runner: its own crate, dev-dependency only, holds the lock across the beacon wait, uses the core round rule;
  "local draw" fallback explicitly dropped.
- Conformance: concrete harness, 14 fairness/contract scenarios, Restate side through the client,
  cost controls (withheld mock, shared server); `tests_restate.rs` kept.
- Participant split deferred with triggers; facts corrected (routes, DO binding, rate limits, config,
  routing, prerequisites); split by audience/data class.

## Changes from v2

- Scenarios 6/7 reconciled with serialization: the engine refuses pool-changing commands only for a
  stalled pending draw and queues them during an active wait; the app's dry run refuses them during
  an active wait (UX). `DrawState::stalled()` added.
- Blocking bound written into the contract; `EngineError::Pending` (→ 202) on adapter deadline;
  client timeout above `lead + timeout`; scenario 7 variant.
- One `restate-server` per (DrandProfile, store behaviour) pair (not one deployment per profile on a
  shared server); covers scenario 13's flaky store.
- `settle` → `conclude` (name clash); "`Unavailable` ≠ not applied"; `Invariant` ↔ 500 both ways;
  engine injected via `AppState::from_parts`; chain parse failure = unknown; test fixture chain.
- drand fault modes (`Withheld`, `Forged`) as a local fake relay over HTTP (relay URL config), so they
  reach `restate_drand` too.
- Scenario 8 split: lead assertion in the suite, round-rule agreement as a unit test.
- Scenario 11 barrier; scenario 12 freeze-after-finish; `Invariant` → 500; dummy round never escapes.
- `RaffleEngine` bound in both endpoints + re-registration in the deploy checklist; Restate harness
  lives in `restate-rafi/tests`; `EngineInfo.chain: ChainHash`; dev mock chain alias.
- Participant store API typed for least privilege.

## Implementation notes (from round-3 reviews, non-blocking)

- On wasm32 reqwest has no timeouts (`engine.rs:31`): the Restate client races the call against a
  timer (`worker::Delay`) on Workers so `Pending` works there too; until then that path is native-only
  and a Workers call waits out the bound. Check Restate Cloud's
  ingress request timeout against `lead + timeout_secs`.
- The flaky `DrawStore` fails the first N writes **per raffle ID**, so scenarios sharing its server
  don't interfere.
- The local runner takes the same configurable deadline as the Restate client (scenario 7 variant
  runs on both).
- `freeze` returning `Pending` leaves the raffle awaiting confirmation; the UI follows the projection
  (the existing "press Start the draw again" copy covers it).
- Round 3 (architect): local runner preserves arrival order (lock + dedup taken before `command(Draw)`
  returns); `stalled()` is an app-only helper.
- Round 3 (engineer):
  - The local runner's `Pending` must not cancel the command: spawn the apply as a task and put the
    deadline on its `JoinHandle`, never `tokio::time::timeout` around the apply.
  - After Step F the app can't derive the engine's bound: the client deadline is a client config
    value (`RESTATE_CLIENT_TIMEOUT`), documented as ≥ the engine's `lead_secs + timeout_secs`
    (default 6 + 60). Too short only degrades to `Pending`.
  - `stalled()` can't see a retry in progress (re-entering `begin` emits no event/version), so the
    app's dry run lets abandon/void/finish through during a retry and the engine queues them safely.
    Documented, not fixed with a new event (would change the published log format). Same ambiguity
    as `ui/admin.rs:763` today.
  - After a 202 the UI waits on the projection with its own limit and message (a `Pending` from a
    deadline looks like an engine stuck retrying `project`).
  - `RaffleEngine/info` sends the chain as a hex string; the client parses it into `ChainHash`
    (wire format independent of drand's serde).

## Implementation status (2026-10-09)

Steps A–F are implemented; G (participant split) stays deferred until a trigger, H needed nothing.
Where the implementation differs from the plan:

- **Local runner ordering:** instead of taking the per-raffle lock before `command(Draw)` returns,
  `LocalEngine` keeps one queue per raffle, worked off by one task. Arrival order is the queue's
  order, an accepted draw returns at once (like `/send`), and a caller's deadline can never cancel a
  queued command (the task owns it). Commands are deduplicated by ID before they are queued.
- **`BeaconOutcome::Cancelled` is `Interrupted`** (cancelled, or a journaled step failed), and
  `DrawState::apply` returns `()`: `state.finished` tells the engine to forget the raffle.
- **`EngineError::Pending`** makes the app's call succeed (the command applies once; the screens
  follow the projection) rather than a distinct 202. `EngineError::Unavailable` is
  `AppError::Unavailable` (503). The client counts a reply as a refusal only when the ingress marks
  it `x-restate-error-source: invocation`: the ingress's own errors (say, an unregistered
  `RaffleEngine`) are `Unavailable`, never "not found".
- **drand fault profiles:** `Withheld` is quicknet behind an unreachable relay with a 3 s lead (a
  stall takes about 4 s); `Forged` is quicknet behind an in-process relay serving beacons signed by
  the mock key. Both reach `restate_drand` because they are relays.
- **Scenario 8:** the suite checks the announced round's `round_time` is not before the draw was
  requested. `restate_drand`'s clock context is sealed, so the round-rule agreement is a unit test in
  `rafi-drand` that restates its formula (`round_at(now_ms + lead)`) against `target_round`.
- **Freeze after finish:** the scenario replays the freeze within Restate's idempotency window (it
  can't wait an hour); beyond it, the store's highest-version guard is what keeps the record.
- **Deadline on wasm32:** not raced yet; on Workers a call waits out the bound.
- **Config:** the client timeout is `RESTATE_TIMEOUT_SECS` (default 90).
- `Event::Fail` is `Event::Abandon`, still `"fail"` on the wire.
- **Offline draws were dropped later** (`7b8721f`): no `BeaconOutcome::Offline`, no scenario 4, no
  offline branch in either engine. When drand is down a draw stalls on its round; a retry fetches the
  same round and the organizer's only way out is to abandon it (logged; the verifier shows whom the
  round would have picked).
