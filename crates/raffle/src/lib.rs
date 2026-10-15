//! The Rafi raffle: a verifiable raffle of a fixed set of items among a fixed
//! list of entrants, drawn item by item. The state machine, the draw
//! algorithm, the manifest and the verifier. A manifest and an engine run a
//! whole raffle; it knows nothing of the app (registration, names, settings).
//!
//! Everything in this crate is pure (no IO, no clocks, no randomness), so every
//! engine and every verifier computes identical results. It also builds for
//! WASM, so the verifier can run in a browser.

mod admit;
pub mod algorithm;
mod beacon;
mod command;
mod draw;
mod error;
mod hash;
mod id;
mod manifest;
mod raffle;
mod verify;

pub mod drand;

pub use beacon::BeaconSource;
pub use command::{BeaconFetch, Command, Decision, DrawRequest, Resolution};
pub use draw::{CompletedDraw, Outcome, PendingDraw};
pub use error::{Error, ErrorKind};
pub use hash::{Hash, InvalidHash, ManifestHash};
pub use id::{EntrantId, ItemId, RaffleId};
pub use manifest::{Manifest, ManifestItem};
pub use raffle::{Event, Item, ItemState, ItemStatus, Raffle};
pub use verify::{AbandonedDraw, DrawCheck, Report, verify, verify_public};
