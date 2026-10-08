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
//! hands the state to a [`Projector`]; in production [`SqliteProjector`] writes
//! it into the Rafi app's SQLite database, and the app pushes it to the screens.
//!
//! What the service does is independent of how it is configured or served;
//! the `restate-rafi-endpoint` crate is the binary that serves it.

mod projector;
mod service;

#[cfg(not(target_arch = "wasm32"))]
pub use projector::SqliteProjector;
pub use projector::{BoxFuture, DatabaseConfig, Projector, RecordingProjector};
pub use service::{Raffle, SERVICE_NAME};
