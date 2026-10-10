//! Rafi drawing: a verifiable selection of winners for a fixed set of items
//! from a fixed list of entrants. The state machine, the draw algorithm, the
//! manifest and the verifier; it knows nothing of raffles.
//!
//! Everything in this crate is pure (no IO, no clocks, no randomness), so every
//! engine and every verifier computes identical results. It also builds for
//! WASM, so the verifier can run in a browser.

mod admit;
pub mod algorithm;
mod beacon;
mod command;
mod draw;
mod drawing;
mod error;
mod hash;
mod id;
mod manifest;
mod verify;

/// The drand types this crate's API is made of (a [`BeaconSource`] names a
/// drand chain; a draw completes with a verified beacon), and the public
/// chains the verifier knows.
pub mod drand {
    pub use drand::{Beacon, ChainHash, ChainInfo, VerifiedBeacon, Verifier, chains};
}

pub use beacon::BeaconSource;
pub use command::{BeaconFetch, Command, Decision, DrawRequest, Resolution};
pub use draw::{CompletedDraw, Outcome, PendingDraw};
pub use drawing::{Drawing, Event, Item, ItemState, ItemStatus};
pub use error::{Error, ErrorKind};
pub use hash::{Hash, InvalidHash, ManifestHash};
pub use id::{DrawingId, EntrantId, ItemId};
pub use manifest::{Manifest, ManifestItem};
pub use verify::{AbandonedDraw, DrawCheck, Report, verify, verify_public};
