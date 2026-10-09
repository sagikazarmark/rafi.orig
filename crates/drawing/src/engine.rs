//! The draw engine's protocol: the commands it accepts (the wire format of the
//! Restate handlers, shared by every engine and its callers) and how a draw's
//! beacon turned out. Refusals and their classification are in
//! [`error`](crate::error).
//!
//! The engine owns each [`Drawing`](crate::drawing::Drawing); the
//! rules it applies live on the state ([`Drawing::open_or_check`],
//! [`Drawing::conclude`], [`Drawing::apply`]), so every engine (the
//! Restate virtual object, the local engine) behaves the same
//! and only does I/O.
//!
//! [`Drawing::open_or_check`]: crate::drawing::Drawing::open_or_check
//! [`Drawing::conclude`]: crate::drawing::Drawing::conclude
//! [`Drawing::apply`]: crate::drawing::Drawing::apply

use serde::{Deserialize, Serialize};

use crate::{Hash, drand::VerifiedBeacon, drawing::Outcome};

/// A command for the draw engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Draws a winner for one item (`offline`: server randomness, not verifiable).
    Draw { item_id: String, offline: bool },
    /// Records the organizer's decision about a drawn item.
    Resolve { item_id: String, outcome: Outcome },
    /// Withdraws a item that will not be given away.
    Void { item_id: String },
    /// Abandons the pending draw of this item without its committed beacon.
    /// Logged publicly; the verifier shows what the round would have picked.
    Abandon { item_id: String },
    /// Ends the drawing.
    Finish,
}

/// How the beacon for a pending draw turned out: the input of
/// [`Drawing::conclude`](crate::drawing::Drawing::conclude).
#[derive(Debug, Clone)]
pub enum BeaconOutcome {
    /// The committed round's beacon, verified against the engine's chain.
    Verified(VerifiedBeacon),
    /// The organizer drew offline: server randomness, not publicly verifiable.
    Offline(Hash),
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
        let json = serde_json::to_string(&Command::Draw {
            item_id: "u1".into(),
            offline: false,
        })
        .unwrap();
        assert_eq!(json, r#"{"cmd":"draw","item_id":"u1","offline":false}"#);
        let json = serde_json::to_string(&Command::Resolve {
            item_id: "u1".into(),
            outcome: Outcome::NotHere,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"cmd":"resolve","item_id":"u1","outcome":"not_here"}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::Abandon {
                item_id: "u1".into()
            })
            .unwrap(),
            r#"{"cmd":"abandon","item_id":"u1"}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::Finish).unwrap(),
            r#"{"cmd":"finish"}"#
        );
    }
}
