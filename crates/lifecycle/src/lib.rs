//! A raffle's lifecycle in the app and the rules that hang on it: how it
//! starts ([`start`]) and follows the raffle as drawn ([`raffle`]), who its
//! entrants are ([`entrant`]), what its settings may hold ([`settings`]) and
//! what phones see of it ([`live`]).
//! The raffle itself, from its manifest to its finish, is `rafi-raffle`'s,
//! which knows nothing of the app.
//!
//! Everything here is pure: the app and the store ask these rules, and keep
//! none of their own.

pub mod dates;
pub mod entrant;
pub mod live;
pub mod raffle;
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
    /// The entrant list is frozen and published in the raffle's manifest;
    /// drawing can start.
    Frozen,
    /// At least one draw happened.
    Drawing,
    Finished,
    /// Personal data has been removed.
    Purged,
}

/// An organizer's request to move a raffle through its lifecycle; the rules
/// may refuse it ([`RaffleState::apply`]). Drawing and finishing are not
/// commands: the lifecycle follows the raffle there ([`raffle::lifecycle`]),
/// in the states that [follow it](RaffleState::follows_raffle).
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

    /// Whether the raffle expires when its erasure date passes: it was never
    /// frozen, so there is no manifest and nothing to keep verifiable. Its
    /// entrants are deleted outright and it becomes [`Purged`](Self::Purged).
    /// Not a command: no organizer asks for it; housekeeping does it.
    pub fn expires(self) -> bool {
        !self.frozen_or_later()
    }

    /// The state an expired raffle ends in.
    pub fn expire(self) -> Option<Self> {
        self.expires().then_some(Self::Purged)
    }

    /// Whether the lifecycle follows the raffle ([`raffle::lifecycle`]):
    /// from the freeze until the raffle finishes. A stored raffle moves the
    /// lifecycle only in these states.
    pub fn follows_raffle(self) -> bool {
        matches!(self, Self::Frozen | Self::Drawing)
    }

    /// Whether prizes and their quantities can still change.
    pub fn prizes_editable(self) -> bool {
        matches!(self, Self::Draft | Self::Ready | Self::Open | Self::Closed)
    }

    /// Whether the organizer can add, remove or edit entrants. The same states
    /// as [`prizes_editable`](Self::prizes_editable) today: both end with the
    /// freeze, which fixes the entrants and the prizes alike.
    pub fn entrants_editable(self) -> bool {
        matches!(self, Self::Draft | Self::Ready | Self::Open | Self::Closed)
    }

    /// Whether the raffle can switch test mode on or off: not once its
    /// entrant list is frozen.
    pub fn test_mode_editable(self) -> bool {
        self.entrants_editable()
    }

    /// How an entrant is removed, if they can be: deleted while the list can
    /// change; once the raffle is over, or its erasure date has passed
    /// (`past_erase_date`), their ID stays in the published list and only
    /// their personal data is erased. In between (from the freeze until the
    /// raffle finishes or the date passes) nobody is removed: sharing their
    /// entry is part of the rules they accepted. The organizer can still hide
    /// or rename them.
    pub fn removal(self, past_erase_date: bool) -> Option<entrant::Removal> {
        if self.entrants_editable() {
            Some(entrant::Removal::Delete)
        } else if self.personal_data_erasable(past_erase_date) {
            Some(entrant::Removal::Erase)
        } else {
            None
        }
    }

    /// Whether entrants' personal data can be erased, one entrant at a time:
    /// once the raffle is over, or once its erasure date has passed
    /// (`past_erase_date`) and the list is frozen.
    pub fn personal_data_erasable(self, past_erase_date: bool) -> bool {
        self.is_over() || (self.frozen_or_later() && past_erase_date)
    }

    /// Whether the entrant list is frozen: from the freeze on, the raffle
    /// is (or is getting) opened, with its draw log and winners to export.
    pub fn frozen_or_later(self) -> bool {
        matches!(
            self,
            Self::Frozen | Self::Drawing | Self::Finished | Self::Purged
        )
    }

    /// Whether participants can join on their own.
    pub fn joinable(self) -> bool {
        self == Self::Open
    }

    /// Whether the public screens can show the raffle.
    pub fn is_public(self) -> bool {
        self != Self::Draft
    }

    /// Whether the join code still leads to the raffle. A purged raffle keeps
    /// its code but answers nothing to it: its entrants' tokens are erased,
    /// so no phone can be recognized, and the page would show only that the
    /// raffle is over. Not answering keeps the codes that can be guessed to
    /// the raffles still inside their hold, and the raffle stays reachable by
    /// its slug (the projector, the verification page).
    pub fn answers_join_code(self) -> bool {
        self.is_public() && self != Self::Purged
    }

    /// Whether draws can happen.
    pub fn drawable(self) -> bool {
        matches!(self, Self::Frozen | Self::Drawing)
    }

    /// Whether screens follow the raffle live: public, and not over yet (its
    /// view changes on its own).
    pub fn is_live(self) -> bool {
        self.is_public() && !self.is_over()
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
    fn removing_an_entrant() {
        use crate::entrant::Removal;
        for s in S::ALL {
            let want = match s {
                S::Draft | S::Ready | S::Open | S::Closed => Some(Removal::Delete),
                S::Frozen | S::Drawing => None,
                S::Finished | S::Purged => Some(Removal::Erase),
            };
            assert_eq!(s.removal(false), want, "{s:?} before the erasure date");
            let want_past = match s {
                S::Draft | S::Ready | S::Open | S::Closed => Some(Removal::Delete),
                S::Frozen | S::Drawing | S::Finished | S::Purged => Some(Removal::Erase),
            };
            assert_eq!(s.removal(true), want_past, "{s:?} past the erasure date");
        }
    }

    #[test]
    fn never_frozen_raffles_expire() {
        for s in S::ALL {
            let want = match s {
                S::Draft | S::Ready | S::Open | S::Closed => Some(S::Purged),
                S::Frozen | S::Drawing | S::Finished | S::Purged => None,
            };
            assert_eq!(s.expire(), want, "{s:?}");
        }
    }

    #[test]
    fn the_join_code_answers_while_the_raffle_is_public_and_not_purged() {
        for s in S::ALL {
            let want = !matches!(s, S::Draft | S::Purged);
            assert_eq!(s.answers_join_code(), want, "{s:?}");
            assert!(
                !s.answers_join_code() || s.is_public(),
                "{s:?}: a code never leads to what the public can't see"
            );
        }
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
