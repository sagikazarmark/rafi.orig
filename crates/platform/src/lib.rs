//! The interfaces a Rafi deployment provides.
//!
//! Rafi's code is written against these; each deployment plugs in its own
//! implementations (Cloudflare's live in `rafi-cloudflare`, the native ones
//! here and in `rafi-store`):
//!
//! - [`Notifier`]: "this raffle changed", "end its live updates", and which
//!   [`LiveTransport`] clients use;
//! - [`RateLimit`], with the [`Limit`] keys;
//! - [`DrawingEngine`]: the draw engine, with its contract;
//! - [`DrawStore`]: where the draw engine writes drawings;
//! - [`rule::decide`]: when a live channel publishes a participant view;
//! - [`sendable`]: a `Send` future from one that is only `Send` natively
//!   (wasm32 is single-threaded).
//!
//! This crate depends on no database and no platform.

mod engine;
mod notifier;
mod ratelimit;
pub mod rule;
mod send;
mod store;

use std::{future::Future, pin::Pin};

pub use engine::{DrawingEngine, EngineError, EngineInfo, NOT_OPEN, refusal_kind, status_code};
pub use notifier::{Notifier, Polling, Recording};
pub use rafi_live::LiveTransport;
#[cfg(not(target_arch = "wasm32"))]
pub use ratelimit::Limiter;
pub use ratelimit::{Limit, RateLimit};
pub use send::sendable;
pub use store::{DrawStore, MemoryDrawStore};

/// A boxed, sendable future, so the interfaces work as trait objects (axum
/// and the Restate SDK require `Send`). Implementations whose futures aren't
/// `Send` (Workers' bindings, single-threaded) wrap them, for example with
/// [`sendable`] or `worker::send::SendFuture`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
