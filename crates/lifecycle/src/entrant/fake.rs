use crate::RaffleState;

/// The most fake entrants added at once.
pub const MAX_FAKES: u32 = 500;

/// Why fake entrants can't be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakesRefusal {
    /// Only a raffle in test mode takes fake entrants.
    NotTestMode,
    /// The entrant list is frozen.
    Frozen,
}

/// How many of `requested` fake entrants (rehearsal entrants) a raffle in
/// `state` takes: only in test mode, while its entrants can change, and at
/// most [`MAX_FAKES`] at once.
pub fn fakes(test_mode: bool, state: RaffleState, requested: u32) -> Result<u32, FakesRefusal> {
    if !test_mode {
        return Err(FakesRefusal::NotTestMode);
    }
    if !state.entrants_editable() {
        return Err(FakesRefusal::Frozen);
    }
    Ok(requested.min(MAX_FAKES))
}

#[cfg(test)]
mod tests {
    use super::*;
    use RaffleState as S;

    #[test]
    fn fake_entrants() {
        assert_eq!(fakes(true, S::Open, 3), Ok(3));
        assert_eq!(fakes(true, S::Draft, MAX_FAKES + 1), Ok(MAX_FAKES));
        assert_eq!(fakes(false, S::Open, 3), Err(FakesRefusal::NotTestMode));
        assert_eq!(fakes(true, S::Frozen, 3), Err(FakesRefusal::Frozen));
    }
}
