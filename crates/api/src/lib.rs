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
    /// Extra rules text appended to the standard rules.
    pub rules: String,
    pub event_date: String,
    pub test_mode: bool,
}

/// What the organizer's new raffles start with. Each raffle gets a copy, so
/// changing them changes no existing raffle.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RaffleDefaults {
    pub organizer_name: String,
    pub contact_email: String,
    /// Extra rules text appended to the standard rules.
    pub rules: String,
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
    pub claim_code: String,
    pub marketing: bool,
    pub created_at: i64,
    pub delivered: bool,
    /// Whether their prize can be marked delivered (or not): they claimed it.
    #[serde(default)]
    pub deliverable: bool,
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
    /// Whether the additional rules can still change: not once an entrant
    /// has accepted them.
    #[serde(default)]
    pub edit_rules: bool,
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

/// Everything needed to independently verify a raffle's draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyData {
    pub title: String,
    pub state: RaffleState,
    /// The raffle's canonical manifest, exactly as hashed.
    pub manifest: String,
    pub manifest_hash: String,
    pub events: Vec<Event>,
    /// Entrant ID → display name (until personal data is purged).
    pub names: Vec<(String, String)>,
}

/// Current organizer identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Me {
    pub owner_id: String,
    pub dev_auth: bool,
}
