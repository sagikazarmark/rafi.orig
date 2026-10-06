# restate-rafi

**The Rafi draw engine: the `Raffle` virtual object for [Restate](https://restate.dev/).**

One object per raffle (keyed by raffle ID) owns the draw state while the raffle is drawn. Handlers for one
raffle run one at a time, the committed drand round is journaled before the durable sleep until it is
published, and a crash or redeploy replays to the very same result. The object only ever sees anonymous
entrant IDs.

| Handler | Kind | |
|---|---|---|
| `freeze` | exclusive | Takes the frozen entrant list (the commit document). Idempotent for the same list. |
| `draw` | exclusive | Commits to a future drand round, sleeps until it is published, picks the winner. |
| `resolve` | exclusive | Claimed / not here / declined, void a unit, or finish (clears the state). |
| `get` | shared | The current state, for recovery and debugging. |

After every change the state goes to a `Projector`. `SqliteProjector` writes it into the Rafi app's SQLite
database through [`rafi-store`](../store), where the app picks it up; `RecordingProjector` keeps it in memory
for tests.

```rust
use rafi_drand::{DrandClient, DrandConfig};
use restate_rafi::{DatabaseConfig, Raffle, SqliteProjector};

let raffle = Raffle::new(
    DrandClient::new(DrandConfig::default()),
    SqliteProjector::new(&DatabaseConfig { url: "sqlite://rafi.db".into() })?,
);
let endpoint = restate_sdk::endpoint::Endpoint::builder().bind(raffle).build();
```

[`restate-rafi-endpoint`](../restate-rafi-endpoint) is the binary that serves it.

## Tests

```sh
cargo test -p restate-rafi
RESTATE_SERVER_BIN=$(./scripts/restate-server.sh) cargo test -p restate-rafi -- --ignored  # against a real server
```
