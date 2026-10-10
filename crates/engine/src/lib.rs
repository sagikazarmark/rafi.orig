//! The raffle engine's contract, between the raffle's rules
//! (`rafi_raffle`) and the engines that run them.
//!
//! An engine owns each raffle from opening to finish: it applies one command
//! at a time, applies each command once, does the I/O the rules can't (the
//! clock, the beacon, the projection, notifying clients) and, in production,
//! survives restarts. The app can't tell one engine from another: it sees
//! only this contract.
//!
//! - [`RaffleEngine`]: the port the app draws through, with its contract;
//! - [`RaffleStore`]: where an engine writes every new state (the projection);
//! - [`RaffleNotifier`]: how it tells clients a raffle changed (a host maps
//!   raffles to what clients follow, `rafi_platform::Notify` for rafi).
//!
//! Whether an engine would take a command now is the raffle's own rule
//! ([`Raffle::admit`](rafi_raffle::Raffle::admit)), checked against the
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

pub use engine::{EngineError, EngineInfo, NOT_OPEN, RaffleEngine};
pub use notify::{RaffleNotifier, RecordingNotifier};
pub use store::{MemoryRaffleStore, RaffleStore};

use std::{future::Future, pin::Pin};

/// A boxed, sendable future, so the contract works with trait objects (axum
/// and the Restate SDK require `Send`). The same type as
/// `rafi_platform::BoxFuture`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
