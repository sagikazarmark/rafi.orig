# rafi-restate

**Standalone endpoint serving the Rafi raffle engine's Restate services ([`rafi-engine-restate`](../../crates/engine-restate)) for [Restate](https://restate.dev/).**

It writes the draw state into the Rafi app's database (SQLite or Postgres), so it needs access to the same
database as the app (for SQLite: the same file, on the same machine or a shared volume). The app owns the schema.

## Install

Build from source (from the repository root):

```bash
cargo build --release -p rafi-restate     # target/release/rafi-restate
```

## Quick Start

Run it on the app's database and register it with Restate:

```bash
RAFI_RESTATE_DATABASE__URL=sqlite://$PWD/rafi.db rafi-restate
# Restate's admin API (here a local restate-server's, on :9070):
curl -X POST localhost:9070/deployments -H 'content-type: application/json' \
  -d '{"uri": "http://localhost:9080"}'
```

## Configuration

Pass configuration through CLI arguments:

```text
--config <FILE>    Configuration file path (also CONFIG_FILE)
```

Configuration files may use JSON, YAML, or TOML. Every setting can also be set with a `RAFI_RESTATE_`
environment variable, `__` separating the levels (`RAFI_RESTATE_DATABASE__URL`); unknown settings are errors.

| Setting | Variable | Default | |
|---|---|---|---|
| `database.url` | `RAFI_RESTATE_DATABASE__URL` | `sqlite://rafi.db` | The app's database (its `RAFI_DATABASE_URL`): `sqlite://…` or `postgres://user:password@host/db` |
| `endpoint.listener` | `RAFI_RESTATE_ENDPOINT__LISTENER` | `0.0.0.0:9080` | Listen address |
| `endpoint.identity_keys` | `RAFI_RESTATE_ENDPOINT__IDENTITY_KEYS` | | Restate's request signing keys (`publickeyv1_…`, comma separated) |
| `drand.relays` | `RAFI_RESTATE_DRAND__RELAYS` | api.drand.sh, api2, api3 (v2), drand.cloudflare.com (v1) | Relays to ask in order (a list: `[a, b]`; `v1:`/`v2:` prefixes, a bare URL is v1). Every beacon is BLS-verified |
| `drand.lead_secs` | `RAFI_RESTATE_DRAND__LEAD_SECS` | `6` | Seconds until the committed round (≥ 3) |
| `drand.timeout_secs` | `RAFI_RESTATE_DRAND__TIMEOUT_SECS` | `60` | Stop fetching after this; the draw stays pending (stalled) on its round |
| `drand.mock` | `RAFI_RESTATE_DRAND__MOCK` | `false` | Public test chain for offline development (draws are *not* verifiable; build with `--features mock-drand`). Must match the app's `RAFI_DRAND_CHAIN`: the app refuses to start a draw on an engine on another chain, and the engine refuses raffles committed to another chain |
| | `RUST_LOG` | `info` | Log level |

```toml
[database]
url = "sqlite:///data/rafi.db"

[endpoint]
identity_keys = ["publickeyv1_..."]

[drand]
lead_secs = 6

# Restate policy for the Raffle object: timeouts, retention, retry policy, per-handler overrides.
[services.raffle]
inactivity_timeout = "5m"
```
