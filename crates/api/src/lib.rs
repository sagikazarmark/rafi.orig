//! The contract between the Rafi app's server and browser: the views screens
//! poll, and the [`admin`] API: its functions, their arguments and results.

pub mod admin;

/// The public paths, built in one place: the server's pages and redirects,
/// the app's absolute URLs and the web app's links all use these.
pub mod routes {
    /// The join page (`/j/<code>`).
    pub fn join(code: &str) -> String {
        format!("/j/{code}")
    }

    /// The raffle's rules (`/rules/<slug>`).
    pub fn rules(slug: &str) -> String {
        format!("/rules/{slug}")
    }

    /// The raffle's privacy notice (`/privacy/<slug>`).
    pub fn privacy(slug: &str) -> String {
        format!("/privacy/{slug}")
    }

    /// The verification page (`/v/<slug>`).
    pub fn verify(slug: &str) -> String {
        format!("/v/{slug}")
    }

    /// The projector screen (`/s/<slug>`).
    pub fn screen(slug: &str) -> String {
        format!("/s/{slug}")
    }

    /// The full view the screens poll (`/api/r/<slug>/view`).
    pub fn view(slug: &str) -> String {
        format!("/api/r/{slug}/view")
    }

    /// The participant view phones poll (`/api/r/<slug>/participant`).
    pub fn participant(slug: &str) -> String {
        format!("/api/r/{slug}/participant")
    }

    /// What verifiers download (`/api/r/<slug>/verify.json`).
    pub fn verify_json(slug: &str) -> String {
        format!("/api/r/{slug}/verify.json")
    }
}

use rafi_lifecycle::RaffleState;
pub use rafi_lifecycle::entrant::{Origin, RowError};
pub use rafi_lifecycle::names::NameFormat;
use rafi_raffle::{EntrantId, Event, ItemId, ItemStatus, Outcome};
use serde::{Deserialize, Serialize};

/// The public, live state of a raffle: the projector's and the admin's full
/// view, which they poll (`/api/r/{slug}/view`).
///
/// Not a contract: unversioned, read by the web app of the same deployment,
/// and free to change in any release (a page older than the server reloads
/// when it can't read it). Programs read [`VerifyData`], which has a format.
///
/// Must never contain emails, entrant tokens or claim codes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaffleView {
    /// The raffle's version in the database. Clients accept a view with
    /// `v >=` theirs: a poll can return the same version with newer
    /// time-dependent content (`recent_names`).
    pub v: u64,
    /// How to follow the raffle besides polling this view: where the
    /// deployment pushes the participant view (`partyline`), screens
    /// subscribe to it for the reveal; otherwise they only poll.
    pub follow: rafi_live_client::Follow,
    /// The organizer ended live updates.
    #[serde(default)]
    pub live_ended: bool,
    pub raffle_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub state: RaffleState,
    pub join_code: String,
    pub join_url: String,
    pub entrant_count: usize,
    pub recent_names: Vec<String>,
    pub prizes: Vec<PrizeView>,
    pub items: Vec<ItemView>,
    pub current: Option<CurrentDraw>,
    /// Draws in chronological order.
    pub draws: Vec<DrawView>,
    pub last_failure: Option<String>,
    pub manifest_hash: Option<String>,
    pub server_now_ms: u64,
    pub test_mode: bool,
    /// Whether the raffle's beacon source is publicly verifiable (not the
    /// mock drand chain).
    pub verifiable: bool,
    /// Whether people join with the QR code. Without registration the
    /// projector shows no QR code, join link or code: its entrants are the
    /// people the organizer added.
    #[serde(default = "yes")]
    pub registration: bool,
    /// The organizer's name, as the rules and the privacy notice give it;
    /// empty if not given yet. The projector's intro says whose list is in
    /// the draw.
    #[serde(default)]
    pub organizer: String,
    /// The raffle's privacy notice, absolute (`/privacy/<slug>`): the
    /// projector's intro shows it to people on the organizer's list, who
    /// never saw a join form.
    #[serde(default)]
    pub privacy_url: String,
    /// The raffle's verification page, absolute (`/v/<slug>`): the
    /// projector's thank-you screen tells the room where to check the draw.
    #[serde(default)]
    pub verify_url: String,
    /// Whether someone on the list was added from a list (an add from the
    /// list tab), rather than one by one or by joining: the projector tells
    /// registered people they are in already, and the admin's freeze
    /// reminds the organizer that they must have been told.
    #[serde(default)]
    pub list_added: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrizeView {
    pub prize_id: String,
    pub name: String,
    pub sponsor: String,
    pub qty: u32,
    pub left: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemView {
    pub item: ItemId,
    pub prize_id: String,
    pub prize_name: String,
    pub status: ItemStatus,
    pub entrant: Option<EntrantId>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurrentDraw {
    pub k: u64,
    pub item: ItemId,
    pub prize_name: String,
    pub round: u64,
    /// Unix time (seconds) the round is published.
    pub round_time: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawView {
    pub k: u64,
    pub item: ItemId,
    pub prize_name: String,
    pub entrant: EntrantId,
    pub display_name: String,
    pub outcome: Outcome,
    pub pass: u8,
    pub round: u64,
    /// The winner has no phone to tell them they won: the organizer added
    /// them ([`Source::no_phone`](rafi_lifecycle::entrant::Source::no_phone)).
    /// The projector calls them to the front. Not personal: the
    /// verification page counts who was added.
    #[serde(default)]
    pub no_phone: bool,
}

/// Raffle list entry for the admin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaffleSummary {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub state: RaffleState,
    pub entrant_count: i64,
    pub test_mode: bool,
    /// The event's day (`YYYY-MM-DD`), which the raffle's card shows.
    #[serde(default)]
    pub event_date: String,
}

/// Editable raffle settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaffleSettings {
    pub title: String,
    pub description: String,
    pub organizer_name: String,
    pub contact_email: String,
    /// Where privacy requests go; blank means `contact_email`.
    #[serde(default)]
    pub privacy_email: String,
    /// The organizer's own privacy text, shown after the generated notice.
    #[serde(default)]
    pub privacy_text: String,
    /// The organizer's own privacy notice, linked after the generated one.
    #[serde(default)]
    pub privacy_url: String,
    /// Extra rules text appended to the standard rules.
    pub rules: String,
    /// The event's calendar day, `YYYY-MM-DD`. Fixed once an entrant joined.
    pub event_date: String,
    /// The instant the event day ends where the organizer is (unix seconds),
    /// as the browser says; `None` for the end of the day in UTC.
    #[serde(default)]
    pub event_ends_at: Option<i64>,
    /// Days after the event date before non-winners are erased. Fixed once
    /// an entrant joined.
    #[serde(default)]
    pub retention_days: i64,
    pub test_mode: bool,
    /// Whether people may join with the QR code ("Let people join with the
    /// QR code"). Off, the raffle has no registration: it freezes from
    /// Published, with the people the organizer added. Switched before the
    /// freeze, but not while registration is open.
    #[serde(default = "yes")]
    pub registration: bool,
}

/// What the organizer's new raffles start with. Each raffle gets a copy, so
/// changing them changes no existing raffle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaffleDefaults {
    pub organizer_name: String,
    pub contact_email: String,
    #[serde(default)]
    pub privacy_email: String,
    #[serde(default)]
    pub privacy_text: String,
    #[serde(default)]
    pub privacy_url: String,
    /// Extra rules text appended to the standard rules.
    pub rules: String,
    /// `None`: the installation's default.
    #[serde(default)]
    pub retention_days: Option<i64>,
    /// Whether new raffles let people join with the QR code.
    #[serde(default = "yes")]
    pub registration: bool,
}

