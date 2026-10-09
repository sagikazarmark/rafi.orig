//! The draw engine's protocol: the commands it accepts (the wire format of the
//! Restate handlers, shared by every engine and its callers) and how a draw's
//! beacon turned out. Refusals and their classification are in
//! [`error`](crate::error).
//!
//! The engine owns each raffle's [`DrawState`](crate::state::DrawState); the
//! rules it applies live on the state ([`DrawState::freeze_or_check`],
//! [`DrawState::conclude`], [`DrawState::apply`]), so every engine (the
//! `Raffle` Restate virtual object, the local test runner) behaves the same
//! and only does I/O.
//!
//! [`DrawState::freeze_or_check`]: crate::state::DrawState::freeze_or_check
//! [`DrawState::conclude`]: crate::state::DrawState::conclude
//! [`DrawState::apply`]: crate::state::DrawState::apply

use serde::{Deserialize, Serialize};

use crate::{Hash, drand::VerifiedBeacon, state::Outcome};

/// A command for the draw engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Draws a winner for one prize unit (`offline`: server randomness, not verifiable).
    Draw { unit_id: String, offline: bool },
    /// Records the organizer's decision about a drawn unit.
    Resolve { unit_id: String, outcome: Outcome },
    /// Withdraws a unit that will not be given away.
    Void { unit_id: String },
    /// Abandons the pending draw of this unit without its committed beacon.
    /// Logged publicly; the verifier shows what the round would have picked.
    Abandon { unit_id: String },
    /// Ends the raffle.
    Finish,
}

/// How the beacon for a pending draw turned out: the input of
/// [`DrawState::conclude`](crate::state::DrawState::conclude).
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
            unit_id: "u1".into(),
            offline: false,
        })
        .unwrap();
        assert_eq!(json, r#"{"cmd":"draw","unit_id":"u1","offline":false}"#);
        let json = serde_json::to_string(&Command::Resolve {
            unit_id: "u1".into(),
            outcome: Outcome::NotHere,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"cmd":"resolve","unit_id":"u1","outcome":"not_here"}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::Abandon {
                unit_id: "u1".into()
            })
            .unwrap(),
            r#"{"cmd":"abandon","unit_id":"u1"}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::Finish).unwrap(),
            r#"{"cmd":"finish"}"#
        );
    }
}
