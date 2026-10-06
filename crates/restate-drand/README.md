# restate-drand

**[drand](https://drand.love) randomness in [Restate](https://restate.dev) handlers: commit to a future
round, wait durably, fetch and verify the beacon.**

Keep a `Drand` in your service and pass the handler's context:

```rust
use restate_drand::{AwaitError, Drand};
use restate_sdk::prelude::*;

struct Lottery {
    drand: Drand, // Drand::new(drand_beacon::client::Client::builder(chains::quicknet()).build()?)
}

#[restate_sdk::service]
impl Lottery {
    #[handler]
    async fn draw(&self, ctx: Context<'_>) -> HandlerResult<String> {
        let commitment = self.drand.commit_round(&ctx).await?;
        // … publish the commitment before the round exists …
        match self.drand.await_beacon(&ctx, commitment.round).await {
            Ok(beacon) => Ok(hex::encode(beacon.randomness())),
            // Keep the commitment and await the same round later; never draw again on a new one.
            Err(e @ AwaitError::Unavailable { .. }) => Ok(format!("try again later: {e}")),
            Err(e) => Err(e.into_handler_error()),
        }
    }
}
```

- **`commit_round(&ctx)`** reads the clock as a journaled step and commits to the latest round due at
  `now + lead` (default: two periods). A replay commits to the same round.
- **`await_beacon(&ctx, round)`** sleeps durably until the round is due, then fetches it in a journaled step,
  retried for the fetch timeout (default 60 s). It verifies the journaled beacon against the chain with
  [`drand-beacon`](../drand-beacon).
- **Errors.**
  - `AwaitError::Unavailable` means no valid beacon arrived within the budget. The round exists, or will: keep
    the commitment and call `await_beacon` again for the same round. Redrawing on a new round would let a relay
    that dislikes a published round force a redraw.
  - `AwaitError::Terminal` is a cancellation or a failed step: propagate it with `into_handler_error()`.
    Don't use `?`: the SDK turns any `std::error::Error` into a *retryable* handler error.
- **Contexts.** It works with all five SDK contexts (`Context`, `ObjectContext`, `SharedObjectContext`,
  `WorkflowContext`, `SharedWorkflowContext`).
- **Journal steps.** They are named `drand: now` and `drand: beacon` (`CLOCK_STEP`, `FETCH_STEP`).

## Tests

End to end against a real `restate-server` (commit and await, the durable sleep, the fetch budget, cancellation):

```sh
RESTATE_SERVER_BIN=/path/to/restate-server cargo test -p restate-drand -- --ignored
```

MSRV 1.92 (from `restate-ext`). Publishing requires `drand-beacon` on crates.io first.

## License

MIT or Apache-2.0.