impl Default for RaffleDefaults {
    /// None saved: blank, and registration on.
    fn default() -> Self {
        Self {
            organizer_name: String::new(),
            contact_email: String::new(),
            privacy_email: String::new(),
            privacy_text: String::new(),
            privacy_url: String::new(),
            rules: String::new(),
            retention_days: None,
            registration: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prize {
    pub id: String,
    pub name: String,
    pub sponsor: String,
    pub description: String,
    pub qty: u32,
    pub position: i64,
    /// How it reaches its winner: handed over on the spot (claiming it marks
    /// it delivered) or sent later (the organizer marks it delivered).
    #[serde(default)]
    pub delivery: rafi_lifecycle::prize::Delivery,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PrizeInput {
    /// `None` creates a new prize.
    pub id: Option<String>,
    pub name: String,
    pub sponsor: String,
    pub description: String,
    pub qty: u32,
    pub position: i64,
    /// Sent later unless said otherwise.
    #[serde(default)]
    pub delivery: rafi_lifecycle::prize::Delivery,
}

/// An entrant as the organizer sees it (includes personal data).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminEntrant {
    pub id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub source: rafi_lifecycle::entrant::Source,
    pub hidden: bool,
    /// `None` once their personal data is erased.
    pub claim_code: Option<String>,
    /// When they entered (unix seconds): the People tab lists the latest
    /// first.
    pub created_at: i64,
    pub delivered: bool,
    /// Whether their prize can be marked delivered (or not): they claimed it.
    #[serde(default)]
    pub deliverable: bool,
    /// A winner whose prize isn't delivered: when their data is erased unless
    /// it is (unix seconds).
    #[serde(default)]
    pub held_until: Option<i64>,
    /// Their personal data was erased; the anonymous entry stays.
    pub erased: bool,
    /// Outcome and prize, once drawn.
    pub outcome: Option<Outcome>,
    pub prize: Option<String>,
    /// The add the organizer put them in with; `None` if they joined, or
    /// are a fake.
    #[serde(default)]
    pub batch_id: Option<String>,
    /// Their ID in the organizer's registration system, if their add gave
    /// one. `None` once erased.
    #[serde(default)]
    pub list_ref: Option<String>,
    /// They took themselves off after the freeze ("Took themselves off"):
    /// their name is hidden, a draw that picks them can only be declined,
    /// and their data is erased once the raffle is over.
    #[serde(default)]
    pub objected: bool,
    /// What the organizer can do with this entrant now, besides what
    /// [`AdminActions`] says for everyone.
    #[serde(default)]
    pub actions: EntrantActions,
    /// They have no phone in the raffle: nothing tells them they won and
    /// they have no claim code to show
    /// ([`Source::no_phone`](rafi_lifecycle::entrant::Source::no_phone)).
    /// Drawn, the console has the organizer check it is them and confirm
    /// it ("Confirmed: it's them") instead of comparing a code.
    #[serde(default)]
    pub no_phone: bool,
}

/// What the organizer can do with one entrant now: the server's rules say
/// ([`rafi_lifecycle::entrant::may_object`]), the page derives none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EntrantActions {
    /// "Took themselves off": someone the organizer added, between the
    /// freeze and the end, who hasn't claimed a prize.
    #[serde(default)]
    pub object: bool,
    /// Undoing it, until the raffle is over.
    #[serde(default)]
    pub unobject: bool,
}

/// Someone removed before the freeze at their request, as a later add
/// matches them: their email key ([`Email::key`](rafi_lifecycle::entrant::Email::key)),
/// their registration ID, or both. Admin only: an email key is personal
/// data, kept only while the raffle can take an add.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedAtRequest {
    #[serde(default)]
    pub email_key: Option<String>,
    #[serde(default)]
    pub list_ref: Option<String>,
    /// When (unix seconds).
    pub removed_at: i64,
}

impl RemovedAtRequest {
    /// Whether a row with this email key or registration ID is the one
    /// removed.
    pub fn matches(&self, email_key: Option<&str>, list_ref: Option<&str>) -> bool {
        (email_key.is_some() && self.email_key.as_deref() == email_key)
            || (list_ref.is_some() && self.list_ref.as_deref() == list_ref)
    }
}

/// One person to add ([`AddRequest`]): the name shown on the screen, and
/// optionally their email (to reach a winner, and one entry per address)
/// and their ID in the organizer's registration system (to match them on a
/// later add). Blank means none.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AddRow {
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub list_ref: Option<String>,
    /// The organizer approved the name, though the blocklist refuses it
    /// ([`RowError::BlockedName`]): its substring matching catches real
    /// names. It lifts the blocklist only, never the length or the
    /// characters a name may hold.
    #[serde(default)]
    pub approved: bool,
    /// The organizer adds them though they were removed at their request
    /// ([`SkipReason::RemovedAtRequest`]): the removal is deleted with the
    /// add, in the same all-or-nothing write, and counted in its audit row.
    #[serde(default)]
    pub lift_removal: bool,
}

