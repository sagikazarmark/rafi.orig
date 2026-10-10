//! Why a use case didn't do what it was asked.

use std::fmt;

use rafi_engine::EngineError;
use rafi_lifecycle::{
    self as raffle, RaffleState,
    entrant::{FakesRefusal, NameError},
    settings::{self, SettingsError},
    start,
};
use rafi_raffle::{BeaconSource, Command, Decision, Error as RaffleError, ErrorKind};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No such raffle (for this organizer), prize, entrant or item.
    #[error("not found")]
    NotFound,
    /// The rules refuse it; nothing changed.
    #[error("{0}")]
    Refused(#[from] Refusal),
    /// The drawing engine is unreachable; retrying is safe.
    #[error("The drawing engine is not reachable right now. Try again in a moment.")]
    Unavailable,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<rafi_store::Error> for Error {
    fn from(e: rafi_store::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl From<RaffleError> for Error {
    fn from(e: RaffleError) -> Self {
        match e.kind() {
            ErrorKind::NotFound => Self::NotFound,
            ErrorKind::BadRequest | ErrorKind::Conflict => Self::Refused(Refusal::Drawing {
                command: None,
                error: e,
            }),
            ErrorKind::Invariant => Self::Internal(e.into()),
        }
    }
}

impl From<EngineError> for Error {
    /// What the organizer sees of a call the drawing engine didn't confirm.
    /// (A call it accepted but hasn't settled, [`EngineError::Pending`],
    /// isn't an error: callers treat it as done.)
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::Rejected { kind, message } => match kind {
                ErrorKind::NotFound => Self::NotFound,
                ErrorKind::BadRequest => Self::Refused(Refusal::Engine {
                    invalid: true,
                    message,
                }),
                ErrorKind::Conflict => Self::Refused(Refusal::Engine {
                    invalid: false,
                    message,
                }),
                ErrorKind::Invariant => {
                    Self::Internal(anyhow::anyhow!("drawing engine invariant: {message}"))
                }
            },
            EngineError::Pending => {
                Self::Internal(anyhow::anyhow!("a pending engine call is not an error"))
            }
            EngineError::Unavailable(e) => {
                tracing::warn!(error = %format!("{e:#}"), "drawing engine unavailable");
                Self::Unavailable
            }
        }
    }
}

/// Why the rules refuse a request. Its text is for the organizer (the admin
/// page is in English); [`invalid`](Self::invalid) tells a request that can
/// never succeed from one the raffle's state refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    // Settings and prizes.
    Title,
    Texts,
    TestModeLocked,
    PrizeName,
    PrizeQty,
    PrizesFrozen,
    // Raffles.
    DrawPending,
    JoinCodes,
    Lifecycle {
        command: raffle::Command,
        from: RaffleState,
    },
    /// The raffle changed between reading and writing it.
    Changed,
    // Starting and running the draw.
    NoEngine,
    BeaconMismatch {
        engine: BeaconSource,
        raffles: BeaconSource,
    },
    Start(start::Refusal),
    NotDrawing,
    NotOpened,
    /// The drawing's rules refuse `command` (`None`: whatever the engine was
    /// asked).
    Drawing {
        command: Option<Command>,
        error: RaffleError,
    },
    /// The engine refused, in its own words.
    Engine {
        invalid: bool,
        message: String,
    },
    // Entrants.
    EntrantsFrozen,
    Name(NameError),
    Email,
    EmailTaken,
    Fakes(FakesRefusal),
    Erased,
    NotDeliverable,
    NotPurgeable,
    NotOver,
}

impl Refusal {
    /// Whether the request itself is invalid (it can never succeed), rather
    /// than refused in the raffle's current state.
    pub fn invalid(&self) -> bool {
        match self {
            Self::Title
            | Self::Texts
            | Self::PrizeName
            | Self::PrizeQty
            | Self::Name(_)
            | Self::Email => true,
            Self::Engine { invalid, .. } => *invalid,
            Self::Drawing { error, .. } => error.kind() == ErrorKind::BadRequest,
            _ => false,
        }
    }
}

impl std::error::Error for Refusal {}

