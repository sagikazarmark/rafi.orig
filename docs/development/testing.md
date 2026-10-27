# Testing

## Running the tests

In the devenv shell (the end-to-end tests run its `restate-server`, `RESTATE_SERVER_BIN`):

```sh
# the rules, the app's use cases, drand, the store, the engine conformance suite
# (local runner, and the Raffle object on restate-server)
cargo test --workspace --exclude rafi-server --exclude rafi-web -- --include-ignored

# the app (with the in-process raffle engine), and app + rafi-restate + restate-server
cargo test -p rafi-server -- --include-ignored

# the independent verifier against the shared vectors
python3 tools/verify.py --vectors testdata/draw-vectors.json
```

Without `RESTATE_SERVER_BIN` (outside the devenv shell, set it to a `restate-server` binary), the end-to-end tests
skip locally (CI fails them instead).

CI ([`.github/workflows/ci.yaml`](../../.github/workflows/ci.yaml)) also checks formatting, runs Clippy on native
and `wasm32-unknown-unknown` targets, with and without the development features, and runs `cargo-deny`.

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

`./scripts/workers-check.sh` runs every Workers check locally, under `wrangler dev`:

- `rafi-store`'s conformance suite on D1 (`testbeds/store-d1`)
- the live channel over WebSockets (`testbeds/cloudflare`)
- the participant page in headless Chromium (devenv's on Linux, or `CHROMIUM=…`), the fallback included
- a whole evening on the Worker (`apps/worker/evening.mjs`): the admin API, a phone joining in Chromium, draws
  through Restate (a local `restate-server` calling the engine in the Worker, under `/restate`), the live channel,
  the end
- the hourly housekeeping fired as the Cron Trigger fires it (`wrangler dev --test-scheduled`, `/__scheduled`),
  checked to have recorded its success, which is what the admin's staleness warning reads
- the production shape: `wrangler deploy --dry-run` of the top level of `wrangler.toml`, built without the `dev`
  feature, against a size budget (2.5 MiB gzipped; Workers allow 3 on the free plan, 10 paid), and the same
  Worker served with no secret at all answering 500 `misconfigured` to everything, so a deploy can't run open

CI's `workers` job runs it. Testbeds are never deployed, and nothing deploys from CI: a deploy is the operator's
`wrangler deploy -c wrangler.<site>.toml` ([Cloudflare](../operations/cloudflare.md)), and the dry run above is
its gate.
