//! The raffle: its lifecycle and the rules that hang on it, how it starts
//! and uses a drawing ([`start`], [`drawing`]), who its entrants are
//! ([`entrant`]) and what its settings may hold ([`settings`]). The drawing
//! itself lives in `rafi-drawing`, which knows nothing of raffles.
//!
//! Everything here is pure: the app and the store ask these rules, and keep
//! none of their own.

pub mod drawing;
pub mod entrant;
pub mod settings;
pub mod start;

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

/// An organizer's request to move a raffle through its lifecycle; the rules
/// may refuse it ([`RaffleState::apply`]). Drawing and finishing are not
/// commands: the raffle follows its drawing there ([`drawing::lifecycle`]),
/// in the states that [follow it](RaffleState::follows_drawing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Publish,
    Unpublish,
    Open,
    Close,
    Freeze,
    Purge,
}

impl RaffleState {
    /// Every state, in lifecycle order.
    pub const ALL: [Self; 8] = [
        Self::Draft,
        Self::Ready,
        Self::Open,
        Self::Closed,
        Self::Frozen,
        Self::Drawing,
        Self::Finished,
        Self::Purged,
    ];

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
            // Purging again finishes a purge that stopped partway.
            (S::Finished | S::Purged, A::Purge) => S::Purged,
            _ => return None,
        })
    }

    /// The state a freeze that found nobody to draw from goes back to: `to`,
    /// the state it froze from. Not a command (organizers can't unfreeze): it
    /// undoes the freeze's first step ([`start::Start::Freeze`]), which a
    /// store without transactions can't roll back.
    pub fn unfreeze(self, to: Self) -> Option<Self> {
        (self == Self::Frozen && to.apply(Command::Freeze) == Some(Self::Frozen)).then_some(to)
    }

    /// Whether the raffle follows its drawing ([`drawing::lifecycle`]): from
    /// the freeze until the drawing finishes. A stored drawing moves only a
    /// raffle in these states.
    pub fn follows_drawing(self) -> bool {
        matches!(self, Self::Frozen | Self::Drawing)
    }

    /// Whether prizes and their quantities can still change.
    pub fn prizes_editable(self) -> bool {
        matches!(self, Self::Draft | Self::Ready | Self::Open | Self::Closed)
    }

    /// Whether the organizer can add, remove or edit entrants. The same states
    /// as [`prizes_editable`](Self::prizes_editable) today: both end with the
    /// freeze, which commits to the entrants and the prizes alike.
    pub fn entrants_editable(self) -> bool {
        matches!(self, Self::Draft | Self::Ready | Self::Open | Self::Closed)
    }

    /// Whether the raffle can switch test mode on or off: not once its
    /// entrant list is committed.
    pub fn test_mode_editable(self) -> bool {
        self.entrants_editable()
    }

    /// How an entrant is removed: deleted while the list can change; after
    /// the freeze their ID is in the published list, so only their personal
    /// data is erased.
    pub fn removal(self) -> entrant::Removal {
        if self.entrants_editable() {
            entrant::Removal::Delete
        } else {
            entrant::Removal::Erase
        }
    }

    /// Whether a prize can be marked delivered: once the raffle is published.
    pub fn deliverable(self) -> bool {
        self != Self::Draft
    }

    /// Whether participants can join on their own.
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

    /// Whether the raffle is under way: registration is open or the draw is
    /// (about to be) running.
    pub fn in_progress(self) -> bool {
        matches!(self, Self::Open | Self::Frozen | Self::Drawing)
    }

    /// Whether the raffle is over: its live updates may end.
    pub fn is_over(self) -> bool {
        matches!(self, Self::Finished | Self::Purged)
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
        assert_eq!(s.apply(A::Freeze), Some(S::Frozen));
        assert_eq!(S::Finished.apply(A::Purge), Some(S::Purged));
        assert_eq!(
            S::Purged.apply(A::Purge),
            Some(S::Purged),
            "finishes a purge"
        );
    }

    #[test]
    fn unfreezing_only_undoes_a_freeze() {
        assert_eq!(S::Frozen.unfreeze(S::Closed), Some(S::Closed));
        assert_eq!(S::Frozen.unfreeze(S::Open), Some(S::Open));
        assert_eq!(S::Frozen.unfreeze(S::Draft), None);
        assert_eq!(S::Drawing.unfreeze(S::Closed), None);
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
        for s in S::ALL {
            assert_eq!(S::parse(s.as_str()), Some(s));
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(json, format!("\"{}\"", s.as_str()), "serde and SQL agree");
        }
    }
}
