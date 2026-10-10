//! The drawing engine's contract, between the drawing's rules
//! (`rafi_drawing`) and the engines that run them.
//!
//! An engine owns each drawing from opening to finish: it applies one command
//! at a time, applies each command once, does the I/O the rules can't (the
//! clock, the beacon, the projection, notifying clients) and, in production,
//! survives restarts. The app can't tell one engine from another: it sees
//! only this contract.
//!
//! - [`DrawingEngine`]: the port the app draws through, with its contract;
//! - [`DrawStore`]: where an engine writes every new state (the projection);
//! - [`admit`]: whether the engine would take a command now, checked against
//!   the projection.
//!
//! The engines: `rafi-engine-restate` (production) and, with the `local`
//! feature, [`local::LocalEngine`] (tests and development only). With the
//! `conformance` feature, [`conformance`] is the scenario suite every engine
//! must pass.

mod admit;
mod engine;
mod store;

#[cfg(all(feature = "conformance", not(target_arch = "wasm32")))]
pub mod conformance;
#[cfg(all(feature = "local", not(target_arch = "wasm32")))]
pub mod local;

pub use admit::admit;
pub use engine::{DrawingEngine, EngineError, EngineInfo, NOT_OPEN, refusal_kind, status_code};
pub use store::{DrawStore, MemoryDrawStore};
