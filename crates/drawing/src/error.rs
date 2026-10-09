//! How the drawing refuses something, and how every engine and caller
//! classifies that refusal.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DrawError {
    #[error("the raffle is not frozen")]
    NotFrozen,
    #[error("the raffle is already frozen with a different entrant list")]
    AlreadyFrozen,
    #[error("the raffle commits to drand chain {committed}, but the draw engine uses {engine}")]
    ChainMismatch { committed: String, engine: String },
    #[error("invalid commit document: {0}")]
    InvalidCommit(String),
    #[error("draws go through the draw step, not as a command")]
    NotACommand,
    #[error("the raffle is finished")]
    Finished,
    #[error("unknown prize unit {0}")]
    UnknownUnit(String),
    #[error("prize unit {0} is not available for drawing")]
    UnitNotPending(String),
    #[error("prize unit {0} has no undecided winner")]
    UnitNotDrawn(String),
    #[error("another draw is in progress")]
    DrawInProgress,
    #[error("no draw is in progress")]
    NoDrawInProgress,
    #[error("there are no eligible entrants left")]
    NoEligibleEntrants,
    #[error("the beacon is for round {got}, but the draw committed to round {expected}")]
    WrongRound { expected: u64, got: u64 },
    #[error("the beacon is from another drand chain")]
    WrongChain,
    #[error("invalid list hash")]
    BadListHash,
    #[error("invalid resolution outcome")]
    BadOutcome,
}

impl DrawError {
    /// How engines and their callers classify this refusal.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::NotFrozen | Self::UnknownUnit(_) => ErrorKind::NotFound,
            Self::InvalidCommit(_) | Self::NotACommand | Self::BadOutcome => ErrorKind::BadRequest,
            Self::AlreadyFrozen
            | Self::ChainMismatch { .. }
            | Self::Finished
            | Self::UnitNotPending(_)
            | Self::UnitNotDrawn(_)
            | Self::DrawInProgress
            | Self::NoDrawInProgress
            | Self::NoEligibleEntrants => ErrorKind::Conflict,
            // A beacon for the wrong round or chain only ever stalls a draw,
            // and the list hash is the engine's own: these are engine bugs.
            Self::WrongRound { .. } | Self::WrongChain | Self::BadListHash => ErrorKind::Invariant,
        }
    }
}

/// How a refused command is classified, in every engine and at every hop
/// (handler status code, client error, app response).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The raffle or prize unit doesn't exist (or the raffle isn't frozen).
    NotFound,
    /// The command itself is malformed.
    BadRequest,
    /// The command doesn't fit the raffle's current state.
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
