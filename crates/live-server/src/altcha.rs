//! Issuing and verifying Rafi's proof-of-work: the server's side of
//! [`rafi_live_client::altcha`], with the server's key.
//!
//! The clock and the randomness are passed in, so this runs on wasm32. Each
//! challenge must be accepted only once: [`verify`] doesn't track that, the
//! store does (a duplicate insert of [`Solution::challenge`] fails).

use rafi_live_client::altcha::{ALGORITHM, Challenge, Solution, TTL_SECS, challenge_for};
use sha2::Sha256;

fn mac(key: &[u8]) -> hmac::Hmac<Sha256> {
    use hmac::Mac as _;
    hmac::Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length")
}

/// Issues a challenge valid for [`TTL_SECS`] from `now_secs`.
///
/// `random_number` and `salt_bytes` must come from a CSPRNG; the secret
/// number is `random_number % (max + 1)` (`max` is at least 1).
pub fn issue(
    key: &[u8],
    max: u64,
    now_secs: u64,
    random_number: u64,
    salt_bytes: [u8; 12],
) -> Challenge {
    use hmac::Mac as _;
    let max = max.max(1);
    let expires = now_secs + TTL_SECS;
    let salt = format!("{}?expires={expires}", hex::encode(salt_bytes));
    let number = random_number % (max + 1);
    let challenge = challenge_for(&salt, number);
    let mut mac = mac(key);
    mac.update(challenge.as_bytes());
    Challenge {
        algorithm: ALGORITHM.into(),
        signature: hex::encode(mac.finalize().into_bytes()),
        challenge,
        maxnumber: max,
        salt,
    }
}

/// Checks a solution: the algorithm, the expiry, the number, and the
/// signature (in constant time). It doesn't refuse replays; see the module
/// docs.
pub fn verify(key: &[u8], s: &Solution, now_secs: u64) -> bool {
    use hmac::Mac as _;
    if s.algorithm != ALGORITHM {
        return false;
    }
    let Some(expires) = s.expires_at() else {
        return false;
    };
    if expires < now_secs {
        return false;
    }
    if challenge_for(&s.salt, s.number) != s.challenge {
        return false;
    }
    let Ok(sig) = hex::decode(&s.signature) else {
        return false;
    };
    let mut mac = mac(key);
    mac.update(s.challenge.as_bytes());
    mac.verify_slice(&sig).is_ok()
}

#[cfg(test)]
mod tests {
    use rafi_live_client::altcha::solve;

    use super::*;

    const KEY: &[u8] = b"k";
    const NOW: u64 = 1_700_000_000;

    fn challenge(max: u64, random: u64) -> Challenge {
        issue(KEY, max, NOW, random, [7; 12])
    }

    #[test]
    fn issue_solve_verify() {
        let c = challenge(100, 1234);
        assert_eq!(c.algorithm, "SHA-256");
        assert_eq!(c.maxnumber, 100);
        assert_eq!(
            c.salt,
            format!("{}?expires={}", "07".repeat(12), NOW + TTL_SECS)
        );
        let s = solve(&c).unwrap();
        assert_eq!(s.number, 1234 % 101);
        assert!(verify(KEY, &s, NOW));
        assert!(verify(KEY, &s, NOW + TTL_SECS), "valid up to the second");
        let s = Solution::from_payload(&s.to_payload()).unwrap();
        assert!(verify(KEY, &s, NOW + 1));
    }

    #[test]
    fn max_is_at_least_one() {
        let c = challenge(0, 1);
        assert_eq!(c.maxnumber, 1);
        assert_eq!(solve(&c).unwrap().number, 1);
    }

    #[test]
    fn expired() {
        let s = solve(&challenge(100, 5)).unwrap();
        assert!(!verify(KEY, &s, NOW + TTL_SECS + 1));
    }

    #[test]
    fn tampered() {
        let good = solve(&challenge(100, 42)).unwrap();

        let mut s = good.clone();
        s.signature.replace_range(0..2, "00");
        if s.signature == good.signature {
            s.signature.replace_range(0..2, "11");
        }
        assert!(!verify(KEY, &s, NOW), "signature");

        let mut s = good.clone();
        s.signature = "zz".into();
        assert!(!verify(KEY, &s, NOW), "signature not hex");

        assert!(!verify(b"other", &good, NOW), "another key");

        let mut s = good.clone();
        s.number += 1;
        assert!(!verify(KEY, &s, NOW), "number");

        let mut s = good.clone();
        s.salt = s.salt.replace(
            &format!("expires={}", NOW + TTL_SECS),
            &format!("expires={}", NOW + 10 * TTL_SECS),
        );
        assert!(!verify(KEY, &s, NOW), "extended expiry");

        let mut s = good.clone();
        s.algorithm = "SHA-1".into();
        assert!(!verify(KEY, &s, NOW), "algorithm");

        let mut s = good;
        s.salt = "no-expiry".into();
        assert!(!verify(KEY, &s, NOW), "no expiry");
    }
}
