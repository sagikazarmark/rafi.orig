//! The Rafi drawing engine as a Restate service.
//!
//! [`Drawing`] is a virtual object keyed by the drawing's ID.
//! It owns the drawing ([`rafi_drawing::Drawing`]) from opening to finish:
//! handlers for one drawing run one at a time, the committed drand round is journaled before the durable sleep,
//! and a crash or redeploy replays to the very same result. A beacon that
//! cannot be fetched leaves the draw pending on its round (never a redraw on a
//! new one), and every beacon is BLS-verified against the committed chain.
//!
//! The object only ever sees anonymous entrant IDs. After every change it
//! writes the state to a [`DrawStore`](rafi_platform::DrawStore) (the app's
//! database), then tells clients through a
//! [`Notifier`](rafi_platform::Notifier), best effort. Which database and which
//! live transport is up to the deployment: this crate knows neither.
//!
//! What the service does is independent of how it is configured or served;
//! the `restate-rafi-endpoint` crate (native) and `rafi-restate-worker`
//! (Cloudflare Workers) serve it.

//!
//! [`DrawingEngine`] is a keyless service next to it: `info` reports the
//! engine's beacon source, which the app checks before opening a drawing.

mod info;
mod service;

pub use info::{DrawingEngine, INFO_SERVICE_NAME, Info};
pub use service::{Drawing, SERVICE_NAME};
