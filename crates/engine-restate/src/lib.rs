//! The Rafi drawing engine on Restate.
//!
//! [`Drawing`] is a virtual object keyed by the drawing's ID.
//! It owns the drawing ([`rafi_drawing::Drawing`]) from opening to finish:
//! handlers for one drawing run one at a time, the committed drand round is journaled before the durable sleep,
//! and a crash or redeploy replays to the very same result. A beacon that
//! cannot be fetched leaves the draw pending on its round (never a redraw on a
//! new one), and every beacon is BLS-verified against the committed chain.
//!
//! The object only ever sees anonymous entrant IDs. After every change it
//! writes the state to a [`DrawStore`](rafi_engine::DrawStore) (the app's
//! database), then tells clients through a
//! [`Notifier`](rafi_platform::Notifier), best effort. Which database and which
//! live transport is up to the deployment: this crate knows neither.
//!
//! [`EngineInfo`] is a keyless service next to it: `info` reports the
//! engine's beacon source, which the app checks before opening a drawing.
//!
//! What the services do is independent of how they are configured or served;
//! the `restate-rafi-endpoint` crate (native) and `rafi-restate-worker`
//! (Cloudflare Workers) serve them. The app reaches them through
//! [`RestateClient`], its [`DrawingEngine`](rafi_engine::DrawingEngine) over
//! the ingress.

mod client;
mod info;
mod service;

pub use client::{ClientConfig, RestateClient};
pub use info::{EngineInfo, EngineInfoIngressClient, INFO_SERVICE_NAME, Info};
pub use service::{Drawing, DrawingIngressClient, SERVICE_NAME};
