//! Why a use case didn't do what it was asked.

use std::fmt;

use rafi_engine::EngineError;
use rafi_lifecycle::{
    self as lifecycle, RaffleState,
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
    /// The raffle engine is unreachable; retrying is safe.
    #[error("The raffle engine is not reachable right now. Try again in a moment.")]
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
            ErrorKind::BadRequest | ErrorKind::Conflict => Self::Refused(Refusal::Raffle {
                command: None,
                error: e,
            }),
            ErrorKind::Invariant => Self::Internal(e.into()),
        }
    }
}

impl From<EngineError> for Error {
    /// What the organizer sees of a call the raffle engine didn't confirm.
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
                    Self::Internal(anyhow::anyhow!("raffle engine invariant: {message}"))
                }
            },
            EngineError::Pending => {
                Self::Internal(anyhow::anyhow!("a pending engine call is not an error"))
            }
            EngineError::Unavailable(e) => {
                tracing::warn!(error = %format!("{e:#}"), "raffle engine unavailable");
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
    EventDate,
    EventDateTooFar,
    Retention {
        max_days: i64,
    },
    PrivacyUrl,
    ContactEmail,
    TestModeLocked,
    /// The named fixed settings can't change: someone accepted the rules
    /// they were shown with.
    SettingsFixed(Vec<&'static str>),
    PrizeName,
    PrizeQty,
    PrizesFrozen,
    // Raffles.
    DrawPending,
    JoinCodes,
    /// Registration can't open: the privacy notice can't name the organizer
    /// or a way to reach them. The fields still missing.
    OrganizerMissing(Vec<&'static str>),
    /// Registration can't open: the erasure date has passed, so the raffle
    /// expires within the hour. The date can move while nobody has joined.
    ErasureDatePassed,
    /// Prizes are marked delivered once the raffle is over.
    NotFinishedForDelivery,
    /// The organizer's data can't be removed while one of their raffles is
    /// frozen or drawing.
    EventLive,
    /// The caller's tier in the organization doesn't allow it: who may.
    Forbidden(rafi_api::Access),
    Lifecycle {
        command: lifecycle::Command,
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
    /// The raffle's rules refuse `command` (`None`: whatever the engine was
    /// asked).
    Raffle {
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
    /// From the freeze until the raffle finishes, nobody leaves the list.
    NotRemovable,
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
            | Self::EventDate
            | Self::EventDateTooFar
            | Self::Retention { .. }
            | Self::PrivacyUrl
            | Self::ContactEmail
            | Self::PrizeName
            | Self::PrizeQty
            | Self::Name(_)
            | Self::Email => true,
            Self::Engine { invalid, .. } => *invalid,
            Self::Raffle { error, .. } => error.kind() == ErrorKind::BadRequest,
            _ => false,
        }
    }
}

impl std::error::Error for Refusal {}

impl Refusal {
    /// Why settings were refused, with the installation's longest retention
    /// period for the message.
    pub(crate) fn settings(e: SettingsError, max_days: i64) -> Self {
        match e {
            SettingsError::Title => Self::Title,
            SettingsError::TooLong => Self::Texts,
            SettingsError::EventDate => Self::EventDate,
            SettingsError::EventDateTooFar => Self::EventDateTooFar,
            SettingsError::Retention => Self::Retention { max_days },
            SettingsError::PrivacyUrl => Self::PrivacyUrl,
            SettingsError::Email => Self::ContactEmail,
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
            Self::Texts => {
                f.write_str("Description, rules, privacy text or organizer name are too long.")
            }
            Self::EventDate => f.write_str("The event date must be a calendar day (YYYY-MM-DD)."),
            Self::EventDateTooFar => write!(
                f,
                "The event date can be at most {} days ahead: entrants' data is kept until a set time after it.",
                settings::MAX_EVENT_AHEAD_DAYS
            ),
            Self::ContactEmail => f.write_str("The contact or privacy email is not an email address."),
            Self::Retention { max_days } => write!(
                f,
                "The retention period must be between 0 and {max_days} days."
            ),
            Self::PrivacyUrl => {
                f.write_str("The privacy notice link must be an absolute https:// URL.")
            }
            Self::TestModeLocked => f.write_str("Test mode cannot change once the list is frozen."),
            Self::SettingsFixed(fields) if fields.is_empty() => f.write_str(
                "These settings cannot change: entrants have already accepted them.",
            ),
            Self::SettingsFixed(fields) => write!(
                f,
                "The {} cannot change: entrants have already accepted the rules they were shown with.",
                fields.join(", ")
            ),
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
            Self::OrganizerMissing(fields) => write!(
                f,
                "Before registration can open, fill in {} in the settings: the privacy notice must name the data controller and how to reach them.",
                fields.join(" and ")
            ),
            Self::ErasureDatePassed => f.write_str(
                "The erasure date has passed: this raffle expires within the hour. Move the event date (possible until someone joins) or duplicate the raffle.",
            ),
            Self::NotFinishedForDelivery => {
                f.write_str("Prizes can be marked delivered once the raffle has finished.")
            }
            Self::EventLive => f.write_str(
                "Your data can't be removed while one of your raffles is being drawn. Finish it first.",
            ),
            Self::Forbidden(needs) => write!(f, "Only {} can do this.", needs.who()),
            Self::Lifecycle { command, from } => {
                write!(f, "Cannot {command:?} a raffle that is {}.", from.as_str())
            }
            Self::Changed => f.write_str("The raffle changed in the meantime; reload and try again."),
            Self::NoEngine => f.write_str("Restate is not configured on this server."),
            Self::BeaconMismatch { engine, raffles } => write!(
                f,
                "The raffle engine draws from {engine}, but raffles here commit to {raffles}. Fix the configuration before starting the draw."
            ),
            Self::Start(start::Refusal::State(s)) => write!(
                f,
                "Cannot start the draw of a raffle that is {}.",
                s.as_str()
            ),
            Self::Start(start::Refusal::NoEntrants) => f.write_str("Nobody has entered yet."),
            Self::Start(start::Refusal::NoPrizes) => f.write_str("Add at least one prize first."),
            Self::Start(start::Refusal::FakeEntrants) => f.write_str(
                "Fake entrants are on the list, but test mode is off. Switch test mode on and off again to remove them.",
            ),
            Self::NotDrawing => f.write_str("The draw has not started (or is already over)."),
            Self::NotOpened => f.write_str(
                "The raffle engine has not confirmed the frozen list yet; press “Retry handing the list to the raffle engine”.",
            ),
            Self::Raffle { command, error } => f.write_str(&raffle_refusal(command.as_ref(), error)),
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
            Self::NotRemovable => f.write_str(
                "Entrants stay on the list until the raffle has finished or its erasure date has passed; then their personal data can be erased. You can hide or rename them now.",
            ),
            Self::NotDeliverable => {
                f.write_str("Only a winner who claimed their prize can get it delivered.")
            }
            Self::NotPurgeable => {
                f.write_str("Personal data can be erased at once only once the raffle has finished.")
            }
            Self::NotOver => {
                f.write_str("Live updates can only end once the raffle has finished.")
            }
        }
    }
}

/// The organizer's words for a command the draw rules refuse.
fn raffle_refusal(command: Option<&Command>, e: &RaffleError) -> String {
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
