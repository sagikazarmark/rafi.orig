//! Why a use case didn't do what it was asked.

use std::fmt;

use rafi_engine::EngineError;
use rafi_lifecycle::{
    self as lifecycle, RaffleState,
    entrant::{AddRefusal, FakesRefusal, MAX_ADD_ROWS, NameError, Origin},
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
    /// How a prize reaches its winner can't change once the raffle is over.
    PrizeDeliveryFixed,
    // Raffles.
    /// The raffle has drawn (or announced a draw): its record stays.
    Drawn,
    JoinCodes,
    /// Registration can't open: the privacy notice can't name the organizer
    /// or a way to reach them. The fields still missing.
    OrganizerMissing(Vec<&'static str>),
    /// Registration can't open: the erasure date has passed, so the raffle
    /// expires within the hour. The date can move while nobody has joined.
    ErasureDatePassed,
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
    /// The raffle has as many entrants as the installation allows.
    Full,
    Name(NameError),
    Fakes(FakesRefusal),
    /// An add without a valid batch ID ([`entrant::batch_id`](lifecycle::entrant::batch_id)).
    BatchId,
    /// An add with more or fewer rows than its origin takes: exactly one
    /// from the one-person form, 1 to [`MAX_ADD_ROWS`] from a list.
    RowCount(Origin),
    /// An add with invalid rows, each with its problem: refused whole, so
    /// the browser can show them and send it again.
    Rows {
        origin: Origin,
        rows: Vec<rafi_api::RowProblem>,
    },
    /// The raffle doesn't take an add from there now.
    Add(AddRefusal),
    /// The add doesn't fit under the entrant cap: `left` more people would.
    /// Nothing was added.
    Room {
        adding: usize,
        left: i64,
    },
    /// Over the organizer's rate limit on adds and undos.
    Limited,
    Erased,
    /// From the freeze until the raffle finishes, nobody leaves the list.
    NotRemovable,
    NotDeliverable,
    /// The raffle hasn't finished, which what was asked needs.
    NotOver(Unfinished),
}

/// What waits for the raffle to finish ([`Refusal::NotOver`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unfinished {
    /// "Erase personal data now".
    Purge,
    /// "All prizes delivered" (one winner's prize can be marked delivered
    /// any time after they claimed it).
    DeliverAll,
    /// Ending live updates.
    EndLive,
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
            | Self::BatchId
            | Self::RowCount(_)
            | Self::Rows { .. } => true,
            Self::Engine { invalid, .. } => *invalid,
            Self::Raffle { error, .. } => error.kind() == ErrorKind::BadRequest,
            _ => false,
        }
    }
}

impl std::error::Error for Refusal {}