/// An add: people the organizer puts in at once, all of them or none
/// (`add_entrants`). Its rows are checked as a whole first: an invalid row,
/// or a display name someone else is shown with, refuses the add with every
/// row's problem ([`ApiError::rows`](admin::ApiError::rows)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddRequest {
    /// A random ID the browser makes for the add (1 to 64 of `A-Z a-z 0-9 - _`)
    /// and sends again on a retry: an add already made with it answers its
    /// first result and adds nothing. "Undo last add" names it too.
    pub batch_id: String,
    /// The one-person form (exactly one row), or a list.
    pub origin: Origin,
    /// In the list's order, which the entrants keep.
    pub rows: Vec<AddRow>,
    /// How the browser made a list's display names, for the audit log: the
    /// server takes the names as sent, and checks them.
    #[serde(default)]
    pub name_format: Option<NameFormat>,
}

/// What an add did, row by row (rows by their index in the request).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddResult {
    pub batch_id: String,
    /// The rows added, with their entrant IDs.
    pub added: Vec<AddedEntrant>,
    /// The rows left out, and why.
    pub skipped: Vec<SkippedRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedEntrant {
    pub row: u32,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedRow {
    pub row: u32,
    pub reason: SkipReason,
}

/// Why a row of an add was left out, though the add went through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Someone with this email or registration ID is in the raffle already
    /// (they joined, or an earlier add has them).
    AlreadyIn,
    /// Someone with this email or registration ID was removed at their
    /// request: added only if the organizer lifts it
    /// ([`AddRow::lift_removal`]).
    RemovedAtRequest,
}

/// A row of an add that refused it, and what is wrong with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowProblem {
    /// Its index in the request.
    pub row: u32,
    pub problem: RowError,
    /// For a display name someone else has ([`RowError::NameTaken`],
    /// [`RowError::DuplicateName`]): the name they are shown as, as they
    /// have it, so the organizer sees whom it clashes with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown_as: Option<String>,
}

