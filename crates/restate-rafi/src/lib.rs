//! The Rafi draw engine as a Restate service.
//!
//! [`Raffle`] is a virtual object keyed by raffle ID. It owns the draw state
//! ([`DrawState`]) while a raffle is being drawn: handlers for one raffle run one
//! at a time, the committed drand round is journaled before the durable sleep,
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

mod service;

pub use service::{Raffle, SERVICE_NAME};
