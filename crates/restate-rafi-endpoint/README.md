# restate-rafi-endpoint

**Standalone endpoint serving the Rafi drawing engine's Restate services ([`rafi-engine-restate`](../engine-restate)) for [Restate](https://restate.dev/).**

It writes the draw state into the Rafi app's SQLite database, so it runs next to the app with access to the
same file (the same machine, or a shared volume). The app owns the schema and creates the file.

## Install

Pre-built container images are available from GitHub Container Registry:

```bash
docker pull ghcr.io/sagikazarmark/restate-rafi:latest
```

Build from source (from the repository root):

```bash
cargo build --release -p restate-rafi-endpoint     # target/release/restate-rafi
docker build -f crates/restate-rafi-endpoint/Dockerfile .
```

## Quick Start

Run it on the app's database and register it with Restate:

```bash
RESTATE_RAFI_DATABASE__URL=sqlite://$PWD/rafi.db restate-rafi
restate deployments register http://localhost:9080
```

With Restate in Docker (the repository's `compose.yaml`), register `http://host.docker.internal:9080` instead.

## Configuration

Pass configuration through CLI arguments:

```text
--config <FILE>    Configuration file path (also CONFIG_FILE)
```

Configuration files may use JSON, YAML, or TOML. Every setting can also be set with a `RESTATE_RAFI_`
environment variable, `__` separating the levels (`RESTATE_RAFI_DATABASE__URL`); unknown settings are errors.

| Setting | Variable | Default | |
|---|---|---|---|
| `database.url` | `RESTATE_RAFI_DATABASE__URL` | `sqlite://rafi.db` | The app's database (its `RAFI_DATABASE_URL`): `sqlite://…` or `postgres://user:password@host/db` |
| `database.retention_nonwinners_days` | `RESTATE_RAFI_DATABASE__RETENTION_NONWINNERS_DAYS` | `30` | Days after a raffle finishes until non-winners are erased; dated when the finished state is stored. Must match the app's `RAFI_RETENTION_DAYS` |
| `database.retention_winners_days` | `RESTATE_RAFI_DATABASE__RETENTION_WINNERS_DAYS` | `90` | The same for winners (the app's `RAFI_RETENTION_WINNERS_DAYS`) |
| `endpoint.listener` | `RESTATE_RAFI_ENDPOINT__LISTENER` | `0.0.0.0:9080` | Listen address |
| `endpoint.identity_keys` | `RESTATE_RAFI_ENDPOINT__IDENTITY_KEYS` | | Restate's request signing keys (`publickeyv1_…`, comma separated) |
| `drand.relays` | `RESTATE_RAFI_DRAND__RELAYS` | api.drand.sh, api2, api3 (v2), drand.cloudflare.com (v1) | Relays to ask in order (a list: `[a, b]`; `v1:`/`v2:` prefixes, a bare URL is v1). Every beacon is BLS-verified |
| `drand.lead_secs` | `RESTATE_RAFI_DRAND__LEAD_SECS` | `6` | Seconds until the committed round (≥ 3) |
| `drand.timeout_secs` | `RESTATE_RAFI_DRAND__TIMEOUT_SECS` | `60` | Stop fetching after this; the draw stays pending (stalled) on its round |
| `drand.mock` | `RESTATE_RAFI_DRAND__MOCK` | `false` | Public test chain for offline development (draws are *not* verifiable; build with `--features mock-drand`). Must match the app's `RAFI_DRAND_CHAIN`: the app refuses to start a draw on an engine on another chain, and the engine refuses raffles committed to another chain |
| | `RUST_LOG` | `info` | Log level |

```toml
[database]
url = "sqlite:///data/rafi.db"

[endpoint]
identity_keys = ["publickeyv1_..."]

[drand]
lead_secs = 6

# Restate policy for the Drawing object: timeouts, retention, retry policy, per-handler overrides.
[services.drawing]
inactivity_timeout = "5m"
```