impl RowProblem {
    /// What is wrong with the row, in the organizer's words (lowercase, to
    /// follow "row 3: "): [`row_error_text`], with the name it clashes with
    /// when there is one ("someone is already shown as “Anna K.”").
    pub fn text(&self) -> String {
        match (self.problem, &self.shown_as) {
            (RowError::NameTaken, Some(name)) => {
                format!("someone is already shown as “{name}”")
            }
            (RowError::DuplicateName, Some(name)) => {
                format!("an earlier row is shown as “{name}” too")
            }
            (problem, _) => row_error_text(problem).to_owned(),
        }
    }
}

/// What is wrong with a row, in the organizer's words (lowercase, to follow
/// "row 3: "): the server's refusal says it, and the browser can say it
/// where the row is. [`RowProblem::text`] names the other one of a clash.
pub fn row_error_text(e: RowError) -> &'static str {
    match e {
        RowError::Name => {
            "the name must be 1–64 characters, with at least one letter or digit and nothing invisible"
        }
        RowError::BlockedName => {
            "the name has a word the blocklist refuses; approve it if it is their real name"
        }
        RowError::Email => "not an email address",
        RowError::ListRef => "the registration ID must be 1–64 characters, with nothing invisible",
        RowError::DuplicateEmail => "an earlier row has the same email",
        RowError::DuplicateListRef => "an earlier row has the same registration ID",
        RowError::NameTaken => "someone is already shown with this name",
        RowError::DuplicateName => "an earlier row is shown with this name too",
    }
}

/// The add "Undo last add" undoes: the raffle's latest, while its entrants
/// can change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoBatch {
    pub batch_id: String,
    /// How many people it would take off the list.
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EntrantPatch {
    pub display_name: Option<String>,
    pub hidden: Option<bool>,
    pub delivered: Option<bool>,
}

/// Everything the admin page needs for one raffle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaffleAdmin {
    pub id: String,
    pub settings: RaffleSettings,
    pub prizes: Vec<Prize>,
    pub entrants: Vec<AdminEntrant>,
    pub view: RaffleView,
    pub public_url: String,
    /// What the organizer can do next: the server's rules say, the page
    /// derives none of its own.
    pub actions: AdminActions,
    /// When the raffle's personal data is erased.
    #[serde(default)]
    pub retention: Retention,
    /// Who was removed at their request, oldest first: a later add leaves
    /// them out unless the organizer lifts it. Empty once the list is
    /// frozen for good.
    #[serde(default)]
    pub removed: Vec<RemovedAtRequest>,
}

impl RaffleAdmin {
    /// The paragraph to send the people the organizer adds
    /// ([`paragraph_to_send`]), from the raffle's saved settings, with
    /// names shown in `format`. Its links are absolute, under the
    /// installation's address, and work once the raffle is published.
    pub fn paragraph_to_send(&self, format: NameFormat) -> String {
        let s = &self.settings;
        let date = rafi_lifecycle::dates::Date::parse(&s.event_date)
            .map_or_else(|| s.event_date.clone(), |d| d.long());
        let contact = match s.privacy_email.trim() {
            "" => s.contact_email.trim(),
            privacy => privacy,
        };
        let base = self.public_url.trim_end_matches('/');
        paragraph_to_send(&ToSend {
            title: &s.title,
            event_date: &date,
            organizer: s.organizer_name.trim(),
            contact,
            rules_url: &format!("{base}/rules/{}", self.view.slug),
            privacy_url: &format!("{base}/privacy/{}", self.view.slug),
            own_privacy_url: s.privacy_url.trim(),
            format,
        })
    }
}

/// What the paragraph to send names ([`paragraph_to_send`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToSend<'a> {
    /// The raffle's title.
    pub title: &'a str,
    /// The event day in words, as the privacy notice says it ("28 October
    /// 2026").
    pub event_date: &'a str,
    /// The organizer's name; empty if not given yet.
    pub organizer: &'a str,
    /// Where someone says they'd rather not take part: the privacy contact,
    /// or else the contact email; empty if neither.
    pub contact: &'a str,
    /// The raffle's rules and privacy notice, absolute.
    pub rules_url: &'a str,
    pub privacy_url: &'a str,
    /// The organizer's own privacy notice; empty if none. Linked only if it
    /// is `https:`, as the settings require.
    pub own_privacy_url: &'a str,
    /// How the names are shown.
    pub format: NameFormat,
}

