//! What a raffle is asked to do: draws and decisions (with their wire format,
//! shared by every engine and its callers), and how a draw's beacon fetch
//! turned out. Refusals and their classification are in
//! [`error`](crate::error).
//!
//! The engine owns each [`Raffle`](crate::Raffle); the rules live on the
//! raffle ([`Raffle::open_or_check`], [`Raffle::announce`],
//! [`Raffle::settle`], [`Raffle::decide`]), so every engine behaves the same
//! and only does I/O.
//!
//! [`Raffle::open_or_check`]: crate::Raffle::open_or_check
//! [`Raffle::announce`]: crate::Raffle::announce
//! [`Raffle::settle`]: crate::Raffle::settle
//! [`Raffle::decide`]: crate::Raffle::decide

use serde::{Deserialize, Serialize};

use crate::{drand::VerifiedBeacon, id::ItemId};

/// A command for a raffle: a draw, or a decision. They differ in how they
/// run: a draw is *accepted* and settles later (its round is in the future),
/// a decision is *applied* when the call returns.
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

/// The organizer's decision about a drawn item's winner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// The winner took the item.
    Claimed,
    /// The winner was not there. A forfeit: the item is drawn again, and the
    /// winner is out of pass 2.
    NotHere,
    /// The winner turned the item down. A forfeit, like [`Self::NotHere`].
    Declined,
}

impl Resolution {
    /// Whether the winner gave up the item ([`Outcome::is_forfeit`](crate::Outcome::is_forfeit)): it is
    /// drawn again.
    pub fn is_forfeit(self) -> bool {
        crate::Outcome::from(self).is_forfeit()
    }
}

/// The organizer's decision about a raffle: every command but a draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Decision {
    /// Resolves a drawn item's winner.
    Resolve {
        item: ItemId,
        resolution: Resolution,
    },
    /// Withdraws an item that will not be given away.
    Void { item: ItemId },
    /// Abandons the pending draw of this item without its committed beacon.
    /// Logged publicly; the verifier shows what the round would have picked.
    Abandon { item: ItemId },
    /// Ends the raffle.
    Finish,
}

/// How fetching the beacon for a pending draw turned out: the input of
/// [`Raffle::settle`](crate::Raffle::settle).
#[derive(Debug, Clone)]
pub enum BeaconFetch {
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
        let json = serde_json::to_string(&DrawRequest {
            item: "item-1".into(),
        })
        .unwrap();
        assert_eq!(json, r#"{"item":"item-1"}"#);
        let json = serde_json::to_string(&Decision::Resolve {
            item: "item-1".into(),
            resolution: Resolution::NotHere,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"type":"resolve","item":"item-1","resolution":"not_here"}"#
        );
        assert_eq!(
            serde_json::to_string(&Decision::Abandon {
                item: "item-1".into()
            })
            .unwrap(),
            r#"{"type":"abandon","item":"item-1"}"#
        );
        assert_eq!(
            serde_json::to_string(&Decision::Finish).unwrap(),
            r#"{"type":"finish"}"#
        );
    }
}
