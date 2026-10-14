//! The `Notifier` over the raffles' Durable Objects.

use std::time::Duration;

use rafi_live_client::Follow;
use rafi_platform::{BoxFuture, Notifier};
use worker::{
    Delay, Env,
    send::{SendFuture, SendWrapper},
};

use crate::gate;

/// How often a nudge is tried before it is dropped. A lost nudge is repaired
/// by the object's periodic re-read (about 15 s).
const ATTEMPTS: u32 = 3;

/// Live updates through one [`RaffleRoom`](crate::RaffleRoom) per raffle:
/// clients subscribe over partyline.
pub struct DurableObjects(SendWrapper<Env>);

impl DurableObjects {
    /// Over this request's bindings ([`gate::BINDING`]).
    pub fn new(env: Env) -> Self {
        Self(SendWrapper(env))
    }

    /// Clients follow a raffle over partyline, at its object.
    pub fn follow(&self, _raffle_id: &str) -> Follow {
        Follow::Partyline { base_url: None }
    }
}

impl Notifier for DurableObjects {
    fn notify<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, ()> {
        // The bindings' futures aren't `Send`; a Worker is single-threaded, so
        // the wrapper is sound.
        Box::pin(SendFuture::new(async move {
            let mut delay = Duration::from_millis(100);
            for attempt in 1..=ATTEMPTS {
                match gate::nudge(&self.0, raffle_id).await {
                    Ok(_) => return,
                    // `tracing` has no output on Workers; log where `wrangler tail` sees it.
                    Err(e) if attempt == ATTEMPTS => {
                        worker::console_error!("rafi: nudging {raffle_id} failed, dropped: {e}");
                    }
                    Err(_) => {
                        Delay::from(delay).await;
                        delay *= 2;
                    }
                }
            }
        }))
    }

    fn end<'a>(&'a self, raffle_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(SendFuture::new(async move {
            gate::destroy(&self.0, raffle_id)
                .await
                .map_err(|e| anyhow::anyhow!("ending the live channel of {raffle_id}: {e}"))
        }))
    }
}