/// The paragraph the organizer sends the people they add (in an email, on
/// the event page, before the event): plain text, theirs to edit, in their
/// voice. Adding a list is the organizer's word that its people take part;
/// telling them is how they learn of it, so it says what a privacy notice
/// of data from elsewhere must start with (GDPR Art. 14): that they are in,
/// whose raffle it is, that their name is public and how, where the rules
/// and the notice are, and how to stay out, with a concrete cutoff.
pub fn paragraph_to_send(p: &ToSend<'_>) -> String {
    let by = if p.organizer.is_empty() {
        "us"
    } else {
        p.organizer
    };
    let contact = if p.contact.is_empty() {
        "us"
    } else {
        p.contact
    };
    let own = if p
        .own_privacy_url
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
    {
        format!(" · Our privacy notice: {}", p.own_privacy_url)
    } else {
        String::new()
    };
    format!(
        "You're in the prize draw of {title} on {date}, run by {by} with Rafi, as someone on our guest list. \
         Your name is shown on the event screen and the public results page as {described}, like “{example}”. \
         Rules: {rules} · Privacy: {privacy}{own}. \
         Rather not take part? Tell {contact} before the draw on {date}.",
        title = p.title,
        date = p.event_date,
        described = p.format.described(),
        example = p.format.example(),
        rules = p.rules_url,
        privacy = p.privacy_url,
    )
}

/// When a raffle's personal data is erased, as the admin shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Retention {
    /// When non-winners are erased (unix seconds), and the day, in words.
    pub erase_at: i64,
    pub erase_date: String,
    /// When winners whose prize wasn't delivered are erased at the latest
    /// (unix seconds), and the day, in words.
    pub winners_erase_at: i64,
    pub winners_erase_date: String,
    /// Winners whose data is held: claimed, not delivered, not erased.
    pub held_winners: usize,
    /// The erasure date has passed.
    pub past_erase_date: bool,
    /// The event day has ended and the raffle is still being drawn: its
    /// data is erased on the date regardless, and it should be finished.
    pub unfinished_past_event: bool,
    /// The event day has ended and the list was never frozen: the raffle
    /// expires on the erasure date (within the hour, once it has passed),
    /// its entrants deleted.
    pub expires_unfrozen: bool,
    /// The erasure date is less than a day away and the raffle is still
    /// taking or holding entrants (not drawn, not over): with a period of
    /// zero, or yesterday's date, an evening that runs past midnight loses
    /// its entrants in the middle of it unless it is frozen and finished
    /// before.
    #[serde(default)]
    pub erasure_imminent: bool,
    /// The installation's default period, the longest one a raffle may
    /// choose, and the winner hold, in days.
    pub default_days: i64,
    pub max_days: i64,
    pub winner_hold_days: i64,
}

/// What the organizer can do next with a raffle.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AdminActions {
    /// Where starting the draw stands, while it can start (or has): the
    /// freeze from Open or Closed, or from Published without registration.
    pub start: Option<StartStep>,
    /// The lifecycle commands the raffle takes now, with its registration
    /// setting ([`rafi_lifecycle::RaffleState::apply`]): the header's
    /// buttons. Opening may still be refused for what
    /// [`missing`](Self::missing) names.
    #[serde(default)]
    pub lifecycle: Vec<LifecycleCommand>,
    /// What the settings still lack, by name, for the step that lets the
    /// raffle's entrants in, while it is ahead
    /// ([`opening_ahead`](rafi_lifecycle::RaffleState::opening_ahead):
    /// Draft and Published): opening registration, or, without
    /// registration, the freeze from Published. The header says what is
    /// missing and keeps that step's button disabled. Empty when nothing
    /// is, or once the step is behind.
    #[serde(default)]
    pub missing: Vec<String>,
    /// Whether registration can be switched on or off now
    /// ([`rafi_lifecycle::RaffleState::registration_editable`]): before the
    /// freeze, but not while it is open.
    #[serde(default)]
    pub edit_registration: bool,
    /// Whether the raffle has a draw console: its entrant list is frozen.
    pub draw: bool,
    /// Whether the winners can be exported.
    pub export: bool,
    /// Whether "erase personal data now" applies: finished, with personal
    /// data left (a purge that stopped partway included).
    pub purge: bool,
    /// Whether the fixed settings (the additional rules, the event date, the
    /// retention period, the privacy text and link) can still change: not
    /// once an entrant locks them (one who joined, or the organizer added).
    #[serde(default)]
    pub edit_rules: bool,
    /// How an entrant is removed from the list now, if they can be.
    #[serde(default)]
    pub removal: Option<rafi_lifecycle::entrant::Removal>,
    /// Whether "all prizes delivered" applies: finished, with winners held.
    #[serde(default)]
    pub deliver_all: bool,
    /// Whether the raffle can be deleted: until its first draw is announced.
    /// From then on its record stays, and "erase personal data now" is the
    /// way to be rid of what's personal in it.
    #[serde(default = "yes")]
    pub delete: bool,
    /// The add "Undo last add" takes back, while the entrant list can change
    /// and an add of the organizer's is on it.
    #[serde(default)]
    pub undo: Option<UndoBatch>,
    /// Whether a list can be added now (the Add people dialog's list tab):
    /// the raffle takes an add from [`Origin::List`]
    /// ([`may_add`](rafi_lifecycle::entrant::may_add)).
    #[serde(default)]
    pub add_list: bool,
    /// Why a list can't be added now, for the list tab to say (not
    /// published yet, in test mode, …); `None` when it can be
    /// ([`add_list`](Self::add_list)).
    #[serde(default)]
    pub add_list_refusal: Option<rafi_lifecycle::entrant::AddRefusal>,
}

