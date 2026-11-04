# Testing

## Running the tests

Every check is a recipe of the [`justfile`](../../justfile), and CI ([`.github/workflows/ci.yaml`](../../.github/workflows/ci.yaml))
runs one job per recipe, in the devenv shell, so the two can't drift. `just` lists the recipes; `just ci` runs
everything CI runs except the Workers checks.

| Recipe | What it runs |
|---|---|
| `just fmt` | `cargo fmt --check` |
| `just clippy` | Clippy natively (`clippy-native`: the libraries, the engine, the app and the web app's tests, with and without the development features) and on `wasm32-unknown-unknown` (`clippy-wasm`: the web app, the Workers crates, the local combined Worker with and without `dev`, and production's app and engine Workers each on its own, with a check that the engine's links nothing of the app's), warnings refused |
| `just test` | The rules, the app's use cases, drand, the store, the engine conformance suite (local runner, and the `Raffle` object on `restate-server`); then the app, with the in-process raffle engine and end to end with `rafi-restate` and `restate-server`; then the web app's router, natively |
| `just verify` | The independent verifier against the shared vectors (`tools/verify.py`) |
| `just doc` | `cargo doc` with warnings refused, so a broken link fails: natively with every feature, and the Workers crates on `wasm32` |
| `just audit` | `cargo audit`: `Cargo.lock` against the RustSec advisory database. Advisories are checked here alone; an accepted one is ignored, with its reason, in [`.cargo/audit.toml`](../../.cargo/audit.toml) |
| `just deny` | `cargo deny check bans licenses sources` ([`deny.toml`](../../deny.toml)) |
| `just workers` | [`./scripts/workers-check.sh`](#workers): everything under `wrangler dev`. Long, and it needs a Chromium, so `just ci` leaves it out |
| `just cloudflare-artifact`, `just cloudflare-tarball <version>`, `just cloudflare-release <version>` | Not checks: what a release ships (the app, engine and live Workers, the D1 migrations and `deploy.sh`), packed as `worker.tar.gz`, and the release itself (`scripts/cloudflare-release.sh`: the fixture's `VERSION` naming the release, `deploy.sh`'s own checks, `worker.tar.gz`, and its `deploy.sh dry-run` from a copy of `testdata/cloudflare-installation`, also as a Wrangler environment), which the release workflow runs on a tag |

The end-to-end tests run the shell's `restate-server` (`RESTATE_SERVER_BIN`). Without it (outside the devenv shell,
set it to a `restate-server` binary), they skip locally, and `just test` ends with a `SKIPPED:` line counting them;
CI fails them instead.

The recipes are plain `cargo` commands, to run by hand when one is enough:

```sh
cargo test --workspace --exclude rafi-server --exclude rafi-web -- --include-ignored
cargo test -p rafi-server -- --include-ignored
cargo test -p rafi-web # needs the stylesheet: just css
python3 tools/verify.py --vectors testdata/draw-vectors.json
```

## What is tested where

