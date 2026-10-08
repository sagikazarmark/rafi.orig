# Rafi on Cloudflare Workers (plan)

Status: accepted, 2026-10-08. Reviewed in four passes: fit with the code, distributed design, security/privacy/operations, and structure and client. The findings are folded in below.

This plan builds on the prototypes on the `prototype/participant-view` branch (`docs/prototype-participant-view.md` there).

## 1. Goal

Run the app and the Restate handler on Cloudflare Workers, with no in-memory processes, and push live updates over WebSockets from a Durable Object per raffle using [partyline](https://github.com/sagikazarmark/partyline).

Restate is the only draw engine. The DO is only the live fan-out: it never holds workflow state.

| Today | On Workers |
|---|---|
| `Hub` (SSE fan-out) and its GC task | `RaffleChannel` DO, partyline Latest channel |
| `Watch` (polls the database for Restate's writes) | Restate's projector nudges the DO |
| `refresh_later` (delayed names) | The projector's poll picks them up; phones never show names of non-winners |
| `LocalEngine` (in-process draws) and the admin's "switch to local" | Removed. Rafi relies on Restate (§2.7) |
| `Limiter` (in-memory) | Workers rate-limiting binding (soft), plus a per-raffle entrant cap in D1 (hard) |
| `Altcha.used` (in-memory replay set) | D1 table; a duplicate INSERT fails |
| Hourly retention task | Cron Trigger |
| SQLite + Litestream on Hetzner | D1, EU jurisdiction |
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
- **`Card` renders one root (or nothing)** and calls no browser APIs while rendering. The reload, timers and audio live in `rafi-attendee`.
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
- `LocalEngine` goes;
- so do the admin's "switch to local" action and API;
- so does the engine column guard.

### 2.8 Data layer

- **`rafi-store` exposes its own functions and error type**, with `worker::D1Database` behind them. sqlx stays only for a future SQLite backend (§8).

  `sqlx-d1` 0.4.3 pins sqlx-core 0.8.6, while the workspace is on sqlx 0.9.
- **No interactive transactions** (D1 doesn't have them):
  - **Freeze** becomes: check there are entrants; a batch that sets `frozen` (guarded by state); read the list; write the commit doc.

    "Frozen without a commit doc" becomes recoverable: Start rebuilds the commit doc from the fixed entrant list.
  - **Fake entrants** become one batch of multi-row INSERTs.
- **Retention uses set-based `UPDATE`s**, to stay under D1's per-invocation query limit.
- **`save_draw_state` sets `purge_*_at`** when the state is finished. `Watch` used to do that.
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
- **`reqwest` on wasm** has no `ClientBuilder::timeout`, so use per-request timeouts.

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
| `rafi-core` | unchanged | Domain model, verifiable draw | everyone except the attendee WASM |
| `rafi-live` | **new** | `ParticipantView`, the `RaffleLive` channel, `screen()`, `Card`, `CardStrings`, a lightweight state enum, the ALTCHA module (types, `solve`, `verify`) | DO, app, attendee WASM |
| `rafi-attendee` | **new** | The attendee WASM: mounts `Card`, subscribes, proof-of-work, extras (wake lock module included) | phones |
| `rafi-store` | ported | Own API and error type; D1 backend; `version` | app, Restate handler, DO |
| `restate-rafi` | ported | `Raffle` virtual object; projector writes D1, then nudges the DO in a separate, best-effort `ctx.run` | Restate Worker |
| `rafi` (app) | ported | App Worker: routes, server functions, `RaffleChannel` DO, Cron | |
| `rafi-restate-worker` | **new** | Restate Worker: the adapter around `Endpoint::handle_with_options`, signature checks, D1 and DO bindings | Restate Cloud |
| `restate-rafi-endpoint` | kept, not deployed | The native endpoint, for future self-hosting (§8) | |

`rafi-live` must not depend on `rafi-core`, which pulls in drand and BLS. The server maps `RaffleState` to `rafi-live`'s enum.

### Reuse

- **ALTCHA:** rafi's own scheme for now. Contribute the `web-time` fix to the official `altcha` crate later, then move to protocol v2.
- **Wake lock:** stays in `rafi-attendee`.
- **Vibrate, the chime and the theme toggle:** stay in `rafi-attendee`, which owns the toggle (the inline `<head>` script stays in the page).
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
- **The projector's `ctx.run` retries without limit** (`restate-rafi/src/service.rs:64`).
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
| **M5** | **App on Workers**: `Config` from `Env`, failing closed; routes and server functions; attendee page rendering `Card`; `/participant` endpoint; rate limits; the Cron (retention, ALTCHA purge, DO cleanup); "End live updates"; remove `Hub`, `Watch`, `refresh_later`, `LocalEngine`, the switch-to-local UI and API, the engine guard; `rafi-store` call sites; tests migrated | This admin flow works on staging: create → publish → open → join from two phones → freeze → three draws, including a no-show → finish → End live updates |
| **M6** | **Attendee WASM**: `rafi-attendee` (mount, subscribe, proof-of-work, extras, no `expect`s); inline loader and fallback; disabled Join; Tailwind `@source` for the new crates; CI size check (≤ 280 KB compressed); confirm `dx` 0.7.10 runs wasm-opt | The phone test plan passes (§6) |
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
- **Store:** against D1 in `wrangler dev`.
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

- **`rafi-platform`:** the interfaces a deployment provides:
  - `Notifier`: "this raffle changed" by ID, "end its live updates", and the `LiveTransport` clients should use;
  - `RateLimit`, with the `Limit` keys;
  - `DrawStore`, where the draw engine writes draw states;
  - the publish rule.

  It depends on no database and no platform.
- **`restate-rafi` depends only on `DrawStore` and `Notifier`:** `Raffle::new(drand, store, notifier)`. It knows nothing of Cloudflare or of which database is used.
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
- **`rafi-store` owns all SQL**, as domain functions, the participant snapshot included. It implements `DrawStore`. Backends:
  - SQLite (native; its migrations also serve D1);
  - **Postgres (native, self-hosted)**, chosen by the URL scheme;
  - D1 (Workers).

  Postgres on Workers through Hyperdrive is not planned yet. The store testsuite runs against every backend.
- **Rate limiting has three layers:**
  - edge protection keyed by IP belongs to the platform (Cloudflare WAF; Caddy when self-hosted) and needs no app code;
  - rafi's own limits are app policy over the injected `RateLimit`: per raffle, and per email, never per IP, since a venue shares one;
  - the exact entrant cap lives in the database.

## 8. Self-hosting

Cloudflare only for now. Self-hosting stays possible later:

- **The store sits behind `rafi-store`'s own API.** SQLite through sqlx can become a second backend.
- **Nudging goes through one small interface.** The DO now; in-process fan-out later.
- **Phones fall back to polling `/participant`**, and the projector and admin already poll their full view. Those would be the baseline without a DO.
- **`restate-rafi-endpoint` keeps compiling.** A self-hosted deployment uses Restate too.

## 9. Decided

- **Self-hosting:** later; Cloudflare only for now.
- **Ending live updates:** manual, with Cron cleanup as the safety net.
- **Restate SDK:** the `wasm` git branch until the upstream PR is released.
- **ALTCHA:** rafi's own scheme; upstream fix later.
- **Wake lock:** in `rafi-attendee`.
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
- **`workerd` doesn't implement Durable Object jurisdictions.** `RAFI_DO_JURISDICTION = ""` turns them off for local runs; production keeps the default, `eu`. `worker` has no jurisdiction API, so `rafi_channel::namespace` calls the JS method.
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
- **The native app is stateless (M5, first half).** `rafi` is a library plus a thin binary. Platform pieces sit behind `AppState::from_parts(cfg, db, live, limiter)`: `server::live::Live` (`has_channel`, `nudge`, `destroy`; natively `Polling`, a no-op) and `server::ratelimit::RateLimit` (`allow(Limit, key)`; natively the in-memory `Limiter`). The Workers entry point builds `Db::d1`, a `Live` over `rafi_channel::{nudge, destroy}` and a `RateLimit` over the two bindings, and calls `retention::run` from the Cron. App tests run against an in-process stand-in for Restate's ingress (`server/fake_engine.rs`).
- **M5, the app on Workers.**
  - `rafi` builds as a Worker with the `worker` feature:
    - the same axum routes, `#[worker::send]` on handlers;
    - `Live` over `rafi_channel`, `RateLimit` over the bindings;
    - config from vars and secrets, failing closed;
    - the Cron runs `retention::run`;
    - the `RaffleChannel` class is exported.
  - `crates/app/wrangler.toml` holds production and `[env.dev]`.
  - Locally, `crates/app/evening.mjs` passes a whole evening on the app Worker: setup, a phone joining in Chromium, a draw through Restate reaching the phone live, the finish, End live updates.
- **M7, the projector:** in the same evening, the projector tab shows the countdown and reveals the winner 5 ms after the phone. The reveal comes from the participant channel, and the full view is polled every 2.5 s.
- **M0(e), bundle sizes:** compressed, the app Worker is 847 KB, the Restate Worker 914 KB and the attendee WASM 263 KB. All are under Workers' limits. BLS CPU time still needs a deployed Worker.
- **Still needs your accounts:**
  - staging and production on Cloudflare: D1 with `--jurisdiction eu`, secrets, custom domains;
  - the Restate Cloud deployment;
  - real Clerk verification;
  - BLS CPU time on a deployed Worker;
  - the phone test plan (§6);
  - the cutover (M8).
