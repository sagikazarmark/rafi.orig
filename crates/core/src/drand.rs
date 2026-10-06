//! The drand side of a draw: which chain, which round.
//!
//! Fetching and BLS verification live in the `drand-beacon` crate; this module
//! re-exports what the domain model needs and keeps Rafi's round rule.

pub use drand_beacon::{
    Beacon, ChainHash, ChainInfo, Error as BeaconError, Schedule, VerifiedBeacon, Verifier, chains,
};

/// The chain every raffle used before the chain became configurable (quicknet).
pub const QUICKNET_CHAIN: &str = "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971";

/// The round a draw starting at `now_unix` commits to: the latest round due at
/// `now + lead_secs`.
///
/// With `lead_secs >= period` the round is not published yet at `now` (with
/// `lead_secs >= 2 × period`, by at least a full period). `lead_secs = 0`
/// commits to the round that is already out, which only test chains allow.
pub fn target_round(schedule: &Schedule, now_unix: u64, lead_secs: u64) -> u64 {
    let t = i64::try_from(now_unix.saturating_add(lead_secs)).unwrap_or(i64::MAX);
    schedule.round_at(t).unwrap_or(1)
}

/// Unix time at which `round` is published (0 if it does not exist).
pub fn round_time(schedule: &Schedule, round: u64) -> u64 {
    schedule
        .round_time(round)
        .and_then(|t| u64::try_from(t).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENESIS: u64 = 1_692_803_367;

    /// The quicknet-only rule this module used before chains were configurable.
    fn old_target_round(now: u64, lead: u64) -> u64 {
        let t = now + lead;
        if t < GENESIS {
            return 1;
        }
        (t - GENESIS) / 3 + 1
    }

    #[test]
    fn target_round_is_unchanged_for_quicknet() {
        let s = chains::quicknet().schedule();
        for now in GENESIS - 10..GENESIS + 40 {
            for lead in [0, 3, 6, 9] {
                assert_eq!(target_round(&s, now, lead), old_target_round(now, lead));
            }
        }
    }

    #[test]
    fn target_round_is_in_the_future_with_a_lead() {
        let s = chains::quicknet().schedule();
        for now in GENESIS..GENESIS + 20 {
            let r = target_round(&s, now, 6);
            assert!(round_time(&s, r) > now);
            assert!(round_time(&s, r) <= now + 6);
        }
    }
}
