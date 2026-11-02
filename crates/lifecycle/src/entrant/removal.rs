use serde::{Deserialize, Serialize};

use super::Source;
use crate::RaffleState;

/// How an entrant is removed ([`RaffleState::removal`](crate::RaffleState::removal)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Removal {
    /// Gone entirely: the entrant list isn't frozen yet.
    Delete,
    /// Their personal data is erased; their anonymous ID stays in the
    /// published list, so the draw stays verifiable.
    Erase,
}

/// Why an entrant can't take themselves off ([`may_object`]), or be put
/// back ([`may_unobject`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectionRefusal {
    /// The raffle isn't between the freeze and its end
    /// ([`RaffleState::objections_allowed`]): before the freeze they are
    /// removed at their request instead; once it is over, their data is
    /// erased on request.
    State,
    /// They joined themselves, accepting the rules, or are a fake
    /// ([`Source::may_object`]).
    Source,
    /// They claimed a prize: the result stands, and their data can be
    /// erased on request once the raffle is over.
    Claimed,
}

/// Whether an entrant from `source`, who claimed a prize or not, can take
/// themselves off a raffle in `state`: someone the organizer added, from
/// the freeze until the raffle is over. Their anonymous ID stays in the
/// manifest (removing it would break the verification); their name is
/// hidden, a draw that picks them can only be declined
/// ([`objected_may_be`]), and their data is erased when the raffle is over
/// rather than on the erasure date. Too late once they claimed a prize: a
/// claim is the draw's result, and isn't undone.
pub fn may_object(
    state: RaffleState,
    source: Source,
    claimed: bool,
) -> Result<(), ObjectionRefusal> {
    if !state.objections_allowed() {
        return Err(ObjectionRefusal::State);
    }
    if !source.may_object() {
        return Err(ObjectionRefusal::Source);
    }
    if claimed {
        return Err(ObjectionRefusal::Claimed);
    }
    Ok(())
}

/// Whether someone who took themselves off a raffle in `state` can be put
/// back (the organizer's mistake undone): until the raffle is over, when
/// their data is erased.
pub fn may_unobject(state: RaffleState) -> Result<(), ObjectionRefusal> {
    if state.objections_allowed() {
        Ok(())
    } else {
        Err(ObjectionRefusal::State)
    }
}

/// Whether a drawn winner who took themselves off may be resolved so:
/// declined only. They said they don't take part, so they claim nothing,
/// and "Not here" would record an absence that isn't what happened.
pub fn objected_may_be(resolution: rafi_raffle::Resolution) -> bool {
    resolution == rafi_raffle::Resolution::Declined
}

#[cfg(test)]
mod tests {
    use super::*;
    use RaffleState as S;

    /// Only someone the organizer added takes themselves off, between the
    /// freeze and the end, and not once they claimed a prize.
    #[test]
    fn taking_oneself_off() {
        for s in S::ALL {
            let want = if s.objections_allowed() {
                Ok(())
            } else {
                Err(ObjectionRefusal::State)
            };
            assert_eq!(may_object(s, Source::Manual, false), want, "{s:?}");
            assert_eq!(may_unobject(s), want, "{s:?}");
        }
        for s in [S::Frozen, S::Drawing] {
            assert_eq!(
                may_object(s, Source::SelfService, false),
                Err(ObjectionRefusal::Source),
                "a joiner accepted the rules"
            );
            assert_eq!(
                may_object(s, Source::Fake, false),
                Err(ObjectionRefusal::Source)
            );
            assert_eq!(
                may_object(s, Source::Manual, true),
                Err(ObjectionRefusal::Claimed),
                "a claim stands"
            );
        }
        assert_eq!(
            may_object(S::Open, Source::SelfService, true),
            Err(ObjectionRefusal::State),
            "the state first"
        );
        use rafi_raffle::Resolution as R;
        assert!(objected_may_be(R::Declined));
        assert!(!objected_may_be(R::Claimed));
        assert!(!objected_may_be(R::NotHere));
    }
}
