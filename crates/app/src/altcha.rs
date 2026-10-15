//! The join form's proof-of-work: issuing challenges and redeeming solutions,
//! ALTCHA v2 ([`::altcha`]) with PBKDF2/SHA-256. Phones solve with
//! `dioxus-altcha`.
//!
//! Each challenge is accepted once ([`rafi_store::altcha`]): a redeemed
//! challenge's nonce is kept until it expires, and a second redemption is
//! refused. The retention run deletes expired ones.

use std::fmt;

use ::altcha::{Challenge, CreateChallengeOptions, Payload, VerifySolutionOptions};
use base64::Engine as _;

use crate::{App, ids};

/// The key-derivation algorithm: PBKDF2 is native WebCrypto on phones.
pub const ALGORITHM: &str = "PBKDF2/SHA-256";

/// The derived key's required prefix, one byte. Difficulty is tuned with
/// `cost`, linearly; the prefix only moves in 16× steps.
pub const KEY_PREFIX: &str = "00";

/// How long a challenge stays valid, in seconds.
pub const TTL_SECS: i64 = 20 * 60;

/// The largest posted payload, in bytes: a challenge's `data` is free-form.
pub const MAX_PAYLOAD_LEN: usize = 8 * 1024;

/// The proof-of-work's settings.
#[derive(Clone)]
pub struct ProofOfWork {
    key: String,
    cost: u32,
}

impl ProofOfWork {
    /// `key` signs the challenges; `cost` is the PBKDF2 iteration count, how
    /// much work one attempt takes (about 256 attempts per solve).
    pub fn new(key: impl Into<String>, cost: u32) -> anyhow::Result<Self> {
        let key = key.into();
        // `altcha` treats an empty secret as none: unsigned challenges, and
        // every join refused.
        anyhow::ensure!(!key.is_empty(), "the proof-of-work key is empty");
        // WebCrypto refuses 0 iterations.
        anyhow::ensure!(cost >= 1, "the proof-of-work cost must be at least 1");
        Ok(Self { key, cost })
    }
}

impl fmt::Debug for ProofOfWork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProofOfWork")
            .field("key", &"<redacted>")
            .field("cost", &self.cost)
            .finish()
    }
}

/// A fresh challenge, expiring in [`TTL_SECS`].
pub fn challenge(app: &App) -> Result<Challenge, ::altcha::Error> {
    let pow = &app.policy.proof_of_work;
    ::altcha::create_challenge(CreateChallengeOptions {
        algorithm: ALGORITHM.into(),
        cost: pow.cost,
        key_prefix: KEY_PREFIX.into(),
        expires_at: Some(ids::now() + TTL_SECS),
        hmac_signature_secret: Some(pow.key.clone()),
        ..Default::default()
    })
}

/// Decodes a posted payload (base64 JSON, as the widget posts it); `None` if
/// it's too long or isn't one.
pub fn decode(payload: &str) -> Option<Payload> {
    let payload = payload.trim();
    if payload.len() > MAX_PAYLOAD_LEN {
        return None;
    }
    let json = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&json).ok()
}

/// Encodes a payload the way the join form posts it.
pub fn encode(payload: &Payload) -> String {
    let json = serde_json::to_vec(payload).expect("a Payload always serializes");
    base64::engine::general_purpose::STANDARD.encode(json)
}

/// Redeems a posted solution: `true` if it is valid, unexpired and redeemed
/// for the first time.
pub async fn redeem(app: &App, payload: &str) -> rafi_store::Result<bool> {
    let Some(payload) = decode(payload) else {
        return Ok(false);
    };
    // A challenge that never expires would be replayable once the purge
    // forgets it. Ours always expire; the field is signed.
    let Some(expires_at) = payload.challenge.parameters.expires_at.filter(|&t| t > 0) else {
        return Ok(false);
    };
    // Signature, expiry, then the key: only the server's own challenges
    // reach the table.
    let verified = ::altcha::verify_solution(VerifySolutionOptions::new(
        &payload.challenge,
        &payload.solution,
        app.policy.proof_of_work.key.as_str(),
    ))
    .is_ok_and(|r| r.verified);
    if !verified {
        return Ok(false);
    }
    // The nonce identifies the challenge: every variant of one solved
    // challenge (another counter, extra fields) keeps the same signed nonce.
    rafi_store::altcha::redeem(&app.db, &payload.challenge.parameters.nonce, expires_at).await
}

/// Deletes the challenges that have expired (they can't be redeemed anyway).
pub(crate) async fn purge_expired(app: &App) -> rafi_store::Result<()> {
    rafi_store::altcha::purge_expired(&app.db, ids::now()).await
}
