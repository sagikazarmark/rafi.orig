//! The raffle engine's contract, between the raffle's rules
//! (`rafi_raffle`) and the engines that run them.
//!
//! An engine owns each raffle from opening to finish: it applies one command
//! at a time, applies each command once, does the I/O the rules can't (the
//! clock, the beacon, the projection, notifying clients) and, in production,
//! survives restarts. The app can't tell one engine from another: it sees
//! only this contract.
//!
//! - [`Engine`]: the port the app draws through, with its contract;
//! - [`Store`]: where an engine writes every new state (the projection);
//! - [`Observer`]: who it tells that a raffle changed (for rafi, the
//!   deployment's live notifier, `rafi_live_server::Notifier`).
//!
//! Whether an engine would take a command now is the raffle's own rule
//! ([`Raffle::admit`](rafi_raffle::Raffle::admit)), checked against the
//! projection.
//!
//! With the `drand` feature, [`drand`] is the beacon client every engine
//! draws with: Rafi's drand settings and the rule for which round a draw
//! commits to.
//!
//! The engines: `rafi-engine-restate` (production) and, with the `local`
//! feature, [`local::LocalEngine`] (tests and development only). With the
//! `conformance` feature, [`conformance`] is the scenario suite every engine
//! must pass.

mod engine;
mod observer;
mod store;

#[cfg(all(feature = "conformance", not(target_arch = "wasm32")))]
pub mod conformance;
#[cfg(feature = "drand")]
pub mod drand;
#[cfg(all(feature = "local", not(target_arch = "wasm32")))]
pub mod local;

pub use engine::{Engine, EngineError, EngineInfo, NOT_OPEN};
pub use observer::{Observer, RecordingObserver};
pub use store::{MemoryStore, Store};

use std::{future::Future, pin::Pin};

/// A boxed, sendable future, so the contract works with trait objects (axum
/// and the Restate SDK require `Send`); Rafi's other ports use it too.
/// Implementations whose futures aren't `Send` (Workers' bindings,
/// single-threaded) wrap them, for example with `worker::send::SendFuture`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
