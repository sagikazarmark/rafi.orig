//! Types shared between the server and the browser.

use rafi_core::{
    raffle::RaffleState,
    state::{DrawMode, Event, Outcome, UnitStatus},
};
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
    /// Whether the raffle has a live channel to subscribe to (the participant
    /// view, for the reveal). `false` on servers without one: then clients
    /// only poll.
    #[serde(default)]
    pub channel: bool,
    /// The organizer ended live updates.
    #[serde(default)]
    pub live_ended: bool,
    pub raffle_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub lang: String,
    pub state: RaffleState,
    pub join_code: String,
    pub join_url: String,
    pub entrant_count: usize,
    pub recent_names: Vec<String>,
    pub prizes: Vec<PrizeView>,
    pub units: Vec<UnitView>,
    pub current: Option<CurrentDraw>,
    /// Draws in chronological order.
    pub draws: Vec<DrawView>,
    pub last_failure: Option<String>,
    pub list_hash: Option<String>,
    pub server_now_ms: u64,
    pub test_mode: bool,
    /// Human-readable description of the draw engine (e.g. "restate", "restate (mock drand)").
    pub engine: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrizeView {
    pub item_id: String,
    pub name: String,
    pub sponsor: String,
    pub qty: u32,
    pub left: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitView {
    pub unit_id: String,
    pub item_id: String,
    pub item_name: String,
    pub status: UnitStatus,
    pub entrant_id: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurrentDraw {
    pub k: u64,
    pub unit_id: String,
    pub item_name: String,
    pub round: u64,
    /// Unix time (seconds) the round is published.
    pub round_time: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawView {
    pub k: u64,
    pub unit_id: String,
    pub item_name: String,
    pub entrant_id: String,
    pub display_name: String,
    pub outcome: Outcome,
    pub pass: u8,
    pub mode: DrawMode,
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
    /// "en" or "hu".
    pub lang: String,
    pub organizer_name: String,
    pub contact_email: String,
    /// Extra rules text appended to the standard rules.
    pub rules: String,
    pub event_date: String,
    pub min_age: u32,
    pub test_mode: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub sponsor: String,
    pub description: String,
    pub qty: u32,
    pub position: i64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ItemInput {
    /// `None` creates a new item.
    pub id: Option<String>,
    pub name: String,
    pub sponsor: String,
    pub description: String,
    pub qty: u32,
    pub position: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntrantSource {
    #[serde(rename = "self")]
    SelfService,
    Manual,
    Fake,
}

impl EntrantSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfService => "self",
            Self::Manual => "manual",
            Self::Fake => "fake",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "manual" => Self::Manual,
            "fake" => Self::Fake,
            _ => Self::SelfService,
        }
    }
}

/// An entrant as the organizer sees it (includes personal data).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminEntrant {
    pub id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub source: EntrantSource,
    pub hidden: bool,
    pub claim_code: String,
    pub marketing: bool,
    pub created_at: i64,
    pub delivered: bool,
    pub purged: bool,
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
    pub items: Vec<Item>,
    pub entrants: Vec<AdminEntrant>,
    pub view: RaffleView,
    pub public_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
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
    /// The canonical commit document, exactly as hashed.
    pub commit_doc: String,
    pub list_hash: String,
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

pub fn now_unix_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys_now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_default()
    }
}

#[cfg(target_arch = "wasm32")]
fn js_sys_now() -> u64 {
    #[cfg(feature = "web")]
    {
        js_sys::Date::now() as u64
    }
    #[cfg(not(feature = "web"))]
    {
        0
    }
}
