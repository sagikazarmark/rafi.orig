//! Rafi's self-hosted proof-of-work (an ALTCHA-compatible scheme).
//!
//! The server issues `{salt, challenge = SHA256(salt + number), signature =
//! HMAC(key, challenge)}` for a secret random `number <= maxnumber`, where the
//! salt ends in `?expires=<unix secs>`. The browser brute-forces `number` and
//! posts the [`Solution`] back as base64-encoded JSON. No third party, no
//! cookies.
//!
//! This is the scheme and the browser's side ([`solve`]); issuing and
//! verifying (with the server's key) is `rafi_live_server::altcha`.

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

/// The challenge for `salt` and `number`: hex SHA-256 of the salt followed by
/// the number in decimal.
pub fn challenge_for(salt: &str, number: u64) -> String {
    hex::encode(Sha256::digest(format!("{salt}{number}").as_bytes()))
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
}
