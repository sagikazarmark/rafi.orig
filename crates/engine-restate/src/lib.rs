//! The Rafi drawing engine on Restate.
//!
//! [`Drawing`] is a virtual object keyed by the drawing's ID.
//! It owns the drawing ([`rafi_raffle::Raffle`]) from opening to finish:
//! handlers for one drawing run one at a time, the committed drand round is journaled before the durable sleep,
//! and a crash or redeploy replays to the very same result. A beacon that
//! cannot be fetched leaves the draw pending on its round (never a redraw on a
//! new one), and every beacon is BLS-verified against the committed chain.
//!
//! The object only ever sees anonymous entrant IDs. After every change it
//! writes the state to a [`DrawStore`](rafi_engine::DrawStore) (the app's
//! database), then tells clients through a
//! [`DrawingNotifier`](rafi_engine::DrawingNotifier), best effort. Which
//! database and which live transport is up to the deployment: this crate
//! knows neither.
//!
//! [`EngineInfo`] is a keyless service next to it: `info` reports the
//! engine's beacon source, which the app checks before opening a drawing.
//!
//! What the services do is independent of how they are configured or served;
//! the `restate-rafi-endpoint` crate (native) and `rafi-restate-worker`
//! (Cloudflare Workers) serve them, both built by [`services`]. The app reaches them through
//! [`RestateClient`], its [`DrawingEngine`](rafi_engine::DrawingEngine) over
//! the ingress.

mod client;
mod info;
mod service;
mod status;

pub use client::{ClientConfig, RestateClient};
pub use info::{EngineInfo, EngineInfoIngressClient, INFO_SERVICE_NAME};
pub use service::{Drawing, DrawingIngressClient, SERVICE_NAME};

use rafi_drand::DrandClient;
use rafi_engine::{DrawStore, DrawingNotifier};

/// The engine's services, as every host binds them: the [`Drawing`] object,
/// drawing with `drand`, writing to `store` and telling `notifier`; and
/// [`EngineInfo`], answering for the same drand chain.
pub fn services(
    drand: DrandClient,
    store: impl DrawStore,
    notifier: impl DrawingNotifier,
) -> (Drawing, EngineInfo) {
    let info = EngineInfo::new(&drand);
    (Drawing::new(drand, store, notifier), info)
}
