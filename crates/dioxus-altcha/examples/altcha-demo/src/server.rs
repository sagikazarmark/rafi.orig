//! The server's side, as a real app would have it: issuing challenges and
//! redeeming solutions, each challenge once.

use dioxus::prelude::*;
use dioxus_altcha::RawChallenge;
use serde::{Deserialize, Serialize};

/// The most work the page may ask for: PBKDF2 iterations per attempt.
pub const MAX_COST: u32 = 200_000;

/// What became of a submission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Outcome {
    Joined { name: String, counter: u32 },
    Refused { reason: String },
}

/// A fresh challenge at `cost` (clamped to `1..=MAX_COST`).
#[server]
pub async fn get_challenge(cost: u32) -> Result<RawChallenge, ServerFnError> {
    imp::issue(cost).map_err(ServerFnError::new)
}

/// The form's submission: refused unless `altcha` is a fresh, valid
/// solution to one of this server's challenges.
#[server]
pub async fn join(name: String, altcha: String) -> Result<Outcome, ServerFnError> {
    Ok(match imp::redeem(&altcha) {
        Ok(counter) => Outcome::Joined {
            name: name.trim().to_owned(),
            counter,
        },
        Err(reason) => Outcome::Refused { reason },
    })
}

#[cfg(feature = "server")]
mod imp {
    use std::{
        collections::HashMap,
        sync::{LazyLock, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    use dioxus_altcha::{RawChallenge, altcha, decode_payload};

    use super::MAX_COST;

    /// How long a challenge stays valid, in seconds.
    const TTL_SECS: i64 = 5 * 60;

    /// The HMAC key: `ALTCHA_DEMO_KEY`, or random per start.
    static KEY: LazyLock<String> = LazyLock::new(|| {
        std::env::var("ALTCHA_DEMO_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .unwrap_or_else(|| {
                let mut bytes = [0u8; 32];
                getrandom::getrandom(&mut bytes).expect("the OS has randomness");
                bytes.iter().map(|b| format!("{b:02x}")).collect()
            })
    });

    /// Redeemed challenges' nonces, until they expire. A database table in a
    /// real app (with more than one server).
    static USED: LazyLock<Mutex<HashMap<String, i64>>> = LazyLock::new(Default::default);

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }

    pub fn issue(cost: u32) -> Result<RawChallenge, String> {
        let challenge = altcha::create_challenge(altcha::CreateChallengeOptions {
            algorithm: "PBKDF2/SHA-256".into(),
            cost: cost.clamp(1, MAX_COST),
            key_prefix: "00".into(),
            expires_at: Some(now() + TTL_SECS),
            hmac_signature_secret: Some(KEY.clone()),
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
        Ok(RawChallenge::from_challenge(&challenge))
    }

    /// The solution's counter, or why it's refused.
    pub fn redeem(value: &str) -> Result<u32, String> {
        if value.trim().is_empty() {
            return Err("there's no proof-of-work in the form".into());
        }
        let payload = decode_payload(value).map_err(|e| format!("it isn't a payload: {e}"))?;
        let Some(expires_at) = payload.challenge.parameters.expires_at.filter(|&t| t > 0) else {
            return Err("the challenge never expires, so it could be replayed forever".into());
        };
        let result = altcha::verify_solution(altcha::VerifySolutionOptions::new(
            &payload.challenge,
            &payload.solution,
            KEY.as_str(),
        ))
        .map_err(|e| e.to_string())?;
        if result.expired {
            return Err("the challenge expired".into());
        }
        if result.invalid_signature == Some(true) {
            return Err(
                "the signature doesn't match: the challenge was changed, or isn't this server's"
                    .into(),
            );
        }
        if !result.verified {
            return Err("the solution is wrong".into());
        }
        // Verified: now each challenge once. Every variant of a solved
        // challenge (another counter, extra fields) keeps its signed nonce.
        let mut used = USED.lock().expect("not poisoned");
        let now = now();
        used.retain(|_, expires| *expires >= now);
        if used
            .insert(payload.challenge.parameters.nonce.clone(), expires_at)
            .is_some()
        {
            return Err("it was already used: a replay".into());
        }
        Ok(payload.solution.counter)
    }
}
