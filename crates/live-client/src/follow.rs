//! How a client follows a raffle: the server says, from what its deployment
//! provides, and the client picks the matching strategy.

use serde::{Deserialize, Serialize};

/// The poll interval when the server doesn't name one, in milliseconds.
pub const DEFAULT_POLL_MS: u32 = 2_500;

/// How a client follows a raffle: keeps its view of it fresh. Serialized
/// with a `kind` tag (`{"kind":"partyline"}`, `{"kind":"poll","every_ms":2500}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Follow {
    /// Subscribe to the raffle's [`ParticipantChannel`](crate::ParticipantChannel)
    /// over a partyline WebSocket at `/partyline/raffle/{id}`.
    Partyline {
        /// The server's origin (`https://…`), when it isn't the page's own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
    },
    /// No push: fetch the view every `every_ms` milliseconds.
    Poll { every_ms: u32 },
    /// A way this build doesn't know (the server is newer): the client
    /// polls at [`DEFAULT_POLL_MS`], which every server answers.
    #[serde(other)]
    Unknown,
}

impl Follow {
    /// Polling at [`DEFAULT_POLL_MS`].
    pub fn poll() -> Self {
        Self::Poll {
            every_ms: DEFAULT_POLL_MS,
        }
    }

    /// How often a client polls the view, or `None` when it subscribes instead.
    pub fn poll_every_ms(&self) -> Option<u32> {
        match self {
            Self::Partyline { .. } => None,
            Self::Poll { every_ms } => Some((*every_ms).max(250)),
            Self::Unknown => Some(DEFAULT_POLL_MS),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_by_kind() {
        let json = |t: &Follow| serde_json::to_string(t).unwrap();
        assert_eq!(
            json(&Follow::Partyline { base_url: None }),
            r#"{"kind":"partyline"}"#
        );
        assert_eq!(json(&Follow::poll()), r#"{"kind":"poll","every_ms":2500}"#);
        let newer: Follow = serde_json::from_str(r#"{"kind":"sse","url":"/hub"}"#).unwrap();
        assert_eq!(newer.poll_every_ms(), Some(DEFAULT_POLL_MS));
        assert_eq!(
            newer,
            Follow::Unknown,
            "a kind from a newer server falls back to polling"
        );
        let party: Follow =
            serde_json::from_str(r#"{"kind":"partyline","base_url":"https://x"}"#).unwrap();
        assert_eq!(party.poll_every_ms(), None);
    }
}
