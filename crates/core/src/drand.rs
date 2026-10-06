//! drand quicknet round arithmetic and beacon checks.
//!
//! See <https://docs.drand.love/developer/API-v2/drand-http-api/>.

use serde::{Deserialize, Serialize};

use crate::{Hash, sha256};

/// drand quicknet chain hash.
pub const QUICKNET_CHAIN: &str = "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971";
/// Unix time of quicknet round 1.
pub const QUICKNET_GENESIS: u64 = 1_692_803_367;
/// Seconds between quicknet rounds.
pub const QUICKNET_PERIOD: u64 = 3;

/// Latest round published at or before `unix_secs`.
pub fn round_at(unix_secs: u64) -> u64 {
    if unix_secs < QUICKNET_GENESIS {
        return 1;
    }
    (unix_secs - QUICKNET_GENESIS) / QUICKNET_PERIOD + 1
}

/// Unix time at which `round` is published.
pub fn round_time(round: u64) -> u64 {
    QUICKNET_GENESIS + round.saturating_sub(1) * QUICKNET_PERIOD
}

/// Picks the round a draw starting at `now` commits to.
///
/// With `lead_secs >= QUICKNET_PERIOD` the round is guaranteed to be in the
/// future, i.e. nobody can know its randomness at commit time.
pub fn target_round(now_unix: u64, lead_secs: u64) -> u64 {
    round_at(now_unix + lead_secs)
}

/// `randomness = SHA256(signature)` for unchained schemes such as quicknet.
pub fn randomness_from_signature(signature: &[u8]) -> Hash {
    sha256(&[signature])
}

/// A drand beacon as returned by the `/public/{round}` endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Beacon {
    pub round: u64,
    /// Hex-encoded BLS signature.
    pub signature: String,
    /// Hex-encoded randomness (`SHA256(signature)`).
    pub randomness: String,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum BeaconError {
    #[error("signature is not valid hex")]
    BadSignature,
    #[error("randomness does not match SHA256(signature)")]
    RandomnessMismatch,
    #[error("expected round {expected}, got {actual}")]
    WrongRound { expected: u64, actual: u64 },
}

impl Beacon {
    /// Checks that the randomness is derived from the signature and returns it.
    ///
    /// BLS signature verification against the chain's public key is not done
    /// here; anyone can re-fetch the round from independent relays.
    pub fn checked_randomness(&self, expected_round: u64) -> Result<Hash, BeaconError> {
        if self.round != expected_round {
            return Err(BeaconError::WrongRound {
                expected: expected_round,
                actual: self.round,
            });
        }
        let sig = hex::decode(&self.signature).map_err(|_| BeaconError::BadSignature)?;
        let derived = randomness_from_signature(&sig);
        if hex::encode(derived) != self.randomness.to_ascii_lowercase() {
            return Err(BeaconError::RandomnessMismatch);
        }
        Ok(derived)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_math_roundtrips() {
        assert_eq!(round_at(QUICKNET_GENESIS), 1);
        assert_eq!(round_at(QUICKNET_GENESIS + 2), 1);
        assert_eq!(round_at(QUICKNET_GENESIS + 3), 2);
        for r in [1u64, 2, 10, 1_000_000] {
            assert_eq!(round_at(round_time(r)), r);
        }
    }

    #[test]
    fn target_round_is_in_the_future() {
        for now in QUICKNET_GENESIS..QUICKNET_GENESIS + 20 {
            let r = target_round(now, 6);
            assert!(round_time(r) > now);
            assert!(round_time(r) <= now + 6);
        }
    }

    #[test]
    fn beacon_check() {
        let sig = vec![0xab; 48];
        let beacon = Beacon {
            round: 5,
            signature: hex::encode(&sig),
            randomness: hex::encode(randomness_from_signature(&sig)),
        };
        assert!(beacon.checked_randomness(5).is_ok());
        assert_eq!(
            beacon.checked_randomness(6),
            Err(BeaconError::WrongRound {
                expected: 6,
                actual: 5
            })
        );
        let bad = Beacon {
            randomness: hex::encode([0u8; 32]),
            ..beacon
        };
        assert_eq!(
            bad.checked_randomness(5),
            Err(BeaconError::RandomnessMismatch)
        );
    }
}
