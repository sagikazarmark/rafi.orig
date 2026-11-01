//! A raffle's lifecycle in the app and the rules that hang on it: how it
//! starts ([`start`]) and follows the raffle as drawn ([`raffle`]), who its
//! entrants are ([`entrant`]) and how their names are shown ([`names`]), how
//! its prizes reach their winners ([`prize`]), what its settings may hold
//! ([`settings`]) and what phones see of it ([`live`]).
//! The raffle itself, from its manifest to its finish, is `rafi-raffle`'s,
//! which knows nothing of the app.
//!
//! Everything here is pure: the app and the store ask these rules, and keep
//! none of their own.

pub mod dates;
pub mod entrant;
pub mod live;
pub mod names;
pub mod prize;
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
    /// Without registration, the list is frozen from here.
    Ready,
    /// Registration is open. Never reached without registration.
    Open,
    /// Registration is closed (can be reopened, with registration on).
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

    /// The state after `command` in a raffle with or without registration
    /// (`registration`: whether people may join with the QR code), or `None`
    /// if the transition is not allowed.
    ///
    /// Without registration there is nothing to open: the raffle goes from
    /// Published ([`Ready`](Self::Ready)) straight to the freeze, and its
    /// entrants are the people the organizer added. With it, the list is
    /// frozen from Open or Closed only, so a freeze never cuts registration
    /// short before it opened. A raffle switched off while Closed freezes
    /// from there as one with registration does.
    pub fn apply(self, command: Command, registration: bool) -> Option<Self> {
        use Command as A;
        use RaffleState as S;
        Some(match (self, command) {
            (S::Draft, A::Publish) => S::Ready,
            (S::Ready, A::Unpublish) => S::Draft,
            (S::Ready | S::Closed, A::Open) if registration => S::Open,
            (S::Open, A::Close) => S::Closed,
            (S::Ready, A::Freeze) if !registration => S::Frozen,
            (S::Open | S::Closed, A::Freeze) => S::Frozen,
            // Purging again finishes a purge that stopped partway.
            (S::Finished | S::Purged, A::Purge) => S::Purged,
            _ => return None,
        })
    }

    /// The state a freeze that found nobody to draw from goes back to: `to`,
    /// the state it froze from (Published, Open or Closed, with registration
    /// or without). Not a command (organizers can't unfreeze): it undoes the
    /// freeze's first step ([`start::Start::Freeze`]), which a store without
    /// transactions can't roll back.
    pub fn unfreeze(self, to: Self) -> Option<Self> {
        let froze_from = [false, true]
            .into_iter()
            .any(|registration| to.apply(Command::Freeze, registration) == Some(Self::Frozen));
        (self == Self::Frozen && froze_from).then_some(to)
    }

    /// Whether the raffle expires when its erasure date passes: it was never
    /// frozen, so there is no manifest and nothing to keep verifiable. Its
    /// entrants are deleted outright and it becomes [`Purged`](Self::Purged).
    /// Not a command: no organizer asks for it; housekeeping does it.
    pub fn expires(self) -> bool {
        !self.frozen_or_later()
    }

    /// [`expires`](Self::expires), knowing whether the raffle has a manifest:
    /// a raffle frozen without one (a freeze that stopped after closing
    /// registration, which the organizer then left) has nothing to keep
    /// verifiable either, and would otherwise sit frozen for good.
    pub fn expires_with(self, has_manifest: bool) -> bool {
        self.expires() || (self == Self::Frozen && !has_manifest)
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

    /// Whether a prize's [`Delivery`](prize::Delivery) can change: until the
    /// raffle is over, past the freeze too. It isn't in the manifest (it
    /// changes nothing about who wins), and the organizer may only find out
    /// during the draw how a prize is handed over. It applies to the claims
    /// made after the change; once the raffle is over there are none left.
    pub fn prize_delivery_editable(self) -> bool {
        !self.is_over()
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

    /// Whether the raffle can switch registration (joining with the QR
    /// code) on or off: before the freeze, but not while registration is
    /// open, which would cut off the phones in the middle of the form (close
    /// it first). It isn't fixed with the rules: they describe both ways in
    /// whatever the setting, so nobody accepted anything that depends on it.
    pub fn registration_editable(self) -> bool {
        self.entrants_editable() && self != Self::Open
    }

    /// How an entrant is removed, if they can be: deleted while the list can
    /// change; once the raffle is over, or its erasure date has passed
    /// (`past_erase_date`), their ID stays in the published list and only
    /// their personal data is erased. In between (from the freeze until the
    /// raffle finishes or the date passes) nobody is removed: for someone who
    /// joined, sharing their entry is part of the rules they accepted, and
    /// someone the organizer added takes themselves off instead
    /// ([`objections_allowed`](Self::objections_allowed)). The organizer can
    /// still hide or rename anyone.
    pub fn removal(self, past_erase_date: bool) -> Option<entrant::Removal> {
        if self.entrants_editable() {
            Some(entrant::Removal::Delete)
        } else if self.personal_data_erasable(past_erase_date) {
            Some(entrant::Removal::Erase)
        } else {
            None
        }
    }

    /// Whether someone the organizer added can take themselves off
    /// ([`entrant::may_object`]): from the freeze until the raffle is over,
    /// when nobody can be removed ([`removal`](Self::removal)). Before it
    /// they are removed at their request (and stay out of later adds); once
    /// it is over, their data is erased on request like anyone's.
    pub fn objections_allowed(self) -> bool {
        self.drawable()
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

    /// Whether participants can join on their own: registration is open,
    /// which a raffle without registration never is
    /// ([`apply`](Self::apply)).
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
        let s = S::Draft.apply(A::Publish, true).unwrap();
        let s = s.apply(A::Open, true).unwrap();
        let s = s.apply(A::Close, true).unwrap();
        let s = s.apply(A::Open, true).unwrap();
        assert_eq!(s.apply(A::Freeze, true), Some(S::Frozen));
        assert_eq!(S::Finished.apply(A::Purge, true), Some(S::Purged));
        assert_eq!(
            S::Purged.apply(A::Purge, true),
            Some(S::Purged),
            "finishes a purge"
        );
    }

    #[test]
    fn without_registration_published_freezes() {
        let s = S::Draft.apply(A::Publish, false).unwrap();
        assert_eq!(s, S::Ready);
        assert_eq!(s.apply(A::Open, false), None, "nothing to open");
        assert_eq!(s.apply(A::Unpublish, false), Some(S::Draft));
        assert_eq!(s.apply(A::Freeze, false), Some(S::Frozen));
    }

    /// Every state and command, with registration and without: the setting
    /// changes opening and the freeze from Published, and nothing else.
    #[test]
    fn every_transition() {
        let commands = [
            A::Publish,
            A::Unpublish,
            A::Open,
            A::Close,
            A::Freeze,
            A::Purge,
        ];
        for s in S::ALL {
            for c in commands {
                for registration in [false, true] {
                    let want = match (s, c) {
                        (S::Draft, A::Publish) => Some(S::Ready),
                        (S::Ready, A::Unpublish) => Some(S::Draft),
                        (S::Ready | S::Closed, A::Open) => registration.then_some(S::Open),
                        (S::Open, A::Close) => Some(S::Closed),
                        (S::Ready, A::Freeze) => (!registration).then_some(S::Frozen),
                        (S::Open | S::Closed, A::Freeze) => Some(S::Frozen),
                        (S::Finished | S::Purged, A::Purge) => Some(S::Purged),
                        _ => None,
                    };
                    assert_eq!(
                        s.apply(c, registration),
                        want,
                        "{s:?} {c:?}, registration {registration}"
                    );
                }
            }
        }
    }

    #[test]
    fn unfreezing_only_undoes_a_freeze() {
        assert_eq!(S::Frozen.unfreeze(S::Closed), Some(S::Closed));
        assert_eq!(S::Frozen.unfreeze(S::Open), Some(S::Open));
        assert_eq!(
            S::Frozen.unfreeze(S::Ready),
            Some(S::Ready),
            "frozen from Published, without registration"
        );
        assert_eq!(S::Frozen.unfreeze(S::Draft), None);
        assert_eq!(S::Drawing.unfreeze(S::Closed), None);
        assert_eq!(S::Drawing.unfreeze(S::Ready), None);
    }

    /// Switchable before the freeze, except while registration is open.
    #[test]
    fn registration_switches_before_the_freeze_but_not_while_open() {
        for s in S::ALL {
            let want = matches!(s, S::Draft | S::Ready | S::Closed);
            assert_eq!(s.registration_editable(), want, "{s:?}");
        }
    }

    #[test]
    fn no_unfreezing() {
        for registration in [false, true] {
            assert_eq!(S::Frozen.apply(A::Open, registration), None);
            assert_eq!(S::Drawing.apply(A::Open, registration), None);
            assert_eq!(S::Draft.apply(A::Freeze, registration), None);
        }
        assert!(!S::Frozen.prizes_editable());
        assert!(!S::Frozen.entrants_editable());
    }

    #[test]
    fn a_prizes_delivery_changes_until_the_raffle_is_over() {
        for s in S::ALL {
            assert_eq!(s.prize_delivery_editable(), !s.is_over(), "{s:?}");
            assert!(
                !s.prizes_editable() || s.prize_delivery_editable(),
                "{s:?}: whatever can edit a prize can edit its delivery"
            );
        }
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

    /// Taking oneself off fills the gap removal leaves: from the freeze
    /// until the raffle is over, where (before the erasure date) nobody can
    /// be removed or erased.
    #[test]
    fn objections_between_the_freeze_and_the_end() {
        for s in S::ALL {
            let want = matches!(s, S::Frozen | S::Drawing);
            assert_eq!(s.objections_allowed(), want, "{s:?}");
            assert_eq!(
                s.objections_allowed(),
                s.removal(false).is_none(),
                "{s:?}: exactly where nobody is removed"
            );
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
    fn a_freeze_without_a_manifest_expires_too() {
        for s in S::ALL {
            assert_eq!(s.expires_with(true), s.expires(), "{s:?} with a manifest");
            assert_eq!(
                s.expires_with(false),
                s.expires() || s == S::Frozen,
                "{s:?} without one"
            );
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
