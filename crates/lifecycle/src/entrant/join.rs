use crate::RaffleState;

/// Why someone can't join on their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRefusal {
    /// Registration isn't open.
    Closed,
    /// They didn't accept the rules.
    Rules,
}

/// Whether someone may join a raffle in `state` on their own, having
/// accepted the rules or not, before or past its erasure date
/// (`past_erase_date`). Past the date the raffle is closed to them: the form
/// would promise erasure on a day gone by, and the raffle expires within the
/// hour (the organizer can still freeze it meanwhile, which keeps the list).
/// Their name and email are checked on their own
/// ([`display_name`](super::display_name), [`Email::parse`](super::Email::parse)).
pub fn may_join(
    state: RaffleState,
    past_erase_date: bool,
    accepted_rules: bool,
) -> Result<(), JoinRefusal> {
    if !state.joinable() || past_erase_date {
        return Err(JoinRefusal::Closed);
    }
    if !accepted_rules {
        return Err(JoinRefusal::Rules);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use RaffleState as S;

    #[test]
    fn joining() {
        assert_eq!(may_join(S::Open, false, true), Ok(()));
        assert_eq!(may_join(S::Open, false, false), Err(JoinRefusal::Rules));
        assert_eq!(
            may_join(S::Open, true, true),
            Err(JoinRefusal::Closed),
            "past the erasure date, registration is closed"
        );
        for s in S::ALL.into_iter().filter(|s| *s != S::Open) {
            assert_eq!(may_join(s, false, true), Err(JoinRefusal::Closed), "{s:?}");
        }
    }
}
