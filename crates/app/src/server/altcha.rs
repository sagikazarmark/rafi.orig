//! Self-hosted proof-of-work challenge (ALTCHA-compatible scheme).
//!
//! The server issues `{salt, challenge = SHA256(salt + number), signature =
//! HMAC(key, challenge)}` for a secret random `number <= max`; the browser
//! brute-forces `number` and posts it back. No third party, no cookies.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ids;

/// How long a challenge stays valid.
const TTL_SECS: i64 = 20 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Challenge {
    pub algorithm: String,
    pub challenge: String,
    pub maxnumber: u64,
    pub salt: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Solution {
    pub algorithm: String,
    pub challenge: String,
    pub number: u64,
    pub salt: String,
    pub signature: String,
}

#[derive(Clone)]
pub struct Altcha {
    key: Arc<Vec<u8>>,
    max: u64,
    used: Arc<Mutex<HashMap<String, i64>>>,
}

impl Altcha {
    pub fn new(key: Vec<u8>, max: u64) -> Self {
        Self {
            key: Arc::new(key),
            max: max.max(1),
            used: Arc::default(),
        }
    }

    fn sign(&self, challenge: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("any key length");
        mac.update(challenge.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    pub fn challenge(&self) -> Challenge {
        let expires = ids::now() + TTL_SECS;
        let salt = format!(
            "{}?expires={expires}",
            hex::encode(ids::random_bytes::<12>())
        );
        let r = u64::from_be_bytes(ids::random_bytes::<8>());
        let number = r % (self.max + 1);
        let challenge = hex::encode(Sha256::digest(format!("{salt}{number}").as_bytes()));
        Challenge {
            algorithm: "SHA-256".into(),
            signature: self.sign(&challenge),
            challenge,
            maxnumber: self.max,
            salt,
        }
    }

    /// Verifies a base64-encoded JSON solution. Each challenge is accepted once.
    pub fn verify(&self, payload: &str) -> bool {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(payload.trim()) else {
            return false;
        };
        let Ok(s) = serde_json::from_slice::<Solution>(&bytes) else {
            return false;
        };
        self.verify_solution(&s)
    }

    pub fn verify_solution(&self, s: &Solution) -> bool {
        if s.algorithm != "SHA-256" {
            return false;
        }
        let Some(expires) = s
            .salt
            .split_once("?expires=")
            .and_then(|(_, e)| e.parse::<i64>().ok())
        else {
            return false;
        };
        let now = ids::now();
        if expires < now {
            return false;
        }
        let expected = hex::encode(Sha256::digest(format!("{}{}", s.salt, s.number).as_bytes()));
        if expected != s.challenge {
            return false;
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("any key length");
        mac.update(s.challenge.as_bytes());
        let Ok(sig) = hex::decode(&s.signature) else {
            return false;
        };
        if mac.verify_slice(&sig).is_err() {
            return false;
        }
        let mut used = self.used.lock().expect("altcha lock");
        used.retain(|_, exp| *exp >= now);
        used.insert(s.challenge.clone(), expires).is_none()
    }

    /// Solves a challenge (tests only; the browser does this in JS).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn solve(c: &Challenge) -> Option<Solution> {
        (0..=c.maxnumber)
            .find(|n| {
                hex::encode(Sha256::digest(format!("{}{n}", c.salt).as_bytes())) == c.challenge
            })
            .map(|number| Solution {
                algorithm: c.algorithm.clone(),
                challenge: c.challenge.clone(),
                number,
                salt: c.salt.clone(),
                signature: c.signature.clone(),
            })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn encode(s: &Solution) -> String {
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(s).expect("serializes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_replay() {
        let a = Altcha::new(b"k".to_vec(), 100);
        let c = a.challenge();
        let s = Altcha::solve(&c).unwrap();
        let payload = Altcha::encode(&s);
        assert!(a.verify(&payload));
        assert!(!a.verify(&payload), "replay must fail");
    }

    #[test]
    fn rejects_forgery() {
        let a = Altcha::new(b"k".to_vec(), 100);
        let b = Altcha::new(b"other".to_vec(), 100);
        let s = Altcha::solve(&b.challenge()).unwrap();
        assert!(!a.verify(&Altcha::encode(&s)));
        assert!(!a.verify("not base64!"));
    }
}
