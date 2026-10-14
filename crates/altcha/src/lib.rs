//! Rafi's self-hosted proof-of-work (an ALTCHA-compatible scheme).
//!
//! The server issues `{salt, challenge = SHA256(salt + number), signature =
//! HMAC(key, challenge)}` for a secret random `number <= maxnumber`, where the
//! salt ends in `?expires=<unix secs>`. The browser brute-forces `number` and
//! posts the [`Solution`] back as base64-encoded JSON. No third party, no
//! cookies.
//!
//! [`solve`] is the browser's side. With the `server` feature, [`issue`] and
//! [`verify`] are the server's, with its HMAC key.
//!
//! The clock and the randomness are passed in, so this runs on wasm32. Each
//! challenge must be accepted only once: `verify` doesn't track that, the
//! caller does (Rafi: a duplicate insert of [`Solution::challenge`] fails).

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The only supported algorithm.
pub const ALGORITHM: &str = "SHA-256";

/// How long a challenge stays valid, in seconds.
pub const TTL_SECS: u64 = 20 * 60;

/// A challenge, as `GET /api/altcha` returns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Challenge {
    pub algorithm: String,
    /// Hex SHA-256 of `salt` followed by the secret number in decimal.
    pub challenge: String,
    /// The largest number to try.
    pub maxnumber: u64,
    /// Random hex, then `?expires=<unix secs>`.
    pub salt: String,
    /// Hex HMAC-SHA256 of `challenge` with the server's key.
    pub signature: String,
}

/// A solved challenge, as the join form posts it (base64 JSON, see
/// [`to_payload`](Self::to_payload)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Solution {
    pub algorithm: String,
    pub challenge: String,
    pub number: u64,
    pub salt: String,
    pub signature: String,
}

impl Solution {
    /// Encodes the solution the way the join form posts it: base64 (standard
    /// alphabet, padded) of its JSON.
    pub fn to_payload(&self) -> String {
        let json = serde_json::to_vec(self).expect("a Solution always serializes");
        base64::engine::general_purpose::STANDARD.encode(json)
    }

    /// Decodes a posted payload; `None` if it isn't base64 JSON of a solution.
    pub fn from_payload(payload: &str) -> Option<Self> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// When the challenge expires, in Unix seconds, read from the salt. The
    /// store keeps the challenge until then to refuse replays.
    pub fn expires_at(&self) -> Option<u64> {
        expires_at(&self.salt)
    }
}

fn expires_at(salt: &str) -> Option<u64> {
    salt.split_once("?expires=")?.1.parse().ok()
}

/// Finds the number behind `c`: `None` if no number up to `maxnumber` matches
/// (or the challenge isn't hex).
pub fn solve(c: &Challenge) -> Option<Solution> {
    let mut target = [0u8; 32];
    hex::decode_to_slice(&c.challenge, &mut target).ok()?;
    let salted = Sha256::new_with_prefix(c.salt.as_bytes());
    let mut digits = [0u8; 20];
    (0..=c.maxnumber)
        .find(|&n| {
            let mut h = salted.clone();
            h.update(decimal(n, &mut digits));
            h.finalize()[..] == target
        })
        .map(|number| Solution {
            algorithm: c.algorithm.clone(),
            challenge: c.challenge.clone(),
            number,
            salt: c.salt.clone(),
            signature: c.signature.clone(),
        })
}

/// `n` in decimal ASCII, without allocating.
fn decimal(mut n: u64, buf: &mut [u8; 20]) -> &[u8] {
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            return &buf[i..];
        }
    }
}

#[cfg(any(feature = "server", test))]
fn challenge_for(salt: &str, number: u64) -> String {
    hex::encode(Sha256::digest(format!("{salt}{number}").as_bytes()))
}

#[cfg(feature = "server")]
fn mac(key: &[u8]) -> hmac::Hmac<Sha256> {
    use hmac::Mac as _;
    hmac::Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length")
}

/// Issues a challenge valid for [`TTL_SECS`] from `now_secs`.
///
/// `random_number` and `salt_bytes` must come from a CSPRNG; the secret
/// number is `random_number % (max + 1)` (`max` is at least 1).
#[cfg(feature = "server")]
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
#[cfg(feature = "server")]
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
    use super::*;

    #[test]
    fn decimal_digits() {
        let mut buf = [0u8; 20];
        for n in [0, 7, 10, 12345, u64::MAX] {
            assert_eq!(decimal(n, &mut buf), n.to_string().as_bytes());
        }
    }

    /// A challenge built the way `participant.js` and today's server do, by hand.
    #[test]
    fn solves_a_hand_made_challenge() {
        let salt = "prototype?expires=9999999999";
        let c = Challenge {
            algorithm: ALGORITHM.into(),
            challenge: challenge_for(salt, 2345),
            maxnumber: 3000,
            salt: salt.into(),
            signature: "test".into(),
        };
        let s = solve(&c).unwrap();
        assert_eq!(s.number, 2345);
        assert_eq!(s.expires_at(), Some(9_999_999_999));
        assert_eq!(Solution::from_payload(&s.to_payload()), Some(s));
    }

    #[test]
    fn unsolvable() {
        let salt = "x?expires=1";
        let mut c = Challenge {
            algorithm: ALGORITHM.into(),
            challenge: challenge_for(salt, 50),
            maxnumber: 10,
            salt: salt.into(),
            signature: String::new(),
        };
        assert_eq!(solve(&c), None, "the number is above maxnumber");
        c.challenge = "not hex".into();
        assert_eq!(solve(&c), None);
    }

    /// The payload `participant.js` posts: `btoa(JSON.stringify({...}))`.
    #[test]
    fn decodes_the_js_payload() {
        let json = r#"{"algorithm":"SHA-256","challenge":"ab","number":3,"salt":"s?expires=5","signature":"cd"}"#;
        let payload = base64::engine::general_purpose::STANDARD.encode(json);
        let s = Solution::from_payload(&format!(" {payload}\n")).unwrap();
        assert_eq!(s.number, 3);
        assert_eq!(s.expires_at(), Some(5));
        assert_eq!(Solution::from_payload("not base64!"), None);
        assert_eq!(
            Solution::from_payload("e30="),
            None,
            "{{}} isn't a solution"
        );
    }

    #[cfg(feature = "server")]
    mod server {
        use super::super::*;

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
}