impl Refusal {
    /// The refusal's kind as a stable `snake_case` name: the admin API's
    /// error `code`, for programs. The message is for people.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Texts => "texts",
            Self::EventDate => "event_date",
            Self::EventDateTooFar => "event_date_too_far",
            Self::Retention { .. } => "retention",
            Self::PrivacyUrl => "privacy_url",
            Self::ContactEmail => "contact_email",
            Self::TestModeLocked => "test_mode_locked",
            Self::SettingsFixed(_) => "settings_fixed",
            Self::PrizeName => "prize_name",
            Self::PrizeQty => "prize_qty",
            Self::PrizesFrozen => "prizes_frozen",
            Self::PrizeDeliveryFixed => "prize_delivery_fixed",
            Self::Drawn => "drawn",
            Self::JoinCodes => "join_codes",
            Self::OrganizerMissing(_) => "organizer_missing",
            Self::ErasureDatePassed => "erasure_date_passed",
            Self::EventLive => "event_live",
            Self::Forbidden(_) => "forbidden",
            Self::Lifecycle { .. } => "lifecycle",
            Self::Changed => "changed",
            Self::NoEngine => "no_engine",
            Self::BeaconMismatch { .. } => "beacon_mismatch",
            Self::Start(_) => "start",
            Self::NotDrawing => "not_drawing",
            Self::NotOpened => "not_opened",
            Self::Raffle { .. } => "raffle",
            Self::Engine { .. } => "engine",
            Self::EntrantsFrozen => "entrants_frozen",
            Self::Full => "full",
            Self::Name(_) => "name",
            Self::Fakes(_) => "fakes",
            Self::BatchId => "batch_id",
            Self::RowCount(_) => "row_count",
            Self::Rows { .. } => "rows",
            Self::Add(AddRefusal::Frozen) => "entrants_frozen",
            Self::Add(AddRefusal::ErasureDatePassed) => "erasure_date_passed",
            Self::Add(AddRefusal::Unpublished) => "list_unpublished",
            Self::Add(AddRefusal::TestMode) => "list_test_mode",
            Self::Room { .. } => "room",
            Self::Limited => "limited",
            Self::Erased => "erased",
            Self::NotRemovable => "not_removable",
            Self::NotDeliverable => "not_deliverable",
            Self::NotOver(_) => "not_over",
        }
    }

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
            Self::PrizeDeliveryFixed => f.write_str(
                "The raffle is over, so how a prize is handed over no longer changes anything. Mark prizes delivered in the People tab.",
            ),
            Self::Drawn => f.write_str(
                "This raffle has drawn prizes, so its record stays verifiable and it can't be deleted. Finish it, then erase its personal data from the menu.",
            ),
            Self::JoinCodes => f.write_str("Could not allocate a unique join code; please try again."),
            Self::OrganizerMissing(fields) => write!(
                f,
                "Before registration can open, fill in {} in the settings: the privacy notice must name the data controller and how to reach them.",
                fields.join(" and ")
            ),
            Self::ErasureDatePassed => f.write_str(
                "The erasure date has passed: this raffle expires within the hour. Move the event date (possible until someone joins) or duplicate the raffle.",
            ),
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
            Self::Start(start::Refusal::TooManyItems) => write!(
                f,
                "Too many prizes to draw for: at most {} items (prize copies) in one raffle.",
                settings::MAX_ITEMS
            ),
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
            Self::Full => f.write_str("The raffle is full: it has as many entrants as this installation allows."),
            Self::Name(NameError::Invalid) => f.write_str(
                "Please enter a name: 1–24 characters, with at least one letter or digit and nothing invisible.",
            ),
            Self::Name(NameError::Blocked) => f.write_str("Please choose a different name."),
            Self::BatchId => {
                f.write_str("This add has no valid batch ID. Reload the page and try again.")
            }
            Self::RowCount(Origin::One) => f.write_str("Add one person at a time here."),
            Self::RowCount(Origin::List) => write!(
                f,
                "A list adds 1 to {MAX_ADD_ROWS} people at once. Add a longer one in parts."
            ),
            Self::Rows { origin, rows } => f.write_str(&rows_refusal(*origin, rows)),
            Self::Add(AddRefusal::Frozen) => f.write_str("The entrant list is frozen."),
            Self::Add(AddRefusal::ErasureDatePassed) => f.write_str(
                "The erasure date has passed: this raffle expires within the hour, so nobody can be added.",
            ),
            Self::Add(AddRefusal::Unpublished) => f.write_str(
                "Publish the raffle before adding a list: what you tell the people on it links the raffle's rules and privacy notice.",
            ),
            Self::Add(AddRefusal::TestMode) => f.write_str(
                "A list can't be added in test mode: a rehearsal's draws are public. Rehearse with fake entrants, and add the list once test mode is off.",
            ),
            Self::Room { adding, left } if *left <= 0 => write!(
                f,
                "The raffle is full: it has as many entrants as this installation allows. Nobody of these {adding} was added."
            ),
            Self::Room { adding, left } => write!(
                f,
                "Only {left} more people fit in this raffle, and this add has {adding}. Nothing was added: add fewer."
            ),
            Self::Limited => f.write_str("Too many adds just now. Wait a minute and try again."),
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
            Self::NotOver(Unfinished::Purge) => {
                f.write_str("Personal data can be erased at once once the raffle has finished.")
            }
            Self::NotOver(Unfinished::DeliverAll) => f.write_str(
                "All prizes can be marked delivered at once once the raffle has finished. One winner's prize can be marked delivered any time after they claimed it.",
            ),
            Self::NotOver(Unfinished::EndLive) => {
                f.write_str("Live updates can end once the raffle has finished.")
            }
        }
    }
}

/// The organizer's words for an add refused for its rows: the problem itself
/// for the one-person form, which has one row; for a list, how many rows
/// and the first few, numbered from 1 as a spreadsheet does.
fn rows_refusal(origin: Origin, rows: &[rafi_api::RowProblem]) -> String {
    const SHOWN: usize = 3;
    match (origin, rows) {
        (_, []) => "Some rows can't be added.".to_owned(),
        (Origin::One, [first, ..]) => {
            let text = rafi_api::row_error_text(first.problem);
            let mut chars = text.chars();
            let capital: String = chars
                .next()
                .into_iter()
                .flat_map(char::to_uppercase)
                .collect();
            format!("{capital}{}.", chars.as_str())
        }
        (Origin::List, rows) => {
            let listed: Vec<String> = rows
                .iter()
                .take(SHOWN)
                .map(|r| format!("row {}: {}", r.row + 1, rafi_api::row_error_text(r.problem)))
                .collect();
            let more = match rows.len().saturating_sub(SHOWN) {
                0 => String::new(),
                n => format!("; and {n} more"),
            };
            let count = match rows.len() {
                1 => "1 row".to_owned(),
                n => format!("{n} rows"),
            };
            format!(
                "{count} can't be added, so nothing was: {}{more}.",
                listed.join("; ")
            )
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
