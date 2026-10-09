//! The draw engine's protocol: the commands it accepts (draws and decisions,
//! the wire format of the Restate handlers, shared by every engine and its
//! callers) and how a draw's beacon turned out. Refusals and their classification are in
//! [`error`](crate::error).
//!
//! The engine owns each [`Drawing`](crate::drawing::Drawing); the
//! rules it applies live on the drawing ([`Drawing::open_or_check`],
//! [`Drawing::conclude`], [`Drawing::decide`]), so every engine (the
//! Restate virtual object, the local engine) behaves the same
//! and only does I/O.
//!
//! [`Drawing::open_or_check`]: crate::drawing::Drawing::open_or_check
//! [`Drawing::conclude`]: crate::drawing::Drawing::conclude
//! [`Drawing::decide`]: crate::drawing::Drawing::decide

use serde::{Deserialize, Serialize};

use crate::{drand::VerifiedBeacon, drawing::Outcome, id::ItemId};

/// A command for the draw engine: a draw, or a decision. They differ in how
/// they run: a draw is *accepted* and settles later (its round is in the
/// future), a decision is *applied* when the call returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Draw(DrawRequest),
    Decide(Decision),
}

impl From<DrawRequest> for Command {
    fn from(request: DrawRequest) -> Self {
        Self::Draw(request)
    }
}

impl From<Decision> for Command {
    fn from(decision: Decision) -> Self {
        Self::Decide(decision)
    }
}

/// Draws a winner for one item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawRequest {
    pub item: ItemId,
}

/// The organizer's decision about a drawing: every command but a draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Decision {
    /// Records the outcome of a drawn item.
    Resolve { item: ItemId, outcome: Outcome },
    /// Withdraws an item that will not be given away.
    Void { item: ItemId },
    /// Abandons the pending draw of this item without its committed beacon.
    /// Logged publicly; the verifier shows what the round would have picked.
    Abandon { item: ItemId },
    /// Ends the drawing.
    Finish,
}

/// How the beacon for a pending draw turned out: the input of
/// [`Drawing::conclude`](crate::drawing::Drawing::conclude).
#[derive(Debug, Clone)]
pub enum BeaconOutcome {
    /// The committed round's beacon, verified against the engine's chain.
    Verified(VerifiedBeacon),
    /// The beacon could not be had (relays down or withholding it).
    Unavailable(String),
    /// The draw was interrupted (cancelled, or a step failed) before its beacon
    /// arrived. The reason is logged as is.
    Interrupted(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_is_stable() {
        let json = serde_json::to_string(&DrawRequest { item: "u1".into() }).unwrap();
        assert_eq!(json, r#"{"item":"u1"}"#);
        let json = serde_json::to_string(&Decision::Resolve {
            item: "u1".into(),
            outcome: Outcome::NotHere,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"cmd":"resolve","item":"u1","outcome":"not_here"}"#
        );
        assert_eq!(
            serde_json::to_string(&Decision::Abandon { item: "u1".into() }).unwrap(),
            r#"{"cmd":"abandon","item":"u1"}"#
        );
        assert_eq!(
            serde_json::to_string(&Decision::Finish).unwrap(),
            r#"{"cmd":"finish"}"#
        );
    }
}
