# Rafi on Cloudflare Workers (plan)

Status: proposal, 2026-10-08. Builds on the prototypes on the `prototype/participant-view` branch (`docs/prototype-participant-view.md` there).

## 1. Goal

Run the app and the Restate handler on Cloudflare Workers, with no in-memory processes, and push live updates over WebSockets from a Durable Object per raffle using [partyline](https://github.com/sagikazarmark/partyline).

Restate stays the draw engine. The DO is only the live fan-out: it never holds workflow state.

What goes away:

| Today | On Workers |
|---|---|
| `Hub` (SSE fan-out) and its GC task | `RaffleChannel` DO, partyline Latest channel |
| `Watch` (polls D1 for Restate's writes) | Restate's projector nudges the DO |
| `refresh_later` (delayed names) | DO alarm |
| `LocalEngine` (in-process draws) | Removed; Restate only (see §8) |
| `Limiter` (in-memory) | Workers rate-limiting binding |
| `Altcha.used` (in-memory replay set) | D1 table with a unique key |
| Hourly retention task | Cron Trigger |
| SQLite + Litestream on Hetzner | D1 (EU jurisdiction) |
| Restate tunnel from the server | Restate Cloud calls the Worker over HTTPS (request-response) |

## 2. Decisions already made

These come from the prototypes and the discussion that led to them.

1. **Two views.**
   - **Phones** get a five-field **participant view** (`state`, `current`, `last`, `winners`, `prizes_left`, plus `v`), pushed in full on a Latest channel.
   - **The projector and admin** get the same channel's events only as a signal to fetch their full view over HTTP. They poll every 2–3 s while registration is open, because `recent_names` changes with time, not only with D1 writes.
2. **Only the DO builds the participant view.**
   - The app and Restate only nudge the DO.
   - On a nudge, the DO reads D1 and builds the view. It publishes only if the version is newer than any it has seen and the content changed.
   - `v` is the raffle's version in D1, never a process counter.
   - A DO alarm coalesces bursts of joins and handles the name delay.
3. **The page embeds the participant view, and subscribes only while the raffle is live.** No partyline cursor is needed: the version check orders views. Phones close their socket themselves when they receive the final state.
4. **Restate destroys the DO after a grace period** once the raffle has finished.
   - After the grace period, the Worker refuses sockets for the raffle and the DO ignores nudges.
   - Personal data then lives only in D1, and retention only touches D1.
5. **The attendee page is server-rendered Rust plus a small WASM.**
   - The server renders the live card from the **same Dioxus component** the WASM mounts.
   - The WASM adds:
     - live updates;
     - the proof-of-work;
     - wake lock, chime, vibration and the theme toggle.
   - A small JS fallback polls `/view` when `WebAssembly` is unavailable (iOS Lockdown Mode).
   - The budget is about 220 KB compressed, measured at about 205 KB.
6. **Two existing phone bugs get fixed while porting.**
   - `Winner` gains `claimed`, so a claimed winner is no longer told to go to the organizer.
   - `last` stops showing no-shows.

## 3. Crate layout, and what to reuse

### Rule

- **Start as a module in the workspace.**
- **Make it a crate** when a second consumer needs it, across a stable boundary.
- **Contribute it upstream** when the code is generic and an existing crate is its natural home.
- **Publish a new crate only** when none of the above applies.

### Workspace after the move

| Crate | Status | Contents | Used by |
|---|---|---|---|
| `rafi-core` | unchanged | Domain model, verifiable draw | everyone except the attendee WASM |
| `rafi-live` | **new** | `ParticipantView`; the `RaffleLive` partyline channel; `screen()`; the Dioxus `Card` component; its own lightweight state enum | DO, app (server rendering), attendee WASM |
| `rafi-attendee` | **new** | The attendee WASM: mounts `Card`, subscribes, browser extras | phones |
| `rafi-store` | ported | Queries on D1, `version` bumping | app, Restate handler, DO |
| `restate-rafi` | ported | The `Raffle` virtual object; projector writes D1 and nudges the DO | Restate handler Worker |
| `rafi` (app) | ported | Worker: routes, server functions, `RaffleChannel` DO, Cron Trigger | |
| `restate-rafi-endpoint` | dropped, or kept for self-hosting (§8) | | |

`rafi-live` must not depend on `rafi-core`. `rafi-core` pulls in drand and BLS (arkworks), which would bloat the attendee WASM. The server maps `RaffleState` to `rafi-live`'s state enum.

This split is the extraction that matters. It's the only way one `Card` component serves both the server-rendered page and the WASM, and one set of view types serves the DO, the server and the phone.

### The client pieces

| Piece | Recommendation | Why |
|---|---|---|
| ALTCHA | **Keep rafi's own scheme for now**, as a shared module in `rafi-live` (types, `solve`, `verify`). In parallel, contribute the wasm clock fix to the official `altcha` crate (§4), then switch to it and to protocol v2. | The official `altcha` 1.0 compiles for wasm but aborts at runtime in both `solve_challenge` and `verify_solution`. rafi's scheme already ran in the prototype's WASM. Replay protection is rafi's own job either way (a D1 table). |
| Wake lock | **Contribute `use_wake_lock` to `dioxus-sdk`.** Until it merges, keep it as a module in `rafi-attendee`. | It's generic, and `dioxus-sdk` (geolocation, notification, storage, …) has no wake lock. web-sys gates `WakeLock` behind an unstable flag; the prototype's 8-line binding avoids that. |
| Vibrate | Keep it in `rafi-attendee`. | It's one call. |
| Chime and audio unlock | Keep them in `rafi-attendee`. | The sound is rafi's own. The generic part (unlock on gesture, iOS `audioSession`) is about 15 lines. |
| Theme toggle | Keep it in the app. Consider `dioxus-sdk`'s `storage` for the saved value. | It's daisyUI-specific, and the inline `<head>` script stays JS anyway. |
| Restate on Workers | **Upstream the `wasm` branch fixes, then a Workers adapter**, to `restate-sdk`. | This is the most reusable piece of the whole move (§4, §5 M4). |
| partyline gaps | Add `Hub::destroy`, and create the tables lazily instead of in the constructor. | rafi is partyline's first real consumer. |

## 4. Verified facts and assumptions

**Known from prior use:**

- **Dioxus runs on Workers.** Mark has used it there before, so M0(a) is a quick check rather than a go/no-go.
- **The Restate SDK runs on Workers with the `wasm` branch of [sagikazarmark/restate-sdk-rust](https://github.com/sagikazarmark/restate-sdk-rust/tree/wasm)** (commit `91bf758`, on top of 0.12). On `wasm32-unknown-unknown` it:
  - uses `web-time` clocks;
  - skips the 60 s `tokio::time::timeout` in `drain_input`;
  - selects the JS `getrandom` backends;
  - adds a `check-wasm32` CI job.

  rafi still needs its own adapter. SDK 0.12 ships only `http_server` and `lambda`. The Lambda adapter is about 30 lines around `Endpoint::handle_with_options(request, RequestResponse)`, so a Workers adapter is the same shape.

**Verified by reading the code, crate metadata, or a `cargo check`** (not by running on Workers):

- **The official `altcha` crate (1.0) doesn't work on wasm yet.**
  - It compiles for `wasm32-unknown-unknown` once the consumer enables `getrandom` 0.2's `js` feature.
  - It **aborts at runtime**: `solve_challenge` and `verify_solution` call `std::time::Instant::now()`, and `verify_solution` also calls `SystemTime::now()`. That's the same problem the Restate branch fixes, and the same `web-time` fix applies.
- **rafi's own code has the same clock problem.** On `wasm32-unknown-unknown` these must move to `web-time` or `Date.now()`:
  - `app/src/server/ids.rs:59` (`now()`, used everywhere on the server);
  - `store/src/lib.rs:64`;
  - `drand/src/lib.rs:223`;
  - `app/src/model.rs:229`.

  The `Instant::now()` calls in `ratelimit.rs` and `watch.rs` disappear with those modules. The same audit applies to `getrandom` backends and to any other dependency that reads the std clock.
- **`sqlx-d1` 0.4.3 exists** ("SQLx for Cloudflare D1").
  - rafi has about 40 query call sites.
  - All are runtime `sqlx::query`/`query_as` calls, with no compile-time macros, so the port is mechanical.
- **`dioxus-sdk` 0.7 has no wake lock.**

**Measured in the prototypes:**

- The attendee WASM is about 205 KB compressed.
- Proof-of-work: 10–17 ms in WASM against 330 ms in JS.
- `dx` 0.7.9 skipped wasm-opt.
- The Dioxus mount adds to `#live` instead of replacing it.

**Assumed, to confirm in M0:**

- The app Worker fits the Workers bundle size limit. It includes Dioxus server rendering, rafi-core and BLS.
- BLS verification fits within Workers CPU limits. The free plan's 10 ms per request is likely too little, so plan on the paid plan.
- D1 and DOs can be pinned to the EU jurisdiction.

## 5. Milestones

Each M0 spike ends in a go or no-go before the work that depends on it starts.

| # | Milestone | Done when |
|---|---|---|
| **M0** | **Spikes**, each a throwaway Worker: (a) quick check that Dioxus 0.7.10 fullstack works on Workers with rafi's routes; (b) the Restate adapter on the `wasm` SDK branch, called by Restate Cloud with identity keys in request-response mode; (c) `sqlx-d1` running `rafi-store`'s queries; (d) `dioxus-clerk` server-side verification on Workers; (e) bundle size and BLS verification CPU time on a deployed Worker; (f) an audit of std clock and `getrandom` use in rafi and its dependencies (§4) | Each one either works, or has a chosen fallback (§7) |
| **M1** | **`rafi-live`**: lift `participant.rs`; add `claimed` and fix the no-show bug; `Card` component; lightweight state enum; the ALTCHA decision | Server rendering and a native test render the same `Card` from the same view |
| **M2** | **D1 store**: `rafi-store` on D1; a `version` column bumped by triggers on every write that affects the raffle; migrations via `wrangler d1 migrations`; the ALTCHA replay table | rafi's existing store tests pass against D1 locally (`wrangler dev` / miniflare) |
| **M3** | **`RaffleChannel` DO**: partyline hub embedded by hand; nudge → read D1 → version and content check → publish; alarm for coalescing and the name delay; destroy; Worker connection gate. partyline: `Hub::destroy` and lazy tables | The prototype's scripted evening passes against `wrangler dev` |
| **M4** | **Restate on Workers**: depend on the `wasm` SDK branch (git dependency until upstream releases it); Workers adapter (in-repo, then upstream); projector writes D1 and nudges the DO inside `ctx.run`; delayed self-call that destroys the DO after the grace period | A draw runs end to end through Restate Cloud and a phone sees it |
| **M5** | **App on Workers**: routes and server functions; attendee page rendered with `Card`; rate limiting via the binding; retention via a Cron Trigger; delete `Hub`, `Watch`, `refresh_later` and `LocalEngine` | The admin flow works end to end on a preview deployment |
| **M6** | **Attendee WASM**: `rafi-attendee` (clear `#live` before mounting, subscribe, extras, no `expect`s); the JS fallback when WASM is missing; a CI size check (≤ 220 KB compressed); confirm `dx` 0.7.10 runs wasm-opt | The phone test plan passes (§6) |
| **M7** | **Projector and admin live**: subscribe for nudges, fetch the full view over HTTP, poll during registration | The projector shows the reveal within about 1 s of the round |
| **M8** | **Cutover**: SQLite → D1 import; update the privacy notice (Cloudflare as processor); register the Restate deployment; DNS; switch off Hetzner after one event | One real meetup runs on Workers |

M1 depends only on M0. M2 and M3 can run in parallel. M4 and M5 need M2.

## 6. Testing

- **Unit:** `screen()` and the DO's publish rule, using the prototype's cases as tests. The partyline loopback harness covers resume.
- **Local end to end:** `wrangler dev` with D1 and the DOs, a local Restate server, and the CDP smoke test (`smoke.mjs` from the prototype) extended to the full join → draw → win flow.
- **Phones (manual, before M8):**
  - iOS Safari, including Lockdown Mode (the fallback path);
  - Android Chrome;
  - with the phone locked and unlocked during a draw (wake);
  - with Wi-Fi switched off and on (resume);
  - the chime actually playing on iOS.

## 7. Fallbacks if a spike fails

| Spike fails | Fallback |
|---|---|
| (b) Restate adapter | Keep the Restate handler as a container (current `restate-rafi-endpoint`, e.g. Cloudflare Containers or Fly) writing to D1 over its HTTP API, and nudging the DO through an authenticated Worker route. |
| (c) `sqlx-d1` | Use `worker::D1Database` directly behind the `rafi-store` interface. |
| (e) Bundle size | Split into Workers by role (attendee/public, admin, DO, Restate handler), connected by service bindings. |

## 8. Open questions

1. **Self-hosting:** keep a non-Workers deployment (SQLite, SSE, `LocalEngine`)? Supporting both doubles the store and live-update code. I recommend Workers only, unless self-hosting is a goal for other organizers.
2. **Grace period** before the DO is destroyed: 1 hour? The rest of the evening? It decides how long winner names live in DO storage.
3. **Workers plan:** paid, assuming M0(e) confirms the free plan's 10 ms CPU limit is too tight for BLS verification.
4. **Restate SDK fork:** is the `wasm` branch going upstream? Until it's released, rafi pins a git dependency.
5. **ALTCHA:** contribute the `web-time` fix to `altcha-org/altcha-lib-rs`, then move to the official crate and protocol v2? Until then, rafi keeps its own scheme.
6. **Where the Workers adapter for Restate lives:** in this repo, in the SDK fork next to the wasm fixes, or in a small crate of its own.
7. **Wake lock:** contribute `use_wake_lock` to `dioxus-sdk`, or keep it in `rafi-attendee`?
