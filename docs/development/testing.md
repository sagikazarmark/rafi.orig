# Testing

## Running the tests

Every check is a recipe of the [`justfile`](../../justfile), and CI ([`.github/workflows/ci.yaml`](../../.github/workflows/ci.yaml))
runs one job per recipe, in the devenv shell, so the two can't drift. `just` lists the recipes; `just ci` runs
everything CI runs except the Workers checks.

| Recipe | What it runs |
|---|---|
| `just fmt` | `cargo fmt --check` |
| `just clippy` | Clippy natively (`clippy-native`: the libraries, the engine and the app, with and without the development features) and on `wasm32-unknown-unknown` (`clippy-wasm`: the web app, the Workers crates and the Worker), warnings refused |
| `just test` | The rules, the app's use cases, drand, the store, the engine conformance suite (local runner, and the `Raffle` object on `restate-server`); then the app, with the in-process raffle engine and end to end with `rafi-restate` and `restate-server` |
| `just verify` | The independent verifier against the shared vectors (`tools/verify.py`) |
| `just doc` | `cargo doc` with warnings refused, so a broken link fails: natively with every feature, and the Workers crates on `wasm32` |
| `just audit` | `cargo audit`: `Cargo.lock` against the RustSec advisory database. Advisories are checked here alone; an accepted one is ignored, with its reason, in [`.cargo/audit.toml`](../../.cargo/audit.toml) |
| `just deny` | `cargo deny check bans licenses sources` ([`deny.toml`](../../deny.toml)) |
| `just workers` | [`./scripts/workers-check.sh`](#workers): everything under `wrangler dev`. Long, and it needs a Chromium, so `just ci` leaves it out |
| `just cloudflare-artifact`, `just cloudflare-release <version>` | Not checks: what a release ships (`just artifact` in `apps/worker`), and the release itself (`scripts/cloudflare-release.sh`: the tarball, its checksums, a dry deploy of that tarball through `deploy/cloudflare`), which the release workflow runs on a tag |

The end-to-end tests run the shell's `restate-server` (`RESTATE_SERVER_BIN`). Without it (outside the devenv shell,
set it to a `restate-server` binary), they skip locally; CI fails them instead.

The recipes are plain `cargo` commands, to run by hand when one is enough:

```sh
cargo test --workspace --exclude rafi-server --exclude rafi-web -- --include-ignored
cargo test -p rafi-server -- --include-ignored
python3 tools/verify.py --vectors testdata/draw-vectors.json
```

## What is tested where

| Layer | Tests |
|---|---|
| `rafi-raffle` | Unit, property and vector tests. They include a chi-square bias test of the selection, and properties that whatever `Raffle::admit` allows, the engine can apply. |
| `rafi-lifecycle` | The lifecycle's rules, including an exhaustive table for starting the draw (`start::next`) |
| `rafi-store` | The [storage conformance suite](#storage-conformance) on every backend |
| `rafi-engine`, `rafi-engine-restate` | The [engine conformance suite](#engine-conformance), on the local runner and on the `Raffle` object |
| `rafi-app` | Use-case tests ([`crates/app/tests/use_cases.rs`](../../crates/app/tests/use_cases.rs)) |
| `rafi-server` | HTTP tests, and an end-to-end test of the app, `rafi-restate` and `restate-server` on one database |
| `tools/verify.py` | The same vectors as the Rust code ([`testdata/draw-vectors.json`](../../testdata/draw-vectors.json), from real quicknet beacons) |
| Workers | [`./scripts/workers-check.sh`](#workers) |

## Time

The use cases, housekeeping and every row the store stamps go by the app's clock (`App::clock`, a
`rafi_store::Clock`), read once per use case; the store's domain functions take `now` from it. The use-case and HTTP
tests run on a fixed clock that starts at the real time and moves only when the test moves it
(`app.clock.advance(10 * DAY)`, `app.clock.set(raffle.erase_at + hold + 1)`), and the in-process engine writes by the
same clock. A test of the erasure dates lets time pass instead of dating its raffles back. What is real time stays
real time: drand's rounds, the screens' countdowns, the proof-of-work's expiry and the rate limits. The storage
conformance suite passes `now` to the domain functions directly.

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

## Engine conformance

`rafi_engine::conformance` is the contract every raffle engine must pass, through the `Engine` interface alone.

- **The fairness rules:** the round is committed before its beacon is asked for. A withheld or forged beacon stalls
  the draw on its round, and a retry fetches the same round. Abandons are logged. Nothing changes a pending draw's
  pool. Every log passes the verifier.
- **The contract:** idempotency, one command at a time per raffle, asynchronous draws, refusals with their kind, and
  forgetting a raffle once it is finished.

It deploys once per drand profile: instant, with a lead, a withheld relay, a forging relay, and a flaky store. It
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
- the participant page in headless Chromium (devenv's on Linux, or `CHROMIUM=…`), the fallback included
- a whole evening on the Worker (`apps/worker/evening.mjs`): the admin API, a phone joining in Chromium, draws
  through Restate (a local `restate-server` calling the engine in the Worker, under `/restate`), the live channel,
  the end; every tab (the phone, the projector, a phone without WebAssembly) listens for Content-Security-Policy
  violations, enforced or reported, and one fails the run. It runs under dev auth, so Clerk's part of the web
  app's policy is checked by hand (`--profile clerk`, signing in and out, the organization switcher), and by
  `csp_by_page` in `rafi-server`'s tests for what the header says
- the hourly housekeeping fired as the Cron Trigger fires it (`wrangler dev --test-scheduled`, `/__scheduled`),
  checked to have recorded its success, which is what the admin's staleness warning reads
- the production shape: `wrangler deploy --dry-run` of the top level of `wrangler.toml`, built without the `dev`
  feature, against a size budget (2.5 MiB gzipped; Workers allow 3 on the free plan, 10 paid), and the same
  Worker served with no secret at all answering 500 `misconfigured` to everything, so a deploy can't run open;
  by then it has read `legal/terms.md` and `legal/privacy.md` through the assets binding (the refusal is a
  secret's, not theirs), and served without `legal/` it refuses naming them, from the 404 Workers Assets
  answers, which only the real runtime can show
- `deploy/cloudflare`, what an installation deploys: its `wrangler.toml` agrees with the Worker's on the paths and
  the required secrets, and the release of the version it pins is rehearsed the way a tag releases it
  (`scripts/cloudflare-release.sh`): packaged, fetched by a copy of the template with its own `deploy.sh` each
  way it checks a download (the release's `SHA256SUMS` with the template's empty pin, `RAFI_SHA256` pinned to the
  tarball's hash, a wrong pin refused), and deployed (dry)

CI's `workers` job runs it. Testbeds are never deployed, and nothing deploys from CI: a deploy is the operator's,
from their copy of `deploy/cloudflare` ([Cloudflare](../operations/cloudflare.md)), and the dry runs above are
its gate. A tag `v<version>` runs `just cloudflare-release` and publishes the release with the tarball and its
checksums, then checks the tarball's digest as GitHub computed it on upload against the build's
(`.github/workflows/release.yaml`); `RAFI_VERSION` in `deploy/cloudflare/deploy.sh` must be that version first,
and `RAFI_SHA256` there stays empty (the template can't know the hash of a release built from it).
