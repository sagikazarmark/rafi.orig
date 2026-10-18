//! The server's side, as a real app would have it: issuing challenges and
//! redeeming solutions, each challenge once ([`altcha_server`]).

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
    Ok(match imp::redeem(&altcha).await {
        Ok(counter) => Outcome::Joined {
            name: name.trim().to_owned(),
            counter,
        },
        Err(reason) => Outcome::Refused { reason },
    })
}

#[cfg(feature = "server")]
mod imp {
    use std::sync::LazyLock;

    use altcha_server::{MemoryStore, RedeemError, Refusal, Verifier};
    use dioxus_altcha::RawChallenge;

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
    static USED: LazyLock<MemoryStore> = LazyLock::new(MemoryStore::default);

    /// A verifier asking `cost`. The page picks the cost per challenge, and
    /// only issuing uses it: any verifier with the key redeems them all.
    fn verifier(cost: u32) -> Verifier {
        Verifier::new(KEY.as_str(), cost.clamp(1, MAX_COST))
            .expect("a key and a cost")
            .ttl_secs(TTL_SECS)
    }

    pub fn issue(cost: u32) -> Result<RawChallenge, String> {
        let challenge = verifier(cost).challenge().map_err(|e| e.to_string())?;
        Ok(RawChallenge::from_challenge(&challenge))
    }

    /// The solution's counter, or why it's refused.
    pub async fn redeem(value: &str) -> Result<u32, String> {
        let verifier = verifier(1);
        let Ok(()) = verifier.forget_expired(&*USED).await;
        match verifier.redeem(&*USED, value).await {
            Ok(payload) => Ok(payload.solution.counter),
            Err(RedeemError::Refused(why)) => Err(reason(&why)),
            Err(RedeemError::Store(never)) => match never {},
        }
    }

    /// What the page says about a refusal.
    fn reason(why: &Refusal) -> String {
        match why {
            Refusal::Malformed(e) => format!("it isn't a payload: {e}"),
            Refusal::NoExpiry => {
                "the challenge never expires, so it could be replayed forever".into()
            }
            Refusal::Signature => {
                "the signature doesn't match: the challenge was changed, or isn't this server's"
                    .into()
            }
            Refusal::Replayed => "it was already used: a replay".into(),
            other => other.to_string(),
        }
    }
}
