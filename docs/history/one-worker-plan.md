# One Worker (plan)

> **Historical document.** A working note from Rafi's development, kept for the reasoning behind its decisions. Names, paths and details are of their time and may not match the current code, which is the source of truth.

Status: implemented, 2026-10-21. Follows [workers-plan.md](workers-plan.md), which put the app and the raffle engine
in two Workers (`rafi` and `rafi-restate`) sharing one D1 database and one Durable Object namespace. Nothing is
deployed yet ([cloudflare.md](../operations/cloudflare.md)), so this is the cheap moment.

## 1. Goal

Rafi on Cloudflare is **one Worker**, `rafi`: the app's routes, the static files, the Cron, the `RaffleRoom`
Durable Object **and the raffle engine's Restate endpoint**, served under `/restate/*`. Restate Cloud registers
`https://<app host>/restate` as its deployment.

`apps/restate-worker` goes. Natively nothing changes: `rafi` and `rafi-restate` stay two processes (§2.9).

**What changed from the text below while implementing it:**

- **M0's numbers:** the two Workers were 887 KB and 851 KB compressed (`gzip -9` of the wasm); the merged Worker
  is 4.54 MB raw, **1.44 MB compressed**, the same with or without `dev`. Within the limits, as expected.
- **The Worker owns `/restate` itself too**, not only `/restate/*` (`engine::owns`, and both patterns in
  `run_worker_first`), so no path under its prefix ever reaches the SPA's `index.html`. Under `wrangler dev`,
  `/restate` and `/restate/` answer the SDK's 400, `/restatex` is the SPA's.
- **Discovery needs the manifest's `accept` header.** `curl` sends `accept: */*`, which the SDK refuses with 415;
  Restate itself sends `application/vnd.restate.endpointmanifest.v3+json` (or v2/v4). §6 is corrected.
- **`evening.mjs` reads the engine's state back over the ingress** (`INGRESS`, default `:18080`), for the check it
  took over from `e2e.mjs`. Both new checks pass; the whole evening passes on the merged Worker.
- **`scripts/workers-check.sh` leaves a `workerd` behind if interrupted** between `serve` and the trap: nothing
  new, but a stale one on `:8787` answers `/healthz` with the previous build's asset manifest and makes the next
  run's phone fail to join. Kill the `wrangler` process, not only `workerd`.
- Verified on the release build (no `dev`): without `RESTATE_IDENTITY_KEYS` every request is `500 misconfigured`
  with the reason logged; with a key, unsigned `/restate/discover`, `/restate/health` and `/restate/invoke/…`
  are 401 while the app serves normally.

## 2. Decisions

### 2.1 Why merge, and why not Worker RPC

**What two Workers cost today**, all for one process boundary that protects nothing the code doesn't already:

- the D1 binding is repeated, and its ID must be put into two files (`cloudflare.md` §1);
- the engine reaches the DO through a cross-Worker binding, `script_name = "rafi"`, with a `"rafi-dev"` and a
  `"rafi-cloudflare-check"` variant (`apps/restate-worker/wrangler.toml`);
- deploys are ordered (app, then engine), and the engine is a second hostname with a second secrets setup;
- locally, two `wrangler dev` sessions must share one `--persist-to` **and** find each other through the dev
  registry ([getting-started.md](../development/getting-started.md#cloudflare-workers), `devenv.nix`, three
  `serve` calls in `scripts/workers-check.sh`).

**What the boundary doesn't give.** D1 has no per-binding or per-table permissions, so the engine Worker has the
same access to the database as the app ([engine-plan.md](engine-plan.md) step G already notes this). Least
privilege is enforced in code: the engine gets a `RaffleWriter` (`crates/store`), and the `Raffle` object sees
anonymous IDs only. That stays exactly as it is in one Worker.

**Cloudflare doesn't ask for one owner per D1.** Their docs describe several Workers binding the same
`database_id` (the Previews docs do so explicitly) and say nothing about routing other Workers' access through
RPC. "One database per service" is the general microservices rule; rafi already has what it protects, one schema
owner (`rafi-store`, one `migrations/sqlite`).

**Worker RPC between the two Workers is rejected.** It wouldn't remove the shared local state (service bindings
between two `wrangler dev` sessions use the same dev-registry discovery the DO `script_name` binding needs
today), `workers-rs` 0.8's RPC is experimental (`wasm-bindgen` exports server-side, hand-written client bindings,
complex arguments untested), and it would mean a second, versioned internal API for `save_raffle` inside the
engine's `ctx.run`. All the costs above would stay.

