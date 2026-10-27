//! The Rafi raffle engine on Restate.
//!
//! [`Raffle`] is a virtual object keyed by the raffle's ID.
//! It owns the raffle ([`rafi_raffle::Raffle`]) from opening to finish:
//! handlers for one raffle run one at a time, the committed drand round is journaled before the durable sleep,
//! and a crash or redeploy replays to the very same result. A beacon that
//! cannot be fetched leaves the draw pending on its round (never a redraw on a
//! new one), and every beacon is BLS-verified against the committed chain.
//!
//! The object only ever sees anonymous entrant IDs. After every change it
//! writes the state to a [`Store`] (the app's database), then tells clients
//! through an [`Observer`], best effort. Which database, and how live updates
//! reach clients, is up to the deployment: this crate knows neither.
//!
//! [`EngineInfo`] is a keyless service next to it: `info` reports the
//! engine's beacon source, which the app checks before opening a raffle.
//!
//! What the services do is independent of how they are configured or served;
//! the `rafi-restate` crate (native) and `rafi-worker` (Cloudflare Workers,
//! under `/restate` of the app's Worker) serve them, both built by
//! [`services`]. The app reaches them through [`RestateClient`], its
//! [`Engine`](rafi_engine::Engine) over the ingress.

mod client;
mod info;
mod send;
mod service;
mod status;

pub use client::{ClientConfig, RestateClient};
pub use info::{EngineInfo, EngineInfoIngressClient, INFO_SERVICE_NAME};
pub use service::{Raffle, RaffleIngressClient, SERVICE_NAME};

use std::sync::Arc;

use rafi_engine::drand::DrandClient;
use rafi_engine::{Observer, Store};

/// The engine's services, as every host binds them: the [`Raffle`] object,
/// drawing with `drand`, writing to `store` and telling `observer` (`None`
/// where clients poll and nobody needs telling); and [`EngineInfo`],
/// answering for the same drand chain.
pub fn services(
    drand: DrandClient,
    store: impl Store,
    observer: Option<Arc<dyn Observer>>,
) -> (Raffle, EngineInfo) {
    let info = EngineInfo::new(&drand);
    (Raffle::new(drand, store, observer), info)
}
