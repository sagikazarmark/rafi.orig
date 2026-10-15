# rafi-engine-restate

**The Rafi raffle engine on [Restate](https://restate.dev/): the `Raffle` virtual object, and the app's client.**

One object per raffle (keyed by the raffle's ID) owns the raffle from opening to finish. Handlers for one
raffle run one at a time, the committed drand round is journaled before the durable sleep until it is
published, and a crash or redeploy replays to the very same result. The object only ever sees anonymous
entrant IDs. The rules are `rafi-raffle`'s; the object only does I/O.

| Handler | Kind | |
|---|---|---|
| `open` | exclusive | Opens the raffle from its manifest. Idempotent for the same manifest; refuses another manifest, and a beacon source other than the engine's. |
| `draw` | exclusive | Takes a `DrawRequest`: commits to a future drand round, sleeps until it is published, fetches and BLS-verifies it, picks the winner. If the beacon can't be had, the draw stalls on that round; drawing the item again retries the same round. |
| `decide` | exclusive | Takes a `Decision`: claimed / not here / declined, void an item, abandon a stalled draw, or finish (clears the state). |
| `get` | shared | The current raffle, for recovery and debugging. |

`EngineInfo` is a keyless service next to it: `info` reports the engine's beacon source.

The app reaches both through `RestateClient`, its `Engine` ([`rafi-engine`](../engine)) over the ingress. It
wraps the clients the SDK generates from the services (`RaffleIngressClient`, `EngineInfoIngressClient`), so the
handlers' names and types are the services' own; what it adds is the engine contract: a handler's terminal error is a
refusal with its kind, anything else means the engine is unavailable, and a call that outlasts the timeout is pending.

Committing to a round and awaiting its beacon go through [`restate-drand`](https://crates.io/crates/restate-drand).
After every change the raffle goes to a `Store` (the app's database), then, if the host gave one, clients are
told through an `Observer`, best effort, as a journaled step of its own; both are [`rafi-engine`](../engine)'s
ports. A host whose clients poll the store passes no observer, and no step is journaled (`restate-rafi`). On Workers,
the observer is the live notifier (a [`rafi-live-server`](../live-server) `Notifier`), which nudges the raffle's
room. `services` binds both services the way every host does.

```rust
use std::sync::Arc;

use rafi_engine::drand::{DrandClient, DrandConfig};
use rafi_engine::{MemoryStore, RecordingObserver};

let drand = DrandClient::new(DrandConfig::default())?;
// Clients that poll the store need no observer: pass None.
let observer = Arc::new(RecordingObserver::default());
let (raffle, info) = rafi_engine_restate::services(drand, MemoryStore::default(), Some(observer));
let endpoint = restate_sdk::endpoint::Endpoint::builder().bind(raffle).bind(info).build();
```

[`restate-rafi`](../../apps/restate-rafi) is the binary that serves the services;
[`rafi-restate-worker`](../../apps/restate-worker) serves them on Cloudflare Workers.

## Tests

```sh
cargo test -p rafi-engine-restate
RESTATE_SERVER_BIN=$(./scripts/restate-server.sh) cargo test -p rafi-engine-restate -- --ignored  # against a real server
```