**`wrangler dev -c app -c engine`** (one process, shared state for free) is rejected too: only the primary Worker
gets a local URL, and Restate must reach the engine over HTTP. Proxying `/restate/*` to the auxiliary Worker is
the merge with an extra hop.

### 2.2 The engine under a path prefix

Verified against the pinned SDK (`restate-sdk` 0.12.1 on the `wasm` branch, `restate-sdk-shared-core` 7.0.3):

- the endpoint routes on the **last** path segments, `…/discover`, `…/health` and `…/invoke/{service}/{handler}`
  (`src/endpoint/mod.rs`, `handle_with_verifier`), so any prefix works;
- identity verification normalises the path before checking the JWT's `aud`
  (`src/request_identity.rs`, `normalise_path`), so Restate's signatures verify under a prefix as well.

Restate accepts a deployment URI with a path and appends `/discover` and `/invoke/…` to it. The prefix is
**`/restate`**. It's the one word operators will look for; it doesn't clash with any app route (`/api`, `/j`,
`/rules`, `/privacy`, `/partyline`, `/healthz`, `/participant`). Restate's *ingress* also has `/restate/…` paths,
but that's on the Restate server, never on this Worker.

### 2.3 Routing in `fetch`

In `apps/worker/src/lib.rs`, in this order:

1. `/restate/*` → the SDK endpoint, `handle_with_options(req, RequestResponse)`, with the request's path
   **unmodified** (the SDK needs the full path for `aud`). Nothing else sees these requests: not Workers Assets,
   not Clerk's layer, not axum.
2. `/partyline/raffle/{id}` → `rafi_cloudflare::connect`, as today.
3. everything else → `rafi_server::routes` behind `ClerkAuthLayer`, as today.

