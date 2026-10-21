//! Rafi on Cloudflare Workers: the Cloudflare glue the Worker (`apps/worker`)
//! and the live channel's testbed (`testbeds/cloudflare`) share.
//!
//! - [`RaffleRoom`]: one Durable Object per raffle, pushing the participant
//!   view to phones over a partyline Latest channel. Only the object builds
//!   the view: writers [`nudge`] it, and it reads the raffle back in one
//!   snapshot (`rafi_store::live::snapshot`), builds the view with
//!   `rafi_live_server::raffle::view` and publishes by
//!   `rafi_live_server::publish::decide`;
//! - [`connect`]: the Worker's gate for phones' sockets, and [`namespace`],
//!   the objects' namespace in the configured jurisdiction;
//! - [`DurableObjects`]: the `rafi_live_server::Notifier` over those objects
//!   (clients use partyline), and so the raffle engine's `Observer`;
//! - [`lookup`] and [`database`]: configuration from vars and secrets, and the
//!   D1 binding.
//!
//! The Worker's entry point only wires these up. A `#[durable_object]` in a
//! dependency is exported from every Worker that links it, so every Worker
//! linking this crate hosts the objects. Everything here exists on `wasm32`
//! only.

#![cfg(target_arch = "wasm32")]

mod config;
mod gate;
mod notifier;
mod object;

pub use config::{D1_BINDING, database, lookup};
pub use gate::{BINDING, CONNECT_LIMITER, JURISDICTION_VAR, connect, destroy, namespace, nudge};
pub use notifier::DurableObjects;
pub use object::RaffleRoom;
