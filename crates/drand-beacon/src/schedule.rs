use core::num::NonZeroU32;

/// Round timing of a chain. Pure: callers pass the current time.
///
/// Round 1 is published at `genesis_time`, round `r` at
/// `genesis_time + (r - 1) * period`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    genesis_time: i64,
    period: NonZeroU32,
}

impl Schedule {
    /// A schedule from genesis time (unix seconds) and period.
    pub const fn new(genesis_time: i64, period: NonZeroU32) -> Self {
        Self {
            genesis_time,
            period,
        }
    }

    /// Unix time of round 1.
    pub const fn genesis_time(&self) -> i64 {
        self.genesis_time
    }

    /// Seconds between rounds.
    pub const fn period(&self) -> NonZeroU32 {
        self.period
    }

    /// The latest round due at or before `unix_secs`; `None` before genesis.
    pub fn round_at(&self, unix_secs: i64) -> Option<u64> {
        let elapsed = i128::from(unix_secs) - i128::from(self.genesis_time);
        if elapsed < 0 {
            return None;
        }
        let round = elapsed / i128::from(self.period.get()) + 1;
        u64::try_from(round).ok()
    }

    /// [`Schedule::round_at`] for a time in unix milliseconds.
    pub fn round_at_ms(&self, unix_ms: i64) -> Option<u64> {
        self.round_at(unix_ms.div_euclid(1000))
    }

    /// Unix time at which `round` is due; `None` for round 0 or on overflow.
    pub fn round_time(&self, round: u64) -> Option<i64> {
        let offset = round
            .checked_sub(1)?
            .checked_mul(u64::from(self.period.get()))?;
        self.genesis_time.checked_add(i64::try_from(offset).ok()?)
    }

    /// [`Schedule::round_time`] in unix milliseconds.
    pub fn round_time_ms(&self, round: u64) -> Option<i64> {
        self.round_time(round)?.checked_mul(1000)
    }

    /// The first round due strictly after `unix_secs` (`round_at(t) + 1`, or 1
    /// before genesis).
    ///
    /// Its randomness is unknown at `unix_secs`, assuming fewer than a threshold
    /// of drand nodes collude and the local clock is right. Leave at least one
    /// period of margin when committing to a round.
    pub fn round_after(&self, unix_secs: i64) -> u64 {
        self.round_at(unix_secs).map_or(1, |r| r.saturating_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quicknet() -> Schedule {
        Schedule::new(1_692_803_367, NonZeroU32::new(3).unwrap())
    }

    #[test]
    fn boundaries() {
        let s = quicknet();
        let g = s.genesis_time();
        assert_eq!(s.round_at(g - 1), None);
        assert_eq!(s.round_at(g), Some(1));
        assert_eq!(s.round_at(g + 2), Some(1));
        assert_eq!(s.round_at(g + 3), Some(2));
        assert_eq!(s.round_time(0), None);
        assert_eq!(s.round_time(1), Some(g));
        assert_eq!(s.round_time(2), Some(g + 3));
        assert_eq!(s.round_after(g - 100), 1);
        assert_eq!(s.round_after(g), 2);
        assert_eq!(s.round_at_ms(g * 1000 - 1), None);
        assert_eq!(s.round_at_ms(g * 1000 + 2999), Some(1));
        assert_eq!(s.round_time(u64::MAX), None);
    }
}
