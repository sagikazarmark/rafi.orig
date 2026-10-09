//! Raffle lifecycle.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaffleState {
    /// Being prepared; not visible to the public.
    Draft,
    /// Published: the intro screen is visible, registration not yet open.
    Ready,
    /// Registration is open.
    Open,
    /// Registration is closed (can be reopened).
    Closed,
    /// The entrant list is frozen and committed; drawing can start.
    Frozen,
    /// At least one draw happened.
    Drawing,
    Finished,
    /// Personal data has been removed.
    Purged,
}

/// A request to move a raffle through its lifecycle; the rules may refuse it
/// ([`RaffleState::apply`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Publish,
    Unpublish,
    Open,
    Close,
    Freeze,
    StartDrawing,
    Finish,
    Purge,
}

impl RaffleState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Frozen => "frozen",
            Self::Drawing => "drawing",
            Self::Finished => "finished",
            Self::Purged => "purged",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "draft" => Self::Draft,
            "ready" => Self::Ready,
            "open" => Self::Open,
            "closed" => Self::Closed,
            "frozen" => Self::Frozen,
            "drawing" => Self::Drawing,
            "finished" => Self::Finished,
            "purged" => Self::Purged,
            _ => return None,
        })
    }

    /// The state after `command`, or `None` if the transition is not allowed.
    pub fn apply(self, command: Command) -> Option<Self> {
        use Command as A;
        use RaffleState as S;
        Some(match (self, command) {
            (S::Draft, A::Publish) => S::Ready,
            (S::Ready, A::Unpublish) => S::Draft,
            (S::Ready | S::Closed, A::Open) => S::Open,
            (S::Open, A::Close) => S::Closed,
            (S::Open | S::Closed, A::Freeze) => S::Frozen,
            (S::Frozen, A::StartDrawing) => S::Drawing,
            (S::Frozen | S::Drawing, A::Finish) => S::Finished,
            (S::Finished, A::Purge) => S::Purged,
            _ => return None,
        })
    }

    /// Whether prizes and their quantities can still change.
    pub fn prizes_editable(self) -> bool {
        matches!(self, Self::Draft | Self::Ready | Self::Open | Self::Closed)
    }

    /// Whether the organizer can add, remove or edit entrants.
    pub fn entrants_editable(self) -> bool {
        matches!(self, Self::Draft | Self::Ready | Self::Open | Self::Closed)
    }

    /// Whether attendees can join on their own.
    pub fn joinable(self) -> bool {
        self == Self::Open
    }

    /// Whether the public screens can show the raffle.
    pub fn is_public(self) -> bool {
        self != Self::Draft
    }

    /// Whether draws can happen.
    pub fn drawable(self) -> bool {
        matches!(self, Self::Frozen | Self::Drawing)
    }
}

#[cfg(test)]
mod tests {
    use super::{Command as A, RaffleState as S};

    #[test]
    fn happy_path() {
        let s = S::Draft.apply(A::Publish).unwrap();
        let s = s.apply(A::Open).unwrap();
        let s = s.apply(A::Close).unwrap();
        let s = s.apply(A::Open).unwrap();
        let s = s.apply(A::Freeze).unwrap();
        let s = s.apply(A::StartDrawing).unwrap();
        let s = s.apply(A::Finish).unwrap();
        assert_eq!(s.apply(A::Purge), Some(S::Purged));
    }

    #[test]
    fn no_unfreezing() {
        assert_eq!(S::Frozen.apply(A::Open), None);
        assert_eq!(S::Drawing.apply(A::Open), None);
        assert_eq!(S::Draft.apply(A::Freeze), None);
        assert!(!S::Frozen.prizes_editable());
        assert!(!S::Frozen.entrants_editable());
    }

    #[test]
    fn roundtrip_names() {
        for s in [
            S::Draft,
            S::Ready,
            S::Open,
            S::Closed,
            S::Frozen,
            S::Drawing,
            S::Finished,
            S::Purged,
        ] {
            assert_eq!(S::parse(s.as_str()), Some(s));
        }
    }
}
