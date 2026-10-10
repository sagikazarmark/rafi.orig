//! The drawing engine's contract, between the drawing's rules
//! (`rafi_raffle`) and the engines that run them.
//!
//! An engine owns each drawing from opening to finish: it applies one command
//! at a time, applies each command once, does the I/O the rules can't (the
//! clock, the beacon, the projection, notifying clients) and, in production,
//! survives restarts. The app can't tell one engine from another: it sees
//! only this contract.
//!
//! - [`DrawingEngine`]: the port the app draws through, with its contract;
//! - [`DrawStore`]: where an engine writes every new state (the projection);
//! - [`DrawingNotifier`]: how it tells clients a drawing changed (a host maps
//!   drawings to what clients follow, `rafi_platform::Notify` for rafi).
//!
//! Whether an engine would take a command now is the drawing's own rule
//! ([`Drawing::admit`](rafi_raffle::Drawing::admit)), checked against the
//! projection.
//!
//! The engines: `rafi-engine-restate` (production) and, with the `local`
//! feature, [`local::LocalEngine`] (tests and development only). With the
//! `conformance` feature, [`conformance`] is the scenario suite every engine
//! must pass.

mod engine;
mod notify;
mod store;

#[cfg(all(feature = "conformance", not(target_arch = "wasm32")))]
pub mod conformance;
#[cfg(all(feature = "local", not(target_arch = "wasm32")))]
pub mod local;

pub use engine::{DrawingEngine, EngineError, EngineInfo, NOT_OPEN};
pub use notify::{DrawingNotifier, RecordingNotifier};
pub use store::{DrawStore, MemoryDrawStore};

use std::{future::Future, pin::Pin};

/// A boxed, sendable future, so the contract works with trait objects (axum
/// and the Restate SDK require `Send`). The same type as
/// `rafi_platform::BoxFuture`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
