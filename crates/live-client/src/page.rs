//! The participant page's embedded state: what the server writes into
//! `#rafi-config`, and the phone (the WebAssembly, or `fallback.js`) reads.

use serde::{Deserialize, Serialize};

use crate::{CardStrings, Follow, ParticipantView, Phase};

/// What the server embeds in `#rafi-config`. One type for both sides, so
/// they can't disagree; `fallback.js` reads the same fields (a test here
/// checks it reads no others).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// How to follow the raffle: subscribe to its partyline channel
    /// (`/partyline/raffle/{id}`), or poll `participant_url`.
    pub follow: Follow,
    /// The join form is on the page and needs a proof-of-work.
    #[serde(default)]
    pub needs_pow: bool,
    #[serde(default = "challenge_url")]
    pub challenge_url: String,
    #[serde(default)]
    pub screen_on: String,
    #[serde(default)]
    pub keep_screen_on: String,
    /// Generated names the join form's 🎲 button fills in, one after the
    /// other.
    #[serde(default)]
    pub name_ideas: Vec<String>,
}

impl PageConfig {
    /// Where the proof-of-work challenge comes from.
    pub const CHALLENGE_URL: &str = "/api/challenge";
}

fn challenge_url() -> String {
    PageConfig::CHALLENGE_URL.into()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    const FALLBACK: &str = include_str!("../../../apps/participant/public/fallback.js");

    fn config() -> PageConfig {
        PageConfig {
            raffle_id: "r1".into(),
            entrant_id: Some("e1".into()),
            claim_code: Some("ABCD".into()),
            page_state: Phase::Open,
            view: ParticipantView::new(1, Phase::Open, None, [], 2),
            strings: CardStrings::default(),
            participant_url: "/api/r/s/participant".into(),
            server_now_ms: 1,
            follow: Follow::poll(),
            needs_pow: true,
            challenge_url: challenge_url(),
            screen_on: "on".into(),
            keep_screen_on: "keep".into(),
            name_ideas: vec!["Brave Otter".into(), "Calm Koala".into()],
        }
    }

    #[test]
    fn round_trips() {
        let json = serde_json::to_string(&config()).unwrap();
        assert_eq!(serde_json::from_str::<PageConfig>(&json).unwrap(), config());
    }

    /// Every `cfg.<field>` the fallback reads is a field the server writes.
    #[test]
    fn the_fallback_reads_only_what_the_server_writes() {
        let written: BTreeSet<String> = serde_json::to_value(config())
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let read: BTreeSet<String> = FALLBACK
            .match_indices("cfg.")
            .map(|(i, _)| {
                FALLBACK[i + 4..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect()
            })
            .collect();
        let unknown: Vec<_> = read.difference(&written).collect();
        assert!(unknown.is_empty(), "fallback.js reads {unknown:?}");
        assert!(read.len() > 5, "found the reads: {read:?}");
    }

    /// The fallback draws the same card as [`Card`](crate::CARD).
    #[test]
    fn the_fallback_uses_the_card_classes() {
        assert!(FALLBACK.contains(&format!("\"{}\"", crate::CARD)));
        assert!(FALLBACK.contains(&format!("\"{}\"", crate::WON)));
    }
}
