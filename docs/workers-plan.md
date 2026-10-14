# Rafi on Cloudflare Workers (plan)

Status: accepted, 2026-10-08. Reviewed in four passes: fit with the code, distributed design, security/privacy/operations, and structure and client. The findings are folded in below.

**Where it stands (2026-10-10).** M1–M7 are done locally on the `cloudflare` branch and checked by `scripts/workers-check.sh` (CI's `workers` job); see §11. M8, the cutover, waits on accounts: Cloudflare staging and production (D1 created with `--jurisdiction eu`, the secrets, custom domains), the Restate Cloud deployment, Clerk with real keys, BLS CPU time measured on a deployed Worker (M0(e)), and the phone test plan (§6). Crate names below are of their time where they differ; [PLAN.md](PLAN.md) has the current layout.

This plan builds on the prototypes on the `prototype/participant-view` branch (`docs/prototype-participant-view.md` there).

## 1. Goal

Run the app and the Restate handler on Cloudflare Workers, with no in-memory processes, and push live updates over WebSockets from a Durable Object per raffle using [partyline](https://github.com/sagikazarmark/partyline).

Restate is the only draw engine. The DO is only the live fan-out: it never holds workflow state.

| Today | On Workers |
|---|---|
| `Hub` (SSE fan-out) and its GC task | `RaffleChannel` DO, partyline Latest channel |
| `Watch` (polls the database for Restate's writes) | Restate's projector nudges the DO |
| `refresh_later` (delayed names) | The projector's poll picks them up; phones never show names of non-winners |
| `LocalEngine` (in-process draws) and the admin's "switch to local" | Out of the app: rafi relies on Restate (§2.7). `LocalEngine` stays for tests and development (`rafi-engine`'s `local` feature) |
| `Limiter` (in-memory) | Workers rate-limiting binding (soft), plus a per-raffle entrant cap in D1 (hard) |
| `Altcha.used` (in-memory replay set) | D1 table; a duplicate INSERT fails |
| Hourly retention task | Cron Trigger |
| SQLite on one VM (`deploy/compose.yaml`) | D1, EU jurisdiction |
| Restate tunnel from the server | Restate Cloud calls a dedicated Worker over HTTPS (request-response, signed) |
| `std::env` configuration | `worker::Env` vars and secrets |

## 2. Decisions

### 2.1 Two views

- **Phones** get a five-field **participant view**, pushed in full on a Latest channel:
  - `state`;
  - `current`: the draw in progress, with its prize, round and round time;
  - `last`: the latest draw that didn't end in a no-show;
  - `winners`, each with `entrant_id`, `item_name` and `claimed`;
  - `prizes_left`.

  It also carries `v`.
- **The projector and admin:**
  - render the reveal straight from the participant channel's events, so they're never behind the phones;
  - also poll their full view every 2–3 s for as long as the raffle is live. The full view also changes without any write to D1, because `recent_names` depends on time;
  - fetch it on mount and on every reconnect.
- **Versions:**
  - **Both views' `v` is the raffle's version in D1**, never a process counter.
  - **For the full view, clients accept `v >= current`**, since a poll can return the same version with newer time-dependent content.
  - **For the participant view, phones accept only a newer `v`.**
  - **`server_now_ms` stays in the full view.** The participant view has no time-dependent fields.

### 2.2 Only the DO builds the participant view

- **Nudges come from:**
  - the app, for lifecycle changes, admin edits, resolutions and End live updates;
  - Restate, for draw states.

  **Joins don't nudge**, because they never change the participant view.
- **On a nudge, the DO:**
  1. reads the raffle's `version` and all content in **one `db.batch()`**, which is one consistent snapshot, from the primary (never a read replica);
  2. publishes if `v` is newer than the stored `seen_v` and the content changed;
  3. stores `seen_v` in its own SQL in the same synchronous block as the publish.
- **Every write that spans several statements is one D1 batch**, so no nudge can see half of it.
- **The DO answers 2xx when it ignores a nudge.** A nudge that fails is retried a few times; after that it's dropped. The periodic re-read covers it.
- **Periodic re-read:** while sockets are connected and the raffle isn't over, the DO alarm re-reads D1 about every 15 s, so a lost nudge is repaired.

  This is the only use of the DO's single alarm. rafi doesn't pass the alarm to `hub.on_alarm()`; Latest mode never sets one.
- **A destroyed flag:** the DO keeps one in memory. Destroy sets it first, and every nudge and alarm checks it after each `await`, before publishing or setting an alarm.

### 2.3 The page and the socket

- **The page embeds the participant view.** It subscribes while the raffle is **public and not over** (Ready, Open, Closed, Frozen, Drawing). Ready is included so a visitor's page can reload when registration opens.
- **No partyline cursor is needed**, because `v` orders views. Phones close their socket when they receive the final state.
- **Clients fetch `/api/r/{slug}/participant`:**
  - on `visibilitychange`, `pageshow`, `focus` and `online`;
  - on reconnect;
  - once, on a close code that tells them not to reconnect.
- **The Worker refuses a socket by accepting it and closing it with 4403**, through partyline's `reject`. Never with an HTTP error, which the browser sees as 1006 and keeps retrying.
- **The clock offset is recorded by an inline script while the page parses**, and re-synced from the `Date` header of every `/participant` fetch.

### 2.4 Ending live updates

- **The trigger:** the admin's **"End live updates"** action, available once the raffle has finished.
- **What it does:** it sets `live_ended_at` in D1, then the app tells the DO to destroy itself. Restate isn't involved.
- **Destroy:**
  1. sets the destroyed flag;
  2. publishes the final view from a fresh D1 read;
  3. closes every socket with 4403;
  4. deletes the alarm;
  5. deletes all storage.
- **After destroy:**
  - the Worker refuses sockets for the raffle;
  - the DO ignores nudges (it checks `live_ended_at`);
  - a DO with empty storage checks `live_ended_at` before accepting a socket.
- **The DO is also destroyed:**
  - when a raffle is deleted;
  - when a raffle is purged by hand;
  - by the Cron, for any raffle that finished more than 24 h ago. This runs **whether or not erasure is enabled.**
- **Personal data then lives only in D1.**

### 2.5 The attendee page

**Shape:** the server renders the page in Rust, and a small WASM adds behaviour.

- **One `Card` component**, in `rafi-live`, is used by both:
  - the server, through `dioxus::ssr`;
  - the WASM.
- **`Card` renders one root (or nothing)** and calls no browser APIs while rendering. The reload, timers and audio live in `rafi-participant`.
- **`#live` is a plain `aria-live` mount point owned by the page.** The WASM clears it and mounts in the same task. `role="alert"` and the pop animation are added only when the win happens on the client, not on page load.
- **Strings:** `Card` takes a `CardStrings` struct that the server fills in from i18n and embeds in the page JSON.

**The WASM adds:**
- live updates;
- the proof-of-work;
- wake lock, chime and vibration;
- the theme toggle.

The inline `<head>` theme script stays JS.

**The join form:**
- **The Join button is server-rendered disabled**, with the spinner. It's enabled once a proof-of-work solve finishes, in the WASM or the fallback.
- **On the form page there's no `#live`**, so the WASM doesn't mount the card there.
- **The audio unlock and the wake-lock request stay inside the submit gesture.**

**The JS fallback is an inline loader.**
- **It starts when:**
  - `typeof WebAssembly !== "object"`;
  - the WASM hasn't signalled ready within about 8 s;
  - the WASM's panic hook fires.
- **What it does:**
  - polls `/api/r/{slug}/participant` and reloads the page when `v` rises;
  - ticks the countdown;
  - solves the proof-of-work in JS;
  - keeps the theme toggle and the wake/audio button working.

**Size budget:** ≤ 280 KB compressed. The first build is 262 KB (250 KB wasm plus 11 KB JS).
- **Against the prototype:** the prototype measured 189 KB through the same build. Most of the difference is Dioxus's runtime for the real app (+53 KB before optimisation), JSON decoding of the page config and views (+43 KB), and `rafi-live` (+23 KB).
- **Why it's acceptable:** the page is server-rendered first, so the size only delays live updates.
- **Follow-up:** trimming it.

### 2.5a Serving the admin and projector

`dioxus-server` doesn't compile for Workers (§11), so the admin and projector are a **Dioxus web app served as static assets** (Workers Assets, single-page fallback). There's no server-side rendering or hydration for them.

- **`api.rs` keeps its signatures.** Each server function becomes a typed client: it POSTs JSON to `/api/admin/<name>` and decodes the reply.
- **The server side is plain axum routes** that call the existing `service` layer. The same routes run natively (tests, development, self-hosting) and on Workers (`worker` runs axum routers).
- **Auth stays as it is:**
  - the routes are same-origin, so Clerk's session cookie comes along;
  - `dioxus-clerk`'s `worker` feature verifies it on Workers.
- **The attendee page is unaffected:** plain routes with `dioxus::ssr`.
- **One `rafi` crate with three features:** `web` (the UI), `server` (native axum) and `worker` (the Workers entry point: fetch, the Cron, and `RaffleChannel`).

### 2.6 Phone bugs fixed while porting

- **`Winner` gains `claimed`**, so a claimed winner is no longer told to go to the organizer.
- **`last` skips no-shows.**

### 2.7 Restate is the only engine

Rafi relies on Restate. There is no fallback draw path:
- `LocalEngine` goes from the app (it lives on as `rafi-engine`'s `local` feature, for tests and development only);
- so do the admin's "switch to local" action and API;
- so does the engine column guard.

### 2.8 Data layer

- **`rafi-store` exposes its own functions and error type**: domain functions holding all of rafi's SQL (§7a), over SQLite (sqlx) natively and `worker::D1Database` on Workers.

  `sqlx-d1` 0.4.3 pins sqlx-core 0.8.6, while the workspace is on sqlx 0.9.
- **No interactive transactions** (D1 doesn't have them):
  - **Freeze** becomes: check there are entrants; a batch that sets `frozen` (guarded by state); read the list; write the manifest (planned as the "commit doc", the manifest's old name).

    "Frozen without a manifest" becomes recoverable: Start writes the manifest from the fixed entrant list (`rafi_raffle::start::next`, `Start::WriteManifest`; the admin offers it since revision 9).
  - **Fake entrants** become one batch of multi-row INSERTs.
- **Retention uses set-based `UPDATE`s**, to stay under D1's per-invocation query limit.
- **The retention run sets `purge_*_at`** when it first sees a raffle finished (`retention::date_purges`). `Watch` used to do that, then `save_draw_state`, until the engine stopped carrying app settings ([engine-plan.md](engine-plan.md) revision 8).
- **A `version` column on `raffles`**, bumped by triggers on every write that affects either view.

### 2.9 Configuration and runtime

- **`Config` is built from `worker::Env`.** `std::env::var` always fails on wasm, so every setting would silently fall back to its default.
- **Required secrets stop the Worker if missing**; none of them falls back to a default:
  - `RAFI_ALTCHA_KEY`;
  - the Clerk keys;
  - the Restate identity keys;
  - the ingress API key.

  Don't rotate the ALTCHA key during an event: challenges are valid for 20 minutes.
- **`RAFI_DEV_AUTH` and mock drand are compiled out of the release build.**
- **Clocks:** `ids::now`, `rafi-store` and `model::now_unix_ms` use `Date.now()` on wasm. `now_unix_ms` currently returns 0 on a wasm server build.
- **`reqwest` on wasm** has no `ClientBuilder::timeout`, so use per-request timeouts. The Restate client races each synchronous call against a timer instead ([engine-plan.md](engine-plan.md) revision 8).

### 2.10 Privacy

- **Jurisdiction:** D1 is created with `--jurisdiction eu`. DO IDs come from an EU-jurisdiction namespace. A DO's jurisdiction is fixed when its ID is created, so this must be right from the first deployment.

  rafi builds its own stubs for this; partyline's `stub()` gets a jurisdiction option upstream later.
- **Logs:**
  - invocation logs are off, or have confirmed header redaction (client IP, the `Cookie` header);
  - no Web Analytics;
  - no names, emails or tokens in `console`.
- **The privacy notice, in Hungarian and English, adds:**
  - Cloudflare, Inc. (US, DPF/SCCs) for hosting, the database and logs;
  - that D1 Time Travel keeps erased data for up to 30 more days.
- **Erasure log and restores:** keep a log of erasures with no personal data in it (entrant ID and time). Any restore must re-apply it.
- **Retention:**
  - on by default in the Workers build;
  - records `last_retention_ok_at`, and the admin shows a warning when it's stale;
  - erasure is written as SQL statements that return no personal data.

### 2.11 Security and operations

- **Sockets:**
  - the Worker accepts them only for public raffles that aren't over;
  - connections are rate-limited per raffle with the binding (never by IP);
  - the DO refuses sockets beyond a cap with 4403, and those clients fall back to polling.
- **Restate endpoint:**
  - its own Worker, on its own hostname;
  - signatures are verified before any routing, discovery included;
  - a new versioned Restate deployment is registered per release.
- **Deploy freeze:** CI refuses `wrangler deploy` while a raffle is Open, Frozen or Drawing, using `/api/status`. A deploy restarts DOs and drops every socket.
- **Staging:** a separate staging Worker with its own D1 and a Clerk dev instance. Production has `workers_dev = false` and `preview_urls = false`.
- **Rate limits:** the binding counts per location and only supports 10 s or 60 s windows, so it's a soft limit. That means two bindings: per raffle, and per email. The hard limit is a per-raffle entrant cap in D1.
- **ALTCHA replay:** the check runs after the HMAC check, and is a plain INSERT that fails on a duplicate key. The Cron purges expired rows.

## 3. Crate layout

### Rule

- **Start as a module.**
- **Make it a crate** when a second consumer needs it, across a stable boundary.
- **Contribute it upstream** when it's generic and an existing crate is its natural home.
- **Publish a new crate only** when none of the above applies.

### Workspace

| Crate | Status | Contents | Used by |
|---|---|---|---|
| `rafi-core` (now `rafi-raffle`; join codes moved into the app, now `rafi-app-core`) | unchanged | The raffle's rules, all pure: its lifecycle, starting the draw (freeze, write the manifest, resend), who may join, entrants' names and email identity, settings, prizes as the drawing's items | app, store, `rafi-live` (`raffle` feature), both draw engine hosts |
| `rafi-drawing` | split out of `rafi-core` later ([drawing-plan.md](drawing-plan.md)) | The drawing: manifest, state machine, verifiable draw, verifier | everyone except the attendee WASM |
| `rafi-live` | **new** | `ParticipantView`, the `RaffleLive` channel, `screen()`, `Card` and `CardStrings` (`card` feature), a lightweight state enum, the ALTCHA module (types, `solve`, `verify`), `LiveTransport`, `PageConfig`, the publish rule (`publish::decide`), `public_name`; with the `raffle` feature, `raffle::view`, the view built from a raffle and its drawing | DO, app, attendee WASM |
| `rafi-platform` | **new** (§7a) | The ports: `Notifier` (`Polling`, `Recording`), `RateLimit` and `Limit` (native `Limiter`), `Notify` (a `Notifier` as the engine's `DrawingNotifier`), `BoxFuture`, `sendable` (`DrawStore` moved to `rafi-engine`, [engine-plan.md](engine-plan.md) revision 7; the publish rule to `rafi-live`, revision 9) | everyone but the attendee WASM |
| `rafi-participant` (planned as `rafi-attendee`) | **new** | The attendee WASM: mounts `Card`, follows the server's `LiveTransport` (partyline or polling), proof-of-work, extras (wake lock module included) | phones |
| `rafi-store` | ported | All SQL, as domain functions (raffles, items, entrants, audit, ALTCHA, retention, the full view); own error type; SQLite and D1 backends; `version`; `live::Snapshot`, the data phones are shown; `DrawWriter`, its `DrawStore` | app, both draw engine hosts, DO |
| `restate-rafi` (now `rafi-engine-restate`, with the app's client) | ported | `Drawing` virtual object over a `DrawStore` and a `DrawingNotifier`: the store write, then `notify` in a separate, best-effort `ctx.run`. No database or platform dependency | both draw engine hosts |
| `rafi-cloudflare` | **new** (`rafi-channel` renamed) | All Cloudflare glue: the `RaffleChannel` DO, the socket gate and jurisdiction helper, `DurableObjects` (`Notifier`), `RateLimitBindings` (`RateLimit`), config from vars and secrets | both Workers |
| `rafi-model`, `rafi-app-core` | **new** (revision 9) | The types the app's server and browser exchange; every use case as a function over `App { db, chain, engine, notifier, policy }`, without HTTP | app |
| `rafi` (app) | ported | Routes, auth, i18n, retention, the web UI, over `AppState` (which derefs to `rafi_app_core::App`); the `worker` feature is the App Worker's thin entry point (fetch, Cron, exports `RaffleChannel`) | |
| `rafi-restate-worker` | **new** | Restate Worker: the adapter around `Endpoint::handle_with_options`, signature checks; wires `DrawWriter` over D1 and `DurableObjects`, built once per isolate | Restate Cloud |
| `restate-rafi-endpoint` | kept, not deployed | The native endpoint, for self-hosting (§8): `DrawWriter` over SQLite (opened lazily), the `Polling` notifier | |
| `rafi-store-d1-check`, `rafi-cloudflare-check` | local checks | Workers that drive the store and the channel under `wrangler dev` (`scripts/workers-check.sh`) | |

The phone builds `rafi-live` without its `raffle` feature, so it doesn't depend on `rafi-raffle`, which pulls in drand and BLS through `rafi-drawing`. Servers turn it on: `rafi_live::raffle` maps `RaffleState` to `rafi-live`'s enum and builds the view.

### Reuse

- **ALTCHA:** rafi's own scheme for now. Contribute the `web-time` fix to the official `altcha` crate later, then move to protocol v2.
- **Wake lock:** stays in `rafi-participant`.
- **Vibrate, the chime and the theme toggle:** stay in `rafi-participant`, which owns the toggle (the inline `<head>` script stays in the page).
- **Restate:** the `wasm` branch fixes are in an open upstream PR; rafi uses the git branch until it's released. The Workers adapter can go upstream later.
- **partyline gaps.** rafi works around these first and upstreams them later:
  - stubs with a jurisdiction;
  - `Hub::destroy`;
  - `head()` free of writes on an empty log;
  - creating tables lazily;
  - `pageshow` and `focus` as wake sources.

## 4. Facts

**Known from prior use:**

- **Dioxus runs on Workers.**
- **The Restate SDK runs on wasm** with [sagikazarmark/restate-sdk-rust `wasm`](https://github.com/sagikazarmark/restate-sdk-rust/tree/wasm) (`91bf758`, on 0.12; upstream PR open). It uses `web-time` clocks, skips the `drain_input` timeout and selects the JS `getrandom` backends.

  SDK 0.12 ships only `http_server` and `lambda`, so rafi writes the Workers adapter. The Lambda adapter is about 30 lines around `Endpoint::handle_with_options(request, RequestResponse)`.

**Verified by reading code, crate metadata or a `cargo check`:**

- **The official `altcha` 1.0 compiles for wasm** (with `getrandom` 0.2 `js`), but **aborts at runtime**: `solve_challenge` and `verify_solution` read `Instant::now()` / `SystemTime::now()`.
- **`sqlx-d1` 0.4.3 pins sqlx-core 0.8.6**; the workspace is on 0.9.
- **rafi uses interactive transactions** in `start_draw` (`service.rs:406–443`) and `add_fakes` (`service.rs:788–810`).
- **Configuration uses `std::env::var`** (`config.rs:49`).
- **The projector's `ctx.run` retries without limit** (then `restate-rafi`; now the `project` step in `crates/engine-restate/src/service.rs`, `Drawing::project`).
- **The Restate SDK requires `Send` futures.** `worker` D1 and DO futures aren't `Send`, so they need wrapping (for example `SendWrapper`), and the projector is built from each request's `Env`.
- **partyline:**
  - `SqlLog::head()` inserts the epoch row on read (`sql_log.rs:86`);
  - `stub()` sets no jurisdiction (`helpers.rs:16`);
  - the client stops reconnecting only on 1000, 4400, 4403 and 4426.
- **The `std::time` audit:**
  - `ids.rs:59` and `store/src/lib.rs:64` need fixing;
  - the `drand/src/lib.rs:223` and `Instant` call sites go away with `LocalEngine` and its modules.
- **rafi has 37 query call sites, plus `migrate!` and `FromRow`.** There are no compile-time query macros.

**Measured in the prototypes:**

- the attendee WASM is about 205 KB compressed;
- the proof-of-work takes 10–17 ms in WASM against 330 ms in JS;
- `dx` 0.7.9 skipped wasm-opt;
- the Dioxus mount adds to `#live` instead of replacing it.

**Assumed, to confirm in M0:**

- the bundle size limits;
- the CPU time for BLS verification, on both the app and the Restate Worker;
- the Workers plan (paid).

## 5. Milestones

| # | Milestone | Done when |
|---|---|---|
| **M0** | **Spikes and setup**: (a) Dioxus 0.7.10 fullstack Worker serving rafi's routes; (b) Restate Worker on the `wasm` branch writing D1 and calling a DO stub inside `ctx.run`, called by a Restate server with identity keys; (c) D1 through `worker::D1Database`; (d) `dioxus-clerk` server-side verification on Workers; (e) bundle size and BLS CPU time on both Workers; (f) the clock and `getrandom` audit. Also: `wrangler.toml` (D1 with EU jurisdiction, the DO class and its migration tag, both rate-limit bindings, the Cron, the secrets list), the build for the fullstack Worker, and a dev loop (`wrangler dev` and a local `restate-server`) to replace compose | Each spike works, or has a chosen fallback (§7) |
| **M1** | **`rafi-live`**: `ParticipantView` (with `claimed` and `last` skipping no-shows), `screen()`, `Card`, `CardStrings`, the state enum, the ALTCHA module. It needs no spike, so it can start now | For every `Screen` variant, SSR HTML equals the first render of the WASM; `screen()` tests from the prototype's cases |
| **M2** | **Store on D1**: `rafi-store` API and error type; D1 backend; `version` triggers; batches instead of transactions (freeze, fakes); set-based retention; `purge_*_at` in `save_draw_state`; ALTCHA replay table; erasure log; entrant cap | The store tests pass against D1 under `wrangler dev`, including a test that kills the freeze between its steps |
| **M3** | **`RaffleChannel` DO**: hub embedded by hand; batch read; `seen_v` stored; destroyed flag; periodic re-read alarm; destroy (final view, 4403, alarm, storage); jurisdiction; socket cap; Worker gate and `reject(4403)` | The prototype's scripted evening, ported to a harness against `wrangler dev`, passes, including the destroy races |
| **M4** | **Restate on Workers**: `rafi-restate-worker` (adapter, signatures before routing); projector writes D1, then a best-effort nudge `ctx.run` | A draw runs end to end through a local `restate-server`, and the DO publishes it |
| **M5** | **App on Workers**: `Config` from `Env`, failing closed; routes and server functions; attendee page rendering `Card`; `/participant` endpoint; rate limits; the Cron (retention, ALTCHA purge, DO cleanup); "End live updates"; remove `Hub`, `Watch`, `refresh_later`, the app's `LocalEngine` (it stays as `rafi-engine`'s `local` feature, for tests and development), the switch-to-local UI and API, the engine guard; `rafi-store` call sites; tests migrated | This admin flow works on staging: create → publish → open → join from two phones → freeze → three draws, including a no-show → finish → End live updates |
| **M6** | **Attendee WASM**: `rafi-participant` (mount, subscribe, proof-of-work, extras, no `expect`s); inline loader and fallback; disabled Join; Tailwind `@source` for the new crates; CI size check (≤ 280 KB compressed); confirm `dx` 0.7.10 runs wasm-opt | The phone test plan passes (§6) |
| **M7** | **Projector and admin live**: reveal from participant events; full-view poll every 2–3 s while live; fetch on mount and reconnect; `v >=` | The projector shows the reveal within about 1 s of the round |
| **M8** | **Cutover**: gate (no raffle Open, Frozen or Drawing); SQLite → D1 import; privacy notice; Restate deployment; DNS; deploy-freeze check in CI; a tested D1 → SQLite rollback export; Hetzner on standby until a second event succeeds; then delete Hetzner volumes, snapshots and dumps, and record it | Two real meetups run on Workers |

**Dependencies:**
- **Critical path:** M0 → M2 → M3 → M4 → M5 → M7 → M8.
- **M1** can run any time before M5.
- **M6** needs M1, M3 and M5.

## 6. Testing

- **Unit:**
  - `screen()`;
  - the DO's publish rule (`seen_v`, unchanged, stale, destroyed);
  - the ALTCHA module;
  - the freeze recovery.
- **Store:** the conformance suite (§11), against D1 in `wrangler dev` as well as natively.
- **App and integration:** the current `server/tests.rs` and `tests_restate.rs` cases, ported. Specifically:
  - neither view contains personal data;
  - retention, including DO destroy;
  - a join racing the freeze;
  - auth;
  - Restate draws through a local `restate-server`.
- **Browser:** the CDP smoke test (`smoke.mjs`) extended to join → draw → win, plus a fallback run with `WebAssembly` removed.
- **Phones (manual, before M8):**
  - iOS Safari, including Lockdown Mode;
  - Android Chrome;
  - locking and unlocking during a draw;
  - Wi-Fi off and on;
  - the chime playing on iOS.

## 7. Fallbacks if a spike fails

| Spike fails | Fallback |
|---|---|
| (b) Restate Worker | Keep the Restate handler as a container (current `restate-rafi-endpoint`) writing to D1 over its HTTP API, and nudging through an authenticated Worker route. |
| (e) Bundle size | Split Workers by role (public/attendee, admin, DO, Restate), connected by service bindings. |

## 7a. Ports: Workers supported, not required

Decided 2026-10-08. Cloudflare is one deployment, not an assumption of the code. Every platform-specific piece sits behind an interface, and Cloudflare's implementations live in one crate.

In three passes: (1) the interfaces, `rafi-cloudflare` and `LiveTransport` (done; §11); (2) all SQL into `rafi-store`, written for both dialects (done; §11); (3) Postgres, natively for self-hosting (done; §11).

- **`rafi-platform`:** the interfaces a deployment provides:
  - `Notifier`: "this raffle changed" by ID, "end its live updates", and the `LiveTransport` clients should use;
  - `RateLimit`, with the `Limit` keys;
  - `DrawStore`, where the draw engine writes draw states (since moved to `rafi-engine`);
  - the publish rule (since moved to `rafi_live::publish`);
  - `Notify`, which adapts a `Notifier` to the engine's `DrawingNotifier` (since revision 9; `rafi-platform` depends on `rafi-engine`, not the other way).

  It depends on no database and no platform.
- **`rafi-engine-restate` depends only on `DrawStore` and `DrawingNotifier`:** `services(drand, store, notifier)`. It knows nothing of Cloudflare, of which database is used, or of raffles: each host maps a drawing to its raffle's channel (`rafi_platform::Notify` with `rafi_raffle::drawing::raffle_id`).
- **Clients choose a transport from the server's `LiveTransport`** (in `rafi-live`):
  - `partyline`: Durable Objects over WebSockets;
  - `poll`;
  - later `sse`, for example Mercure.

  Adding a strategy means a `Notifier` implementation plus a client transport.
- **`rafi-cloudflare`** (`rafi-channel` renamed) holds all the Cloudflare glue:
  - the `RaffleChannel` DO, the gate and the jurisdiction helper;
  - the `Notifier` over Durable Objects;
  - the `RateLimit` over the bindings;
  - config from vars and secrets.

  The app's `worker.rs` and `rafi-restate-worker` only wire it up.
- **`rafi-store` owns all SQL**, as domain functions, the phones' snapshot included (data only; `rafi-live` builds the view). It implements `DrawStore`. Backends:
  - SQLite (native, the default; its migrations also serve D1);
  - **Postgres (native, self-hosted)**, chosen by the URL scheme (`postgres://`), with its own migrations in `migrations/postgres`;
  - D1 (Workers).

  Postgres on Workers through Hyperdrive is not planned. The storage conformance suite (`rafi_store::conformance`) is the contract every backend passes: SQLite and an embedded Postgres (Oliphaunt) always, a real Postgres when `RAFI_TEST_POSTGRES_URL` is set (as on CI), D1 under `wrangler dev`. A new backend (an embedded Postgres, libSQL, Postgres over Hyperdrive) is an implementation, a `Harness` and a passing suite.
- **Rate limiting has three layers:**
  - edge protection keyed by IP belongs to the platform (Cloudflare WAF; Caddy when self-hosted) and needs no app code;
  - rafi's own limits are app policy over the injected `RateLimit`: per raffle, and per email, never per IP, since a venue shares one;
  - the exact entrant cap lives in the database.

## 8. Self-hosting

Self-hosting is supported alongside Workers (§7a). The native server takes:
- **a database:** SQLite (the default) or Postgres, chosen by `RAFI_DATABASE_URL`;
- **a notifier:** `Polling` for now; Mercure over SSE is the reserved `sse` transport;
- **a rate limiter:** in-memory.

The draw engine runs as `restate-rafi-endpoint` against the same database. `deploy/compose.yaml` keeps SQLite, with a commented-out Postgres variant.

## 9. Decided

- **Self-hosting:** supported alongside Workers (§7a, §8): the native server with SQLite or Postgres and `restate-rafi-endpoint`.
- **Ending live updates:** manual, with Cron cleanup as the safety net.
- **Restate SDK:** the `wasm` git branch until the upstream PR is released.
- **ALTCHA:** rafi's own scheme; upstream fix later.
- **Wake lock:** in `rafi-participant`.
- **Emergency draws:** none; rely on Restate.
- **Store:** own API, with `worker::D1Database` behind it.
- **Projector:** reveal from participant events, plus polling.
- **Backups:** accept D1 Time Travel's 30-day window, and say so in the notice.

## 10. Open questions

1. **Workers plan:** paid, assuming M0(e) confirms the free plan's 10 ms CPU limit is too tight.
2. **Where the Restate Workers adapter lives long-term:** this repo, the SDK fork, or its own crate.

## 11. Implementation notes

Findings from building M1–M4 on the `cloudflare` branch, verified locally under `wrangler dev`:

- **Dioxus fullstack server doesn't compile for wasm32 as published.** `dioxus-server` 0.7.10 and 0.8.0-alpha.1 depend on `hyper-util` (`full`) and axum's `tokio`/`http1` features, which pull in tokio `net`/mio. `serve_dioxus_application` and `register_server_functions` are `cfg(not(wasm32))`. Plain `dioxus-ssr` works on Workers: a spike page rendered under `wrangler dev`, and its wasm is 144 KB compressed. This decides how M5 serves the admin and projector.
- **`workerd` doesn't implement Durable Object jurisdictions.** `RAFI_DO_JURISDICTION = ""` turns them off for local runs; production keeps the default, `eu`. `worker` has no jurisdiction API, so `rafi_cloudflare::namespace` calls the JS method.
- **D1 counts rows changed by triggers in `changes`.** The version triggers make a one-row `UPDATE` report 2, so only zero versus non-zero is portable; `Query::run` documents this.
- **D1 allows at most 100 bound parameters per statement**, so multi-row INSERTs are chunked (32 rows of fake entrants).
- **`D1Result::results` decodes through serde and loses column order**, so rafi-store reads the JS result objects directly, and `batch_rows` calls the binding's `batch`.
- **A raffle's channel object stays destroyed in memory until it is evicted.** Raffle IDs are unique, so this only matters for harnesses, which use a fresh raffle per run.
- **Locally, a server-side WebSocket close reaches Node clients only after about 10 s** (the dev proxy's close timeout). The code (4403) arrives intact. Phones also close on the final view.
- **The Restate Worker is 914 KB of wasm compressed.**
- **A failing nudge works as designed.** With the cross-Worker DO binding disconnected, draws still completed, `notify` gave up after three attempts, and the channel's 15 s re-read delivered the winner.
- **Local dev:** two `wrangler dev` sessions find each other through `~/.wrangler/registry`. Stopping a second session with the same Worker name unregisters it. Restate's ingress may need `http://[::1]:8080` if another process holds `127.0.0.1:8080`.
- **M0(d), Clerk on Workers:** `dioxus-clerk`'s `worker` feature runs axum with `ClerkAuthLayer` and the `ClerkAuth` extractor under `workerd`. Anonymous requests pass through, and bogus bearer tokens are rejected without a crash. The spike Worker was 509 KB compressed. Verifying real tokens needs a Clerk instance, so it waits for staging.
- **`scripts/workers-check.sh`** runs all local Workers checks: D1, the channel, the attendee page in Chromium, and a Restate draw. CI's `workers` job runs it.
- **The native app is stateless (M5, first half).** `rafi` is a library plus a thin binary. Platform pieces sit behind `AppState::from_parts(cfg, db, notifier, limiter)`, with the interfaces from `rafi-platform` (§7a): natively the `Polling` notifier and the in-memory `Limiter`. The Workers entry point builds `Db::d1`, `rafi_cloudflare::DurableObjects` and `RateLimitBindings`, and calls `retention::run` from the Cron. App tests run against an in-process stand-in for Restate's ingress (`server/fake_engine.rs`; since replaced by the in-process draw engine, now `rafi-engine`'s `local` feature, see [engine-plan.md](engine-plan.md)).
- **M5, the app on Workers.**
  - `rafi` builds as a Worker with the `worker` feature:
    - the same axum routes (since pass 2 without `#[worker::send]`: the store's futures are `Send` on wasm32 too);
    - the `Notifier` over the Durable Objects, `RateLimit` over the bindings (both from `rafi-cloudflare`);
    - config from vars and secrets, failing closed;
    - the Cron runs `retention::run`;
    - the `RaffleChannel` class is exported.
  - `crates/app/wrangler.toml` holds production and `[env.dev]`.
  - Locally, `crates/app/evening.mjs` passes a whole evening on the app Worker: setup, a phone joining in Chromium, a draw through Restate reaching the phone live, the finish, End live updates.
- **M7, the projector:** in the same evening, the projector tab shows the countdown and reveals the winner 5 ms after the phone. The reveal comes from the participant channel, and the full view is polled every 2.5 s.
- **M0(e), bundle sizes:** compressed, the app Worker is 847 KB, the Restate Worker 914 KB and the attendee WASM 263 KB. All are under Workers' limits. BLS CPU time still needs a deployed Worker.
- **Ports and adapters, pass 1 (§7a).** `rafi-platform` holds the interfaces; `rafi-channel` became `rafi-cloudflare`, the only crate (with the two Worker entry points) that knows Cloudflare. In practice:
  - `Notifier::notify` returns nothing: implementations log and drop their own failures. `DurableObjects` tries a nudge three times (100 ms, then 200 ms apart), so the Restate `notify` step's own attempts only cover the step failing. The journal step names (`project`, `notify`) are unchanged.
  - `LiveTransport` is chosen per raffle by `AppState::transport`: the notifier's while live updates run, plain polling once they've ended. The page config and the full view carry it as `live` (`{"kind":"partyline"}`, `{"kind":"poll","every_ms":2500}`). The JS fallback always polls.
  - The participant snapshot moved from the channel crate to `rafi_store::live`, which the app's `/participant` endpoint and the attendee page read too. It is data only since the domain pass (below): `rafi_live::raffle::view` builds the view.
  - `crates/channel-check` became `crates/cloudflare-check`; its Worker and D1 names are now `rafi-cloudflare-check` (the Restate Worker's `[env.check]` binds to that name).
  - Pass 2 moved the SQL that was still spread over the app into `rafi-store` (below).
- **Ports and adapters, pass 2 (§7a): all SQL in `rafi-store`.**
  - Modules: `raffles` (lifecycle, settings, the freeze's three guarded steps `freeze`, `commit`, `unfreeze`), `items`, `entrants` (the capped `join`, manual and fake entrants, `erase` with its log), `audit`, `altcha`, `retention` (set-based erasure, channels to end, `meta`), `view::Full` (the full view's one-batch read) and `live::Snapshot`. The row types moved with them. The app keeps validation, Restate calls, notifications and auth; `server/db.rs` is gone and the app has no SQL. The check Workers keep their own test-setup SQL.
  - Portable SQL: no `rowid`. Items are ordered by `position, id`, entrants by `created_at, id`, so ties (same position; same second, as with fake entrants) now break by ID rather than insertion order. Boolean parameters are compared (`?4 = 1`); flags stay 0/1 integers.
  - The only dialect-specific SQL is reading entrant IDs out of the draw state's JSON (`json_each`, `json_extract`), in `dialect::draw_state_entrants`. Two queries use it: the snapshot's drawn entrants and retention's claimed winners. Since migration 0004 ([drawing-review.md](drawing-review.md)) the drawing is stored as its manifest and event log, and those entrants are JSON array columns written with it (`raffles.drawn_entrants`, `raffles.winners`), read by `dialect::entrants_in`.
  - `Send` everywhere: the D1 backend wraps its futures (`worker::send::SendFuture`), so `Db`'s futures are `Send` on wasm32 too, and the handlers dropped `#[worker::send]`. reqwest's wasm32 futures aren't `Send` either, so the Restate client wraps its calls in `rafi_platform::sendable`, which is a no-op natively. The app's server code no longer mentions Cloudflare outside `worker.rs`.
  - The store testsuite (native and D1) gained scenarios for the freeze steps, the entrant cap and unique email, fake entrants across statements, erasure and its log, retention, ALTCHA replay, the full view and the participant snapshot.
  - For pass 3 (Postgres):
    - `?N` placeholders are reused within a statement. No statement mixes `?` and `?N`, and no `?` appears inside a string literal, though `$` does (JSON paths).
    - `Value::Null` is bound for text and integer columns, and in `(?2 IS NULL OR id = ?2)` (`entrants::erase`). The native backend binds it as a typed `i64` null, which Postgres would reject against text.
    - `SUM` over a `BIGINT` column is `NUMERIC` in Postgres (the snapshot's `COALESCE(SUM(qty), 0)`), and `SELECT 1` is `INT4` (`entrants::email_taken`). Decoding has to accept both, or the SQL casts.
    - Unique violations are recognized by sqlx's `is_unique_violation`. The message fallback, `UNIQUE constraint failed`, is SQLite's.
    - The `retention::DUE` predicate is built once (`LazyLock`) from the dialect function, so it needs one per dialect. `draw_state` is `TEXT`, so the Postgres variant casts it to `jsonb`.
- **Ports and adapters, pass 3 (§7a): Postgres for self-hosting.** Native only; SQLite stays the default and D1 the Workers backend, both unchanged.
  - **Selection:** `Db::connect` and `connect_lazy` open Postgres for `postgres://` and `postgresql://` URLs, SQLite for anything else (as before). `Db::dialect()` returns `Dialect::Sqlite` (SQLite, D1) or `Dialect::Postgres`. The backend is rafi-store's default `postgres` feature (sqlx's Postgres driver with rustls); it is inert on wasm32. Natively, `Native` is an enum over the `sqlite` and `postgres` backends.
  - **Placeholders:** the Postgres backend rewrites `?N` to `$N` and each bare `?` to the next `$N`, outside string literals, quoted names and comments. `$` is left alone.
  - **Parameters:** values are bound as `INT8`, `FLOAT8` or `TEXT`. `NULL` is bound untyped (OID 0), so Postgres infers its type from the column or comparison. Statements that bind a `NULL` aren't cached (sqlx caches prepared statements by SQL text alone), so an inferred type never sticks to a later run that binds a value. `(?2 IS NULL OR id = ?2)` on its first run is refused (Postgres types `$2` at `IS NULL` and can't: SQLSTATE 42P08); the backend then runs the whole operation again with `NULL`s typed `TEXT`. The SQL is unchanged.
  - **Decoding** by type OID, from the binary format: `INT2`/`INT4`/`INT8` and `BOOL` as `Int`; `NUMERIC` (from `SUM`) as `Int` when integral, else `Real`; `FLOAT4`/`FLOAT8` as `Real`; text types as `Text`. No SQL needed a cast: `COALESCE(SUM(qty), 0)` and `SELECT 1` decode as they are.
  - **Transactions are serializable** (`default_transaction_isolation` on every connection), as SQLite's effectively are, so the entrant cap (`INSERT … SELECT … WHERE (SELECT COUNT(*) …) < cap`) stays exact when joins race; under Postgres's default read committed it would not. A serialization failure or deadlock reruns the operation (a statement or a whole batch, both self-contained), up to ten times with a short jittered sleep. A test races 16 joins against a cap of 5.
  - **Batches** run in a transaction, as on SQLite; `batch_rows` therefore reads one serializable snapshot.
  - **The one dialect statement:** `jsonb_array_elements((draw_state)::jsonb -> 'events') AS x(value)` with `x.value ->> 'type'` and `->> 'entrant_id'`. A `NULL` draw state has no elements, as with `json_each`. `retention::due(dialect)` builds its predicate once per dialect.
  - **Migrations:** `migrations/postgres/0001_init.sql` and `0002_workers.sql` mirror the SQLite files: `BIGINT` for every integer (flags stay 0/1), `GENERATED BY DEFAULT AS IDENTITY` for `AUTOINCREMENT`, the same constraints and indexes. The version bump is a `BEFORE UPDATE` trigger on `raffles` that increments `NEW.version` unless the write already changed it (SQLite's `AFTER UPDATE` re-updates the row; the result is the same), and one `AFTER INSERT OR UPDATE OR DELETE` trigger function per child table bumps the raffle. sqlx and wrangler skip the subdirectory in `migrations/`, so D1 and SQLite are unaffected.
  - **Unique violations** come from sqlx's `is_unique_violation` (SQLSTATE 23505), as before.
  - **Tests:** with `RAFI_TEST_POSTGRES_URL` set, the store testsuite runs every scenario on a fresh schema (`rafi_store::testing::fresh_schema`: `CREATE SCHEMA`, then `search_path` through the URL's `options`), and so do the app's tests, including the end-to-end test where `restate-server` and `restate-rafi` share the schema. The schemas are left behind: use a throwaway database. The `erase` scenario now erases everyone first, which caught the `NULL` typing above. CI's `test` job runs a `postgres:16` service.
- **The storage conformance suite.** The store testsuite became `rafi_store::conformance` (feature `conformance`): the contract every storage backend passes. A new backend is an implementation, a `Harness` and a passing suite.
  - **Registry:** `SCENARIOS`, a static list of named scenarios, each declaring the `Capabilities` it needs. `query.*` covers the query layer: `decode_types`, `null_params`, `batch_atomicity`, `batch_rows_snapshot`, `batch_rows_under_writes`, `unique_violation`, `change_counts`, `ordering_ties`, `param_limit` (100 bound parameters, D1's limit) and `version_triggers`. `domain.*` covers the domain functions: `save_draw_state`, `freeze_steps`, `join_cap`, `join_cap_race`, `fakes_across_statements`, `erasure_log`, `retention`, `altcha_replay`, `full_view` and `participant_snapshot`. Scenarios use only the public `Db` and domain API.
  - **Capabilities:** one so far, `CONCURRENT_SESSIONS` (transactions that really overlap). Only Postgres has it; `domain.join_cap_race` and `query.batch_rows_under_writes` need it, and are skipped elsewhere with the reason.
  - **Harnesses:** `Harness` has `name()`, `capabilities()` and `fresh()`, which returns an empty, migrated database. `conformance::Sqlite` gives a new in-memory database per scenario; `conformance::Postgres` gives a new schema in `RAFI_TEST_POSTGRES_URL`'s database. `store-d1-check` has D1's: local D1 has one database, so it empties the tables.
  - **Runners:** `crates/store/tests/conformance.rs` (`harness = false`, libtest-mimic) makes one test per backend and scenario, `<backend>::<scenario>`. A backend that isn't available, or a missing capability, shows as ignored with the reason. `store-d1-check` answers with each scenario's outcome as JSON (`ok`, `skipped: …` or the failure); `workers-check.sh` fails on any failure.
  - **Found:** a batch mixing a column-typed `NULL` with a `NULL` that only `?N IS NULL` types failed on Postgres. The retry typed every statement's `NULL`s as `TEXT`, and `delivered_at = NULL::text` is refused. It went unnoticed because sqlx reuses a statement cached from an earlier run with values, even when it isn't asked to cache. Now only the refused statement's `NULL`s become `TEXT`. One statement still can't bind both kinds.
  - **Found on D1:** `SELECT hidden = 1, purged = 1, 1, COUNT(*)` decoded out of order. D1 rows are JavaScript objects, which put integer-like keys (the column `1`) first and keep one value per name. The rule is now part of the contract (`query.decode_types`): every selected column of a multi-column row has a distinct, non-integer name. The store's SQL already followed it (`SELECT 1` only appears alone or in `EXISTS`).
  - **Embedded Postgres** (feature `embedded-postgres`, which only rafi-store's own tests turn on): `conformance::EmbeddedPostgres` runs Oliphaunt (`=0.3.2`, pinned: young and fast-moving), which starts a normal local PostgreSQL 18 from prebuilt native binaries (macOS arm64, Linux x64/arm64, Windows). One server per test run, started on the first scenario that needs it (about 3 s: it initializes a fresh data directory), and a fresh schema per scenario, as on a real Postgres; with `CONCURRENT_SESSIONS`, so the race scenarios run too. So `cargo test` covers the Postgres dialect with nothing installed, on Postgres 18 next to CI's 16. The server is a child process and libtest-mimic's `exit` drops nothing, so the runner stops it explicitly. `RAFI_TEST_EMBEDDED_POSTGRES=off` skips the tier. Tried first: `pglite-oxide` (PGlite on Wasmer), dropped for a single client, pinned Wasmer alphas, four unmaintained-crate advisories and a shutdown deadlock.
  - **Portability rules the suite pins down:** only zero versus non-zero change counts (D1 counts trigger writes); integers exact up to 2^53 (D1); ties in `ORDER BY` broken by ID (lowercase hex, which sorts the same under any collation); `NULLS FIRST`/`NULLS LAST` wherever a nullable column is ordered (SQLite and Postgres differ by default).
- **Domain and engine pass (2026-10-10).**
  - **Each Worker builds once per isolate:** the Restate Worker caches its `Endpoint`, the app Worker its `(AppState, ClerkAuthLayer)` (a `thread_local` `OnceCell`; a failed build isn't kept, so a fixed secret takes effect on the next request). Clerk's signing keys (JWKS) are no longer fetched per request.
  - **The Restate Worker has no retention settings:** `RAFI_RETENTION_DAYS` and `RAFI_RETENTION_WINNERS_DAYS` are gone from its `wrangler.toml`. The app's retention run dates erasure ([engine-plan.md](engine-plan.md) revision 8).
  - **The wasm32 deadline race is done:** `RestateClient` races each synchronous call against a `gloo-timers` timer, so on the app Worker a call past `RESTATE_TIMEOUT_SECS` is `Pending`, as natively.
  - **`rafi-store` no longer depends on `rafi-live`:** `live::Snapshot` is data (the raffle's `RaffleState`; hidden names come back as `NULL` from the SQL). `rafi_cloudflare::view` and the app's `server::view::participant` build the view with `rafi_live::raffle::view` (revision 9 made them one function, `Snapshot::participant_view`, behind rafi-store's `live` feature; since 2026-10-14 they are two again, `rafi_cloudflare::participant_view` and `rafi_app_core::view::participant`, and rafi-store has no `live` feature); `HIDDEN_NAME` is `rafi-live`'s. The phone still pulls in neither `rafi-drawing` nor drand.
  - **Unknown raffle states fail loudly:** `RaffleRow.state` is a `RaffleState`, decoded by the row decoder; nothing falls back to `draft`. The store's SQL state lists come from `rafi-raffle`'s predicates (`raffles::states`).
- **Still needs your accounts:**
  - staging and production on Cloudflare: D1 with `--jurisdiction eu`, secrets, custom domains;
  - the Restate Cloud deployment;
  - real Clerk verification;
  - BLS CPU time on a deployed Worker;
  - the phone test plan (§6);
  - the cutover (M8).
