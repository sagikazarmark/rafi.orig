//! Rafi on Cloudflare Workers: the Cloudflare glue every Rafi Worker shares
//! (the app's, `rafi-cloudflare-app`; the raffle engine's,
//! `rafi-cloudflare-engine`; the live Worker, `apps/worker-live`; the live
//! channel's testbed, `testbeds/cloudflare`).
//!
//! - [`RaffleRoom`] (feature `object`): one Durable Object per raffle,
//!   pushing the participant view to phones over a partyline Latest channel.
//!   Only the object builds the view: writers [`nudge`] it, and it reads the
//!   raffle back in one snapshot (`rafi_store::live::snapshot`), builds the
//!   view with `rafi_live_server::raffle::view` and publishes by
//!   `rafi_live_server::publish::decide`;
//! - [`connect`] (feature `connect`): the Worker's gate for phones' sockets,
//!   forwarding each to its raffle's object, and [`namespace`], the objects'
//!   namespace in the configured jurisdiction;
//! - [`DurableObjects`]: the `rafi_live_server::Notifier` over those objects
//!   (clients use partyline), and so the raffle engine's `Observer`. It
//!   reaches the objects through the `RAFFLE_ROOM` binding, which in a Worker
//!   not hosting the class names the hosting Worker (`script_name`);
//! - [`lookup`] and [`database`]: configuration from vars and secrets, and the
//!   D1 binding;
//! - [`is_restate_path`]: the paths the raffle engine's Restate endpoint is
//!   served under, which the app's Worker leaves alone;
//! - [`log`] (feature `log`): the `tracing` subscriber on the console.
//!
//! The Workers' entry points only wire these up. A `#[durable_object]` in a
//! dependency is exported from every Worker that links it, so the class is
//! behind its own feature, `object`, apart from the objects' clients: in
//! production only the live Worker turns it on, and the app's (`connect`)
//! and the engine's Workers reach the objects through their binding to it,
//! their bundles without a `RaffleRoom`. Everything here exists on `wasm32`
//! only.

#![cfg(target_arch = "wasm32")]

mod config;
mod gate;
#[cfg(feature = "log")]
pub mod log;
mod notifier;
#[cfg(feature = "object")]
mod object;

pub use config::{D1_BINDING, database, lookup};
#[cfg(feature = "connect")]
pub use gate::connect;
pub use gate::{
    BINDING, CONNECT_LIMITER, JURISDICTION_VAR, NO_JURISDICTION, destroy, namespace, nudge,
};
pub use notifier::DurableObjects;
#[cfg(feature = "object")]
pub use object::RaffleRoom;

/// The path prefix the raffle engine's Restate endpoint is served under.
/// Registered with Restate as `https://<host>/restate`; Restate appends
/// `/discover` and `/invoke/…`.
pub const RESTATE_PREFIX: &str = "/restate";

/// Whether a request path is the raffle engine's: [`RESTATE_PREFIX`] or
/// anything under it (not `/restatement`).
pub fn is_restate_path(path: &str) -> bool {
    path.strip_prefix(RESTATE_PREFIX)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}
