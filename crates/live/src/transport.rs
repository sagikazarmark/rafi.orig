//! How a client follows a raffle live: the server says which transport its
//! deployment provides, and the client picks the matching strategy.

use serde::{Deserialize, Serialize};

/// The poll interval when the server doesn't name one, in milliseconds.
pub const DEFAULT_POLL_MS: u32 = 2_500;

/// How clients get live updates of a raffle. Serialized with a `kind` tag
/// (`{"kind":"partyline"}`, `{"kind":"poll","every_ms":2500}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveTransport {
    /// Subscribe to the raffle's [`RaffleLive`](crate::RaffleLive) channel over
    /// a partyline WebSocket at `/partyline/raffle/{id}`.
    Partyline {
        /// The server's origin (`https://…`), when it isn't the page's own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
    },
    /// No push: fetch the view every `every_ms` milliseconds.
    Poll { every_ms: u32 },
    /// Server-sent events at `url` (for example a Mercure hub). Reserved: no
    /// client implements it yet, so clients poll instead (see
    /// [`LiveTransport::poll_every_ms`]).
    Sse { url: String },
}

impl LiveTransport {
    /// Polling at [`DEFAULT_POLL_MS`].
    pub fn poll() -> Self {
        Self::Poll {
            every_ms: DEFAULT_POLL_MS,
        }
    }

    /// How often a client polls the view, or `None` when it subscribes instead.
    ///
    /// SSE has no client yet: until it does, clients poll at
    /// [`DEFAULT_POLL_MS`].
    pub fn poll_every_ms(&self) -> Option<u32> {
        match self {
            Self::Partyline { .. } => None,
            Self::Poll { every_ms } => Some((*every_ms).max(250)),
            Self::Sse { .. } => Some(DEFAULT_POLL_MS),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_by_kind() {
        let json = |t: &LiveTransport| serde_json::to_string(t).unwrap();
        assert_eq!(
            json(&LiveTransport::Partyline { base_url: None }),
            r#"{"kind":"partyline"}"#
        );
        assert_eq!(
            json(&LiveTransport::poll()),
            r#"{"kind":"poll","every_ms":2500}"#
        );
        let sse: LiveTransport = serde_json::from_str(r#"{"kind":"sse","url":"/hub"}"#).unwrap();
        assert_eq!(
            sse.poll_every_ms(),
            Some(DEFAULT_POLL_MS),
            "SSE polls for now"
        );
        let party: LiveTransport =
            serde_json::from_str(r#"{"kind":"partyline","base_url":"https://x"}"#).unwrap();
        assert_eq!(party.poll_every_ms(), None);
    }
}
