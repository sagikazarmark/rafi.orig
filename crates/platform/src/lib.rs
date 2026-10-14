//! The interfaces a Rafi deployment provides.
//!
//! Rafi's code is written against these; each deployment plugs in its own
//! implementations (Cloudflare's live in `rafi-cloudflare`, the native ones
//! here and in `rafi-store`):
//!
//! - [`Notifier`]: "this raffle changed" and "end its live updates";
//! - [`RateLimit`], with the [`Limit`] keys;
//! - [`Notify`]: a [`Notifier`] as the raffle engine's `Observer`;
//! - [`sendable`]: a `Send` future from one that is only `Send` natively
//!   (wasm32 is single-threaded).
//!
//! The raffle engine and the store it writes to are `rafi-engine`'s.
//!
//! This crate depends on no database, no platform and no UI framework.

mod notifier;
mod notify;
mod ratelimit;
mod send;

use std::{future::Future, pin::Pin};

pub use notifier::{Notifier, Polling, Recording};
pub use notify::Notify;
#[cfg(not(target_arch = "wasm32"))]
pub use ratelimit::Limiter;
pub use ratelimit::{Limit, RateLimit};
pub use send::sendable;

/// A boxed, sendable future, so the interfaces work as trait objects (axum
/// and the Restate SDK require `Send`). Implementations whose futures aren't
/// `Send` (Workers' bindings, single-threaded) wrap them, for example with
/// [`sendable`] or `worker::send::SendFuture`. The same type as
/// `rafi_engine::BoxFuture`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