| Layer | Tests |
|---|---|
| `rafi-raffle` | Unit, property and vector tests. They include a chi-square bias test of the selection, and properties that whatever `Raffle::admit` allows, the engine can apply. |
| `rafi-lifecycle` | The lifecycle's rules, including exhaustive tables for its transitions and for starting the draw (`start::next`), with registration and without |
| `rafi-store` | The [storage conformance suite](#storage-conformance) on every backend; the [query plans](#regression-guards) of its hot statements; the SQLite and Postgres migrations declaring the same schema (`tests/schemas.rs`) |
| `rafi-engine`, `rafi-engine-restate` | The [engine conformance suite](#engine-conformance), on the local runner and on the `Raffle` object |
| `rafi-app` | Use-case tests ([`crates/app/tests/use_cases.rs`](../../crates/app/tests/use_cases.rs)); the runbook's restore on SQLite files, with the SQL read from the runbook and the Cloudflare Workers page (`a_restore_puts_nobody_back`, [`crates/app/tests/restore.rs`](../../crates/app/tests/restore.rs)) |
| `rafi-server` | HTTP tests, and an end-to-end test of the app, `rafi-restate` and `restate-server` on one database. What the request log says is a test binary of its own (`tests/trace_log.rs`): its subscriber would otherwise see other tests' events |
| `rafi-web` | The router, server-rendered natively at a path (`dioxus-ssr`, a `MemoryHistory`): the admin refuses to render inside a frame. Only pages that touch no browser API on their first render can be rendered so; the rest are the evening's. The list tab's parser, column detection and preview, on export fixtures (`apps/web/fixtures/import`), and the tab's static state; a person's row in the People tab; parts of the evening that need no browser: the projector's intro (with registration and without, a list added or not) and "Come to the front!" at the reveal, the draw console's winner (the masked email, "added by you", the check to make) and its "Confirmed: it's them", the freeze's reminder, the verification page's source counts; the projector before its first view (loading, no raffle there, older than the server), its "Reconnecting…", its thank-you screen's verification link, and the admin's "Offline"; the server clock behind the countdowns; every theme colour pair's contrast (`theme_contrast`, reading `tailwind.css`) |
| `tools/verify.py` | The same vectors as the Rust code ([`testdata/draw-vectors.json`](../../testdata/draw-vectors.json), from real quicknet beacons), and the scenario tampered six ways (another winner, another index, another round than announced, the pool changed while a draw was pending, an unknown event type, a byte of the manifest), each of which both verifiers must fail |
| Workers | [`./scripts/workers-check.sh`](#workers) |

## Time

The use cases, housekeeping and every row the store stamps go by the app's clock (`App::clock`, a
`rafi_store::Clock`), read once per use case; the store's domain functions take `now` from it. The use-case and HTTP
tests run on a fixed clock that starts at the real time and moves only when the test moves it
(`app.clock.advance(10 * DAY)`, `app.clock.set(raffle.erase_at + hold + 1)`), and the in-process engine writes by the
same clock. A test of the erasure dates lets time pass instead of dating its raffles back. What is real time stays
real time: drand's rounds, the screens' countdowns, the proof-of-work's expiry and the rate limits. The storage
conformance suite passes `now` to the domain functions directly.

## Interleavings

A use case reads, decides and writes in several steps, and the store has no interactive transactions, so a second
organizer can act between two of them. The races that matter have a test that runs the second actor *in* that gap,
not two use cases one after the other. Both tools are in
[`crates/app/tests/use_cases.rs`](../../crates/app/tests/use_cases.rs):

- **A paused database** (`paused_at`): `Db::with_hook` (rafi-store's `hooks` feature, on for rafi-app's tests only)
  gives a handle that awaits a hook before each statement it sends, with its SQL. `paused_at(&app, |sql| …)` returns
  a copy of the app on such a handle, paused before the first statement the predicate matches, and a `Pause`: the
  test spawns the use case on the paused copy, waits for `pause.reached()`, acts on `app` (the same database, no
  hook), then `pause.resume()`. Example: a prize saved between the freeze's checks and its first write
  (`a_prize_saved_while_freezing_is_in_the_manifest`).
- **A paused engine** (`Gated`, `paused_before_finish`): the in-process engine behind a wrapper that awaits a gate
  before each decision it passes on, for a gap between a use case's reads and its call to the engine. Example: a
  draw announced while a raffle is being deleted (`a_draw_announced_while_deleting_keeps_the_raffle`). Stub engines
  in the same file refuse what the test needs refused (`RefusesFinish`).

What two consoles do over HTTP with a stale page (`a_stale_console_cannot_resolve_the_redraw` in
`crates/server/src/tests/draws.rs`) and what the store must hold whichever order two writes land in
(`domain.objected_winner`, `domain.delete_guard`, `domain.add_retry_after_undo`, `domain.delivery_snapshot`) are
tested where those layers are.

## Storage conformance

`rafi_store::conformance` is the contract every database backend must pass. It has named scenarios for:

- the query layer (`query.*`): decoding, `NULL` parameters, batch atomicity and snapshots, unique violations, change
  counts, ordering, D1's parameter limit, the version triggers
- the domain functions (`domain.*`)

`cargo test -p rafi-store` lists each scenario per backend, as `<backend>::<scenario>` (for example
`sqlite::domain.join_cap`). A backend that isn't available, or a capability a scenario needs that the backend lacks,
is reported as ignored, with the reason.

| Backend | Runs |
|---|---|
| SQLite (in memory) | Always |
| Embedded Postgres | By default: a local PostgreSQL 18 from prebuilt binaries ([Oliphaunt](https://oliphaunt.dev)). One server per run, started on first use (about 3 s), with a fresh schema per scenario. No install needed. The first build downloads about 13 MB. `RAFI_TEST_EMBEDDED_POSTGRES=off` skips it. |
| Postgres | When `RAFI_TEST_POSTGRES_URL` names a throwaway database (a fresh schema per scenario). CI sets it. |
| D1 | Under `wrangler dev`, through `testbeds/store-d1` ([`./scripts/workers-check.sh`](#workers)) |

A new backend implements `rafi_store::conformance::Harness` (a name, its capabilities, and a fresh, migrated
database per scenario) and passes the suite. See the module's documentation.

## Regression guards

Two checks fail what would otherwise only get slower or bigger:

- **The web app's gzip budget**: `just check` in `apps/web` builds the bundle and fails when the module and its glue
  (`rafi-web_bg-*.wasm` and `rafi-web-*.js`) are over 1,100,000 bytes `gzip -9`, or `tailwind.css` over 40 KB. The
  `web` recipe runs it, so `just clippy` (its `clippy-wasm` half, a CI job) and `just doc` do. The participant page
  has its own, 280 KB, the same way (`just check` in `apps/participant`, the `participant` recipe).
- **The query plans** (`query_plans` in `crates/store/src`, a test of `cargo test -p rafi-store`, so `just test`):
  the hot use cases (the screens', the admin's and phones' reads, the join and its lookups, the engine's write,
  housekeeping's sweeps) run on an in-memory SQLite, and every statement they sent is put through `EXPLAIN QUERY
  PLAN`. A `SCAN` of `entrants`, `raffles` or `raffle_events`, through an index or not, fails it, and so does a
  `SEARCH` of one without a constraint (a whole index walked); a partial index whose rows are what is left to do
  (`entrants_unerased`) may be walked. A new hot statement goes in the test's list.

## Engine conformance

`rafi_engine::conformance` is the contract every raffle engine must pass, through the `Engine` interface alone.

- **The fairness rules:** the round is committed before its beacon is asked for. A withheld or forged beacon stalls
  the draw on its round, and a retry fetches the same round. Abandons are logged. Nothing changes a pending draw's
  pool. Every log passes the verifier.
- **The contract:** idempotency, one command at a time per raffle, asynchronous draws, refusals with their kind, and
  forgetting a raffle once it is finished.

It deploys once per drand profile: instant, with a lead, a withheld relay, a forging relay, a flaky store, and two
over HTTP from a local relay signing with the mock chain's key (`DrandClientBuilder::mock_over_http`): past a relay
that never answers, the draw completes from the next; from a relay failing a round for a while, the draw stalls and
a retry draws the same round. It
runs on the local runner (`cargo test -p rafi-engine`) and on the `Raffle` object through the app's client
(`cargo test -p rafi-engine-restate -- --include-ignored`, one `restate-server` per profile).

## Postgres

With `RAFI_TEST_POSTGRES_URL` set, the app's tests (the end-to-end one included) also run against Postgres, each on
a fresh schema, which they leave behind. Locally, for example:

```sh
D=$(mktemp -d); initdb -D $D/pg -U rafi --auth=trust >/dev/null
pg_ctl -D $D/pg -o "-p 54329 -k ''" -l $D/pg.log start && createdb -h 127.0.0.1 -p 54329 -U rafi rafi_test
export RAFI_TEST_POSTGRES_URL=postgres://rafi@127.0.0.1:54329/rafi_test
# afterwards: pg_ctl -D $D/pg stop && rm -rf $D
```

## Workers

`just workers` (`./scripts/workers-check.sh`) runs every Workers check locally, under `wrangler dev`:

- `rafi-store`'s conformance suite on D1 (`testbeds/store-d1`)
- the live channel over WebSockets (`testbeds/cloudflare`)
- the participant page in headless Chromium (devenv's on Linux, or `CHROMIUM=…`), the fallback included, a
  module that arrives after the fallback started leaving the page to it, and a phone the live channel refuses
  (4403) polling instead
- a whole evening on the Worker (`apps/worker/evening.mjs`): the admin API, a phone joining in Chromium, draws
  through Restate (a local `restate-server` calling the engine in the Worker, under `/restate`), drawn, claimed and
  finished by trusted clicks in the draw console (only the first prize's Draw primary; a double click on Draw
  drawing once; a winner the organizer added marked Not here, confirmed, and the prize drawn again from its
  re-enabled Draw), the projector's and the
  console's countdowns running between fetches (with a drand lead of 8 s), the live channel, the end, and the
  raffle's own `verify.json` through `tools/verify.py`, passing with `--test-chain` and failing without (the evening
  draws from the mock chain);
  a projector cut off from `/view` saying "Reconnecting…" and recovering, and one at an unknown address saying
  there's no raffle there; the verification page's counts by source; a 64-character name the organizer added from a list, in a
  raffle of its own, the projector telling registered people they're in and calling the winner to the front,
  fitting a projector-sized screen without scrolling sideways in the recent names, at the reveal and on the thanks screen; the
  list import clicked through in the Add people dialog, in a raffle of its own (a fixture pasted with
  `Input.insertText` and added, the add undone, a fixture uploaded with `DOM.setFileInputFiles`, and an add whose
  reply is lost added again with the same batch ID, adding nobody twice); the security headers of
  each kind of page and of a static file; and every tab (the phone, the
  projector, the admin, the verification page, a phone without WebAssembly) listens for Content-Security-Policy
  violations, enforced or reported, one of which fails the run. The admin is also reloaded, revalidating its page
  (a 304), and must still refuse an inline script. It runs under dev auth, so Clerk's part of the web app's policy
  is checked by hand (`--profile clerk`, signing in and out, the organization switcher), and by `rafi-server`'s
  tests for what the header says (`the_web_app_allows_eval_inline_styles_and_clerk`)
- the hourly housekeeping fired as the Cron Trigger fires it (`wrangler dev --test-scheduled`, `/__scheduled`),
  checked to have recorded its success, which is what the admin's staleness warning reads; then again with the
  evening's raffles dated past their erasure date and the winner hold: everyone erased, every raffle purged and its
  live channel ended, a socket to one refused
- the production shape: `wrangler deploy --dry-run` of the top level of `wrangler.toml`, built without the `dev`
  feature, against a size budget (2.5 MiB gzipped; Workers allow 3 on the free plan, 10 paid), and the same
  Worker served with no secret at all answering 500 `misconfigured` to everything, so a deploy can't run open;
  then served with stand-in secrets: without `legal/` (`just assets` copies none) it answers, and `/terms` and
  `/privacy` say the operator hasn't provided them, from the 404 Workers Assets answers, which only the real
  runtime can show; with the check's own `legal/terms.md` and `legal/privacy.md` (removed after) the pages show
  them with their versions, read through the assets binding; with a `terms.md` without its front matter it
  refuses naming the file (`misconfigured: legal/terms.md: no front matter`), in the answer too
- `testdata/cloudflare-installation`, what an installation deploys (the deploy template's configuration, as a
  test fixture): its `wrangler.app.toml`, `wrangler.engine.toml` and `wrangler.live.toml` agree with the production
  Workers' (`apps/worker-app`, `apps/worker-engine`, `apps/worker-live`) on the bindings, limits, Durable Object
  migrations, required secrets and addresses (the app on workers.dev, no routes: the default), and a release (of
  the version its `VERSION` names) is rehearsed the way a tag releases it (`scripts/cloudflare-release.sh`): first
  that the fixture's `VERSION` names it, that its `wrangler.app.toml` puts the app on workers.dev by default
  (`workers_dev = true`, the custom domain's `routes` commented out) and that `release.yaml` attests the tarball;
  then the deploy
  script's own checks
  (`scripts/cloudflare-deploy-test.sh`: each subcommand run from a copy of the fixture with a release laid out in
  `release/`, against a stubbed wrangler and Restate CLI and a fake Cloudflare and Restate admin API, a local node
  server; the wrangler stub's dry runs write a `deploy` entry with the Worker's name into its ND-JSON output, as
  wrangler does: `dry-run` (`legal/`'s documents copied in, the tests' own: both, the terms alone with what a run
  before copied removed, and none at all; the release's `VERSION` the one deployed, the installation's version files not read and
  the template's placeholders not looked for, `check` gone, the names read from the dry runs whatever the files
  say, a dry run failing or naming no Worker, the engine Worker's versioned name, `jq` missing, the tools' defaults
  `npx -y wrangler` and `npx -y @restatedev/restate` and their overrides, and Wrangler environments: none, one
  without a name of its own, one with, one wrangler refuses), `migrate`, `deploy-live` skipping an unchanged
  fingerprint (an edit of `wrangler.live.toml`, a comment too, changes it), `upload-app`'s first install and upgrade,
  `deploy-engine`'s name and address, `register-engine`'s downgrade guard, metadata (`instance` the app Worker's
  name), `"force": false` and retries, `check-remote`'s same guard before anything is deployed (a first install,
  the same or a newer release passing, an older refused, an unreachable Restate or a bad key or token failing,
  nothing but reads), `promote-app`, the outputs (`RAFI_OUTPUT_FILE`) and reports (`RAFI_REPORT_FILE`), and
  `cleanup`'s filtering by metadata, its order (each drained deployment removed from Restate before its Worker is
  deleted; a deletion failing leaves the Worker behind and fails the run, a removal refused leaves the Worker
  alone), the leftover engine Workers it reports and never deletes (those none of this installation's deployments
  refers to: a Worker only another installation's deployment refers to is reported), and stopping before anything
  is removed or deleted when a listing fails, and the artifacts (`RAFI_ARTIFACTS_DIR`: wrangler's ND-JSON,
  Restate's listings, the registration's answer, the Workers listing and `cleanup.json`'s lists, with no token among
  them); the fake server is stopped on exit and on a signal); then `worker.tar.gz`, its layout
  and each Worker's exports checked, unpacked into a copy of the template's `release/`, whose `release/deploy.sh
  dry-run` (with the devenv's wrangler through `WRANGLER`, nothing installed, nothing asked of the network) copies
  no legal documents (the fixture has none) and dry-deploys the live Worker `rafi-live` (`DB`, `RAFFLE_ROOM` its own), the engine Worker
  `rafi-engine`, this release's `rafi-engine-v<version>` (`DB`, `RAFFLE_ROOM` defined in `rafi-live`, the identity
  keys) and the app Worker `rafi-app` (`ASSETS`, `DB`, `RAFFLE_ROOM` defined in `rafi-live`, the four rate
  limiters; on workers.dev, the template's default), the names read from the dry runs' ND-JSON and no
  `RAFI_DO_JURISDICTION` (the template's default);
  then the same for a copy with an `[env.eu]` in each configuration (with `RAFI_DO_JURISDICTION = "eu"`, and the app
  on a custom domain, `workers_dev = false` and its route, which wrangler's dry run validates) and a
  `VERSION.eu` (and the top level's `VERSION` another release, which `deploy.sh` doesn't read), with
  `CLOUDFLARE_ENV=eu`: `rafi-live-eu`, `rafi-engine-eu-v<version>` and `rafi-app-eu`, bound to
  `rafi-live-eu`, each with the jurisdiction. What `deploy.sh` does against the real
  Cloudflare and Restate Cloud (the API's answers, wrangler's uploads and deploys, the registration, the removals
  and deletions, the Workers listing) needs an account and is not run here

Each `wrangler dev` gets a fixed port (8789 to 8792, 8787 for the evening), and the script refuses one something
already listens on: a developer's own `wrangler dev`, or one left over from an earlier run, would answer in its
place. The evening's moves with `WORKERS_APP_PORT` (`WORKERS_APP_PORT=8797 just workers`) when 8787 is the
development server's. CI's `workers` job runs it. Testbeds are never deployed, and nothing deploys from this repository's CI: a deploy is
an installation's, its repository's workflows running a release's `deploy.sh` ([Cloudflare Workers](../operations/cloudflare.md)),
and the dry runs above are its gate. The workflows themselves are checked with actionlint (`nix run
nixpkgs#actionlint -- .github/workflows/*.y*ml`); installations' workflows are the deploy template's, and what they
do on GitHub (the download, the attestation, the secrets) runs only there. A tag `v<version>` runs `just cloudflare-release` and publishes the release with `worker.tar.gz`, then
checks the tarball's digest as GitHub computed it on upload against the build's (`.github/workflows/release.yaml`).
