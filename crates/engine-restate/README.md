# rafi-engine-restate

**The Rafi drawing engine on [Restate](https://restate.dev/): the `Drawing` virtual object, and the app's client.**

One object per drawing (keyed by the drawing's ID) owns the drawing from opening to finish. Handlers for one
drawing run one at a time, the committed drand round is journaled before the durable sleep until it is
published, and a crash or redeploy replays to the very same result. The object only ever sees anonymous
entrant IDs. The rules are `rafi-drawing`'s; the object only does I/O.

| Handler | Kind | |
|---|---|---|
| `open` | exclusive | Opens the drawing from its manifest. Idempotent for the same manifest; refuses another manifest, and a beacon source other than the engine's. |
| `draw` | exclusive | Takes a `DrawRequest`: commits to a future drand round, sleeps until it is published, fetches and BLS-verifies it, picks the winner. If the beacon can't be had, the draw stalls on that round; drawing the item again retries the same round. |
| `decide` | exclusive | Takes a `Decision`: claimed / not here / declined, void an item, abandon a stalled draw, or finish (clears the state). |
| `get` | shared | The current drawing, for recovery and debugging. |

`EngineInfo` is a keyless service next to it: `info` reports the engine's beacon source.

The app reaches both through `RestateClient`, its `DrawingEngine` ([`rafi-engine`](../engine)) over the ingress. It
wraps the clients the SDK generates from the services (`DrawingIngressClient`, `EngineInfoIngressClient`), so the
handlers' names and types are the services' own; what it adds is the engine contract: a handler's terminal error is a
refusal with its kind, anything else means the engine is unavailable, and a call that outlasts the timeout is pending.

Committing to a round and awaiting its beacon go through [`restate-drand`](https://crates.io/crates/restate-drand).
After every change the drawing goes to a `DrawStore` (the app's database), then clients are told through a
`DrawingNotifier`, best effort; both are [`rafi-engine`](../engine)'s ports. `rafi_platform::Notify` adapts a
[`rafi-platform`](../platform) `Notifier`, with the host saying what each drawing's clients follow (for rafi, its
raffle: `rafi_raffle::drawing::raffle_id`). `services` binds both services the way every host does.

```rust
use rafi_drand::{DrandClient, DrandConfig};
use rafi_drawing::DrawingId;
use rafi_engine::MemoryDrawStore;
use rafi_platform::{Notify, Polling};

let drand = DrandClient::new(DrandConfig::default())?;
let notifier = Notify::new(Polling::default(), DrawingId::as_str);
let (drawing, info) = rafi_engine_restate::services(drand, MemoryDrawStore::default(), notifier);
let endpoint = restate_sdk::endpoint::Endpoint::builder().bind(drawing).bind(info).build();
```

[`restate-rafi-endpoint`](../restate-rafi-endpoint) is the binary that serves the services;
[`rafi-restate-worker`](../restate-worker) serves them on Cloudflare Workers.

## Tests

```sh
cargo test -p rafi-engine-restate
RESTATE_SERVER_BIN=$(./scripts/restate-server.sh) cargo test -p rafi-engine-restate -- --ignored  # against a real server
```
