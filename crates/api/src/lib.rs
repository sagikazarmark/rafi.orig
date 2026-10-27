//! The contract between the Rafi app's server and browser: the views screens
//! poll, and the [`admin`] API: its functions, their arguments and results.

pub mod admin;

use rafi_lifecycle::RaffleState;
use rafi_raffle::{EntrantId, Event, ItemId, ItemStatus, Outcome};
use serde::{Deserialize, Serialize};

/// The public, live state of a raffle: the projector's and the admin's full
/// view, which they poll (`/api/r/{slug}/view`).
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
    pub created_at: i64,
}

/// Editable raffle settings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
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
}

/// What the organizer's new raffles start with. Each raffle gets a copy, so
/// changing them changes no existing raffle.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prize {
    pub id: String,
    pub name: String,
    pub sponsor: String,
    pub description: String,
    pub qty: u32,
    pub position: i64,
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
    /// What the raffle still needs before registration can open, by name;
    /// empty when it may.
    #[serde(default)]
    pub missing_for_open: Vec<String>,
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
    /// The event day has ended.
    pub past_event: bool,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AdminActions {
    /// Where starting the draw stands, while it can start (or has).
    pub start: Option<StartStep>,
    /// Whether the raffle has a draw console: its entrant list is frozen.
    pub draw: bool,
    /// Whether the winners can be exported.
    pub export: bool,
    /// Whether "erase personal data now" applies: finished, with personal
    /// data left (a purge that stopped partway included).
    pub purge: bool,
    /// Whether the fixed settings (the additional rules, the event date, the
    /// retention period, the privacy text and link) can still change: not
    /// once an entrant has accepted them.
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
}

fn yes() -> bool {
    true
}

/// Where starting a raffle's draw stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartStep {
    /// Registration can be frozen and the draw started.
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
}

fn verify_format() -> u32 {
    VERIFY_FORMAT
}

/// Current organizer identity, and what the admin must know before it
/// shows the raffles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Me {
    pub owner_id: String,
    pub dev_auth: bool,
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
