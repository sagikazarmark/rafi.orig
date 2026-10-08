//! The page's embedded state.

use rafi_live::{CardStrings, ParticipantView, Phase};
use serde::Deserialize;

/// What the server embeds in `#rafi-config`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PageConfig {
    pub raffle_id: String,
    /// The visitor's entrant, from their cookie.
    #[serde(default)]
    pub entrant_id: Option<String>,
    #[serde(default)]
    pub claim_code: Option<String>,
    /// The raffle's state when the page was rendered.
    pub page_state: Phase,
    /// The participant view at render time; the card starts from it.
    pub view: ParticipantView,
    pub strings: CardStrings,
    /// Where to fetch the current participant view (`/api/r/{slug}/participant`).
    pub participant_url: String,
    /// The server's clock at render time, Unix milliseconds.
    pub server_now_ms: u64,
    /// The join form is on the page and needs a proof-of-work.
    #[serde(default)]
    pub needs_altcha: bool,
    #[serde(default = "altcha_url")]
    pub altcha_url: String,
    #[serde(default)]
    pub screen_on: String,
    #[serde(default)]
    pub keep_screen_on: String,
}

fn altcha_url() -> String {
    "/api/altcha".into()
}

pub fn load() -> Option<PageConfig> {
    let json = web_sys::window()?
        .document()?
        .get_element_by_id("rafi-config")?
        .text_content()?;
    serde_json::from_str(&json).ok()
}
