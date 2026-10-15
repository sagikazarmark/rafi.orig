//! Rafi on Cloudflare Workers: the Cloudflare glue the app's Worker and the
//! Restate Worker share.
//!
//! - `RaffleRoom` (the `room` feature): one Durable Object per raffle, pushing the
//!   participant view to phones over a partyline Latest channel. Only the
//!   object builds the view: writers [`nudge`] it, and it reads the raffle
//!   back in one snapshot (`rafi_store::live::snapshot`), builds the view with
//!   `rafi_live_server::raffle::view` and publishes by
//!   `rafi_live_server::publish::decide`;
//! - `connect` (the `room` feature): the Worker's gate for phones' sockets,
//!   and [`namespace`], the objects' namespace in the configured jurisdiction;
//! - [`DurableObjects`]: the `rafi_live_server::Notifier` over those objects
//!   (clients use partyline), and so the raffle engine's `Observer`;
//! - [`lookup`] and [`database`]: configuration from vars and secrets, and the
//!   D1 binding.
//!
//! The app's Worker entry point and the Restate Worker only wire these up.
//! Only the app's Worker hosts the objects, so only it turns on `room`.
//! Everything here exists on `wasm32` only.

#![cfg(target_arch = "wasm32")]

mod config;
mod gate;
mod notifier;
#[cfg(feature = "room")]
mod object;

pub use config::{D1_BINDING, database, lookup};
pub use gate::{BINDING, JURISDICTION_VAR, destroy, namespace, nudge};
#[cfg(feature = "room")]
pub use gate::{CONNECT_LIMITER, connect};
pub use notifier::DurableObjects;
#[cfg(feature = "room")]
pub use object::RaffleRoom;