fn yes() -> bool {
    true
}

/// Where starting a raffle's draw stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartStep {
    /// The list can be frozen and the draw started.
    Freeze,
    /// Frozen, but an attempt stopped before the raffle engine had the
    /// list: starting again picks up where it stopped.
    Retry,
    /// The raffle engine has the raffle.
    Started,
}

/// A lifecycle command the organizer gives from the admin page: the
/// organizer's subset of `rafi_lifecycle::Command` (freezing is starting the
/// draw, purging is its own action).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleCommand {
    Publish,
    Unpublish,
    Open,
    Close,
}

impl LifecycleCommand {
    /// Every command, in the order the header offers them.
    pub const ALL: [Self; 4] = [Self::Publish, Self::Open, Self::Close, Self::Unpublish];
}

impl From<LifecycleCommand> for rafi_lifecycle::Command {
    fn from(c: LifecycleCommand) -> Self {
        match c {
            LifecycleCommand::Publish => Self::Publish,
            LifecycleCommand::Unpublish => Self::Unpublish,
            LifecycleCommand::Open => Self::Open,
            LifecycleCommand::Close => Self::Close,
        }
    }
}

/// The shape of [`VerifyData`] as published: `format` in the JSON. A reader
/// refuses a format it doesn't know; a change that adds fields keeps the
/// number, one that removes or renames any bumps it.
pub const VERIFY_FORMAT: u32 = 1;

/// Everything needed to independently verify a raffle's draws. The manifest
/// and the events are [`rafi_raffle`]'s published shapes; `format` versions
/// the envelope around them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyData {
    /// [`VERIFY_FORMAT`].
    #[serde(default = "verify_format")]
    pub format: u32,
    /// The raffle's version in the database, as the views carry it: the
    /// same `v` means the same content, which is what the `ETag` says.
    #[serde(default)]
    pub v: u64,
    pub title: String,
    pub state: RaffleState,
    /// A rehearsal: drawn in test mode, possibly with fake entrants.
    #[serde(default)]
    pub test_mode: bool,
    /// The raffle's canonical manifest, exactly as hashed.
    pub manifest: String,
    pub manifest_hash: String,
    pub events: Vec<Event>,
    /// Entrant ID → display name, hidden names masked. Erased entrants
    /// aren't listed (they are in the manifest, by ID): once personal data
    /// is erased, this is empty.
    pub names: Vec<(String, String)>,
    /// How the manifest's entrants got in, counted: who joined on their
    /// phone and whom the organizer added (their word, which the draw
    /// can't check). `None` from a server that didn't count them.
    /// Outside the manifest: the hash doesn't commit to them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<SourceCounts>,
}

fn verify_format() -> u32 {
    VERIFY_FORMAT
}

/// The manifest's entrants by how they got in ([`VerifyData::sources`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SourceCounts {
    /// Joined on their phones, with the QR code.
    pub joined: u32,
    /// Added by the organizer, one by one or from a list.
    pub added: u32,
    /// Made up for a rehearsal (test mode only).
    #[serde(default)]
    pub fake: u32,
}

impl SourceCounts {
    /// Counts `sources`, one per entrant.
    pub fn of(sources: impl IntoIterator<Item = rafi_lifecycle::entrant::Source>) -> Self {
        use rafi_lifecycle::entrant::Source;
        let mut counts = Self::default();
        for source in sources {
            let n = match source {
                Source::SelfService => &mut counts.joined,
                Source::Manual => &mut counts.added,
                Source::Fake => &mut counts.fake,
            };
            *n = n.saturating_add(1);
        }
        counts
    }

    /// Everyone counted.
    pub fn total(&self) -> u64 {
        u64::from(self.joined) + u64::from(self.added) + u64::from(self.fake)
    }

    /// The counts in words, as the verification page says them: "320
    /// entrants: 8 joined on their phones, 312 added by the organizer".
    /// Both counts are said even when zero, so "0 added by the organizer"
    /// is said too; made-up ones only if there are any.
    pub fn text(&self) -> String {
        let total = self.total();
        let mut text = format!(
            "{total} {}: {} joined on {}, {} added by the organizer",
            if total == 1 { "entrant" } else { "entrants" },
            self.joined,
            if self.joined == 1 {
                "their phone"
            } else {
                "their phones"
            },
            self.added,
        );
        if self.fake > 0 {
            text.push_str(&format!(", {} made up for the test", self.fake));
        }
        text
    }
}