impl From<SettingsError> for Refusal {
    fn from(e: SettingsError) -> Self {
        match e {
            SettingsError::Title => Self::Title,
            SettingsError::TooLong => Self::Texts,
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Title => write!(
                f,
                "The title must be 1–{} characters.",
                settings::MAX_TITLE_CHARS
            ),
            Self::Texts => f.write_str("Description or rules are too long."),
            Self::TestModeLocked => f.write_str("Test mode cannot change after the draw started."),
            Self::PrizeName => write!(
                f,
                "The prize name must be 1–{} characters.",
                settings::MAX_TITLE_CHARS
            ),
            Self::PrizeQty => write!(
                f,
                "The quantity must be between 1 and {}.",
                settings::MAX_PRIZE_QTY
            ),
            Self::PrizesFrozen => {
                f.write_str("Prizes cannot change after the entrant list is frozen.")
            }
            Self::DrawPending => f.write_str("A draw is in progress."),
            Self::JoinCodes => f.write_str("Could not allocate a unique join code; please try again."),
            Self::Lifecycle { command, from } => {
                write!(f, "Cannot {command:?} a raffle that is {}.", from.as_str())
            }
            Self::Changed => f.write_str("The raffle changed in the meantime; reload and try again."),
            Self::NoEngine => f.write_str("Restate is not configured on this server."),
            Self::BeaconMismatch { engine, raffles } => write!(
                f,
                "The drawing engine draws from {engine}, but raffles here commit to {raffles}. Fix the configuration before starting the draw."
            ),
            Self::Start(start::Refusal::State(s)) => write!(
                f,
                "Cannot start the draw of a raffle that is {}.",
                s.as_str()
            ),
            Self::Start(start::Refusal::NoEntrants) => f.write_str("Nobody has entered yet."),
            Self::Start(start::Refusal::NoPrizes) => f.write_str("Add at least one prize first."),
            Self::NotDrawing => f.write_str("The draw has not started (or is already over)."),
            Self::NotOpened => f.write_str(
                "The drawing engine has not confirmed the frozen list yet; press “Start the draw” again.",
            ),
            Self::Drawing { command, error } => f.write_str(&drawing_refusal(command.as_ref(), error)),
            Self::Engine { message, .. } => f.write_str(message),
            Self::EntrantsFrozen => f.write_str("The entrant list is frozen."),
            Self::Name(NameError::Invalid) => f.write_str("Please enter a name (1–24 characters)."),
            Self::Name(NameError::Blocked) => f.write_str("Please choose a different name."),
            Self::Email => f.write_str("Invalid email address."),
            Self::EmailTaken => f.write_str("This email is already entered."),
            Self::Fakes(FakesRefusal::NotTestMode) => {
                f.write_str("Fake entrants are only allowed in test mode.")
            }
            Self::Fakes(FakesRefusal::Frozen) => f.write_str("The entrant list is frozen."),
            Self::Erased => f.write_str("This entrant's data has been erased."),
            Self::NotDeliverable => {
                f.write_str("Only a winner who claimed their prize can get it delivered.")
            }
            Self::NotPurgeable => f.write_str("Only finished raffles can be purged."),
            Self::NotOver => {
                f.write_str("Live updates can only end once the raffle has finished.")
            }
        }
    }
}

/// The organizer's words for a command the draw rules refuse.
fn drawing_refusal(command: Option<&Command>, e: &RaffleError) -> String {
    let message = match (command, e) {
        (_, RaffleError::Finished) => "The raffle is finished.",
        (Some(Command::Draw(_)), RaffleError::DrawInProgress) => "A draw is already in progress.",
        (Some(Command::Draw(_)), RaffleError::ItemNotAvailable(_)) => {
            "This prize has already been drawn or withdrawn."
        }
        (_, RaffleError::NoEligibleEntrants) => "There are no eligible entrants left.",
        (Some(Command::Decide(Decision::Resolve { .. })), RaffleError::ItemNotDrawn(_)) => {
            "This prize has no undecided winner."
        }
        (Some(Command::Decide(Decision::Resolve { .. })), RaffleError::DrawInProgress) => {
            "Wait for the current draw to finish: “not here” and “declined” could change who it can pick."
        }
        (Some(Command::Decide(Decision::Abandon { .. })), RaffleError::NoDrawInProgress) => {
            "This prize has no draw in progress."
        }
        (Some(Command::Decide(Decision::Abandon { .. })), RaffleError::DrawInProgress) => {
            "The draw is still waiting for its drand round; it can be abandoned if it stalls."
        }
        (Some(Command::Decide(Decision::Void { .. })), RaffleError::ItemNotAvailable(_)) => {
            "Only prizes that have not been won can be withdrawn."
        }
        (_, RaffleError::DrawInProgress) => "Wait for the current draw to finish.",
        _ => return e.to_string(),
    };
    message.to_owned()
}
