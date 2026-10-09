# restate-rafi

**The Rafi draw engine: the `Drawing` virtual object for [Restate](https://restate.dev/).**

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

`DrawingEngine` is a keyless service next to it: `info` reports the engine's beacon source.

Committing to a round and awaiting its beacon go through [`restate-drand`](https://crates.io/crates/restate-drand).
After every change the drawing goes to a `DrawStore` (the app's database), then clients are told through a
`Notifier`, best effort; both come from [`rafi-platform`](../platform).

```rust
use rafi_drand::{DrandClient, DrandConfig};
use rafi_platform::{MemoryDrawStore, Polling};
use restate_rafi::{Drawing, DrawingEngine};

let drand = DrandClient::new(DrandConfig::default())?;
let info = DrawingEngine::new(&drand);
let drawing = Drawing::new(drand, MemoryDrawStore::default(), Polling::default());
let endpoint = restate_sdk::endpoint::Endpoint::builder().bind(drawing).bind(info).build();
```

[`restate-rafi-endpoint`](../restate-rafi-endpoint) is the binary that serves it.

## Tests

```sh
cargo test -p restate-rafi
RESTATE_SERVER_BIN=$(./scripts/restate-server.sh) cargo test -p restate-rafi -- --ignored  # against a real server
```