/// Current organizer identity, and what the admin must know before it
/// shows the raffles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Me {
    pub owner_id: String,
    /// What the signed-in user may do for the owner.
    #[serde(default = "Access::admin")]
    pub access: Access,
    /// The owner is an organization: its raffles, defaults and data are
    /// shared with every member.
    #[serde(default)]
    pub shared: bool,
    /// Whether automatic erasure looks stopped: no housekeeping run has
    /// succeeded for two hours, counting from the first raffle.
    #[serde(default)]
    pub retention_stale: bool,
    /// Which Rafi the server is (its version).
    #[serde(default)]
    pub version: String,
}

/// Who is calling the admin API: the owner key (a Clerk organization, or
/// else the user) that raffles belong to, the user acting for it, and what
/// they may do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caller {
    pub owner: String,
    pub user: String,
    pub access: Access,
}

/// What a caller may do for the owner. A personal account does everything.
/// In a Clerk organization, the tier comes from the session's claims:
/// `org:admin` is [`Admin`](Self::Admin); the custom role `org:manager`, or
/// the custom permission `org:raffles:manage`, is [`Manager`](Self::Manager);
/// every other member is a [`Member`](Self::Member). Each tier includes the
/// ones below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// Runs raffles: registration, the draw, entrants, prizes, exports,
    /// erasure.
    Member,
    /// Also creates, copies, deletes and configures raffles and the defaults.
    Manager,
    /// Also removes the owner's data.
    Admin,
}

impl Access {
    /// Clerk's role for an organization's administrators.
    pub const ADMIN_ROLE: &str = "org:admin";
    /// The custom role (made in the Clerk dashboard) for managers.
    pub const MANAGER_ROLE: &str = "org:manager";
    /// The custom permission (made in the Clerk dashboard) that makes a
    /// manager, whatever the role is called.
    pub const MANAGE_PERMISSION: &str = "org:raffles:manage";

    /// The tier an organization member with `role` and `permissions` has.
    pub fn of_organization(role: Option<&str>, permissions: &[String]) -> Self {
        match role {
            Some(Self::ADMIN_ROLE) => Self::Admin,
            Some(Self::MANAGER_ROLE) => Self::Manager,
            _ if permissions.iter().any(|p| p == Self::MANAGE_PERMISSION) => Self::Manager,
            _ => Self::Member,
        }
    }

    /// Whether this tier may do what `needs`.
    pub fn allows(self, needs: Self) -> bool {
        self >= needs
    }

    /// For serde defaults: the tier an old server, which had none, implied.
    fn admin() -> Self {
        Self::Admin
    }

    /// Who may do what `needs`, for the organizer.
    pub fn who(self) -> &'static str {
        match self {
            Self::Member => "any member of the organization",
            Self::Manager => "an organization admin or manager",
            Self::Admin => "an organization admin",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The limits the words give are the limits the rules apply.
    #[test]
    fn row_errors_name_the_limits() {
        use rafi_lifecycle::entrant::{MAX_ADDED_NAME_CHARS, MAX_LIST_REF_CHARS};
        assert!(row_error_text(RowError::Name).contains(&format!("1–{MAX_ADDED_NAME_CHARS} ")));
        assert!(row_error_text(RowError::ListRef).contains(&format!("1–{MAX_LIST_REF_CHARS} ")));
    }

    fn to_send(format: NameFormat) -> ToSend<'static> {
        ToSend {
            title: "Cloud Native Budapest #12",
            event_date: "28 October 2026",
            organizer: "Cloud Native Budapest",
            contact: "dpo@example.com",
            rules_url: "https://rafi.example/rules/cnb-12",
            privacy_url: "https://rafi.example/privacy/cnb-12",
            own_privacy_url: "",
            format,
        }
    }

    #[test]
    fn the_paragraph_to_send() {
        assert_eq!(
            paragraph_to_send(&to_send(NameFormat::Full)),
            "You're in the prize draw of Cloud Native Budapest #12 on 28 October 2026, run by Cloud Native Budapest with Rafi, as someone on our guest list. \
             Your name is shown on the event screen and the public results page as your full name, like “Anna Kovács”. \
             Rules: https://rafi.example/rules/cnb-12 · Privacy: https://rafi.example/privacy/cnb-12. \
             Rather not take part? Tell dpo@example.com before the draw on 28 October 2026."
        );
        let initials = paragraph_to_send(&to_send(NameFormat::FirstInitial));
        assert!(
            initials
                .contains("as your first name and the initial of your last name, like “Anna K.”."),
            "{initials}"
        );

        // The organizer's own notice, only over https.
        let mut p = to_send(NameFormat::Full);
        p.own_privacy_url = "https://example.org/privacy";
        assert!(
            paragraph_to_send(&p).contains(
                "Privacy: https://rafi.example/privacy/cnb-12 · Our privacy notice: https://example.org/privacy. Rather"
            ),
            "{}",
            paragraph_to_send(&p)
        );
        for not_https in [
            "http://example.org/privacy",
            "javascript:alert(1)",
            "ftp://example.org",
        ] {
            p.own_privacy_url = not_https;
            assert!(!paragraph_to_send(&p).contains(not_https), "{not_https}");
        }

        // Nobody named yet: the organizer's voice.
        let mut p = to_send(NameFormat::Full);
        p.organizer = "";
        p.contact = "";
        let text = paragraph_to_send(&p);
        assert!(text.contains(", run by us with Rafi,"), "{text}");
        assert!(
            text.contains("Tell us before the draw on 28 October 2026."),
            "{text}"
        );
    }

