//! How the drawing refuses something, and how every engine and caller
//! classifies that refusal.

use crate::{beacon::BeaconSource, id::ItemId};

/// Why the drawing refused something.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("no drawing is open")]
    NotOpen,
    #[error("the drawing is already open with another manifest")]
    AlreadyOpen,
    #[error("the drawing draws from {committed}, but the draw engine from {engine}")]
    BeaconMismatch {
        committed: BeaconSource,
        engine: BeaconSource,
    },
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("the drawing is finished")]
    Finished,
    #[error("unknown item {0}")]
    UnknownItem(ItemId),
    #[error("item {0} is not available for drawing")]
    ItemNotPending(ItemId),
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
    WrongBeacon,
    #[error("invalid manifest hash")]
    BadManifestHash,
    #[error("invalid resolution outcome")]
    BadOutcome,
}

impl Error {
    /// How engines and their callers classify this refusal.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::NotOpen | Self::UnknownItem(_) => ErrorKind::NotFound,
            Self::InvalidManifest(_) | Self::BadOutcome => ErrorKind::BadRequest,
            Self::AlreadyOpen
            | Self::BeaconMismatch { .. }
            | Self::Finished
            | Self::ItemNotPending(_)
            | Self::ItemNotDrawn(_)
            | Self::DrawInProgress
            | Self::NoDrawInProgress
            | Self::NoEligibleEntrants => ErrorKind::Conflict,
            // A beacon for the wrong round or source only ever stalls a draw,
            // and the manifest hash is the engine's own: these are engine bugs.
            Self::WrongRound { .. } | Self::WrongBeacon | Self::BadManifestHash => {
                ErrorKind::Invariant
            }
        }
    }
}

/// How a refused command is classified, in every engine and at every hop
/// (handler status code, client error, app response).
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

impl ErrorKind {
    /// The HTTP status code an engine answers with.
    pub fn status(self) -> u16 {
        match self {
            Self::NotFound => 404,
            Self::BadRequest => 400,
            Self::Conflict => 409,
            Self::Invariant => 500,
        }
    }

    /// The kind an engine's status code stands for; `None` for anything an
    /// engine never refuses with (the engine is unavailable).
    pub fn from_status(status: u16) -> Option<Self> {
        match status {
            404 => Some(Self::NotFound),
            400 => Some(Self::BadRequest),
            409 => Some(Self::Conflict),
            500 => Some(Self::Invariant),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kinds_round_trip_through_status_codes() {
        for kind in [
            ErrorKind::NotFound,
            ErrorKind::BadRequest,
            ErrorKind::Conflict,
            ErrorKind::Invariant,
        ] {
            assert_eq!(ErrorKind::from_status(kind.status()), Some(kind));
        }
        assert_eq!(ErrorKind::from_status(502), None);
    }
}
