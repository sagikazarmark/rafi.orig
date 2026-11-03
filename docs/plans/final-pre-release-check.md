# Final pre-release check

**Status (2026-11-02): Tiers 0 and 1 done; Tier 2's repository side done; [the next run](#the-next-run) done, but
what [What it left](#what-it-left) lists; `0001_init.sql` frozen; the open items are [tickets](#tickets).**
Findings done before the next run say what shipped under **Done**; the next run's are summarized under it. Before
the tag: Tier 2's GitHub and Cloudflare side (the rename, a dispatch dry run and `v0.1.0-rc.1`, a real deploy, a
stranger's deploy, the attestation, the ruleset, the screenshot), the maintainer's.

### Progress

| Step | State |
|---|---|
| 1. Tier 0 | **Done** (`cf56888`). `0001_init.sql` gained `draws.delivery` and `add_batches`. Verified: fmt, `just clippy` (native and wasm32), `just test` with `RESTATE_SERVER_BIN`, `just doc`, `just verify`, `just deny`, the D1 conformance testbed (every scenario ok). `just workers` ran later, with Tier 2 (the evening on another port): every check ok, so `evening.mjs` exercised the stricter admin API. |
| 2. Tier 1 | **Done**, with the `evening.mjs` parts of the Accepts (the countdowns sampled between fetches, `/view` blocked, a projector at an unknown slug), `just workers` green. Verified: fmt, `just clippy`, `just test` with `RESTATE_SERVER_BIN`, `just doc`, `just verify`, `just deny`; the D1 conformance testbed, the live channel's scenario and the participant smoke (steps 1–3 of `workers-check.sh`), every check ok. |
| 3. Schema and query shapes | **Done**: T3-1, T3-7, T7-6's `erasures` index, T3-9's manifest row measured (natively) and kept. The `EXPLAIN` gate built (T3-11 7); the `rows_read` counter not ([#54](https://github.com/sagikazarmark/rafi.orig/issues/54)). |
| 4. Contract notes | **Done**: T7-1, T0-8's `format` check, the retention-promise decision |
| 5. Launch mechanics | **Done locally** (2026-11-02): T2-2, T2-4 … T2-8 as far as the repository goes; T2-1, T2-3, the stranger, the GitHub runs and settings open. Declined: release-note labels and templates, community files, third-party notices, Python in the release. Verified: fmt, `just clippy`, `just test` with `RESTATE_SERVER_BIN`, `just doc`, `just verify`, `just deny`, `just workers` (the evening on `WORKERS_APP_PORT=8797`, every check ok), two `cloudflare-release.sh 0.1.0` builds byte-identical. |
| 6. Design and UX | **Done** in the next run: T5-1 … T5-7, T6-1 … T6-6 (with what [What it left](#what-it-left) lists) |
| 7. Tests | **Done** in the next run: T4-1, T4-3, T4-4, T4-6, T4-11 (with what it left); T4-5 and T4-7 since; T4-8 … T4-10 to tickets |
| 8. Tiers 7 and 8 | **Done** in the next run but what it left; T8-15 done (`0001_init.sql` frozen) |

Proposed (2026-11-01). The last review before the first version is tagged, after
[pre-release-review.md](../history/pre-release-review.md) (done 2026-10-27) and the entrants-from-a-list feature
([entrant-import.md](../history/entrant-import.md), done 2026-11-01). Two independent reviews, each with five to
seven parallel passes over the code (dead code, architecture, tests, performance, UX, visual design, release
mechanics; and a second set on concurrency, security and operations), merged here. Every finding in Tier 0 and
Tier 1 was verified against the code at `066e91c` by reading the lines named; nothing was reproduced in a
browser unless it says so.

The premise is unchanged from the first review: anything cheaper to change before a database and a deployment
exist is changed now. `0001_init.sql` is still edited in place, so schema changes cost nothing yet; the engine's
`Decision` shape is in Restate journals from the first draw on; the SPA already loaded on a venue laptop can't be
fixed after the fact.

Each finding has an ID for tickets and commits, **Where** (file:line), **Why** (what goes wrong and how likely),
**Change**, **Size** (trivial: minutes · small: an hour or two · medium: a day · large: more) and **Accept**: the
observable condition that closes it, phrased so a reviewer can check it without re-deriving the finding.

## Severity, in one paragraph

Seven correctness findings (Tier 0) are new to this review. Two of them would bite at a real event: **T0-2** is
deterministic (no race needed) and **T0-1**'s window is the whole time a mirrored console is stale, which on
venue wifi is seconds to minutes. **T0-3** is a security hole in the one most destructive admin function. The
remaining four (T0-5 … T0-7) are millisecond windows that need two organizers acting on one raffle at the same
instant; they are fixed because the fixes are cheap and the schema is still open, not because they are likely.
Tier 1 holds six bugs that show on every evening (a countdown that doesn't count, phones that go dark, a
projector that can't say it's offline). Tier 2 is what blocks *publishing* rather than the code: the repository
URL in every deploy script names a repository that doesn't exist. Everything from Tier 3 on is quality.

## Tier 0 · Draw integrity and data correctness

Fix before anything else. Each gets an **interleaving** test (two actors, one paused mid-flight), not two
sequential tests.

#### T0-1 · A stale console resolves the wrong winner — **done**

- **Where**: `crates/raffle/src/command.rs:76` (`Decision::Resolve { item, resolution }` carries the item, not
  the draw); `crates/raffle/src/raffle.rs:642-654` (`resolve` acts on whichever draw `k` currently holds the item;
  `resolve_draw(k, …)` exists at `:657` but nothing on the wire reaches it); `crates/app/src/draw.rs:151-190`
  (`command` admits against the projection, then sends).
- **Why**: console A shows Alice drawn for item X. Console B presses "Not here" (forfeit; X is available again)
  and Draw; Bob is drawn for X. Console A, still showing Alice, presses "Claimed": the engine resolves X's
  *current* draw, Bob's. Command-ID deduplication doesn't help (a new click is a new ID). Mirrored consoles are a
  documented feature; the admin polls `/view` every 2.5 s but a console on bad wifi is stale for as long as its
  wifi is bad, and the redraw takes at least the drand lead (6 s), so the window is real.
- **Change**: `Decision::Resolve` gains `k: Option<u64>` with `#[serde(default)]` (additive: old journals still
  deserialize). `Raffle::decide` refuses with a new `Error::DrawChanged { item, expected, actual }` when `k` is
  `Some` and the item's current draw isn't it; `None` keeps today's behaviour for one release so an in-flight old
  SPA still works, then becomes a refusal. The SPA sends the `k` of the draw it displays (`RaffleView.draws[].k`
  already exists). `Refusal` maps it to "The draw moved on: this prize was drawn again. Reload to see the current
  winner." The verifier is untouched: `Event::Resolved { k }` already names the draw.
- **Size**: small.
- **Accept**: a test in `crates/raffle` where item X is drawn (k=1), forfeited, drawn again (k=2), and
  `Resolve { item: X, k: Some(1), Claimed }` is refused while `k: Some(2)` is accepted; a two-console HTTP test in
  `crates/server/src/tests.rs` where the stale console's `draw_item`-then-`resolve` with the old `k` returns the
  refusal code and the `draws` table shows Bob unclaimed; `cargo test -p rafi-engine-restate -- --include-ignored`
  still passes (the journaled `Decision` deserializes with and without `k`); `docs/reference/draw-algorithm.md`
  says `Resolve` names its draw.
- **Done**: as changed. The refusal's code is `draw_changed`; `resolve_item` takes an optional `k`; the console
  sends the `k` of the item's latest undecided draw (`drawn_k`). Tests: `a_resolve_names_its_draw` (rafi-raffle),
  `a_stale_console_cannot_resolve_the_redraw` (rafi-server), the Restate e2e refusing a stale `k`. Open: make a
  `Resolve` without `k` a refusal one release after the tag.

#### T0-2 · Changing a prize's delivery rewrites what past claims meant — **done**

- **Where**: `crates/lifecycle/src/raffle.rs:94-112` (`on_claim` interprets *every* earlier claim of the winner
  through `delivery(prize)`, the prize's current setting); `crates/store/src/lib.rs:195-205` (`save_raffle` calls
  it with `prizes::deliveries(db, id)`, read at projection time); `crates/app/src/prizes.rs` `set_prize_delivery`
  is allowed until the raffle is finished (`RaffleState::prize_delivery_editable`).
- **Why**: deterministic, no race. Alice claims a voucher set to "Sent later". The organizer switches that prize
  to "On the spot" (for the next copies, they think). Alice claims a mug (on the spot). `on_claim` finds all of
  Alice's claims on-the-spot by the *current* settings and marks her delivered; her email goes on the non-winners'
  erasure clock and is erased before the voucher is sent. Plausible exactly in pass 2 (more prizes than people, a
  small meetup).
- **Change**: snapshot the delivery onto the draw: `draws.delivery TEXT` (both `0001_init.sql`), written by
  `save_raffle` from the prize's setting *at the time of the claim*; `on_claim` reads the draw's own `delivery`
  for earlier claims and the prize's for the one being made. Alternative if the column is unwanted: refuse
  `set_prize_delivery` for a prize with a claimed draw (`Refusal::PrizeClaimed`). The column is the correct one
  and costs nothing while `0001` is edited in place.
- **Size**: small.
- **Accept**: a conformance scenario (`domain.delivery_snapshot`, SQLite, Postgres, D1) that claims a Later
  prize, flips it to OnTheSpot, claims an OnTheSpot prize for the same entrant, and asserts `delivered_at IS NULL`
  and `draws.delivery` reads `later` for the first draw; the existing `domain.delivered_on_claim` still passes;
  `privacy-and-abuse.md` (the clocks) says a prize's delivery is fixed per draw when it is claimed.
- **Done**: the column. `save_raffle` writes it with the claim and never overwrites it (`COALESCE`); `on_claim` takes
  the recorded delivery of the winner's earlier claims (`prizes::claimed_deliveries`), the prize's own for one not
  recorded yet. The app's audit of the claim reads the same.

#### T0-3 · An empty-body admin POST bypasses the JSON requirement — **done**

- **Where**: `crates/server/src/admin.rs:75-89` (`if !json && !args.is_empty()`); `crates/api/src/admin.rs`
  (`remove_my_data` takes no arguments, so its body is empty).
- **Why**: SEC-7 added the `application/json` requirement so a `<form>` can't post to the admin API. An
  argument-less function is exempt, and the exempt one erases the caller's data and deletes every never-frozen
  raffle. The session cookie is `SameSite=Lax`, so a page on a *sibling* origin of the same site (another
  subdomain) can send a simple empty POST with it. Conditional on the deployment, trivial to close.
- **Change**: require `Content-Type: application/json` on every admin POST, body or not (the SPA always sends it,
  `apps/web/src/api.rs`). Additionally refuse when `Sec-Fetch-Site` is present and not `same-origin`/`none`
  (every current browser sends it; absent, the content-type rule alone holds).
- **Size**: trivial.
- **Accept**: an HTTP test posting `remove_my_data` with no body and no content type gets 415 and changes
  nothing; with `Content-Type: application/json` and `Sec-Fetch-Site: same-site` gets 403; the evening still
  signs out and removes data (`apps/worker/evening.mjs`); `docs/reference/routes.md` (security headers) says so.
- **Done**: as changed; the 403's code is `cross_site`. The evening signs out and removes data under it (`just workers`
  with Tier 2).

#### T0-4 · The native engine logs its database credentials — **done**

- **Where**: `apps/restate/src/main.rs:61` (`info!(… database = config.database.url …)`);
  `apps/restate/src/config.rs:129` (`format!("invalid database.url {:?}", self.url)` in the error's context).
- **Why**: Postgres URLs carry the password. The line is at INFO on every start, and ends up in journald or a
  container log.
- **Change**: log a redacted form (scheme, host, port, database name; `url::Url` with `set_password(None)` and
  `set_username("")`, or a small `redact` in `rafi-store` shared with the app's own startup line), and use the
  same form in the error context.
- **Size**: trivial.
- **Accept**: a unit test that `redact("postgres://u:p@h/d")` contains neither `u` nor `p`; the startup line
  under `cargo run -p rafi-restate` with a credentialed URL shows the redacted form; `rg 'database.url' apps
  crates` finds no unredacted log or `format!`.
- **Done**: a small `rafi_store::redact_url` (no `url` dependency) drops the user information and a `password` query
  parameter, and leaves SQLite URLs alone. Checked by hand: the startup line reads `database="postgres://localhost/rafi"`.

#### T0-5 · Deleting a raffle ignores the engine's refusal — **done**

- **Where**: `crates/app/src/raffles.rs:349-367` (`deletable` checked against the projection; `decide(Finish)`;
  any `Err` → `warn!` → `raffles::delete` unconditionally); `crates/raffle/src/raffle.rs:684-691` (`finish`
  returns `DrawInProgress` when a draw is pending: the exact signal); `crates/store/src/raffles.rs:372` (`DELETE
  FROM raffles WHERE id = ?`, no guard).
- **Why**: a Draw that lands between the `drawn()` read and the `DELETE` is announced in the engine; the engine
  refuses `Finish`; the app treats the refusal like an outage and deletes anyway, cascading the announced draw's
  record away. Millisecond window, two organizers; closes SEC-2's remaining hole.
- **Change**: distinguish the engine's *refusal* (`ErrorKind::Refused`, the `x-restate-error-source` the client
  already reads) from unreachability: a refusal returns `Refusal::Drawn`; only unreachability proceeds. Guard the
  delete: `DELETE FROM raffles WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM raffle_events WHERE raffle_id = ?1
  AND type = 'announced')`, and return `Refusal::Drawn` when nothing was deleted.
- **Size**: small.
- **Accept**: a use-case test with a `LocalEngine` paused after `drawn()` where a draw is announced before the
  delete resumes: the delete returns `drawn`, the raffle and its events survive; the conformance suite has
  `domain.delete_guard` asserting the guarded `DELETE` deletes nothing once an `announced` event exists; a frozen
  raffle with no draw still deletes and the engine forgets it (existing test).
- **Done**: a refusal (`Conflict`) *and* `EngineError::Pending` (the finish queued behind a draw) refuse as `drawn`;
  `NotFound` (the engine never held it) proceeds; only unreachability is logged and proceeds. `raffles::delete`
  returns whether it deleted. Tests: `a_draw_announced_while_deleting_keeps_the_raffle`,
  `an_engine_refusing_the_finish_keeps_the_raffle`, `domain.delete_guard`.

#### T0-6 · The freeze commits the prize list read before the freeze — **done**

- **Where**: `crates/app/src/draw.rs:68-81` (`items(app, id)` read first; `count_active`, `count_fakes` awaited;
  then `raffles::freeze`; `write_manifest` reuses the pre-freeze `items`).
- **Why**: a prize edit that commits in that window is in the database but not in the manifest, forever (the
  prize table locks only *after* the freeze). Milliseconds, two organizers.
- **Change**: read the prizes inside `write_manifest`, after the freeze, and re-run `start::ready` there (it
  already unfreezes when nobody is left; the same path handles "no items").
- **Size**: trivial.
- **Accept**: a use-case test with a prize `upsert` interleaved between the pre-checks and the freeze: the
  manifest's items match the prizes table at the time of the manifest write; the existing freeze tests pass.
- **Done**: as changed. Test: `a_prize_saved_while_freezing_is_in_the_manifest`.

#### T0-7 · An objection and a claim can both succeed — **done**

- **Where**: `crates/app/src/draw.rs:179-181` (`objected_winner` checks the stored entrant before `send`);
  `crates/store/src/entrants.rs:721-738` (`object` refuses only if `draws.outcome = 'claimed'`, which the
  engine's projection writes after the round trip); `crates/app/src/housekeeping.rs` (`erase_objected` erases
  objected entrants of a raffle that is over).
- **Why**: Claimed pressed, objection pressed within the engine's round trip: the entrant is objected *and*
  claimed; at the finish, `erase_objected` erases a winner whose prize may be outstanding. Both are organizer
  actions on one person within about a second: unlikely, but the consequence is erasing personal data the
  organizer still needs.
- **Change**: `erase_objected` excludes entrants in a claimed draw (they are on the winners' clock); the
  admin's People row shows both badges when both are set, with "Undo" on the objection offered. No change to
  the engine.
- **Size**: small.
- **Accept**: a conformance scenario where an entrant is objected and then claimed (written in that order to the
  store) and `erase_objected` leaves them un-erased while the winner hold erases them later; the People row test
  in `apps/web` renders both states.
- **Done**: `retention::erase_objected` skips entrants of a claimed draw. The People row already showed both badges
  and the Undo; `objected_and_claimed` pins it. Test: `domain.objected_winner`.

#### T0-8 · Smaller correctness items — **done**

- **Import receipts vanish with an undo** — `crates/app/src/entrants.rs:539-595`: `of_batch` reads the entrant
  rows as the batch's receipt; `undo_add` deletes them; a retry with the same batch ID (the SPA keeps it across a
  network error) adds the people back. **Change**: an `add_batches (raffle_id, batch_id, result_json,
  created_at)` row written in the add's batch and kept until the freeze (deleted with the manifest's write, like
  `add_removals`); `of_batch` reads it first. (small) **Accept**: `domain.add_retry_after_undo` on all three
  backends: add, undo, retry → zero rows inserted, the first result answered.
- **The browser verifier ignores `verify.json.format`** — `apps/web/src/ui/verify.rs:51-53` (no check;
  `tools/verify.py:237-241` refuses an unknown format). **Change**: `VerifyData.format != 1` → the "written by
  a newer version" card. (trivial) **Accept**: a `rafi-web` SSR test with `format: 2` renders that card.
- **Housekeeping's ceiling finish always picks the same five** — `crates/app/src/housekeeping.rs:119`
  (`truncate(FINISHES_PER_RUN)`), `crates/store/src/retention.rs:307` (`ORDER BY id`). Five raffles the engine
  keeps refusing starve the rest. **Change**: `ORDER BY last_finish_attempt NULLS FIRST, id` with a column, or
  `ORDER BY (id > ?last_tried), id` rotating from the last ID tried, kept in `meta`. (trivial) **Accept**: a
  test with seven due raffles and the first five failing: run 2 asks about the other two.
- **Retention deadlines follow the operator's configuration** — `crates/server/src/participant.rs:146`,
  `crates/app/src/housekeeping.rs:73`: the winner hold is `RAFI_RETENTION_WINNERS_DAYS` *now*, so changing it
  moves the dates entrants were shown. A decision, not a bug: either snapshot `winners_days` per raffle at the
  freeze (a column, cheap now) or write in `configuration.md` that the hold applies to every raffle and the
  notice's date is the current configuration's. **Accept**: one or the other is in the code or the docs before
  the tag.
- **Done**:
  - The receipt: `add_batches (raffle_id, batch_id, result, created_at)`, written after the add (not in its batch:
    the result is known only after the read-back); an add that stops before it is answered from its entrants as
    before. Deleted with the manifest's write. Tests: `domain.add_retry_after_undo`, `a_retry_after_an_undo_adds_nobody`.
  - The format: `unknown_format` shows the "newer version" card. Test: `a_newer_format_is_not_checked`.
  - The ceiling: the rotation, through `meta`'s `last_ceiling_finish`. Test: `ceiling_finishes_take_turns`.
  - Retention: documented, not snapshotted. `configuration.md` says the hold applies to every raffle as configured
    now, so changing it moves the dates entrants were shown, and not to raise it while a raffle holds winners.

## Tier 1 · Bugs that show on every evening

#### T1-1 · The countdown doesn't count down

- **Where**: `apps/web/src/ui/screen.rs:239-241,275` (`Drawing`); `apps/web/src/ui/admin/draw.rs:177,183,190`
  (`DrawConsole`). `let now = use_clock(200)` is read in the body, so the body re-runs every tick and recomputes
  `offset = server_now_ms - now_ms()`; `now() + offset` is therefore `server_now_ms` (±200 ms), a constant between
  fetches.
- **Why**: the projector's digit steps only when a poll brings a new `server_now_ms` (every 2.5 s: 6 → 4 → 1);
  the console's digit and progress ring freeze until a version bump. `overdue` (`draw.rs:181-183`) compares the
  same constant, so the Retry/Abandon controls for a lost fetch never appear without a `Stalled` event. The phone
  is correct (`apps/participant/src/clock.rs` fixes the offset once at init).
- **Change**: `LiveView` records `(server_now_ms, local_at_fetch)` when a view is taken (`live.rs:83-85`) and
  exposes `server_now() = server_now_ms + (now_ms() - local_at_fetch)`; both components use it. Same in the
  admin's `LiveRefresh`.
- **Size**: small.
- **Accept**: `evening.mjs` (with `RAFI_DRAND_LEAD_SECS=6` for the run) reads the projector's `.countdown` text
  and the console's `radial-progress` text twice, 1 s apart, with no `/view` version change in between, and
  asserts both decreased; a `rafi-web` unit test of the offset helper.
- **Done**: `live::ServerClock`, the offset taken when a view arrives: `LiveView.clock` for the projector, and the
  admin's `get_raffle` resource carries one with its reply for the console (the console reads `admin.view`, not the
  live view). Test: `the_server_clock_runs_from_the_view`. In `evening.mjs` (the evening's drand lead is 8 s, not
  development's 3, so there is a countdown to watch): the projector's and the console's seconds are sampled every
  250 ms for 3.5 s, and must drop between two samples with no fetch of the view (the projector) or the raffle (the
  console) in between, which the old code couldn't: its digit moved only when a fetch brought the server's time.

#### T1-2 · A phone refused by the live channel goes dark

- **Where**: `apps/participant/src/app.rs:147-170` (`poll_ms` is `None` for `Follow::Partyline`, so the poll
  loop returns at `:153-155`); `:222-233` (`Status::Stopped` → one `on_resync`, then nothing); the comment at
  `:222-223` says it "leaves the endpoint as the only source", which nothing then reads.
  `crates/cloudflare/src/gate.rs:102-123` (4403 over `CONNECT_LIMITER`, 600/min per raffle, `wrangler.toml:72-74`);
  `crates/cloudflare/src/object.rs:35,165` (`MAX_SOCKETS` 2000).
- **Why**: after a Durable Object eviction or a deploy, partyline reconnects with a 500 ms base backoff, so a
  300-phone room reconnects within about half a second; the phones past the limiter get 4403 and never see "YOU
  WON" unless the user switches tabs (the wake listener fires one fetch). Foreground phones with the screen on
  stay on "Waiting".
- **Change**: on `Stopped` whose reason isn't "live updates ended", start the existing poll loop at
  `DEFAULT_POLL_MS` with ±20 % jitter, and keep trying the socket on partyline's schedule; stop polling when the
  socket opens. Fix the comment.
- **Size**: small.
- **Accept**: `testbeds/cloudflare` sets `CONNECT_LIMITER` to `limit = 1` for the smoke run; `smoke.mjs` opens a
  second WebAssembly phone, asserts its channel reports `Stopped`, draws, and asserts that phone shows the win
  within 10 s; the same for the no-WebAssembly fallback is already covered.
- **Done**: a stopped channel (any reason) polls at `DEFAULT_POLL_MS`; one closed by the server (`StopReason::Closed`,
  4403 among them) is tried again after 5 s, doubling to a minute (remounted by key), and polling stops when it
  opens; every poll interval on the phone is jittered ±20 %. The smoke test gets its 4403 from live updates ended
  while the raffle is still drawing (the gate refuses it as it does the limiter), not from a limiter of 1: the
  testbed has no `[[ratelimits]]` and the refusal is the same close code. Check: "refused by the live channel: the
  phone polls and shows the win".

#### T1-3 · A late WebAssembly module and `fallback.js` fight over the card

- **Where**: `apps/participant/index.html:2` (the loader starts `fallback.js` after 8 s without `__rafiReady`);
  `apps/participant/src/lib.rs:54-71` (`start()` mounts without checking `__rafiFallback`);
  `apps/participant/public/fallback.js:10-11,39-43` (`live.textContent = ""` on each render).
- **Why**: on congested venue wifi the module arrives at 9 s: two renderers, one `#live`, the card flickers or
  loses the win.
- **Change**: `start()` returns before `app::mount` if `window.__rafiFallback` is set (one line); or the fallback
  cedes when `__rafiReady` appears. The first is simpler and the fallback is complete on its own.
- **Size**: trivial.
- **Accept**: the testbed delays `/participant/assets/*.wasm` by 9 s when `?slow=1`; `smoke.mjs` asserts `#live`
  has exactly one child after 12 s and shows the win once after a draw.
- **Done**: `start()` returns first thing when `window.__rafiFallback` is set. The testbed's `?late=1` starts the
  fallback before the loader runs, as the loader's timeout does, rather than delaying the module 9 s (the same state,
  12 s quicker); `smoke.mjs` asserts the module doesn't start, one card, and the win shown with one card.

#### T1-4 · The projector and the admin have no offline or stale state

- **Where**: `apps/web/src/ui/live.rs:77-79` (`let Ok(fresh) = … else { continue; }`: every failure is
  discarded, including a JSON parse failure after a deploy added an enum variant);
  `apps/web/src/ui/screen.rs:24-35` (loading dots before the first view, forever; the `Draft => "This raffle does
  not exist."` branch at `:42-44` is unreachable because a Draft raffle 404s); `crates/api/src/lib.rs:33,87,109,
  126,241,257` (`RaffleState`, `ItemStatus`, `Outcome`, `Source` on the SPA wire with no `#[serde(other)]`).
- **Why**: the projector is the one long-lived tab and the worst place to fail silently. Wifi drops
  mid-countdown: the digit hits 0 and dots spin forever. A deploy during the evening: the last view, forever.
  The verify page handles exactly this (`verify.rs:36-43`); the projector and the admin don't.
- **Change**: `LiveView` gains `stale: Signal<Option<Problem>>` counting consecutive failures and the last error
  kind; the projector shows a small "Reconnecting…" badge in the header after ~10 s, a full-screen "No raffle at
  this address, or it isn't published yet." on a 404 before any view, and "This screen is older than the server:
  reloading" plus one guarded `location.reload()` on a parse failure (`codes::CLIENT`). The admin shows an
  "Offline" badge in `AdminShell` from `navigator.onLine` and the same counter. Optionally the server sends
  `x-rafi-build` and the SPA reloads once on mismatch, which also covers T2-5's version.
- **Size**: small.
- **Accept**: `rafi-web` SSR tests render the three states; `evening.mjs` stops `wrangler dev` (or blocks `/view`
  with a request interception) for 15 s during the intro and asserts the badge appears and disappears; a
  projector opened at `/s/does-not-exist` shows the message within 5 s.
- **Done**: `LiveView.trouble` (`live::Trouble`: kind, failures in a row, since when); "Reconnecting…" after 10 s of
  failures; the 404 message before any view (the fetches go on, so a raffle published meanwhile comes up); a reply
  that doesn't decode is a new client code, `unreadable` (network failures stay `client`), on which the projector
  reloads, at most once a minute (`sessionStorage`); the verification page's "newer version" card keys on it too, and
  shows a connection message for a network failure rather than that card. The admin's header shows "Offline" from
  `navigator.onLine` and the raffle's live view (`AdminConnection`, set by `LiveRefresh`). No `x-rafi-build` header.
  Tests: `what_the_screen_says_before_a_view`, `the_screen_says_when_it_is_stale`, `the_admin_says_when_it_is_offline`,
  `trouble_counts_failures_in_a_row`, `the_kind_of_a_failure`. In `evening.mjs`: the long-names projector, its
  `/view` blocked (`Network.setBlockedURLs`), says "Reconnecting…" and keeps the intro, then stops once unblocked; a
  projector at an unknown slug says there's no raffle there (147 ms with the bundle cached).

#### T1-5 · The thanks screen tells the room to visit a relative path

- **Where**: `apps/web/src/ui/screen.rs:450` (`"Check the draw yourself: /v/{view.slug}"`); the intro does it
  right with `without_scheme(&view.join_url)` at `:146`.
- **Change**: `without_scheme(verify_url)` in the same mono pill style, with a small QR beside it (the QR
  component exists).
- **Size**: trivial.
- **Accept**: the `Finished` SSR test asserts the host is in the text; `the-draw.md:149` ("a link to check the
  draw") holds.
- **Done**: the view gains `verify_url` (absolute, `serde(default)`; a view without it shows the path as before, no
  code). Test: `the_thanks_screen_says_where_to_check`.

#### T1-6 · The light theme fails contrast on the win moment and the winner's name

- **Where**: `apps/web/tailwind.css:43-44` (`--color-success: #16b583; --color-success-content: #ffffff`, used by
  `crates/live-client/src/card.rs:18-19,112` and `fallback.js:24`: "YOU WON!" and the claim code at **2.6:1**);
  `:35-36` (`secondary-content` white on `#ff5b8a`, 3.0:1); `:47-48` (`error-content`, 3.8:1); `:185-190`
  (`.gradient-text` ends at accent `#ffc83d`: on cream the tail of the winner's name is **1.5:1**, `screen.rs:354`,
  and "Thanks for playing!", `:435`).
- **Why**: the light theme is what most presenter laptops run, a lit room washes a projector out, and these are
  the two moments the product exists for.
- **Change**: light theme `--color-success-content: #003524` (5.2:1), `--color-secondary-content: #3b0617`
  (5.8:1), `--color-error-content` darkened likewise; the claim-code chip in `card.rs:112` and `fallback.js:53`
  becomes `bg-base-content text-base-100`; `.gradient-text` stops at the secondary (`primary → secondary 70% →
  primary`) with the accent only in the dark theme (`[data-theme=rafi-dark] .gradient-text`). The projector
  defaults to dark unless `localStorage rafi-theme` was set (T6-1).
- **Size**: small.
- **Accept**: a script (or test) computing WCAG contrast for every `--color-X` / `--color-X-content` pair in both
  themes asserts ≥ 4.5:1; the smoke test's screenshot of the won card in the light theme is reviewed once by eye.
- **Done**: as changed, with `--color-error-content: #2e0008` (4.95:1) and the light gradient's middle a deep coral,
  `#c81d56` (5.0:1 on base-200), rather than the secondary (2.9:1); the dark theme's gradient as before, under
  `[data-theme=rafi-dark]` and the dark `prefers-color-scheme` without a theme. The checker is `rafi-web`'s
  `theme_contrast` (every pair, base-content on each base, the light gradient's stops). Open: the screenshot by eye;
  the projector's dark default is T6-1's.

## Tier 2 · Launch mechanics

What blocks *publishing*. T2-1 … T2-3 are coupled: rename → rehearsal → real deploy → tag → `fetch` from a clean
copy → announce.

#### T2-1 · Every public URL names a repository that doesn't exist

- **Where**: `git remote -v` → `sagikazarmark/rafi.orig`; `deploy/cloudflare/deploy.sh:31` (release download
  URL), `deploy/cloudflare/README.md:3,47,67,75` and `docs/operations/cloudflare.md:55` (the Deploy button),
  `Cargo.toml:8`, `deploy/cloudflare/wrangler.toml:95`, `.dev.vars.example:4` all say `sagikazarmark/rafi`; the
  16 deferred-issue links in `pre-release-review.md:972-1031` say `rafi.orig/issues/NN`.
- **Change**: rename the repository to `rafi` (GitHub redirects keep the old links working). Set the description
  and topics (both empty). If a fresh repository is made instead, rewrite the 16 links and transfer the open
  issues.
- **Size**: trivial.
- **Accept**: `gh repo view sagikazarmark/rafi` resolves; `./deploy.sh fetch` from a clean copy of
  `deploy/cloudflare` downloads the release (after T2-3); every `github.com/sagikazarmark/rafi` link in the repo
  resolves (`rg -o 'github.com/sagikazarmark/[a-z.]+' | sort -u` shows one name).
- **Not done**: the rename is GitHub's. The Deploy buttons now name `tree/v0.1.0` (T2-8), so they resolve once the repository is `rafi` and the tag exists.

#### T2-2 · The release workflow has never run and can't be rehearsed

- **Where**: `.github/workflows/release.yaml:14-15` (fires only on `v*` tags); `scripts/cloudflare-release.sh:40-44`
  (refuses a version ≠ `deploy.sh`'s pin); `ci.yaml:3-7` (branches only, so a tag's commit has no CI run of its
  own); `release.yaml:47-68` (`gh release create --generate-notes`, the digest check after upload).
- **Change**: `workflow_dispatch` with `version` and `dry_run` inputs that builds, rehearses and skips `gh
  release create`; the release job `needs:` a reusable CI workflow (or checks the commit's status) before it
  publishes; create as a **draft**, verify the digest, then publish; accept `v*-rc*` with `--prerelease`;
  `.github/release.yml` with label-driven categories (`operator-action`, `engine-handlers`, `config-shape`) and a
  PR-template checkbox, so `deploy/cloudflare/README.md:66` and `upgrades.md:29,84` ("the release notes say
  when…") become true; v0.1.0's notes hand-written and passed with `--notes-file`.
- **Size**: small.
- **Accept**: one `workflow_dispatch` dry run is green on `main`; a `v0.1.0-rc.1` tag produces a pre-release
  whose tarball `./deploy.sh fetch` accepts; the release job is skipped (not failed) when CI for the commit is
  red (tested by dispatching on a commit with a failing test on a branch).
- **Done** (the workflow; its runs need GitHub): `release.yaml` calls `ci.yaml` (now `workflow_call`) and the release job
  `needs:` it, so a red commit skips the release; `workflow_dispatch` with `version` (default: `deploy.sh`'s pin) and
  `dry_run` (default on; off publishes a tag that exists); a draft, the uploaded digest checked against the build's,
  then published; a version with a `-` suffix is a pre-release. Notes stay generated: no `.github/release.yml`, PR
  template or hand-written notes (declined). `docs/development/releasing.md` says how to cut one. Open: the dry run
  and `v0.1.0-rc.1` on GitHub.

#### T2-3 · The production path has never touched real Cloudflare

- **Where**: `docs/operations/cloudflare.md:154-164` (six things "only a real deploy can check": the Worker
  reaching Restate Cloud's ingress on `:8080`, the 5 MB module's startup under 400 ms, HTTP/2 from Restate Cloud,
  the Durable Object's EU jurisdiction, a WAF skip rule for `POST /restate/*`, the paid plan); CI dry-deploys the
  shape only (`scripts/workers-check.sh:260-261`).
- **Change**: one real deploy on a throwaway account from the rehearsal tarball, following
  `deploy/cloudflare/README.md` literally and fixing the README where it fails (T2-4). Record the six answers and
  the measured startup time in `cloudflare.md` as "verified on <date>".
- **Size**: small (an afternoon).
- **Accept**: a raffle with ten real phones (or ten browser tabs over a tethered connection) has been drawn end
  to end on real Cloudflare with real drand, verified at `/v/<slug>` and by `tools/verify.py --fetch`; the six
  items are dated in `cloudflare.md`; the Worker's reported startup time is under 200 ms.
- **Not done**: needs a Cloudflare account; out of this pass.

#### T2-4 · The deploy walk-through has friction a stranger can't get past

- **Where**: `deploy/cloudflare/deploy.sh:74-76` (only *warns* on unedited legal templates, then deploys; every
  path including `/healthz` then answers 500 `misconfigured`, `apps/worker/src/lib.rs:159-186`, reason only in
  `wrangler tail`); `deploy/cloudflare/README.md:24-56` (Deploy before "Your legal documents"; the custom domain
  after `RAFI_PUBLIC_URL` is set); nothing says a Clerk production instance needs DNS CNAMEs, where Restate's
  identity key and API key come from, or that `restate cloud login` precedes registration;
  `docs/development/getting-started.md:11-30` (the non-devenv list omits `just`, `tailwindcss`, `worker-build`,
  `wrangler`, and a macOS path for `restate-server`); `README.md:44-45` (the button creates a D1 without
  jurisdiction).
- **Change**: `deploy.sh` refuses unedited templates unless `--allow-templates`; the `misconfigured` body names
  the marker (it isn't secret) and says "see `wrangler tail`"; the README orders: domain → Clerk (with the DNS
  step) → Restate (with the CLI steps) → D1 → documents → secrets → deploy → register; the button section says
  the quick path makes a non-EU database; `getting-started.md` lists every tool with an install line.
- **Size**: small.
- **Accept**: someone who hasn't seen the repository deploys from the README to a working `/healthz` → `ok` in
  under an hour, noting every place they had to guess; their notes are folded in before the tag.
- **Done** (but the stranger): `deploy.sh deploy` refuses documents still marked **Template.** unless
  `--allow-templates` (a dry run warns, as the release rehearsal needs); the Worker's `misconfigured` names the reason
  when it names only a setting or the documents, never a value (`rafi_server::config::refusal`), and points at
  `wrangler tail`; the README goes domain → Clerk (its DNS records) → Restate (`restate cloud login`, `env
  configure`, the keys) → D1 → documents → secrets → deploy → `restate deployments register`, and says the button's
  database isn't in the EU and that its first deploy fails on the templates; `getting-started.md` has a table with an
  install line for every tool. Open: the stranger.

#### T2-5 · The release is unattested, the tarball isn't reproducible, the version isn't observable

- **Where**: `release.yaml:17-33` (`contents: write` only, no `attest-build-provenance`); `apps/worker/justfile`
  `release` (`tar -czf` without `--sort=name --mtime=@$SOURCE_DATE_EPOCH --owner=0 --group=0 --numeric-owner`,
  gzip without `-n`); no `CARGO_PKG_VERSION` anywhere in `crates/server/src` or `apps/worker/src`; `/healthz` is
  the literal `ok` (`crates/server/src/public.rs:21`); `upgrades.md` keys on "which release" with no way to tell.
- **Change**: `permissions: attestations: write, id-token: write` + `actions/attest-build-provenance@<sha>`;
  `gh attestation verify` in `deploy/cloudflare/README.md` §Upgrading; the reproducible tar flags; the version
  (and `git describe`) compiled in through `build.rs`, returned in `whoami`, shown in the admin footer, and
  `/healthz` → `ok <version>`.
- **Size**: small.
- **Accept**: two CI builds of one commit produce byte-identical tarballs; `gh attestation verify
  rafi-cloudflare-0.1.0.tar.gz -R sagikazarmark/rafi` passes; `curl /healthz` on the deployed Worker prints the
  tag.
- **Done** (but the attestation's first run): `crates/server/build.rs` compiles in the version (`RAFI_VERSION`, else
  a `v*` tag at `HEAD`, else `<crate>-dev+<commit>`) and the source (`RAFI_SOURCE_URL`, else the workspace's
  `repository`) as `rafi_server::build_info`; `/healthz` answers `ok <version>`, `whoami` and `/config.js` carry it,
  the native server logs it. `release.yaml` attests with `actions/attest-build-provenance` (v4.2.2, pinned), the
  README's Upgrading shows `gh attestation verify`. `just release` packs with GNU tar (`--sort=name`,
  `--mtime=@$SOURCE_DATE_EPOCH`, no owner, `gzip -n`): two local builds of 0.1.0 gave the same SHA256. Open: two CI
  builds compared, the attestation verified.
- **Since** (2026-11-03): the template's `deploy.sh` is gone, replaced by `package.json` scripts: `npm run build`
  downloads the release `config.rafi_version` names (`RAFI_VERSION` overrides it) into `worker/` and copies
  `legal/` in, `npm run deploy` applies the D1 migrations and deploys. A release is one asset, `worker.tar.gz`
  (the artifact at the top level, still packed reproducibly), with no `SHA256SUMS` and no `RAFI_SHA256` pin:
  GitHub's digest of the asset (checked against the build's before the draft is published), immutable releases
  and the attestation (now `subject-path`) stand for them. Nothing refuses unedited templates at deploy time any
  more (T2-4's `--allow-templates` went with `deploy.sh`); the Worker refuses every request while one carries
  the marker. T2-8's wrangler pin is the template's devDependency, `wrangler` 4.148.0. Read `./deploy.sh fetch`
  above as `npm run build`, and `rafi-cloudflare-0.1.0.tar.gz` as `worker.tar.gz`.
- **Since** (2026-11-03, later): the Cloudflare path moved to GitHub Actions and two Workers; the `package.json`
  scripts, the Deploy to Cloudflare button and Workers Builds are gone. Production is the app Worker
  (`apps/worker-app`, `rafi`) and a raffle engine Worker per release (`apps/worker-engine`, deployed as
  `rafi-engine-<version dashed>` on workers.dev), split in `f7e070f`; local development keeps the combined
  `apps/worker`. A release (`f37134d`) is still one asset, `worker.tar.gz`, now `VERSION`, `app/`, `engine/`,
  `migrations/sqlite/` and `deploy.sh` (`scripts/cloudflare-deploy.sh`), which checks the two configurations, runs
  the "is anything live?" preflight itself, applies the migrations, deploys the engine and registers it with
  Restate automatically (without `force`), deploys the app, and prunes drained engine deployments and their Workers;
  the manual `restate deployments register` step and the `RESTATE_PREFIX` path change for changed handlers are gone
  (every release is a new URI). An installation is a repository copied from `deploy/cloudflare`, whose
  `.github/workflows/deploy.yml` calls this repository's `.github/workflows/deploy-cloudflare.yml` at the tag: it
  downloads `worker.tar.gz`, verifies its attestation (`gh attestation verify` with `--signer-workflow
  …/release.yaml` and `--source-ref refs/tags/v<version>`; `release.yaml` now refuses to publish from a run not on
  the tag) and runs `release/deploy.sh` with the operator's secrets (`RAFI_SECRETS` optional). The rehearsal
  checks the template's `uses: …@v<version>` and `version:` name the release. T2-1's rename touches
  `RAFI_REPOSITORY` in `deploy-cloudflare.yml` and the template's `uses:` (the rehearsal checks they agree);
  T2-3's real deploy and T2-4's stranger follow the new README.
- **Since** (2026-11-03, last): production is three Workers (`6d921f0`): the app (`rafi`), a rooms Worker
  (`apps/worker-rooms`, `rafi-rooms`) that alone defines the `RaffleRoom` Durable Objects and is redeployed only
  when its code or configuration changed, and an engine Worker per release, now `rafi-engine-v<version dashed>`;
  local development keeps the combined Worker. The deploy (`8425a52`) is two workflows of the installation's own,
  `deploy/cloudflare/.github/workflows/deploy.yaml` and `restate-cleanup.yaml`, copied with the template; the
  reusable `deploy-cloudflare.yml` is gone. They download and verify the release `VERSION` names and run its bundled
  `deploy.sh` one subcommand per job (`check`, `migrate`, `deploy-rooms`, `upload-app` ∥ `deploy-engine`,
  `register-engine`, `promote-app`; `cleanup` daily); `DEPLOY_VERSION` (1) is the interface between the two, and a
  release that changes it tells operators to copy the workflows. Each engine is registered with metadata (`app`,
  `instance`, `version`, `cloudflare_account`, `worker`), never over a newer release's, and the cleanup deletes this
  installation's drained engine Workers, then their Restate deployments. No Deploy button, no Workers Builds, no
  `RAFI_SECRETS` (the app's secrets are set once with `wrangler secret bulk .prod.vars`), and, for now, no
  live-raffle check in the deploy (`RAFI_DEPLOY_WHILE_LIVE` is gone; the runbook's query is the operator's). The
  template's README is the first-deploy manual (the API token with `cf`, the keys, the secrets, each step's
  output), `docs/operations/cloudflare.md` the reference.

#### T2-6 · Licensing and attribution

- **Where**: `apps/web/assets/fonts/bricolage-latin*.woff2` ship without the OFL-1.1 text (the licence requires
  it to accompany the files); no served page links to the source (`Brand` at `apps/web/src/ui/mod.rs:79-88` is a
  `span`; AGPL §13 binds forks, who then can't comply without editing Rust); `apps/web/src/ui/verify.rs:323-331`
  tells the public to run `verify.py` without saying where it is.
- **Change**: `apps/web/assets/fonts/OFL.txt` and a `THIRD_PARTY_NOTICES.md`; `Brand` links to the repository
  with the running version (T2-5) in the admin and verify footers; the verify page links `tools/verify.py` at the
  tag.
- **Size**: trivial.
- **Accept**: `ls apps/web/assets/fonts/OFL.txt`; the deployed `/v/<slug>` has a link whose target returns 200 and
  is `verify.py`; the admin footer shows the version and a link to the source.
- **Done** (but `THIRD_PARTY_NOTICES.md`, declined): `apps/web/assets/fonts/OFL.txt`, also served beside the fonts; a
  footer (`Colophon`) with the version, the source at its tag and drand on the admin and the verification page, and a
  "Source code" link on the participant page; the verification page links `tools/verify.py` at the tag and its
  `curl` command has the absolute URL.

#### T2-7 · Operations runbook gaps

- **Where**: `docs/operations/cloudflare.md:183-190` (says "after restoring with Time Travel", never shows
  `wrangler d1 time-travel info|restore`, nor that Time Travel *is* the backup, 30 days, paid plan);
  `upgrades.md:5` (the deploy-freeze preflight is "look at your admin list", which can't see other tenants);
  `cloudflare.md:117`, `configuration.md:40` (rotation only says never rotate `RAFI_POW_KEY` during an event;
  nothing for Clerk, the engine API key or the identity keys, which already take a comma-separated list for
  overlap); `upgrades.md:52-54` (roll forward only; no `wrangler rollback` for a bad deploy that carried no
  migration); `architecture.md:92` (no fallback engine) with nothing for the organizer at 20:00 when Restate
  Cloud is down; `terms.md:82-83` promises breach notification with no runbook; `cloudflare.md:26` says "paid
  plan" with no cost estimate; `cloudflare.md:183-216` restores D1 alone (a restored database can disagree with
  the engine's state and the room's version); no DSAR recipe for the operator (one email across raffles, with
  the `email_norm` rules); no statement on monitoring (what to alert on: cron failures, 5xx rate, `/healthz`).
- **Change**: one section each in `cloudflare.md`/`self-hosting.md`/`troubleshooting.md`: backup and restore
  (the commands; restore order D1 → `reapply-erasures` → engine state check → room version); a global deploy
  preflight query; rotation for each secret; rollback; "Restate is down tonight" for the organizer (freeze retry
  is safe, in-flight draws resume, otherwise draw by hand and say so); a breach runbook (what logs exist, how
  to find affected controllers from `raffles.owner_id`, Art. 33's 72 h); a cost paragraph for a 500-person
  evening; a DSAR recipe (D1 SQL or `rafi find-entrant <email>`); monitoring (a Workers error-rate and cron-
  failure notification, an external `/healthz` check). Confirm `self-hosting.md:193-201`'s restore covers
  removals and objections (it mentions them; check the SQL).
- **Size**: medium (writing).
- **Accept**: each of the ten topics has a heading that `rg -n` finds in `docs/operations`; the restore
  procedure has been run once against a copy (D1 export → local D1 → restore SQL) with removals and objections
  present and nobody came back; the DSAR recipe finds a known email across two raffles.
- **Done** (but the rehearsal): `docs/operations/runbook.md` (the deploy preflight across every organizer, rotation per
  secret, rollback with `wrangler rollback` and when not to, the engine down during an event, Time Travel as the
  backup and the restore in order, a person's data request by email key, a breach, monitoring, costs), linked from
  the overview, upgrades, troubleshooting, self-hosting and Cloudflare docs. The restore didn't carry removals at a
  person's request or objections (only the erasures, purges and organizers who left): self-hosting's and the
  runbook's now save and put back `add_removals` and `objected_at`. The runbook's SQL runs on the schema. The cost
  figures are from Cloudflare's allowances as remembered, not checked against a bill. Open: the restore rehearsed
  against a copy.
- **Done** (the rehearsal, 2026-11-02): `a_restore_puts_nobody_back` (`crates/app/tests/restore.rs`) runs the
  runbook's restore on SQLite files, a backup taken before an organizer's early erase, a purge, a raffle deleted,
  an organizer leaving (a raffle purged, one deleted, their defaults), removals at a person's request and not, an
  objection after the freeze, a hidden name and an expired raffle: the SQL is read from the runbook and from
  `cloudflare.md`, re-applied natively (`reapply_erasures`) and with the D1 SQL, twice; nobody's data is back,
  what the backup had is as it is live, a later add leaves the removed person out, and the DSAR query finds one
  email in two raffles, the preflight and step 5's queries list what they should. By hand: `sqlite3`, the `rafi
  reapply-erasures` binary, a local D1 (the test's two databases loaded with `wrangler d1 execute --local`, step
  1's query and the restore SQL run there, the result read back: the same as `sqlite3`'s), and Postgres on a few
  rows. Found and fixed: the restore brought back
  raffles deleted since (by their organizer, or the never-frozen ones of an organizer who left) with everyone in
  them, people removed since (at their request too: the removal was put back, their entry with it), the defaults
  of an organizer who left, and names hidden since; "insert the `erasures` rows" collided with the backup's IDs,
  and the put-back was prose, not SQL. Step 1 is now one query per kind writing the statements that put it all
  back (deleting what live no longer has, by `audit`'s `delete` rows and each raffle's entrant IDs), run on D1 or
  `sqlite3` (with `PRAGMA foreign_keys = ON`: without, a deleted raffle's entrants stay) or `psql`; a single
  compound `SELECT` of seven terms ran on SQLite and failed on D1. `cloudflare.md`'s organizer statement now has
  the store's condition (`reapply_organizer_left`).

#### T2-8 · Public face

- **Where**: `README.md` (no screenshot or GIF of the projector, no badges, no demo); the repository has no
  `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, issue or PR templates (community profile 42 %); Scorecard 5.4 with
  Branch-Protection 0 and Code-Review 0 (no ruleset on `main`); `.well-known/security.txt` not served
  (`SECURITY.md` exists at the org level); `compatibility_date = "2026-05-01"` (`wrangler.toml:21`,
  `deploy/cloudflare/wrangler.toml:11`); `deploy.sh:34` floats `wrangler@4`; the button points at `tree/main`
  (`README.md:47`) so between tags `main`'s template can require a secret the released Worker doesn't read.
- **Change**: a projector screenshot (dark theme, mid-reveal) and a 10-second GIF in the README; CI, licence and
  Scorecard badges; `CONTRIBUTING.md` (devenv, `just ci`, the commit message style), a code of conduct, a bug
  template that asks for the request ID; a ruleset on `main` (PR required, CI required); `security.txt` as a
  route; bump `compatibility_date`; pin `wrangler@4.x.y`; point the button at `tree/v0.1.0` (or document the
  discipline).
- **Size**: small.
- **Accept**: the README renders the image on GitHub; the community profile is 100 %; Scorecard ≥ 7 on the next
  run; `curl /.well-known/security.txt` returns the org's policy.
- **Done** (but what lives on GitHub and the screenshot): CI, Scorecard and licence badges; `/.well-known/security.txt`
  (the operator's email, Rafi's policy, an expiry 180 days on); `compatibility_date = "2026-10-01"` in the four
  `wrangler.toml`; `deploy.sh` falls back to `npx wrangler@4.148.0`; the buttons name `tree/v0.1.0`, which
  `cloudflare-release.sh` checks against the version it releases. Declined: `CONTRIBUTING.md`, a code of conduct,
  issue templates. Open: the screenshot and GIF, the ruleset on `main`.

## Tier 3 · Performance and its regression guards

Measured on 2026-11-01 from fresh release builds: web app `rafi-web_bg-*.wasm` 2,581,077 B raw, **1,000,888 B
gzip** (no budget in CI); participant module 200,425 B gzip (budget 280 KB, `apps/participant/justfile:59-83`);
Worker 1,737,921 B gzip (budget 2560 KiB, `scripts/workers-check.sh:179-186`); BLS verify of a quicknet beacon
3.9–6.9 ms in V8 with the Worker's profile. Row counts below are from the SQL, not from a D1 bill.

#### T3-1 · `/api/r/{slug}/view` reads every entrant row on every poll

- **Where**: `crates/store/src/view.rs:38-42` (`Full::read`: `SELECT * FROM entrants WHERE raffle_id = ?`),
  called by `crates/app/src/view.rs:29` for `full_view` (`crates/server/src/public.rs:78`); `SELECT * FROM
  raffles` twice per request (`public_raffle` → `by_slug`, then `Full::read`), both pulling `manifest` (~40 B × N).
- **Why**: the view uses the count, the 20 most recent names, the drawn entrants' names and `no_phone`, and one
  `EXISTS` (`view.rs:45-68,192-209`). Rows per poll ≈ `2 + P + 2N + E` on D1 (index and table). Projector and
  admin at 2.5 s, N = 500, three hours: about 4.3 M rows read for one evening, plus decoding N twenty-column
  rows per request on the Worker. `live::snapshot` (`crates/store/src/live.rs:41-63`) already has the right
  shape (`JOIN draws`, names masked in SQL).
- **Change**: `view::Lite::read`: `COUNT(*)`, `ORDER BY created_at DESC LIMIT 20`, the drawn-names join from
  `live.rs:51`, `EXISTS(… source IN list sources)`, and the raffle row passed in from `public_raffle` rather than
  re-read. `Full::read` stays for the admin and the CSV.
- **Size**: small–medium.
- **Accept**: a D1 conformance scenario seeds N = 500 and asserts `view::build`'s `rows_read` (from
  `D1ResultMeta`, `worker-0.8.7/src/d1/mod.rs:460-468`, surfaced by a counter behind `cfg(feature = "metrics")`
  in `crates/store/src/backend/d1.rs:150-200`) is ≤ `50 + P + E`; a server test asserts `GET /api/r/{slug}/view`
  is ≤ 2 round trips and ≤ 6 statements at N = 200 via a counting `Db`; `view::of`'s output is byte-identical
  before and after (the existing view tests).
- **Done**: `rafi_store::view::Lite` (one batch: the raffle, the prizes, the counts and whether a list was added in
  one aggregate, the 20 latest names by `entrants_raffle` backwards, the drawn entrants from `draws` by key, the
  events) and `view::Summary`, which `Summary::of` computes the same from every row for the admin's `reads::admin`;
  `app::view::build` reads `Lite`, so `/view` decodes no entrant rows. The raffle row is still read twice (in
  `public_raffle` and in the batch, where it keeps the version and the content one snapshot). The count still visits
  each entrant row once (`COUNT` over `entrants_raffle`, with `erased` and `source` from the table): about N rows a
  poll rather than 2N plus N decoded. Test: `domain.view_summary` (SQLite, embedded Postgres, D1). Open: the
  `rows_read` counter and the per-request statement count (T3-11 2, 3;
  [#54](https://github.com/sagikazarmark/rafi.orig/issues/54)).

#### T3-2 · The admin re-downloads the full entrant list on every join

- **Where**: `apps/web/src/ui/admin/mod.rs:376,422-432` (`LiveRefresh` polls `/view` every 2.5 s and
  `data.restart()`s on any `v` bump; every entrant insert bumps `version`, `0001_init.sql:188-191`);
  `crates/app/src/reads.rs:25-131` (`admin` = `Full::read` + `removals`, N `AdminEntrant`s of ~330 B with `email`,
  `claim_code`, `list_ref`); `mod.rs:172,325` (`whoami` fetched twice).
- **Why**: 500 joins over 20 minutes ≈ 480 refetches ≈ 40 MB raw (~10 MB compressed) to the organizer's phone
  and ~120 k rows read, for information the tab already has.
- **Change**: (a) while `state == Open`, `LiveRefresh` restarts `data` only when `state`, `draws.len()`,
  `current` or `entrant_count` changed, at most every 10 s; (b) `get_entrants(id, since: Option<i64>)` as a
  separate paginated call, `RaffleAdmin` carrying counts only, the People tab fetching on open. (a) now, (b) with
  T5-2. Lift `whoami` to the layout's context.
- **Size**: (a) small, (b) medium.
- **Accept**: `evening.mjs` joins ten phones in a loop over 30 s with the admin tab open and asserts
  `get_raffle` was requested at most four times (counted by request interception); payload bytes logged.

#### T3-3 · Each phone socket runs three full snapshot batches

- **Where**: `crates/cloudflare/src/gate.rs:124-132` (`connect()` runs `live::snapshot`, five statements, to
  compute `accepts_sockets()`, which needs `state` and `live_ended_at` only); `crates/cloudflare/src/object.rs:170,
  116` (`accept()` → `refresh()` → another snapshot); `apps/participant/src/app.rs:225-231` and
  `crates/server/src/public.rs:94-99` (the client's `on_resync` on `Open` → a third).
- **Why**: a DO eviction reconnects 300 phones in ~0.5 s (partyline's base backoff): ~4,800 D1 statements for
  one raffle in half a second. `CONNECT_LIMITER` (600/min) doesn't throttle below 600 and above it refuses (T1-2).
- **Change**: the gate reads `SELECT state, live_ended_at FROM raffles WHERE id = ?` (or defers to the DO, which
  decides anyway); `RaffleRoom::accept` skips `refresh()` if the last one was < 1 s ago (the hub handshake sends
  the stored `last` view); the client's resync stays.
- **Size**: small.
- **Accept**: `scenario.mjs` connects 50 sockets within 200 ms and asserts (via the testbed's query counter) at
  most `2 + ceil(0.2 s / 1 s)` snapshot batches ran; every socket still receives the first view.

#### T3-4 · `DrawConsole` rebuilds a map of every entrant four times a second

- **Where**: `apps/web/src/ui/admin/draw.rs:19-24,37` (`use_clock(250)` read in the body; `by_id: HashMap<String,
  AdminEntrant>` built by cloning every entrant each render).
- **Why**: N = 2000 ⇒ ~64 k allocations per second on the organizer's device for the whole draw.
- **Change**: `use_memo` the map keyed on `admin.entrants` (or borrow `HashMap<&str, &AdminEntrant>` from a
  `ReadOnlySignal`); move the `now`-dependent bits (`held`, `use_settle`, the countdown) into a child component
  so the tick re-renders only that. Falls out of T1-1.
- **Size**: small.
- **Accept**: a `rafi-web` test wraps `DrawConsole` in a counter and asserts the entrant map is built once per
  `admin` change, not per tick.

#### T3-5 · Natively the web app is served uncompressed with no caching headers

- **Where**: `crates/server/src/lib.rs:341` (`ServeDir::new(dirs.public.join("assets"))`: no `precompressed_*`,
  no `Cache-Control`; the participant got both in API-7, `:344-353`); `apps/web/Dioxus.toml` (no `[web]
  pre_compress = true`, dx's default is `false`); `apps/worker/_headers:22-23` (`immutable` on
  `/participant/assets/*` only, not on the hashed `/assets/rafi-web*`).
- **Why**: a self-hosted projector loads 2.58 MB + 214 KB uncompressed, re-fetched by heuristic freshness.
- **Change**: `pre_compress = true`; `.gz` and `.br` written by `apps/web/justfile build` like the participant's;
  `ServeDir::precompressed_br().precompressed_gzip()`; `Cache-Control: public, max-age=31536000, immutable` for
  `/assets/rafi-web*` (hashed) and `no-cache` for `tailwind.css` and the fonts (not hashed) on both hosts.
- **Size**: small.
- **Accept**: `curl -sI -H 'Accept-Encoding: br' http://localhost:8787/assets/rafi-web_bg-*.wasm` shows
  `content-encoding: br` and `immutable` natively; `evening.mjs` asserts the same headers on the Worker.
- **Done**: as changed, precompressed rather than a `CompressionLayer` (no CPU per request; nothing else is
  compressed). `apps/web/justfile build` gzips every asset next to dx's brotli and drops a variant no smaller than
  its file (the fonts); `apps/worker`'s `assets` deletes both from all of `worker-assets/`. Natively every dx-hashed
  name (`-dxh<hex>`, so `focus-trap-dxh*.js` too) is `immutable` and every other `/assets/` file `no-cache`, only on
  a file found; the shell was `no-cache` already. Workers gets `/assets/rafi-web*` `immutable` (one splat per rule,
  so `focus-trap` keeps Assets' default) and the stylesheet and fonts keep Assets' `max-age=0, must-revalidate`
  rather than an explicit `no-cache`, which a second matching rule would join onto `immutable`. Natively the wasm
  is 2,696,221 B, 1,047,889 gzip, 785,123 br. `the_web_apps_files_are_compressed_and_cached` (rafi-server); the
  evening checks the glue and the module are `immutable` and the stylesheet isn't.

#### T3-6 · Polling has no jitter, no backoff and ignores tab visibility

- **Where**: `apps/web/src/ui/live.rs:92-105` (fixed `POLL_MS`; on `Err` the next tick fires at the same rate);
  `apps/participant/src/app.rs:148-170` (same, and `every/2` while a draw is pending natively); the wake
  listeners (`live.rs:187-219`, `app.rs:70-92`) fire an extra fetch on every `focus`.
- **Why**: a 5xx storm is polled at full rate by every tab, hidden ones included; a mass wake is a burst.
- **Change**: ±20 % jitter; on error double the interval up to 30 s and reset on success; pause while
  `document.hidden` and resume on the existing `visibilitychange` wake; coalesce wake fetches within 2 s.
  Document `Follow::Poll { every_ms }` as the server's emergency knob.
- **Size**: small.
- **Accept**: a `rafi-web` unit test of the schedule function (interval after k failures, after success, while
  hidden); `evening.mjs` blocks `/view` for 10 s and asserts the projector made ≤ 4 requests in that window.

#### T3-7 · The ceiling queries defeat the `raffles_erase_at` index

- **Where**: `crates/store/src/retention.rs:101-106` (`at_ceiling`: `erase_at + ?2 <= ?1`) and `:302-317`
  (`unfinished_at_ceiling`: the same, and no `purged_at IS NULL` so the partial index can't apply). Verified
  with `EXPLAIN QUERY PLAN` on `0001_init.sql`: `SCAN raffles` three times an hour, for the life of the
  installation; rewritten, `SEARCH raffles USING INDEX raffles_erase_at`.
- **Change**: `erase_at <= ?1 - ?2`, and `AND purged_at IS NULL` on `unfinished_at_ceiling` (a drawable raffle is
  never purged, so it is a no-op semantically).
- **Size**: trivial.
- **Accept**: an `EXPLAIN QUERY PLAN` gate (`just test` runs `sqlite3 :memory:` over the store's hot queries and
  fails on `SCAN entrants|raffles|raffle_events`) passes; the retention conformance scenarios still pass.
- **Done**: as changed, and `due`'s winner-hold term rewritten the same way. Checked by hand: both now `SEARCH raffles
  USING INDEX raffles_erase_at (erase_at<?)`. The gate: done with T3-11 7, which found `erase_due`'s list of raffles
  to purge (`ORDER BY id`) walking every raffle by its key rather than searching `raffles_erase_at`.

#### T3-8 · The verification page re-verifies every beacon on each keystroke

- **Where**: `apps/web/src/ui/verify.rs:53` (`verify_public(…)` runs in the component body) and `:106`
  (`query()` is read in the same body, so typing in the search box re-runs every BLS check on the main thread).
- **Change**: `use_memo` the report keyed on `data`; the search filters the memoized list. Later: a Web Worker
  for the initial verification if it exceeds ~500 ms on a phone.
- **Size**: small.
- **Accept**: a `rafi-web` test asserts `verify_public` is called once for one `data` across three query
  changes; `evening.mjs` types five characters into the search on `/v/<slug>` and asserts no jank (long task
  > 50 ms) via `PerformanceObserver`.
- **Done**: `Checked::of(&VerifyData)` does everything that doesn't depend on the search (the report, the manifest,
  the names, the sorted list) in `Record`'s `use_memo` keyed on the record; the search box and the filtered list
  are a child, `Entrants`, the only reader of the query, so typing renders it alone. The test counts `Checked::of`
  (a `cfg(test)` counter, as `verify_public` has no seam): once across three searches, each filtering the list
  (SSR of a `VirtualDom`). The page renders the same. No `PerformanceObserver` check in the evening; the Web
  Worker stays "later".

#### T3-9 · Measure first, then decide

- **Split the projector and the verification page out of the admin bundle.** Projector-only code is ~60 KB of a
  2.5 MB wasm; the projector downloads Clerk, dioform, the alert dialogs, the 10 k-row CSV parser and the
  People/Draw tabs (twiggy attribution: `dioform` + `forms` ~800 K pre-opt, `admin::*` 322 K, `dioxus_primitives`
  196 K, `dioxus_clerk` 97 K, BLS stack ~210 K used by verify only). A `rafi-screen` crate (routes `/s/`, `/v/`)
  served by `shell_routes` per prefix would land near 350–450 KB gzip; dx `--wasm-split` is the other route.
  **Accept** (for the measurement): a stub crate with `Intro` + `Drawing` + verify built and sized; a decision
  recorded here. Meanwhile drop `logger` from `apps/web/Cargo.toml:11` (~30 KB pre-opt). (medium–large)
- **Entrant triggers rewrite the manifest row per entrant write.** `0001_init.sql:188-201`: each entrant
  INSERT/UPDATE/DELETE fires `UPDATE raffles SET version = version + 1`; after the freeze `manifest` is ~80 KB at
  N = 2000, so a housekeeping `UPDATE entrants SET {ERASED}` over M entrants rewrites that row M times in one
  transaction. **Accept**: freeze a 2000-entrant raffle in the D1 testbed, erase all, read `rows_written` and
  `duration`; if > 1 s, move `manifest` to its own table (now, while `0001` is editable) or bump `version` once
  per statement. **Done** (measured natively, SQLite 3 on a file, 2026-11-02): erasing 2000 entrants of a raffle
  whose manifest is 80 KB takes 18 ms, against 3 ms with no manifest; far under 1 s, so `manifest` stays on
  `raffles`. D1's `rows_written` for it not read (it counts the 2000 trigger updates whatever the row's size).
- **BLS verify CPU on the Worker.** 3.9–6.9 ms in V8 on a laptop; Workers' CPUs are slower. The paid plan is
  mandated (`cloudflare.md:164`) so 30 s is the ceiling. **Accept**: on T2-3's deploy, read the CPU time of a
  `/restate/…/draw` resume from Workers Logs and record it in `cloudflare.md`.
- **Draw-click → reveal latency.** Pushed on Workers (the DO publishes before the projector polls), gated on the
  2.5 s poll natively. **Accept**: `evening.mjs` records `view.current.round_time` and the phone's reveal time
  and asserts `phoneAt − round_time × 1000 < 3000 ms`; the number is logged on every run.
- **People tab at N = 2000.** `apps/web/src/ui/admin/people.rs:40-53,71,97` (filter with `to_lowercase()` on
  name + email for every entrant, `.cloned()` twice, per keystroke and per refetch; 2000 `<tr>` with 4–6
  buttons). **Accept**: render time at 2000 with the fake-entrants button measured; if > 100 ms, `use_memo` the
  filtered list keyed on `(entrants, q)`, lower-case once, and paginate at 200 rows (T5-2).
- **`RaffleAdmin` is deep-cloned ~7× per render.** `apps/web/src/ui/admin/mod.rs:347,378-407`. Pass
  `ReadOnlySignal<RaffleAdmin>`; take `server_now_ms` out of the props' equality. With T3-2(b). (small–medium)

#### T3-10 · Later

- `verify.json`'s 304 path still runs `Full::read` before computing the ETag (`public.rs:125-133`); read
  `version` first.
- `/privacy` and `/terms` render Markdown per request (`pages/text.rs:329-346,367-392`); cache the HTML in
  `Documents`.
- Each join reads all N claim codes (`crates/app/src/entrants.rs:187-191`, `store/entrants.rs:130-137`) and, for
  a blank name, all N display names: Σ ≈ N²/2 over a registration. `SELECT 1 WHERE claim_code IN (3
  candidates)`.
- The shell preloads the JS only (`index.html:17`); `<link rel="preload" as="fetch" type="application/wasm">`
  saves one RTT.
- `apps/web/Dioxus.toml` lacks `[web.wasm_opt] extra_features = ["--disable-threads"]` that the participant has
  for older Safari.
- Shared-email import groups compare quadratically (`apps/web/src/import/preview.rs:392-398`, `name_key`
  recomputed per comparison); bounded by the 10 k-row cap; index normalized pairs if a fixture shows it.
- The Worker bundle carries two JWT stacks and a JS regex engine (`restate-sdk` + `dioxus-clerk`); upstream.
- `notify` is awaited inline in request handlers; on Workers the DO nudge retries 3× with backoff
  (`crates/cloudflare/src/notifier.rs:42-55`), up to ~700 ms on the organizer's request when the DO is slow.
  `ctx.wait_until` if it shows at p99.

#### T3-11 · Regression guards to add (one list)

1. **Web app gzip budget** in `apps/web/justfile` mirroring the participant's `check`: `wasm.gz + js.gz ≤
   1,100,000 B` (now 1,016,039), `tailwind.css.gz ≤ 40 KB`; run from `clippy-wasm`/`web`. Ratchet down with
   T3-9's split. **Accept**: CI fails when the budget is exceeded (tested once by lowering it).
   **Done**: `just check` in `apps/web`, which the root `web` recipe runs (so `clippy-wasm`, a CI job, and `doc`):
   1,062,771 B today (wasm 1,047,532 + js 15,239; the module grew since the 1,016,039 above), `tailwind.css` 30,585 B.
   Lowered to 1,000,000, and the stylesheet's to 30,000, each failed the recipe.
2. **D1 `rows_read` assertions** in `testbeds/store-d1` (T3-1's counter) for `view::build`, `live::snapshot`,
   a join, and a housekeeping erase of M entrants (≤ k·M).
3. **Per-request query count** in `crates/server/src/tests.rs` via a counting `Db` wrapper: `/view` ≤ 2 round
   trips and ≤ 6 statements, `/j/{code}` ≤ 3 round trips, `get_raffle` ≤ 2, at N = 200 so an N+1 scales visibly.
4. **Payload assertions** in `evening.mjs`: `/view` body ≤ 8 KB with 100 entrants; `get_raffle` fetched at most
   once per 10 s while phones join (T3-2).
5. **Draw latency** logged and bounded in `evening.mjs` (T3-9).
6. **Worker startup**: `workers-check.sh` times the first `/healthz` after `serve` and fails above 3 s; T2-3
   records the real `Worker Startup Time` and `cloudflare.md` sets an alert at 200 ms.
7. **`EXPLAIN QUERY PLAN` gate** (T3-7). **Done**: `query_plans` in `crates/store/src` (a test of `cargo test -p
   rafi-store`, so `just test`) runs the hot use cases on an in-memory SQLite with the migrations (the reads of
   `raffles::get`, `by_slug`, `by_join_code`, `drawn`, `view::Lite` and `Full`, `live::snapshot`; the join with
   `pow::redeem` and its lookups; `save_raffle` storing a claim; `live_channels_due`, `expire_due`, `erase_objected`,
   `erase_due`, `forget_removals`, `unfinished_at_ceiling`, `purge_unfinished`), on data that takes each down its
   whole path, and explains every statement they sent (a test-only capture in the query layer sees a batch's every
   statement). It fails on a `SCAN` of `entrants`, `raffles` or `raffle_events`, through an index or not, and on a
   `SEARCH` of one without a constraint (SQLite prints that for a whole index walked as for a `MIN` seek), except
   through a partial index whose rows are what is left to do. It found 13 scans, all housekeeping's, now gone:
   `erase_due` and `erase_objected` read every raffle past its date (purged ones included) and all of their
   entrants, erased or not; they start from a new partial index, `entrants_unerased`; `forget_removals` starts from
   `add_removals`; `live_channels_due` uses a new `raffles_live`; `erase_due`'s raffles to purge drop `ORDER BY id`.
   The indexes are in `0001_init.sql` (both dialects; nothing is deployed before the tag, so no `0002_*`;
   `tests/schemas.rs` compares every migration file regardless).
   Not on the list, left in [#54](https://github.com/sagikazarmark/rafi.orig/issues/54): `oldest_raffle_at`
   (`whoami`) walks `raffles_owner`.
8. **Microbench** as `#[test] #[ignore]` with `Instant`: `view::of` and `reads::admin` at N = 2000 natively
   (< 5 ms), `Raffle::restore` at E = 200.

## Tier 4 · Tests: the risky gaps

What exists is strong: 390 native test functions, 40 store scenarios on SQLite, embedded Postgres, CI Postgres and
D1, 14 engine scenarios on two harnesses, 102 browser and WebSocket checks (`docs/development/testing.md`). The
gaps below are where a bug would ruin an evening, leak personal data or break verifiability, ranked by that.

#### T4-1 · The draw console has never been driven in a browser

- **Where**: `apps/web/src/ui/admin/draw.rs` (`DrawConsole` is never rendered in a test; `Sent`, `use_settle`,
  `drawing.peek()` at `:59-86,105-128,611-628` were added by dd24d94 "not clicked through in a browser yet");
  `apps/worker/evening.mjs:231,354` (every draw goes through `api("draw_item")`; the admin page is only asserted
  to show the title, `:262-266`).
- **Accept**: `evening.mjs` keeps the admin tab on `/admin/r/<id>` during the draw and clicks with trusted CDP
  clicks (`click()` from `smoke.mjs:46-49`): Draw → spinner → "Drawing now" within 3 s → after the reveal the row
  shows the winner and Claimed → Claimed → row reads "claimed"; second prize: Not here → Draw re-enabled; Finish →
  toast; a double-click on Draw yields exactly one `announced` in the engine's state (`evening.mjs:254` already
  reads it). (½ day)

#### T4-2 · Interleaving tests for Tier 0 — **done**

- **Accept**: each of T0-1, T0-2, T0-3, T0-5, T0-6, T0-7 and T0-8's receipt has the test named in its own
  Accept, and `docs/development/testing.md` gains a section "Interleavings" describing the pausable `LocalEngine`
  and the store hook used to run a second actor between two awaits.
- **Done**: `Db::with_hook` (rafi-store's `hooks` feature, on for rafi-app's tests only), `paused_at` and a gated
  engine in `crates/app/tests/use_cases.rs`; `testing.md` § Interleavings.

#### T4-3 · The Worker has never fetched real drand; failover and stall-then-recover are untested

- **Where**: `apps/worker/src/engine.rs:55-59` (`TowerTransport<FetchService>`); `crates/engine/src/conformance.rs:148-171,
  188-189` (`Withheld` is one dead relay forever, `Forged` always fails verification; nothing has a relay succeed
  over HTTP or the second relay take over).
- **Accept**: a conformance profile `HonestRelay` (a local HTTP relay serving mock-chain-signed beacons with
  `mock: false`): (i) a draw completes over HTTP; (ii) first relay dead, second honest → completes; (iii) relay
  503 twice then honest → one `stalled`, then `drawn` on the same round. An evening variant with
  `RAFI_DRAND_MOCK=0 RAFI_DRAND_RELAYS=http://127.0.0.1:<relay>` on the Worker. (1 day)

#### T4-4 · `tools/verify.py` has no negative vectors

- **Where**: `tools/verify.py:219-249`, `testdata/draw-vectors.json` (ten honest `select` vectors, one honest
  scenario); the Rust verifier has eight tamper tests (`crates/raffle/src/verify.rs`).
- **Accept**: `draw-vectors.json` gains a `tampered` list mirroring the Rust cases (winner index, round switch,
  pool change while pending, manifest byte, unknown event type) with `expect: "FAIL …"`; `just verify` asserts
  each fails; a real `verify.json` from `the_evening_over_http` (with `format`, `sources`, `test_mode`) is fed to
  `verify.py` in `just verify`. (2 h)

#### T4-5 · The list import is never clicked

- **Where**: `apps/web/src/ui/admin/list.rs:99-170,220-316` (file input via `read_bytes`, the 150 ms debounce and
  generation guard, the send/400-with-rows/retry/undo flow); parser, detection and preview are tested natively on
  12 fixtures.
- **Accept**: `evening.mjs` → People → Add people → list tab: `Input.insertText` the `semicolon-bom.csv` fixture,
  assert "N to add", click Add, assert the People count; `DOM.setFileInputFiles` with `quoting.csv`; Undo; no CSP
  violation. (½ day)
- **Done**: in a raffle of its own, at a person's pace (each dialog's transition run out before the next click):
  the paste (3 to add, added, People counts 3), Undo last add, confirmed (nobody left), and the upload (5 to add,
  added, by name), as accepted; and a retry: a third list whose add reaches the server and whose reply is failed in
  the browser (`Fetch.failRequest` at the response stage), so the dialog says adding again is safe, and Add again
  sends the same batch ID and adds nobody twice. Besides the CSP check, the admin tab must throw nothing. (It once
  tolerated an uncaught `AbortError` from the Undo confirmation, taken out of the page during its exit transition:
  dioxus-primitives at bf007c1 didn't catch it; gone since the rev moved to 9a75825, which has the upstream fix.)

#### T4-6 · Housekeeping on Workers never runs with anything due

- **Where**: `scripts/workers-check.sh:135-159` (the cron is fired once after the evening on zero due rows);
  `crates/app/src/housekeeping.rs:111-161` (`finish_at_ceiling` over the ingress and `end_channels` destroying
  Durable Objects are never exercised on Workers).
- **Accept**: after the evening, `wrangler d1 execute … UPDATE raffles SET erase_at = 0, finished_at = 0 WHERE id
  IN (…)`, fire `/__scheduled` again, assert `entrants.erased = 1` for the evening's raffles, `live_ended_at`
  set, and a new socket gets 4403. (1 h)

#### T4-7 · Pass 2 (more prizes than people) is tested only in the raffle core

- **Where**: `crates/raffle/src/raffle.rs:903-921`; `crates/live-client/src/view.rs:228-233` (`win_of` returns
  the latest win only, so a double winner's phone shows one prize); the thanks screen, the CSV and `on_claim`'s
  Undeliver are untested above the core.
- **Accept**: a server test with 2 entrants and 3 items: all drawn, `/participant` shows 2 wins for one entrant
  (after deciding what the phone shows for two), the CSV has 3 rows, the audit has `undeliver` when a sent-later
  prize follows an on-the-spot one, and a fourth draw returns 409 with "There are no eligible entrants left."
  (2 h + a product decision)
- **Done**: the decision: the phone lists every prize its entrant holds, each with its own note (to collect, enjoy,
  or sent later), under one "YOU WON!" and above one claim code; one prize renders as before. `wins_of` replaces
  `win_of`, `Screen::Won` carries the prizes, and `Card` and `fallback.js` render them alike; a prize not held
  before is announced (`won_since`, by name, counted), a second one too: the WebAssembly remounts the card for it,
  `fallback.js` no longer announces once only. The view's wire format is unchanged (it already had one entry per
  prize). The CSV had one row per entrant, so a double winner's first prize was missing: it now has a row per prize
  held (`routes.md` says so). `draws::more_prizes_than_people` has 2 entrants and 4 items, not 3: book and mug on
  the spot, then the voucher (sent later) to one of them in pass 2: `/participant` lists 2 prizes for them, the CSV
  has 3 rows, the audit has `deliver` for both claims on the spot and `undeliver` for the voucher's; the sticker
  goes to the other in pass 2, who declines it, and drawn again it is refused 409 "There are no eligible entrants
  left." (with 3 items a fourth draw has no item to name). The Workers evening checks Anna's phone lists "Rust in
  Action" and "Ferris plush" with the code once, the second announced afresh. The thanks screen and the admin's
  People row (one prize per entrant) are unchanged. Since then the People tab lists every prize too: the admin's
  entrant carries `prizes` (each held prize's name and `drawn` or `claimed`, in draw order) instead of `prize`, from
  the walk over the draws the CSV uses, a badge per prize; `outcome` is as before (a claim over anything else), and
  one Mark delivered covers all of a winner's prizes; the server test, a People row test and the evening check it.

#### T4-8 · Concurrency scenarios run on Postgres only

- **Where**: `crates/store/src/conformance/mod.rs:208-235` (`join_cap_race`, `add_cap_race`, `erase_race`,
  `batch_rows_under_writes` need `CONCURRENT_SESSIONS`; skipped on in-memory SQLite's single connection and on
  D1's `Capabilities::NONE`); self-hosted production is a SQLite *file* with 8 connections, never a harness.
- **Accept**: a `SqliteFile` harness in `crates/store/tests/conformance.rs` with `CONCURRENT_SESSIONS`; a `/race`
  route in `testbeds/store-d1` firing 16 parallel joins via `Promise.all`. (2 h)

#### T4-9 · The device matrix is desktop Chromium only

- **Where**: `evening.mjs:59-60,340`, `smoke.mjs:27` (no `mobile: true`, no touch, no WebKit or Firefox, the
  join form submitted by `requestSubmit()` rather than a tap; iOS Safari CSP "not yet" per the #23 commit; no
  axe; no screenshots).
- **Accept**: Playwright `webkit` for `/j/<code>` join + fallback + `/s/` (the `connect-src wss:` for Safari is
  the point); `Emulation.setDeviceMetricsOverride({mobile: true, width: 390, height: 844})` on the phone tab and
  a trusted tap on `#submit`; `axe-core` injected on `/j`, `/s`, `/v`, `/admin/r/<id>` failing on `serious` and
  above; one screenshot per surface per theme committed under `docs/screenshots/` and diffed by hand at release.
  (1 day)

#### T4-10 · Clerk on Workers is checked by hand only

- **Where**: `apps/worker/src/lib.rs:120-129`; every automated Workers check runs dev auth (`wrangler.toml:178`).
  Known as #33.
- **Accept**: a Workers run without `RAFI_DEV_AUTH`, with `CLERK_SECRET_KEY` set and `TestClerk`'s JWKS served
  from a local stub if `ClerkAuthLayerConfig` takes a JWKS URL (else a CI secret for a Clerk dev instance):
  `whoami` 401 without a cookie, 200 with a minted one, 503 with the stub down. (½–1 day)

#### T4-11 · Test infrastructure

- **A tracing test flakes under parallel execution** (reproduced by the second review; shared callsite state in
  the capture). **Accept**: the capture test is in its own test binary; `cargo test -p rafi-server` passes ten
  times in a row in parallel.
- **`workers-check.sh` doesn't refuse an occupied port**: the second review's evening hit a developer's own
  `wrangler dev` on 8787. **Accept**: the script fails fast with "port 8787 in use" before any request.
  **Done**: `serve` refuses a port something listens on. It bit again on 2026-11-02: a `wrangler dev` left over
  from an earlier session held 8789, and the D1 suite answered from a stale build (44 scenarios, not 45).
- **Silent green without `RESTATE_SERVER_BIN`**: the five `#[ignore]` tests return early as *passed* locally.
  **Accept**: they print `skipped: RESTATE_SERVER_BIN unset` and `just test` ends with a count of skipped tests.
- **Likeliest CI flake**: `screenAt − phoneAt < 1500` on a 50 ms CDP polling loop (`evening.mjs:243-244`).
  **Accept**: widened to 2500 with the measured value logged, or the assertion made on the DO's publish
  timestamp.
- **Fixed ports** 8787/8789–8792, 9334/9335, 15122/18080/19070: documented in `testing.md` with the one-line
  override.
- **Property tests** exist only in `rafi-raffle` and `rafi-lifecycle`. Pure candidates: `import::parse`/
  `decode`/`detect` (arbitrary bytes never panic: a panic kills the admin page's wasm), `name_key`/`unique_names`
  (idempotent, unique, ≤ 64 chars), `display_name`/`added_name` (NFC idempotence), `entrant::key`, `ids::slug`,
  `csv_field` round trip, `masked_email`, `screen()` vs `fallback.js` parity on table-driven JSON. **Accept**:
  `import::parse` and `unique_names` have a proptest each before the tag; the rest to the tracker.

## Tier 5 · User experience

Reviewed by reading every `rsx!`, the SSR pages and the guides. The product's affordances are server-driven
(`AdminActions`) and double-submit-safe throughout; the weaknesses are orientation (what next, where did the winner
go, is this thing alive) and copy that leaks the engine's vocabulary.

#### T5-1 · After the reveal the winner is buried in the item list

- **Where**: `apps/web/src/ui/admin/draw.rs:188-208` (the countdown card renders only while `pending`), `:277-313`
  (the winner appears inline in whichever `li` was drawn). With 3 × T-shirt + 5 prizes that is eight rows; on a
  phone the drawn row is below the fold.
- **Change**: a sticky "Now" panel at the top: the pending draw, then every drawn item awaiting a decision with
  its `WinnerCard` and `Claimed / Not here / Declined`; the list below is the queue. Group identical available
  items ("T-shirt · 2 of 3 left · Draw") until one is drawn. The Claimed button names the winner ("Claimed ·
  Anna K.") when more than one waits.
- **Size**: small–medium.
- **Accept**: with eight items and one drawn, the winner card and its buttons are within the first 600 px of
  the Draw tab at 390 × 844 and at 1280 × 720 (a `rafi-web` SSR test on element order plus a screenshot in
  `evening.mjs`); T4-1's click-through passes against the new layout.

#### T5-2 · The People tab is unusable at 390 px, which is where it is used at the door

- **Where**: `apps/web/src/ui/admin/people.rs:92-101` (a five-column `table` in `overflow-x-auto`; the actions cell
  with up to six `btn-xs` buttons is the rightmost column, so hiding a troll's name needs a horizontal scroll
  inside the table and a 24 px target); entrants sort oldest-first (`crates/store/src/entrants.rs:47`), so the
  person who just joined is at the bottom.
- **Change**: below `sm`, each `EntrantRow` renders as a card (name and badges above, actions in a row below);
  the table from `sm` up; icon buttons `btn-sm`; newest first by default with the search as it is; paginate at
  200 rows (T3-9).
- **Size**: small–medium.
- **Accept**: at 390 px every action of a row is reachable without horizontal scrolling and every target is ≥ 40
  × 40 px (an `evening.mjs` check on `getBoundingClientRect`); the first row is the latest join.

#### T5-3 · First run shows warnings, not a path

- **Where**: `apps/web/src/ui/admin/header.rs:253-261` (a Draft raffle lands on a warning alert "Before
  registration can open, fill in organizer and contact email…", then the primary button "Publish the intro
  screen", which is the wrong next step with zero prizes, then stats of zeros and the retention card,
  `mod.rs:494`); nothing in the admin links to `docs/guide/running-a-raffle.md`; test mode is discoverable only at
  the bottom of a 12-field settings form (`forms.rs:908-916`) though `running-a-raffle.md:107` makes rehearsing
  step 4; the "No raffles yet" state (`mod.rs:213-219`) has no guide link.
- **Change**: in Draft and Published, a "Before doors open" checklist from server state: ☐ Add a prize (0) → ☐
  Organizer and contact email (switches to the tab) → ☐ Rehearse in test mode (optional) → Publish; a "Guide"
  link in `AdminShell`'s header and in the empty state.
- **Size**: small.
- **Accept**: a new organizer's path from Create to the first draw needs no more than the clicks in the table
  under §Happy path minus the three marked avoidable; the SSR test for a Draft raffle renders the checklist and
  no warning alert.

#### T5-4 · Accessibility gaps on the two forms that matter

- **Where**: `apps/web/src/ui/admin/people.rs:326-331,333,361-366` (icon-only hide/rename/remove buttons have
  `data-tip` but no `aria_label`; `prizes.rs:137-138` does it right); `crates/server/src/pages/participant.rs:258-271`
  (`fieldset { legend {…} input#name }`: a legend names the group, not the control; VoiceOver reads "edit
  text"); `:289` (the disabled "Getting ready…" submit is unfocusable and silent); `apps/web/src/ui/forms.rs:262`
  (the delivery selector's radio keyboard behaviour is incomplete).
- **Change**: `aria_label` = the tip text; `label { r#for }` per input; an `aria-live="polite"` status line
  beside the submit; native radios for delivery (or arrow-key handling).
- **Size**: trivial.
- **Accept**: axe (T4-9) reports no `serious` or `critical` on `/j/<code>`, `/admin/r/<id>` People and Prizes.

#### T5-5 · Destructive and irreversible actions

- **Where**: `apps/web/src/ui/admin/prizes.rs:138` (delete prize: one tap, no confirm, beside Edit);
  `people.rs:334-341` ("Mark delivered" is a toggle styled as a success badge; tapping it *un*-delivers, and
  after the erasure date marking delivered erases the winner within the hour, `running-a-raffle.md:283`, with no
  confirm); `header.rs:302-310` ("Close registration" and "Unpublish" one tap, un-confirmed, in the primary
  button's row; closing cuts phones off mid-form).
- **Change**: `Confirm` on delete prize (or a toast with Undo); delivered as a non-interactive badge with a
  separate "Undo" and a confirm when `past_erase_date`; "Close registration" gets a toast with "Reopen";
  "Unpublish" confirms once the raffle has entrants.
- **Size**: small.
- **Accept**: every action that deletes, erases, cuts people off or can't be undone has either a confirm naming
  the consequence or an undo, listed in a table in `docs/development/web-app.md`.

#### T5-6 · Flow and orientation

- After Freeze you land on the tab you were on, not Draw (`mod.rs:331,356`: `tab` is `Some("settings")` once
  touched). **Accept**: the freeze's success path sets `tab` to `draw`.
- Error toasts with a reference ID auto-dismiss in 5 s (`mod.rs:98-110`, `toast/component.rs:262`). **Accept**:
  errors are `permanent: true` with a Copy button for the reference.
- Network failures read as stack traces: `api.rs:66` `"Could not reach the server: {e}"` where `e` is `JsValue
  (TypeError: Failed to fetch)`. **Accept**: "Couldn't reach Rafi. Check the venue wifi and try again."; an
  "Offline" badge in `AdminShell` from `navigator.onLine` (T1-4).
- A 401 ("Please sign in.", `api.rs:94`) is a 5 s toast. **Accept**: a blocking re-sign-in card.
- "This email is already entered" hides the form with no way back (`pages/participant.rs:211`). **Accept**: the
  form stays below the notice, or "Not you? Use another email".
- The proof-of-work wait is unexplained and its failure enables the submit anyway (`pages/participant.rs:289-292`,
  `extras.rs:234-237`). **Accept**: "Checking this isn't a bot… (a few seconds)"; on failure "Couldn't reach the
  server for the check. Tap to retry" with submit disabled.
- Raffle cards show no event date (`mod.rs:235-250`; `RaffleSummary` lacks it, `crates/api/src/lib.rs:122-130`).
  **Accept**: `event_date` on the summary and the card.
- The verification page's commands can't be run as shown (`verify.rs:328-329`: `curl -O /api/r/{slug}/verify.json`
  relative; `python3 verify.py` with no link). **Accept**: absolute URL from `location.origin`; the verifier
  linked (T2-6).
- The projector doesn't show the join code the docs promise (`running-a-raffle.md:123`; `screen.rs:205-208` shows
  the URL only). With T6-4.
- The projector overflows with 10+ items or many draws (`screen.rs:113-132,388-417`; nobody scrolls a
  projector). **Accept**: `History` shows the last five and "+N more"; `PrizeList` collapses prizes with `left ==
  0` beyond eight; the thanks screen pages winners; `evening.mjs` asserts no vertical scroll at 1280 × 720 with
  12 items and 10 draws.
- The thanks screen doesn't say what winners do (`screen.rs:420-454`). **Accept**: one line: "Winners: collect
  your prize at the front. Vouchers are emailed by the organizer."
- The QR's quiet zone is ~1.5 modules (spec 4) and the code is tilted (`screen.rs:100-106`, `quiet_zone(false)`,
  `rotate-2`). **Accept**: `quiet_zone(true)`, rotation on the card's shadow only; a phone scans it from 8 m on
  a 1080p projector (checked once by hand).
- Page titles: `/admin` and `/v/<slug>` inherit "Rafi" (`mod.rs:72`). **Accept**: `document::Title` on both.
- The admin tab isn't in the URL, so a reload mid-evening drops you. **Accept**: `/admin/r/:id?tab=people`.

#### T5-7 · Copy

The ten worst strings; sentence case and trailing periods are otherwise consistent.

| Where | Now | Better |
|---|---|---|
| `apps/web/src/ui/forms.rs:238` | "…their data goes on the non-winners' erasure clock." | "Handed over at the event when the winner shows their code. Pressing Claimed also marks it delivered." |
| `apps/web/src/ui/admin/mod.rs:192` | "…check the scheduled job (the Cron Trigger on Workers)." | "Automatic data erasure seems to have stopped. Tell whoever runs this Rafi installation." |
| `crates/app/src/error.rs:400` | `"Cannot {command:?} a raffle that is {}."` (Debug-cased verb) | "This raffle is already {state}. Reload the page to see where it is." |
| `crates/app/src/error.rs:403` | "Restate is not configured on this server." | "This server has no raffle engine set up, so draws can't run. Contact whoever runs it." |
| `crates/app/src/error.rs:439` | "This add has no valid batch ID…" | "Something went wrong with this add. Reload and try again; nobody was added twice." |
| `crates/server/src/participant.rs:311` | "Could not verify that you are human. Please reload…" | "The bot check didn't complete. Please tap Join again." |
| `crates/server/src/participant.rs:310` | `Limited \| Failed => "Too many attempts…"` (a 503 blames the user) | split: `Failed` → "Something went wrong on our side. Please try again in a moment." |
| `apps/web/src/ui/admin/list.rs:29` | "Not in test mode: a rehearsal's draws are public…" (reads as the opposite) | "Lists can't be added while test mode is on…" |
| `apps/web/src/ui/admin/header.rs:232,313` | "mock drand" / "Retry handing the list to the raffle engine" | "Test randomness (not verifiable)" / "Retry starting the draw" |
| `apps/web/src/ui/admin/draw.rs:202` | "Waiting for public drand round {n}, committed before it exists." | "Waiting for public randomness (drand round {n}) that doesn't exist yet, so nobody can know the result." |

Also: `error.rs:493,496` "at once once"; "prize(s)"/"winner(s)" at `draw.rs:321`, `header.rs:125`, `mod.rs:497`
where the rest pluralises; "Took themselves off" as a button (`people.rs:348`) → "Take off at their request";
`participant.rs` "Registration is closed." → "Joining has closed." (the participant's verb is join); the
hardcoded "24 characters" (`participant.rs:304`) and "1–64" (`crates/api/src/lib.rs:436,442`) should interpolate
`MAX_NAME_CHARS` / `MAX_ADDED_NAME_CHARS`. **Accept**: each row above is changed or consciously kept with a note
here; `rg 'command:\?' crates/app` is empty.

#### Happy path: sign-in to the first finished raffle

Twenty clicks and five typed fields today (`/admin` → Create → prize with a forced delivery choice → Settings for
organizer and contact → Publish → projector tab → Open → Freeze (two) → find the Draw tab → Draw → find the drawn
row → Claimed → Finish (two) → Export (two)). Three are avoidable: the delivery default (T5-5's B1: default
`OnTheSpot`), landing on the wrong tab after Freeze (T5-6), scrolling to the drawn row (T5-1). **Accept**: 17
clicks on the same path after this tier, counted in `evening.mjs` by the number of trusted clicks it needs.

## Tier 6 · Visual design

Scores as a 2026 web app, before this tier: admin 7/10, projector 6/10 light · 7.5/10 dark, participant 7.5/10,
home 6.5/10. The token discipline is already right (two hand-tuned daisyUI themes, zero palette classes in
markup, an em-scaled projector, one icon set); what is missing is finish and contrast on the moments that
matter. Inventory: 106 emoji in 17 files, about 30 in semantic roles; 11 named text sizes + 28 arbitrary (em on
the projector); a clear 4 px spacing rhythm; `sm:` 22 · `md:` 12 · `lg:` 4.

#### T6-1 · The projector defaults to dark and scales its chrome

- **Where**: `apps/web/src/ui/screen.rs:54` (`bg-base-100 bg-dots`; the theme follows the presenter's laptop,
  `tailwind.css:63`); `:58,227,279,353,408-409` (daisyUI `badge-lg` is rem-locked at 16 px while the body is 28.8
  px at 1080p: the prize badge above the winner's name, "DRAWING", "TEST MODE", the recent names; "not
  here"/"declined" 12 px; `loading-lg` 28 px); `:67,116,394` (`/50` labels at 3.2:1), `:318` (`bg-base-200/60`
  stage at 1.1:1); the root `clamp(16px, 1.5vw, 34px)` under-scales a 4K wall (0.9 vw).
- **Change**: `div { "data-theme": "rafi-dark", … }` around the screen unless `localStorage rafi-theme` was set
  (the toggle still works); em chips instead of `badge-lg` (`inline-flex rounded-full bg-secondary px-[0.9em]
  py-[0.3em] text-[0.9em] font-bold text-secondary-content`), `size-[2em]` loaders; `/50` → `/70`, the stage
  solid; `clamp(16px, 1.5vw, 58px)`.
- **Size**: small.
- **Accept**: a 1080p screenshot in `evening.mjs` where every text node on the projector measures ≥ 0.8 em of
  the root (an injected script walks the DOM and reports the smallest `font-size`); contrast ≥ 4.5:1 for every
  visible text (T1-6's checker run on the computed colours).

#### T6-2 · Brand and link previews: favicon, theme-color, OG

- **Where**: `apps/web/index.html:3-16`, `crates/server/src/pages/participant.rs:129-136` (no favicon,
  `apple-touch-icon`, `theme-color`, `og:*` or `twitter:card`; `rg 'favicon|og:image'` is empty; `apps/web/assets/`
  has fonts only).
- **Why**: the join link *will* be pasted into Slack and WhatsApp and previews as nothing; the tab shows the
  default globe. `noindex` doesn't stop OG scrapers, so this is safe to add.
- **Change**: `assets/favicon.svg` (the tilted ticket from `Brand`), `apple-touch-icon.png` (180 px),
  `theme-color` for both schemes; in the SSR `document()`: `og:title={raffle.title}`, `og:description="Join the
  raffle from your phone — no app, no account."`, `og:image=/assets/og.png` (1200 × 630, ticket + wordmark),
  `twitter:card=summary_large_image`; the same static set on the shell.
- **Size**: small–medium (the OG image).
- **Accept**: a `/j/<code>` link pasted into Slack shows a card with title, description and image (checked once
  by hand); `curl -s /j/<code> | rg 'og:image'` matches; the favicon shows in Chrome, Safari and Firefox.

#### T6-3 · A blank cream page while 1 MB of WebAssembly loads; the font preload does nothing

- **Where**: `apps/web/index.html:18` (`<div id="main"></div>`); `apps/web/src/ui/mod.rs:65-71` (the Bricolage
  preload is injected by Dioxus *after* the wasm, so the font is discovered when the first `h1` paints: FOUT on
  the title and the winner's name); the shell preloads the JS only.
- **Change**: static markup inside `#main` (`main.grid.min-h-screen.place-items-center` with the brand and
  `loading-dots`; CSP-safe, no script); `<link rel="preload" as="font" type="font/woff2" crossorigin>` in
  `index.html` and `participant.rs:130-134`; the Dioxus one removed; `preload as="fetch"` for the hashed wasm.
- **Size**: small.
- **Accept**: a Lighthouse run on `/s/<slug>` over "Slow 4G" shows first contentful paint < 1.5 s and no layout
  shift from the font; `evening.mjs` asserts the brand is visible before the wasm's `fetch` completes.

#### T6-4 · A giant join code on the projector

- **Where**: `apps/web/src/ui/screen.rs:205-208` (`or visit rafi.example/j/ABCDEF` in `font-mono text-[1.05em]`,
  ~30 px at 1080p; the biggest thing on a Kahoot screen is the PIN); `grouped()` exists at `admin/header.rs:410`.
- **Change**: under "Scan to join": `font-mono text-[3em] font-black tracking-[0.2em] tabular-nums` with the
  grouped code, the domain beneath in `text-[1em] text-base-content/70`; `grouped` moves to `ui/mod.rs`.
- **Size**: trivial.
- **Accept**: the code is legible in a 1080p screenshot downscaled to 480 px wide (what the back row sees);
  `running-a-raffle.md:123` ("the short join URL and code") holds.

#### T6-5 · Emoji as semantic icons, mixed with Lucide

- **Where**: `apps/web/src/ui/admin/mod.rs:76-87` (`state_emoji` ×8), `draw.rs:269-275` (item status),
  `forms.rs:229-230` (delivery labels), toast titles (`header.rs:51,111,125,278`, `draw.rs:162`,
  `people.rs:170`, `forms.rs:341,507,770,789`), button labels (`header.rs:372`, `draw.rs:338`); the projector's
  History `🏆🎉👻` (`screen.rs:400-402`) beside a Lucide `Trophy` at `:353`.
- **Change**: the policy "emoji only as decorative hero glyphs (empty states, the phone's celebration, the 404)";
  semantic ones become Lucide (`FilePen/Megaphone/Ticket/Pause/Lock/Dice5/PartyPopper/BrushCleaning` for states;
  `Gift/Dice5/PartyPopper/Check/Ban` for item status; `Handshake/Mail` for delivery; History `Trophy/PartyPopper/
  Ghost` at `size-[1.1em]`); stripped from toast titles and buttons. `👻` for a declined winner is a little mean:
  a neutral mark.
- **Size**: medium.
- **Accept**: `rg -c '[🎁🎲🎉✅🚫🔒🧪📝🏆👻]' apps/web/src/ui/admin` finds them only in empty states and toasts'
  bodies; the policy is one paragraph in `docs/development/web-app.md`.

#### T6-6 · Button colour discipline, motion, polish

- **Buttons**: up to five colours visible at once in the header (`header.rs:264,292,297,300,303-309,313,319`) and
  the draw rows (`draw.rs:289,296,461,307-308`). Primary = the next lifecycle step only; "Projector screen" and
  "Add …" outline or soft; `btn-success` only for Claimed, `btn-error` only for destructive dialog actions.
  **Accept**: at most one `btn-primary` per view (a `rafi-web` SSR test counts them per page).
- **Secondary text contrast**: `stat-value text-secondary` (`admin/mod.rs:460`), coral on cream 2.9:1. Use the
  secondary as a fill, or darken to `#e6336b`. With T1-6's checker.
- **Motion**: toasts appear with no animation (`components/toast/component.rs:318-320`); dialogs over a plain
  40 % dim (`alert_dialog/component.rs:224-226`); confetti is one 1.8 s radial puff (`tailwind.css:203-229`).
  `animate-pop` on toast items, `alert-soft` like the rest of the admin, `backdrop-blur-sm` on the dialog
  backdrop, a two-phase confetti (rise, then fall and spin, 2.6 s, staggered by `--i`, two sets on `Finished`).
  **Accept**: `prefers-reduced-motion` still disables all of it (the existing rule).
- **Participant safe areas**: `participant.rs:131` has `viewport-fit=cover` but `:345` has plain `px-4 pt-4
  pb-10` and `rg 'env\('` is empty; in landscape the title sits under the notch. `max(1rem, env(safe-area-inset-
  *))`. **Accept**: a WebKit run (T4-9) with `mobile: true` in landscape shows the brand clear of the inset.
- **One-liners** in `tailwind.css`: `::selection` in the primary's tint; `input:-webkit-autofill` painted
  `base-100`; `tabular-nums` on the three counters (`screen.rs:220`, `card.rs:140`, `fallback.js:67`); `@media
  print` hiding header, toasts and buttons for `/v/`.
- **Home**: a footer with Terms · Privacy · Source · "Randomness by drand" (`mod.rs:131-187` has none); ideally
  a live `Intro` with a sample view in the hero instead of the ticket.
- **Size**: small each.

## Tier 7 · Architecture

Checked and found solid: the layering in `architecture.md` is real in the code; `rafi-store`'s SQL is disciplined
(76 `format!` sites, every interpolation an enum list or a constant fragment; `Dialect` has zero production
branches; parity by 40 conformance scenarios on four backends); `admin_functions!` is compile-enforced on the
server side with by-name arguments; the live path has exactly one push implementation and a two-method
`Notifier`; config is one struct, one `from_lookup`, one `validate`; 147 `cfg` attributes with a maximum of 14 per
file. Nothing here blocks the release; each accrues a copy a week.

#### T7-1 · `RaffleView` is a public URL with no format tag and no "not a contract" note

- **Where**: `crates/api/src/lib.rs:13-71` (`/api/r/{slug}/view`, in `routes.md`; `VerifyData` has `format` at
  `:735`, this doesn't).
- **Change**: a paragraph in `routes.md` and on the struct: "unversioned, consumed by the web app of the same
  deployment, may change without notice; use `verify.json`" — or give it `format` now. Decide before the first
  external reader exists.
- **Size**: trivial. **Accept**: one or the other is in `routes.md` before the tag.
- **Done**: the note, in `routes.md` and on `RaffleView`.

#### T7-2 · Twelve hand-rolled "guarded write, then re-read to explain" blocks

- **Where**: `crates/app/src/raffles.rs:250-273`; `prizes.rs:46-60,76-81,102-108,127-133`;
  `entrants.rs:651-660,702-712,763-774,820-825,878-896,933-944`. Each runs a SQL-guarded write, and on
  `!written` re-reads the row, re-runs the pre-check's predicate and returns its refusal or `Refusal::Changed`;
  each differs subtly.
- **Change**: `async fn guarded<T>(app, id, precheck: impl Fn(&RaffleRow) -> Result<(), Refusal>, write: impl
  Future<Output = Result<bool>>) -> Result<()>`; the three deliberate special cases (`delete_prize` tolerates
  "gone", `undo_add` returns `Ok(0)`, `object_entrant` is idempotent) keep their own code. An `Audited` helper so
  the 23 `audit::record` calls stop repeating `&app.db, id, actor, …, now`.
- **Size**: medium. **Accept**: the twelve sites call the helper; the use-case tests are unchanged and pass.
- **#51** (fixed): a row gone by the re-read is not found. `update_settings` explains through `unwritten` (`Changed`
  where no fixed setting would change, never `SettingsFixed` naming none); `patch_entrant`, `object_entrant` and
  `unobject_entrant` through an entrant-level sibling, `unwritten_entrant` (raffle or entrant gone: not found; else
  the pre-check, `patchable`, `objecting` or `unobjecting`, on the rows re-read), so an entrant erased meanwhile is
  `Erased`. Four interleaving tests in `use_cases.rs` (`paused_around`).

#### T7-3 · Tier authorization lives in the HTTP adapter

- **Where**: `crates/server/src/admin.rs:157-163` (`allow`), called at `:187,198,216,221,232,420`; `Actor<'a> {
  owner, user }` carries no access level; `impl Admin for AppState` (`:165-423`) is a 34-method pass-through whose
  content is those six calls.
- **Why**: `rafi-app`'s tests can't test gating; the Clerk-deletion webhook (#31) and any CLI must replicate it.
- **Change**: `Actor { owner, user, access }`; `allow()` moves into the use cases that need it; `impl Admin for
  rafi_app::App` directly, `AppState` derefs.
- **Size**: small–medium. **Accept**: a use-case test that a `Manager` can't `remove_my_data`; `admin.rs` has no
  `allow`.

#### T7-4 · `Refusal` bookkeeping: three parallel 50-arm matches and an implicit fourth

- **Where**: `crates/app/src/error.rs:220-239` (`invalid`), `:247-302` (`code`), `:319-503` (`Display`); the
  HTTP status derived in `crates/server/src/admin.rs:103-119` from `invalid()` plus two special cases. Also
  `error.rs:61-68` (`Refusal::Engine { message }` surfaces the engine's error verbatim while `:545-572`
  rewrites the same errors into organizer wording: two voices for one error).
- **Change**: `Refusal::kind() -> Kind { Invalid, State, Forbidden, Limited }` replacing `invalid()`; `app_error`
  maps `Kind` → status in one match; `code()` derived from the variant name; both engine paths through
  `raffle_refusal`.
- **Size**: small. **Accept**: adding a refusal is one arm in `Display` and one in `kind()`; a test asserts every
  variant has a `code` matching its snake_case name.

#### T7-5 · Web: the same action closure twenty times

- **Where**: `apps/web/src/ui/admin/header.rs:31-149` (×8), `draw.rs:58-170` (×5), `people.rs:218-270` (×5),
  `prizes.rs` (×2): the 15-line `let id = …; move || async move { match api::X(…).await { Ok => on_change,
  Err(e) => error.set(…) } }`; 13 `ask_* = use_signal(|| false)` + `Confirm { open: ask_* }` pairs; loading
  dots / `alert alert-error` / dashed empty state hand-written five or six times each; `AddList` (`list.rs:71-430`)
  holds 12 signals.
- **Change**: `use_action(|args| api::x(args))` returning `(busy, run)` with `error` and `on_change` from
  context; `use_confirm()` returning the signal and the element; an `Async<T>` component over
  `Resource<Result<T, ApiError>>` with loading/error/ok closures; `AddList` as `enum Step { Empty, Reading,
  Mapped {..}, Sending {..} }` in one signal.
- **Size**: medium. **Accept**: a new admin action is ≤ 5 lines at the call site; `rg -c 'use_signal\(\|\| false\)'
  apps/web/src/ui/admin` drops by 13.

#### T7-6 · Smaller

- **Audit vocabulary** is ~30 string literals across eight files (`"create"`, `"deliver"`, `"patch_entrant"`,
  `'auto_erase'` in SQL at `retention.rs:46,153,169,222,280`, `raffles.rs:526,534`), write-only, read by no
  production code. `audit::Action` enum with `as_str()`, used by the app and the store's SQL, documented in the
  schema comment. (small) **Accept**: `SELECT DISTINCT action FROM audit` after the test suite equals
  `Action::ALL`.
- **Route literals** built in five places for `/j/{}` (`participant.rs:210,239,257`, `pages/text.rs:200,292`) and
  more for `/api/r/`, `/s/`, `/v/`, `/rules/`, `/privacy/`. `rafi_api::routes::{join, view, …}`. (small)
- **Two schema files hand-maintained in parallel** (`migrations/sqlite` vs `postgres`, identical but for
  `INTEGER`/`BIGINT` and trigger syntax). A test normalising both and asserting equal tables, columns and
  indexes, before `0002_*` exists. (small)
- **`crates/lifecycle/src/entrant.rs` is seven concerns** (source, names, email, join, fakes, adds, removals;
  792 + 450 lines). `entrant/{source,name,email,join,add,removal}.rs`. (small, mechanical)
- **Two test files of 5,678 and 3,389 lines** (`crates/server/src/tests.rs`, `conformance/domain.rs`). Split by
  concern. (small)
- **Postgres NULL parameters** cost a re-prepare per call (`backend/postgres.rs:198-210,342-377`). `CAST(?N AS
  …)` at the few sites; drop the retry. (low)
- **`erasures` has no index**; `reapply_*` `EXISTS` into it per entrant. `(raffle_id, entrant_id)` in `0001`
  while it's open. (trivial) **Done**: `erasures_entrant`, both dialects; the `EXISTS` is a covering-index search.
- **Workspace dependencies**: `tokio` (10 manifests), `anyhow` (10), `tracing` (7), `worker` (5), `dioxus` (5),
  `dioxus-clerk` (5), `js-sys` (5), `wasm-bindgen` (4), `tracing-subscriber` (4), `futures-util` (4), `axum` (4),
  `gloo-timers` (3) declared inline; move to `[workspace.dependencies]` like `serde`/`sha2`/`drand`. (trivial)
  **Accept**: `rg '^tokio = "' --glob '*Cargo.toml'` matches the root only.

## Tier 8 · Dead code, stale references, hygiene

The sweep (1,152 `pub` symbols cross-referenced, every dependency and feature checked, 276 relative links
resolved, every removed name grepped) found the tree clean apart from what the last two features left behind.
No `TODO`/`FIXME`, no commented-out code, no `todo!()`, no unused dependency or feature; `routes.md`,
`configuration.md` and `crates.md` match the code.

| ID | Where | What | Change |
|---|---|---|---|
| T8-1 | `devenv.nix:106` | `cargo-watch`: CLN-1 removed it on 2026-10-27, a130dd7 re-added it the next day; nothing invokes it (devenv's `watch.paths` restarts processes) | remove |
| T8-2 | `scripts/workers-check.sh:247` | sed patterns for `run_worker_first` and `not_found_handling`, both removed from `wrangler.toml` by #23 | drop the two `-e` clauses |
| T8-3 | `apps/web/src/api.rs:16-18` | `#[allow(dead_code)]` on every generated client function; the comment says the UI "never calls `whoami`" — it does (`ui/admin/mod.rs:172,325`), and all 31 functions have callers | remove the attribute and the comment |
| T8-4 | `crates/api/src/lib.rs:481-486`, `crates/app/src/reads.rs:47-48,128` | `RaffleAdmin.missing_for_open` sent on every read; no client reads it since slice 6 replaced it with `AdminActions.missing`; only tests assert it | drop the field and its two assertions (a wire field, now not later) |
| T8-5 | `crates/api/src/lib.rs:598,836` | `Retention.past_event`, `Me.dev_auth`: read only by tests (`server/tests.rs:4188,1146,1689`); the SPA learns the sign-in mode from `/config.js` | drop, or mark as diagnostics |
| T8-6 | `crates/api/src/lib.rs:129,245` | `RaffleSummary.created_at`, `AdminEntrant.created_at` required but never read by the UI; every web fixture fabricates them | read them (T5-6's date, T5-2's sort) or `#[serde(default)]` |
| T8-7 | `crates/server/src/tests.rs:1850-1874`, `store/conformance/domain.rs:797-799`, `store/raffles.rs:275-277`, `api/lib.rs:655-657` | "fixed once someone **accepted** them", `rules_are_fixed_once_accepted`: vocabulary aa96666 retired (`Source::accepted_rules` → `locks_settings`; added entrants accept nothing) | "once an entrant locks them"; rename the tests |
| T8-8 | `crates/app/src/ids.rs:57-80` vs `apps/web/src/import/columns.rs:204-216` | a Hungarian-only accent table in `slug()` while `columns::fold` does NFD + strip marks; any other accented letter becomes `-` | `slug()` uses a `fold_accents` re-exported from `rafi_lifecycle::names`; delete the table |
| T8-9 | `docs/plans/pre-release-review.md:975-980,1019-1020` | deferred #21 and #30 are done (5d5efd9, 47b52b8) but carry no **Done** marker like #23/#24/#25/#28/#32 | add the markers |
| T8-10 | `docs/plans/pre-release-review.md:1042-1043,1074-1076` | "`RESTATE_INGRESS_URL`, `RESTATE_IDENTITY_KEYS`" (renamed by CFG-1), "three `[[ratelimits]]`" (four since `ADD_LIMITER`), "`run_worker_first` covers every server route" (removed by #23) | fix the three statements |
| T8-11 | `docs/plans/pre-release-review.md` | status "done 2026-10-27" yet accruing "Since #38" notes; `entrant-import.md` moved to history the day it finished | move to `docs/history/` with the banner; the tracker carries §Deferred; this plan links it |
| T8-12 | `docs/development/web-app.md:45-46` | tells a fresh checkout to run `dx build` or `tailwindcss` by hand; `just css` exists and `getting-started.md:97-98` says so | point at `just css` |
| T8-13 | `docs/development/getting-started.md:113` | "dx proxies /api/, /j/, /rules/, /privacy/ and /participant/"; `Dioxus.toml` also proxies `/config.js`, `/terms`, `/healthz` | list them, or "the server's paths (Dioxus.toml)" |
| T8-14 | `migrations/*/0001_init.sql:102,104` | example `consent_version` `2026-10-21.2/age16`; the constant is `2026-11-01` | update the example |
| T8-15 | `migrations/*/0001_init.sql:2-3` | the header says "nothing is deployed yet" | at tag time: "frozen; changes are `0002_*` onward" |
| T8-16 | `.gitignore` | `.claude/` excluded only via `.git/info/exclude`; `.env.local` variants not ignored | add both |
| T8-17 | `.cargo/audit.toml:7-26` | ignores have reasons but no reviewed/revisit dates | add dates |
| T8-18 | `docs/plans/marketing-opt-in.md:101,200` | "Manual and fake entrants": the glossary says "added by you" | optional reword |

Duplicate dependency versions (`cargo tree --duplicates`): `base64` 0.22/0.23 (rafi-server could move to 0.23),
`sha2` 0.10/0.11, `getrandom` 0.2/0.3/0.4, `gloo-timers` 0.3/0.4, `tower-http` 0.6/0.7, and the elliptic-curve
and rand stacks: all upstream but `base64`; `deny.toml` warns on them already. **Accept** for the tier: the
table's rows are done or struck through with a reason; `rg -n 'allow\(dead_code\)' --type rust` matches only the
two justified sites (`engine-restate/src/client.rs:64`, `apps/web/src/main.rs:9`).

## Order of work

1. **Tier 0** (one to two days). T0-2, T0-3, T0-4, T0-6 first (trivial, and T0-2 touches the schema); then T0-1
   (the engine wire shape; the two-console test), T0-5, T0-7; T0-8's receipt and the retention decision with
   them since both touch `0001`.
2. **Tier 1** (one day). T1-1 first (the cleanest test), then T1-2, T1-3 (one line), T1-4, T1-5, T1-6.
3. **Schema and query shapes together** while `0001_init.sql` is still edited in place: T3-1, T3-7, T7-6's
   `erasures` index, T3-9's manifest-row measurement and its decision.
4. **Contract notes before the tag**: T7-1 (`RaffleView`), T0-8's `format` check, the retention-promise decision.
5. **Launch mechanics** (T2-1 → T2-2 → T2-3 → T2-4 → tag), with T2-5, T2-6 and T2-8's quick items in the same
   branch; T2-7's runbook in parallel.
6. **Design top five and UX musts**: T1-6 (done above), T6-1, T6-2, T6-4, T6-3; T5-1, T5-2, T5-3, T5-4.
7. **Tests** T4-1 … T4-6 as each fix lands (most are its acceptance test), T4-7 … T4-11 after.
8. **Tier 7 and Tier 8** as one sweep each; T8-11 at tag time.

Each step ends with `just ci` and `just workers`. The tag is cut when Tiers 0, 1 and 2 are green and the rest are
either done or ticketed with their Accept lines as the ticket's definition of done.

### The next run

Agreed on 2026-11-02, after steps 1 to 5 (Tier 2's repository side): the rest of the order above, re-sorted by what
gets expensive at the tag first, then by what can go wrong in front of a room. In this order:

1. **What is cheap only before the tag** (about a day). After it, `0001_init.sql` is frozen and the admin API's
   fields and what the engine stores are in use: a change then costs a migration or a compatibility shim.
   - T8-4, T8-5, T8-6: drop or settle the admin API's unused fields (`missing_for_open`, `past_event`,
     `dev_auth`, the two `created_at`).
   - T7-6: the test that the SQLite and Postgres schemas match, before a second migration exists; the audit
     action enum (its strings are written into the database); T8-14's schema-comment fix.
   - T8-15: mark `0001_init.sql` frozen, as part of the tag.
   - T3-2(b), decided: if the admin is to fetch entrants page by page, its API changes now; otherwise T3-2(a),
     refetching less often.
2. **What breaks or embarrasses an evening** (two to three days): T4-1 (the console clicked through a whole draw in
   a browser); T5-1 (the winner panel at the top of the Draw tab); T5-2 (the People tab on a phone); T5-5 (confirm
   or undo on destructive actions); T6-1 and T6-4 (the projector dark by default, readable badges, a big join
   code); T3-6 (polling backoff, paused in hidden tabs, on the projector and the admin); T3-4 (the console's map of
   entrants built once per change); T5-4 (accessibility on the join form and the People tab).
3. **Tests that protect verifiability and personal data** (one to two days): T4-4 (`verify.py` failing on tampered
   records); T4-6 (housekeeping on Workers with raffles due); T4-3 (drand over real HTTP, relay failover,
   stall-then-recover); T4-11's rest (the flaky tracing test; tests that pass silently without
   `RESTATE_SERVER_BIN`).
4. **The rest of Tier 5**: T5-3 (the first-run checklist), T5-6 and T5-7 (flow and copy).
5. **The rest of Tier 6**: T6-2 (link previews for shared join links, the most visible: before the tag if there is
   time), T6-3, T6-5, T6-6.
6. **Tier 7 and Tier 8**, one sweep each: safe after the tag, since none of it changes behaviour.

Not in it: the rest of Tier 3 (the measurements and the CI guards) and T4-8 … T4-10, which become tickets
at the tag. The real deploy (T2-3) is best done early, alongside step 1: it
is the one check whose findings could still change code.

**Done** (2026-11-02, `bfdcc44`, `5756fe8`, `4077857` and the Tier 7/8 sweep after them), each verified with `just
clippy`, `just test` (with `RESTATE_SERVER_BIN`) and `just workers` (`WORKERS_APP_PORT=8797`, every check ok):

1. `missing_for_open`, `past_event` and `dev_auth` dropped; `RaffleSummary` has `event_date` (shown on the cards)
   for `created_at`; `AdminEntrant.created_at` kept and read. `crates/store/tests/schemas.rs` holds the two schemas
   together. `rafi_store::audit::Action`, its strings in the store's SQL too. The consent example fixed. T3-2(b)
   decided against (the admin API is the deployment's own: paging can come without a migration); T3-2(a) instead:
   the admin refetches at once for a moment of the evening, otherwise every 10 s at most, which also fixed
   `LiveRefresh` refetching on every poll once anything had changed.
2. The console's top panel and grouped queue (T5-1); People as cards below `sm`, newest first, 200 a page (T5-2);
   confirms and undos, with a table in `web-app.md` (T5-5); labels, hints, the proof-of-work status line and retry,
   native radios (T5-4); the projector dark, em chips, a bigger join code (T6-1, T6-4); the console's map borrowed
   and its clock read only while something runs on time (T3-4); polls jittered, backing off to 10 s, paused while
   hidden (T3-6); the evening clicks Draw (twice), Claimed and Finish in the console (T4-1).
3. Six tampered vectors both verifiers fail (T4-4); housekeeping on Workers with everything due (T4-6); the engine
   suite over HTTP relays, past a dead one and recovering on the same round (T4-3, 16 scenarios); the request log's
   test in its own binary, `SKIPPED:` from `just test` without `RESTATE_SERVER_BIN`, the reveal's bound 2.5 s
   (T4-11).
4. The "Before doors open" checklist and the guide link (T5-3); landing on Draw, permanent error toasts, the wifi
   wording, a sign-in card, "Not you?", the projector's overflow, the thanks line, the QR's quiet zone, titles, the
   tab in the URL (T5-6); the copy table and plurals (T5-7).
5. Favicon, home-screen icon, theme-color, a link preview on the join page (T6-2); a loading screen in `index.html`
   and the font preloaded (T6-3); emoji as decoration only (T6-5); selection, autofill, tabular figures, print,
   safe areas, the home footer (T6-6).
6. Tier 7: `Refusal::kind` and one match for the status (T7-4); the tiers checked in the use cases
   (`rafi_app::Actor` carries `access`; `admin.rs` has no `allow`) (T7-3); `unwritten`, for the prize and lifecycle
   writes (T7-2, partly); `act`, for the console's and People's simple calls (T7-5, partly); `rafi_api::routes`
   (T7-6); `anyhow`, `tokio`, `tracing`, `js-sys`, `wasm-bindgen` and `worker` in `[workspace.dependencies]` (T7-6).
   Tier 8: T8-1, T8-2, T8-3, T8-7, T8-8 (`names::fold_accents`), T8-9, T8-10, T8-11 (`pre-release-review.md` in
   `docs/history`), T8-12, T8-13, T8-14, T8-15, T8-16, T8-17, T8-18; `base64` 0.23 in rafi-server.

### What it left

- T4-3: the evening on the Worker over real HTTP relays. Deferred (2026-11-02): the engine suite fetches over HTTP
  through a test-only switch (`DrandClientBuilder::mock_over_http`) the Worker can't turn on. It needs a `dev`-only
  variable for the Worker to fetch the mock chain from its relays, and a local relay (Node, not Python) that
  `workers-check.sh` starts and points `RAFI_DRAND_RELAYS` at; about half a day. The real deploy (T2-3) exercises
  the Worker's fetch path against the public relays meanwhile. [#46](https://github.com/sagikazarmark/rafi.orig/issues/46).
- T6-2's previews checked by hand in Slack and T6-3's Lighthouse run. Deferred (2026-11-02): Slack fetches a preview
  from a public URL, so it waits for the real deploy (T2-3); Lighthouse can run locally against `wrangler dev`
  (`npx lighthouse`, which downloads it), on the maintainer's word. T6-1's checks in the evening (the smallest
  font, contrast on computed colours) are left too. T6-6: the toasts' and dialogs' motion (vendored components),
  skipped.
- T7-5's `AddList` state machine. Left (2026-11-02): its states overlap (a new read keeps the last list and its
  disabled Add button on screen, a send can be out while a list is read, the batch ID and the name format outlive a
  new read), which one `Step` would change. [#50](https://github.com/sagikazarmark/rafi.orig/issues/50).

Since then (2026-11-02, `d4797d2` … `04d8173`): the list import clicked in the evening (T4-5); a double winner's
phone lists every prize, and the CSV a row per prize (T4-7); the verification page checks a record once, not per
keystroke (T3-8); the restore rehearsed in `crates/app/tests/restore.rs`, which found and fixed what it brought back
(T2-7); the web gzip budget and the `EXPLAIN QUERY PLAN` gate, with two partial indexes in `0001_init.sql` for the 13
scans it found (T3-11 1 and 7); `dioxus-primitives` at `9a75825`, without the dialog's `AbortError`; the native server's
static files precompressed and cached (T3-5).

### Tickets

Filed 2026-11-02 for what goes past v0.1.0: the Worker over real HTTP relays, T4-3 ([#46](https://github.com/sagikazarmark/rafi.orig/issues/46));
the concurrency scenarios on a SQLite file and D1, T4-8 ([#47](https://github.com/sagikazarmark/rafi.orig/issues/47)); WebKit, mobile, axe and
screenshots, T4-9 ([#48](https://github.com/sagikazarmark/rafi.orig/issues/48)); Clerk on Workers, T4-10 ([#33](https://github.com/sagikazarmark/rafi.orig/issues/33));
the projector bundle split and the rest of T3-9 and T3-10 ([#49](https://github.com/sagikazarmark/rafi.orig/issues/49)); `AddList`'s state as two
enums, T7-5 ([#50](https://github.com/sagikazarmark/rafi.orig/issues/50)); three guarded writes answering wrongly when the row is gone, from T7-2
([#51](https://github.com/sagikazarmark/rafi.orig/issues/51), fixed before the tag); refusing a `Resolve` without `k` one release after the tag, T0-1
([#52](https://github.com/sagikazarmark/rafi.orig/issues/52)); the rest of T3-11's regression guards (D1 `rows_read`, the
query count, payloads, Worker startup, microbenchmarks) and `oldest_raffle_at`'s walk of `raffles_owner`
([#54](https://github.com/sagikazarmark/rafi.orig/issues/54)).

Enforcing the audit identifiers' period ([#22](https://github.com/sagikazarmark/rafi.orig/issues/22)) waits too.
Meanwhile the privacy template no longer reads as if Rafi deletes them: the operator names a period and runs the
runbook's "Organizers who left" (SQL checked on SQLite and Postgres) on that schedule.

## Release checklist

The tag is ready when every line is true.

- [x] Tier 0: all seven findings closed with their interleaving tests; `0001_init.sql` carries `draws.delivery`
      and `add_batches` (or the alternatives are recorded here with the reason). (`cf56888`)
- [x] Tier 1: the countdown decreases between polls on both screens; a 4403'd phone shows the win; a 9 s wasm
      doesn't double-render; the projector shows offline/404/newer-server states; the thanks screen names the
      host; every theme pair ≥ 4.5:1. (The newer-server state by SSR test only; the light theme's won card not yet
      looked at by eye.)
- [ ] `gh repo view sagikazarmark/rafi` resolves; `rg -o 'github.com/sagikazarmark/[a-z.]+' | sort -u` is one
      name.
- [ ] One `workflow_dispatch` dry run and one `v0.1.0-rc.1` pre-release exist; the release job depends on CI.
- [ ] One real Cloudflare deploy drew a raffle end to end with real drand; the items under "What only a real
      deploy can check" in `cloudflare.md` are dated; startup time recorded.
- [ ] A first install from a clean repository, following `deploy/cloudflare/README.md` step by step, reaches
      `/healthz` → `ok 0.1.0`; with unedited templates the Worker refuses every request.
- [ ] A second release's deploy registers its engine, and the Restate cleanup deletes the first engine Worker
      (and its Restate deployment) once drained.
- [ ] A stranger deployed from the README in under an hour; their notes are folded in.
- [ ] `gh attestation verify` passes on the tarball; two builds of the tag are byte-identical.
- [x] `OFL.txt` beside the fonts; the admin footer links the source and shows the version; `/v/<slug>` links
      `verify.py`.
- [x] `docs/operations` has backup/restore (rehearsed once), rotation, rollback, "Restate is down tonight", the
      breach runbook, a cost paragraph, the DSAR recipe, monitoring, a global deploy preflight.
- [ ] Web gzip budget, `rows_read` assertions, the query-count test and the `EXPLAIN` gate are in CI. (The gzip
      budget and the `EXPLAIN` gate are; the `rows_read` assertions and the query count are
      [#54](https://github.com/sagikazarmark/rafi.orig/issues/54).)
- [x] `tools/verify.py` fails every tampered vector; the evening's `verify.json` passes it.
- [x] `evening.mjs` clicks through a draw on the console, imports a list, and runs housekeeping with due rows.
- [x] The tracing test is in its own binary; `workers-check.sh` refuses an occupied port.
- [ ] README has a projector screenshot and badges; a ruleset on `main`; `compatibility_date` bumped; `wrangler`
      pinned. (The community profile's 100 % dropped: its templates and community files were declined.)
- [x] `pre-release-review.md` moved to `docs/history/`; this plan's status line says what shipped and what was
      ticketed; `0001_init.sql`'s header says it is frozen.
- [ ] `just ci` and `just workers` green on the tagged commit.

## Not doing, with reasons

- **Splitting the projector bundle before the tag** (T3-9). The measurement is a day and the split is more; the
  projector works today at 1 MB over venue wifi because it is loaded once before doors open. Ticketed, with the
  size budget as the ratchet.
- **`#[serde(other)]` on the SPA's enums** (`RaffleState`, `ItemStatus`, `Outcome`, `Source`). `RaffleState` is
  also the database's column type ("never guessed"), and T1-4's reload-on-parse-failure covers the SPA skew
  without an `Unknown` variant that every `match` would then have to handle.
- **A full mobile layout for the admin** beyond People (T5-2) and the Now panel (T5-1). The admin is laptop-first
  by design; those two are the tabs used standing up.
- **Internationalisation.** English only is stated; every string already sits in one of four places so a locale
  is mechanical later. The one addition now is a sentence in `running-a-raffle.md` that the event's "end of day"
  is the organizer's browser's time zone.
- **Telemetry or an update check.** None, confirmed everywhere (`legal/privacy.md:52`, `pages/text.rs:246`,
  `privacy-and-abuse.md:223-228`); made a stated commitment in `privacy-and-abuse.md` ("Rafi contacts no one").
- **Renaming "Rafi".** No conflicts found (crates.io free, npm's `rafi` unrelated, GitHub's a user handle; the
  nearest is Interledger's `rafiki`).
