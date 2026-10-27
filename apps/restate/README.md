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

Pass a configuration file with `--config <FILE>` (or `RAFI_RESTATE_CONFIG`), in JSON, YAML or TOML. Every setting can also be
set with a `RAFI_RESTATE_` environment variable, with `__` separating the levels (`RAFI_RESTATE_DATABASE__URL`).

Every setting is described in the
[configuration reference](../../docs/reference/configuration.md#the-raffle-engine-rafi-restate). For running it in
production, see [Self-hosting](../../docs/operations/self-hosting.md).
