//! How the drawing refuses something, and how every engine and caller
//! classifies that refusal.

use crate::{beacon::BeaconSource, id::ItemId};

/// Why the drawing refused something.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("the drawing is already open with another manifest")]
    AlreadyOpen,
    #[error("the drawing draws from {drawing}, but the engine from {engine}")]
    BeaconSourceMismatch {
        drawing: BeaconSource,
        engine: BeaconSource,
    },
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("the drawing is finished")]
    Finished,
    #[error("unknown item {0}")]
    UnknownItem(ItemId),
    #[error("item {0} is not available for drawing")]
    ItemNotAvailable(ItemId),
    #[error("item {0} has no undecided winner")]
    ItemNotDrawn(ItemId),
    #[error("another draw is in progress")]
    DrawInProgress,
    #[error("no draw is in progress")]
    NoDrawInProgress,
    #[error("there are no eligible entrants left")]
    NoEligibleEntrants,
    #[error("the beacon is for round {got}, but the draw committed to round {expected}")]
    WrongRound { expected: u64, got: u64 },
    #[error("the beacon is from another source than the drawing's")]
    WrongBeaconSource,
    #[error("draw {0} holds no drawn item")]
    UnknownDraw(u64),
}

impl Error {
    /// How engines and their callers classify this refusal.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::UnknownItem(_) => ErrorKind::NotFound,
            Self::InvalidManifest(_) => ErrorKind::BadRequest,
            Self::AlreadyOpen
            | Self::BeaconSourceMismatch { .. }
            | Self::Finished
            | Self::ItemNotAvailable(_)
            | Self::ItemNotDrawn(_)
            | Self::DrawInProgress
            | Self::NoDrawInProgress
            | Self::NoEligibleEntrants => ErrorKind::Conflict,
            // A beacon for the wrong round or source only ever stalls a draw:
            // these are engine bugs.
            Self::WrongRound { .. } | Self::WrongBeaconSource => ErrorKind::Invariant,
            // Only a log that is not the drawing's own names a missing draw.
            Self::UnknownDraw(_) => ErrorKind::Invariant,
        }
    }
}

/// How a refused command is classified, in every engine and at every hop:
/// callers map it onto their own errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The drawing or item doesn't exist (or no drawing is open).
    NotFound,
    /// The command itself is malformed.
    BadRequest,
    /// The command doesn't fit the drawing's current state.
    Conflict,
    /// A broken engine invariant: a bug, never the caller's fault.
    Invariant,
}