    /// From the admin's data: the date in words, the privacy contact first,
    /// the links under the installation's address.
    #[test]
    fn the_paragraph_from_a_raffle() {
        let admin: RaffleAdmin = serde_json::from_value(serde_json::json!({
            "id": "r1",
            "settings": {
                "title": "Meetup", "description": "", "organizer_name": "Org",
                "contact_email": "hello@example.com", "privacy_email": "",
                "rules": "", "event_date": "2026-10-28", "test_mode": false
            },
            "prizes": [], "entrants": [],
            "view": {
                "v": 1, "follow": { "kind": "poll", "every_ms": 2500 }, "raffle_id": "r1", "slug": "meetup-x", "title": "Meetup",
                "description": "", "state": "ready", "join_code": "ABCDEF", "join_url": "",
                "entrant_count": 0, "recent_names": [], "prizes": [], "items": [],
                "current": null, "draws": [], "last_failure": null, "manifest_hash": null,
                "server_now_ms": 0, "test_mode": false, "verifiable": true
            },
            "public_url": "https://rafi.example/",
            "actions": { "start": null, "draw": false, "export": false, "purge": false }
        }))
        .unwrap();
        let text = admin.paragraph_to_send(NameFormat::Full);
        assert!(
            text.contains("of Meetup on 28 October 2026, run by Org"),
            "{text}"
        );
        assert!(text.contains("Rules: https://rafi.example/rules/meetup-x · Privacy: https://rafi.example/privacy/meetup-x."), "{text}");
        assert!(text.contains("Tell hello@example.com before"), "{text}");
        let mut admin = admin;
        admin.settings.privacy_email = "dpo@example.com".into();
        assert!(
            admin
                .paragraph_to_send(NameFormat::Full)
                .contains("Tell dpo@example.com before"),
            "the privacy contact first"
        );
    }

    #[test]
    fn source_counts_in_words() {
        use rafi_lifecycle::entrant::Source;
        let counts = SourceCounts::of(
            std::iter::repeat_n(Source::SelfService, 8)
                .chain(std::iter::repeat_n(Source::Manual, 312)),
        );
        assert_eq!(
            counts,
            SourceCounts {
                joined: 8,
                added: 312,
                fake: 0
            }
        );
        assert_eq!(
            counts.text(),
            "320 entrants: 8 joined on their phones, 312 added by the organizer"
        );
        assert_eq!(
            SourceCounts::of([Source::SelfService]).text(),
            "1 entrant: 1 joined on their phone, 0 added by the organizer"
        );
        assert_eq!(
            SourceCounts::of([Source::Fake, Source::Fake, Source::Manual]).text(),
            "3 entrants: 0 joined on their phones, 1 added by the organizer, 2 made up for the test"
        );
    }

    /// `verify.json` gains the counts without a new format: one without
    /// them still reads, and theirs is left out rather than null.
    #[test]
    fn verify_data_with_and_without_source_counts() {
        let old = serde_json::json!({
            "format": 1, "v": 3, "title": "Meetup", "state": "drawing", "test_mode": false,
            "manifest": "{}", "manifest_hash": "00", "events": [], "names": []
        });
        let data: VerifyData = serde_json::from_value(old).unwrap();
        assert_eq!(data.sources, None);
        assert!(
            serde_json::to_value(&data)
                .unwrap()
                .get("sources")
                .is_none()
        );
        let mut data = data;
        data.sources = Some(SourceCounts {
            joined: 1,
            added: 2,
            fake: 0,
        });
        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(
            json["sources"],
            serde_json::json!({ "joined": 1, "added": 2, "fake": 0 })
        );
        assert_eq!(json["format"], VERIFY_FORMAT);
    }

    #[test]
    fn row_problems_name_the_clash() {
        let problem = |problem, shown_as: Option<&str>| RowProblem {
            row: 0,
            problem,
            shown_as: shown_as.map(str::to_owned),
        };
        assert_eq!(
            problem(RowError::NameTaken, Some("Anna K.")).text(),
            "someone is already shown as “Anna K.”"
        );
        assert_eq!(
            problem(RowError::DuplicateName, Some("Anna K.")).text(),
            "an earlier row is shown as “Anna K.” too"
        );
        assert_eq!(
            problem(RowError::NameTaken, None).text(),
            row_error_text(RowError::NameTaken)
        );
        assert_eq!(
            problem(RowError::Email, None).text(),
            "not an email address"
        );
        let json = serde_json::to_value(problem(RowError::Email, None)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "row": 0, "problem": "email" }),
            "no name where there is none"
        );
    }
}