`run_worker_first` in `wrangler.toml` gains `"/restate/*"`, so Assets never answers for it (a miss there would
otherwise return the SPA's `index.html` with 200).

The SDK verifies identity before its own routing, discovery included (workers-plan §2.11). That still holds: the
prefix match is a string comparison, the first thing that touches a `/restate/*` request is the verifier.

### 2.4 One bundle, built once, failing closed on every secret

`cached(env)` builds **one** value per isolate: `(AppState, Option<ClerkAuthLayer>, Rc<Endpoint>)`. The required
secrets are the union: `RAFI_ALTCHA_KEY`, `RESTATE_INGRESS_URL`, `CLERK_SECRET_KEY` (unless dev auth),
`RESTATE_API_KEY` as today, plus **`RESTATE_IDENTITY_KEYS`** (empty allowed only with the `dev` feature).

A missing or invalid identity key therefore refuses *every* request, `/healthz` included, not only the engine's.
That is the existing policy ("required secrets stop the Worker if missing", workers-plan §2.9) and it makes the
misconfiguration visible on the first deploy check. Building the endpoint is cheap: `DrandClient::new` doesn't
touch the network; its verified chain is fetched on first use, as now.

The endpoint builder moves from `apps/restate-worker/src/lib.rs` to a module, `apps/worker/src/engine.rs`
(`endpoint(env) -> anyhow::Result<Endpoint>`, `drand_config(env)`). By the crate rule (workers-plan §3): start as
a module; there is no second consumer.

### 2.5 Durable Objects and the `room` feature

The engine's `Observer` is `DurableObjects::new(env)` over the Worker's own `RAFFLE_ROOM` binding. `script_name`
goes everywhere.

`rafi-cloudflare`'s `room` feature existed so that the Restate Worker wouldn't export the `RaffleRoom` class
(`crates/cloudflare/Cargo.toml`). After this, every Worker that links the crate hosts the object (`apps/worker`,
`testbeds/cloudflare`; `testbeds/store-d1` doesn't link it). **The feature goes**; `partyline-worker` and
`serde_json` become plain dependencies.

### 2.6 Configuration: one environment

`apps/worker/wrangler.toml` is the only Workers configuration.

- **Secrets** (`wrangler secret put`, one list): `RAFI_ALTCHA_KEY`, `CLERK_SECRET_KEY`, `RESTATE_API_KEY`,
  `RESTATE_IDENTITY_KEYS`.
- **Vars**: as today, plus the engine's `RAFI_DRAND_RELAYS`, `RAFI_DRAND_LEAD_SECS`, `RAFI_DRAND_TIMEOUT_SECS`
  when they differ from the defaults. `RAFI_DO_JURISDICTION` is set once.
- **The mock chain.** The app's `Config` already reads `RAFI_DRAND_MOCK` as `RAFI_DRAND_CHAIN=mock`
  (`crates/server/src/config.rs`, `drand_chain`). `[env.dev]` sets **`RAFI_DRAND_MOCK = "1"`** and drops
  `RAFI_DRAND_CHAIN`, so the app and the engine can no longer disagree ("must match the app's
  `RAFI_DRAND_CHAIN`" leaves [configuration.md](../reference/configuration.md)). `[env.dev]` also gets
  `RAFI_DRAND_LEAD_SECS = "3"`, the engine's dev value.
- **The `dev` feature** of `rafi-worker` is the union: `rafi-server/mock-drand`, `rafi-engine/mock-drand`, and
  requests accepted without identity keys. Release builds reject dev auth, the mock chain and empty keys.
- `[env.check]` has no successor (§2.8).

### 2.7 Deploying

One `wrangler deploy`, after `wrangler d1 migrations apply rafi --remote`. "Deploy the app before the engine" in
[upgrades.md](../operations/upgrades.md) becomes "apply migrations, deploy". Registering the deployment with
Restate for every release stays, with the URI `https://<app host>/restate`.

A deploy drops every socket (DOs restart). Today that's true of every app deploy; now an engine-only change does
it too. The deploy freeze (workers-plan §2.11) already covers it.

### 2.8 Checks

- **`scripts/workers-check.sh`:** step 4, "a draw through Restate" (`apps/restate-worker/e2e.mjs` against
  `testbeds/cloudflare`'s DO under `[env.check]`), **goes**: step 5, "an evening on the app Worker", draws through
  Restate on the real Worker and sees the result on a phone. Any check `e2e.mjs` has and `evening.mjs` lacks
  (the committed round is a positive integer; Restate's own state has the draw, via `/Raffle/{id}/get`) moves
  into `evening.mjs`. Step 5 registers `http://127.0.0.1:8787/restate` and serves one Worker. The restate-worker
  build goes from the build step. Steps are renumbered; the header comment follows.
- **`testbeds/cloudflare`** keeps its two roles, the live channel scenario and the participant smoke. It no longer
  hosts the DO for anyone else.
- **CI:** the two `cargo clippy … -p rafi-restate-worker` lines go.
- **`devenv.nix`, `workers` profile:** `wrangler.dev.engine` and its readiness probe go;
  `services.restate.deployments.engine.uri = "http://127.0.0.1:8787/restate"` (still `http1 = true`), after
  `wrangler-app`; `rafi:d1-migrations` runs before `wrangler-app` only. `wranglerState` stays (one Worker still
  wants a stable `--persist-to` across restarts), its "shared by both Workers" comment goes.

### 2.9 Natively: unchanged, with a door left open

`rafi` (axum) and `rafi-restate` (`HttpServer`) stay separate processes. Self-hosters run the engine wherever
their Restate can reach it, on its own port and with its own `RAFI_RESTATE_*` configuration; nothing here asks
them to change.

The SDK's `Endpoint::handle` takes an `http::Request`, so the endpoint *could* be mounted at `/restate` in the
native server too, giving one process everywhere and one fewer process in `devenv up`. That's a separate decision
with its own trade-offs (one binary for two scaling profiles, the Restate tunnel, `rafi-restate`'s config
loading) and is **out of scope**. The architecture doc states the asymmetry plainly: on Workers the engine lives
in the app's Worker because a Worker is a routing unit, not a process.

### 2.10 Security, restated

- The Restate endpoint's "own hostname" (workers-plan §2.11) becomes "own path prefix". The control that matters,
  **signature verification before any routing**, is unchanged; the hostname was defence in depth.
- One Worker holds every secret: Clerk, ALTCHA, the ingress API key and the identity keys. Both Workers already
  held full D1 access, so the blast radius of a compromised isolate barely moves.
- Invocation logs stay off. `/restate/*` requests carry no cookies or client identity; they are Restate's.

## 3. What changes

| Where | Change |
|---|---|
| `apps/restate-worker/` | **Deleted**: `Cargo.toml`, `src/lib.rs`, `wrangler.toml`, `e2e.mjs` |
| `apps/worker/src/lib.rs` | `/restate/*` routed first (§2.3); the cached bundle gains the endpoint (§2.4); crate doc updated |
| `apps/worker/src/engine.rs` | **New**: `endpoint(env)`, `drand_config(env)`, moved from the engine Worker |
| `apps/worker/Cargo.toml` | + `rafi-store`, `rafi-engine-restate`, `restate-sdk` (`rust_crypto`), `http-body-util`; `dev` = `rafi-server/mock-drand` + `rafi-engine/mock-drand`; description |
| `apps/worker/wrangler.toml` | `run_worker_first` + `"/restate/*"`; `[env.dev.vars]`: `RAFI_DRAND_MOCK = "1"`, `RAFI_DRAND_LEAD_SECS = "3"`, `RAFI_DRAND_CHAIN` dropped; comments |
| `apps/worker/evening.mjs` | The checks from `e2e.mjs` it lacks (§2.8) |
| `crates/cloudflare/Cargo.toml`, `src/lib.rs` | `room` feature removed; doc: "the Cloudflare glue the Worker and the testbed share" |
| `crates/engine-restate/src/lib.rs`, `README.md` | "`rafi-restate` (native) and `rafi-worker` (Cloudflare Workers) serve them" |
| `testbeds/cloudflare/Cargo.toml` | `features = ["room"]` dropped |
| `devenv.nix` | §2.8 |
| `scripts/workers-check.sh` | §2.8 |
| `.github/workflows/ci.yaml` | The `rafi-restate-worker` clippy lines go |
| `docs/operations/cloudflare.md` | "one Worker and one D1 database"; the Worker table has one row; "First deploy" steps 2 and 3 merge (one secrets list, one deploy, register `https://<host>/restate`); "both files" goes |
| `docs/operations/overview.md` | Workers topology: one `rafi-worker` node; Restate Cloud's signed requests arrive at it |
| `docs/operations/upgrades.md` | Migrations paragraph and "Order of operations" (§2.7) |
| `docs/operations/troubleshooting.md` | The `script_name` hint goes |
| `docs/explanation/architecture.md` | "Who owns what" and the live-updates diagram: one Worker, two roles; the §2.9 sentence |
| `docs/reference/configuration.md` | The two "Workers" sections become one: vars, secrets, bindings, the `dev` feature row; the "must match" note goes |
| `docs/reference/crates.md` | `restate-worker` row goes; `worker` and `cloudflare` rows reworded; the diagram |
| `docs/development/getting-started.md` | `workers` profile: one process; the manual setup: one `wrangler dev`, the deployment URI with `/restate` |
| `docs/development/testing.md` | The Workers checks list (steps renumbered) |
| `README.md` | Component table and diagram: the engine's Workers host is `rafi-worker` |
| `docs/history/workers-plan.md` | A dated note at the top pointing here |

## 4. Milestones

| | What | Done when |
|---|---|---|
| **M0** | **Measure.** Build both Workers as they are and note the compressed sizes (`wrangler deploy --dry-run --outdir`); build the merged Worker from M1 and compare. Check `wrangler dev` startup and the first request's time | The merged Worker is well inside the size limit (3 MB compressed free, 10 MB paid) and starts as fast as the app does today. If not: §5 |
| **M1** | **The merged Worker.** `engine.rs`, the routing, the bundle, `Cargo.toml`, `wrangler.toml`, the `room` feature removed, `devenv.nix` | `devenv --profile workers up` runs one Worker; a test-mode raffle goes from open to finish with draws through the local Restate; §6's curl checks pass |
| **M2** | **Checks.** `workers-check.sh`, `evening.mjs`, CI | `./scripts/workers-check.sh` passes locally and in CI's `workers` job; `cargo clippy --target wasm32-unknown-unknown -p rafi-worker` with and without `--features dev` is clean |
| **M3** | **Removal and docs.** `apps/restate-worker` deleted; every row of §3's table | `rg restate-worker` finds only `docs/history`; the docs describe one Worker consistently |

M1 and M2 can land in one change; M3 can follow or ride along. Each step keeps `main` green.

## 5. Risks

- **Size and startup.** 847 KB and 914 KB compressed (workers-plan §11) share most of their dependencies
  (`rafi-store`, `rafi-cloudflare`, `worker`, `serde`); the sum is expected around 1.2–1.5 MB. If M0 shows the
  merged Worker near a limit or slow to start, the fallback is to keep two Workers and stop here. Nothing in M1
  is wasted then except the merge itself.
- **A `/restate/*` request reaching Assets or the SPA.** Guarded by `run_worker_first` and by routing first in
  `fetch`; §6 checks it.
- **One secret takes everything down.** Intended (§2.4); the first deploy's `/healthz` shows it.
- **The dev feature's reach.** `dev` now also allows empty identity keys on the Worker that serves users. It
  already allowed dev auth, and `[env.dev]` is the only place that builds with it.

## 6. Verification

Under `devenv --profile workers up` (or `scripts/workers-check.sh`'s step 5), against `http://127.0.0.1:8787`:

- `GET /restate/discover` with `accept: application/vnd.restate.endpointmanifest.v3+json` → the manifest listing
  `Raffle` and `EngineInfo` (the `dev` build accepts it unsigned; `curl`'s default `*/*` gets 415);
- `GET /restate/health` → 200 from the SDK;
- `POST /restate/invoke/Nope/nope` → the SDK's error, never the SPA;
- `GET /restate/anything-else`, `GET /restate` → an SDK error (400), not `index.html` (Assets didn't answer);
- `GET /healthz`, `GET /` → as before;
- the Restate deployment is registered at `http://127.0.0.1:8787/restate` and a draw completes
  (`evening.mjs`);
- a release-style build (`worker-build --release`, no `dev`) with `RESTATE_IDENTITY_KEYS` unset → every request
  answers 500 `misconfigured`, and with a key set, an unsigned `GET /restate/discover` → 401.

## 7. Out of scope

- Mounting the endpoint in the native server (§2.9).
- Workflows, Queues or any other Cloudflare primitive for the engine: Restate stays the engine.
- Splitting by audience (engine-plan step G) remains deferred; nothing here makes it harder, since a later
  Worker would bind this one's DO by `script_name` exactly as the engine Worker did.
