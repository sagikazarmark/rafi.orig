//! Rafi on Cloudflare Workers: all of the Cloudflare glue, behind the
//! interfaces in `rafi-platform`.
//!
//! - [`RaffleChannel`]: one Durable Object per raffle, pushing the
//!   participant view to phones over a partyline Latest channel. Only the
//!   object builds the view (`Snapshot::participant_view`): writers [`nudge`] it, and it reads
//!   the raffle back in one snapshot (`rafi_store::live::Snapshot`) and
//!   publishes by `rafi_live::publish::decide`;
//! - [`connect`]: the Worker's gate for phones' sockets, and [`namespace`],
//!   the objects' namespace in the configured jurisdiction;
//! - [`DurableObjects`]: the `Notifier` over those objects (clients use
//!   partyline);
//! - [`RateLimitBindings`]: the `RateLimit` over Workers' rate-limiting
//!   bindings;
//! - [`lookup`] and [`database`]: configuration from vars and secrets, and the
//!   D1 binding.
//!
//! The app's Worker entry point and the Restate Worker only wire these up.
//! Everything here exists on `wasm32` only.

#![cfg(target_arch = "wasm32")]

mod config;
mod gate;
mod notifier;
mod object;
mod ratelimit;

pub use config::{D1_BINDING, database, lookup};
pub use gate::{BINDING, CONNECT_LIMITER, JURISDICTION_VAR, connect, destroy, namespace, nudge};
pub use notifier::DurableObjects;
pub use object::RaffleChannel;
pub use ratelimit::RateLimitBindings;
