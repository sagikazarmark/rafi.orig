//! The server's side of [ALTCHA](https://altcha.org) v2 proof-of-work, on
//! the official [`altcha`] crate (re-exported): issuing signed challenges
//! ([`Verifier::challenge`]) and redeeming each solution once
//! ([`Verifier::redeem`]).
//!
//! A solution is only worth something once: [`Verifier::redeem`] records each
//! challenge's signed nonce in a [`ReplayStore`] until the challenge expires,
//! and refuses it the second time. The store is yours (a database table, so
//! it holds across servers); [`MemoryStore`] does for one process and tests.
//!
//! ```
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use altcha_server::{MemoryStore, Verifier};
//!
//! let verifier = Verifier::new("a long random secret", 5_000)?;
//! let store = MemoryStore::default();
//! // GET /api/altcha: send this to the browser as JSON.
//! let challenge = verifier.challenge()?;
//! # let solution = altcha::solve_challenge(altcha::SolveChallengeOptions::new(&challenge))?.unwrap();
//! # let posted = altcha_server::encode(&altcha::Payload { challenge, solution });
//! // The form's `altcha` field, once you've checked whatever is cheaper:
//! let payload = verifier.redeem(&store, &posted).await?;
//! assert!(verifier.redeem(&store, &posted).await.is_err(), "a replay");
//! // Now and then:
//! verifier.forget_expired(&store).await?;
//! # Ok(()) }
//! ```
//!
//! Redeem where your use case decides, not in a middleware: verifying costs a
//! key derivation (`cost` PBKDF2 iterations), so rate limits go first, and a
//! redeemed solution is spent, so cheap checks that could refuse the request
//! go first too.
//!
//! On `wasm32-unknown-unknown` (Cloudflare Workers), `altcha`'s
//! `verify_solution` reads std clocks, which panic there, until
//! altcha-org/altcha-lib-rs#1 is released: patch it in meanwhile. This crate's
//! own clock reads use `web-time`.

mod memory;

#[cfg(feature = "axum")]
pub mod axum;

use std::future::Future;

use base64::Engine as _;

pub use altcha;
pub use memory::MemoryStore;

use altcha::{Challenge, CreateChallengeOptions, Payload, VerifySolutionOptions};

/// The largest posted payload [`decode`] accepts, in bytes: a challenge's
/// `data` is free-form.
pub const MAX_PAYLOAD_LEN: usize = 8 * 1024;

/// Where each redeemed challenge's nonce is kept until the challenge expires,
/// so it's accepted once.
///
/// The futures carry no `Send` bound, so a store on a single-threaded runtime
/// works too; with a store whose futures are `Send`, so is
/// [`Verifier::redeem`]'s.
pub trait ReplayStore {
    type Error;

    /// Records `nonce` as redeemed until `expires_at` (unix seconds).
    /// `Ok(false)` if it already was: a replay. Must be atomic: of two
    /// concurrent calls with the same nonce, one gets `false`.
    fn redeem(
        &self,
        nonce: &str,
        expires_at: i64,
    ) -> impl Future<Output = Result<bool, Self::Error>>;

    /// Forgets the nonces that expired before `now` (unix seconds). Only
    /// housekeeping: an expired challenge is refused before the store is asked.
    fn forget_expired(&self, now: i64) -> impl Future<Output = Result<(), Self::Error>>;
}

/// Issues challenges and redeems solutions with one secret key.
#[derive(Clone)]
pub struct Verifier {
    key: String,
    cost: u32,
    algorithm: String,
    key_prefix: String,
    ttl_secs: i64,
}

impl Verifier {
    /// The default key-derivation algorithm: PBKDF2 is native WebCrypto in
    /// browsers.
    pub const DEFAULT_ALGORITHM: &str = "PBKDF2/SHA-256";
    /// The default required prefix of the derived key, one byte. Difficulty
    /// is tuned with `cost`, linearly; the prefix only moves in 16× steps.
    pub const DEFAULT_KEY_PREFIX: &str = "00";
    /// How long a challenge stays valid by default, in seconds.
    pub const DEFAULT_TTL_SECS: i64 = 20 * 60;

    /// A verifier signing with `key`, asking `cost` iterations per attempt
    /// (about 256 attempts per solve with the default prefix).
    pub fn new(key: impl Into<String>, cost: u32) -> Result<Self, ConfigError> {
        let key = key.into();
        // `altcha` treats an empty secret as none: unsigned challenges, and
        // every solution refused.
        if key.is_empty() {
            return Err(ConfigError::EmptyKey);
        }
        // WebCrypto refuses 0 iterations.
        if cost == 0 {
            return Err(ConfigError::ZeroCost);
        }
        Ok(Self {
            key,
            cost,
            algorithm: Self::DEFAULT_ALGORITHM.into(),
            key_prefix: Self::DEFAULT_KEY_PREFIX.into(),
            ttl_secs: Self::DEFAULT_TTL_SECS,
        })
    }

    /// How long challenges stay valid, in seconds (at least 1).
    pub fn ttl_secs(mut self, secs: i64) -> Self {
        self.ttl_secs = secs.max(1);
        self
    }

    /// The key-derivation algorithm, such as `"PBKDF2/SHA-512"`.
    pub fn algorithm(mut self, algorithm: impl Into<String>) -> Self {
        self.algorithm = algorithm.into();
        self
    }

    /// The derived key's required prefix, in hex.
    pub fn key_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.key_prefix = prefix.into();
        self
    }

    /// The iterations per attempt the challenges ask for. Only issuing uses
    /// it: a verifier redeems its key's challenges whatever they asked.
    pub fn cost(&self) -> u32 {
        self.cost
    }

    /// A fresh, signed challenge, expiring in the verifier's TTL.
    pub fn challenge(&self) -> Result<Challenge, altcha::Error> {
        altcha::create_challenge(CreateChallengeOptions {
            algorithm: self.algorithm.clone(),
            cost: self.cost,
            key_prefix: self.key_prefix.clone(),
            expires_at: Some(now() + self.ttl_secs),
            hmac_signature_secret: Some(self.key.clone()),
            ..Default::default()
        })
    }

    /// Redeems a posted solution (the form value, [`decode`]d): checks it,
    /// then records its challenge in `store`. Returns the payload, or why
    /// it's refused.
    ///
    /// The checks, in order: it decodes; its challenge expires (one that
    /// never does would be replayable once the store forgets it); its
    /// signature is this verifier's, it hasn't expired and the solution is
    /// right; then the store hasn't seen its nonce. Only this verifier's own
    /// challenges reach the store.
    pub async fn redeem<S: ReplayStore + ?Sized>(
        &self,
        store: &S,
        posted: &str,
    ) -> Result<Payload, RedeemError<S::Error>> {
        let payload = decode(posted).map_err(Refusal::Malformed)?;
        let Some(expires_at) = payload.challenge.parameters.expires_at.filter(|&t| t > 0) else {
            return Err(Refusal::NoExpiry.into());
        };
        let result = altcha::verify_solution(VerifySolutionOptions::new(
            &payload.challenge,
            &payload.solution,
            self.key.as_str(),
        ))
        .map_err(|e| Refusal::Unverifiable(e.to_string()))?;
        if result.expired {
            return Err(Refusal::Expired.into());
        }
        if result.invalid_signature != Some(false) {
            return Err(Refusal::Signature.into());
        }
        if !result.verified {
            return Err(Refusal::Wrong.into());
        }
        // The nonce identifies the challenge: every variant of one solved
        // challenge (another counter, extra fields) keeps the same signed nonce.
        let nonce = &payload.challenge.parameters.nonce;
        if !store
            .redeem(nonce, expires_at)
            .await
            .map_err(RedeemError::Store)?
        {
            return Err(Refusal::Replayed.into());
        }
        Ok(payload)
    }

    /// Forgets the redeemed challenges that have expired: housekeeping, to
    /// run now and then.
    pub async fn forget_expired<S: ReplayStore + ?Sized>(&self, store: &S) -> Result<(), S::Error> {
        store.forget_expired(now()).await
    }
}

impl std::fmt::Debug for Verifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Verifier")
            .field("key", &"<redacted>")
            .field("cost", &self.cost)
            .field("algorithm", &self.algorithm)
            .field("key_prefix", &self.key_prefix)
            .field("ttl_secs", &self.ttl_secs)
            .finish()
    }
}

/// Why a [`Verifier`] can't be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("the proof-of-work key is empty")]
    EmptyKey,
    #[error("the proof-of-work cost must be at least 1")]
    ZeroCost,
}

/// Why [`Verifier::redeem`] didn't accept a solution.
#[derive(Debug, thiserror::Error)]
pub enum RedeemError<E> {
    /// The solution is refused.
    #[error(transparent)]
    Refused(#[from] Refusal),
    /// The store failed; trying again later may work.
    #[error("the replay store failed")]
    Store(#[source] E),
}

/// Why a posted solution is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Refusal {
    #[error(transparent)]
    Malformed(DecodeError),
    /// Its challenge has no expiry: it would be replayable forever.
    #[error("the challenge never expires")]
    NoExpiry,
    #[error("the solution can't be verified: {0}")]
    Unverifiable(String),
    #[error("the challenge expired")]
    Expired,
    /// The challenge was changed, or isn't this verifier's.
    #[error("the challenge's signature doesn't match")]
    Signature,
    #[error("the solution is wrong")]
    Wrong,
    /// Its challenge was already redeemed.
    #[error("the challenge was already used")]
    Replayed,
}

/// Decodes a posted payload: base64 (standard, padded) of the JSON
/// `{"challenge": .., "solution": ..}`, as the official widget posts it. It
/// verifies nothing.
pub fn decode(posted: &str) -> Result<Payload, DecodeError> {
    let posted = posted.trim();
    if posted.is_empty() {
        return Err(DecodeError::Missing);
    }
    if posted.len() > MAX_PAYLOAD_LEN {
        return Err(DecodeError::TooLong);
    }
    let json = base64::engine::general_purpose::STANDARD
        .decode(posted)
        .map_err(|_| DecodeError::Base64)?;
    serde_json::from_slice(&json).map_err(|e| DecodeError::Json(e.to_string()))
}

/// Encodes a payload as a form posts it: the inverse of [`decode`].
pub fn encode(payload: &Payload) -> String {
    let json = serde_json::to_vec(payload).expect("a Payload always serializes");
    base64::engine::general_purpose::STANDARD.encode(json)
}

/// Why [`decode`] failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    #[error("there's no proof-of-work")]
    Missing,
    #[error("the payload is longer than {MAX_PAYLOAD_LEN} bytes")]
    TooLong,
    #[error("the payload isn't base64")]
    Base64,
    #[error("the payload isn't a challenge and a solution: {0}")]
    Json(String),
}

/// Unix seconds.
fn now() -> i64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verifier() -> Verifier {
        Verifier::new("test-key", 1).unwrap()
    }

    fn solve(challenge: Challenge) -> Payload {
        let solution = altcha::solve_challenge(altcha::SolveChallengeOptions::new(&challenge))
            .unwrap()
            .expect("solvable");
        Payload {
            challenge,
            solution,
        }
    }

    fn refusal<E: std::fmt::Debug>(r: Result<Payload, RedeemError<E>>) -> Refusal {
        match r {
            Err(RedeemError::Refused(r)) => r,
            other => panic!("not a refusal: {other:?}"),
        }
    }

    #[test]
    fn config() {
        assert_eq!(Verifier::new("", 1).unwrap_err(), ConfigError::EmptyKey);
        assert_eq!(Verifier::new("k", 0).unwrap_err(), ConfigError::ZeroCost);
        assert!(!format!("{:?}", verifier()).contains("test-key"));
    }

    #[tokio::test]
    async fn redeems_once() {
        let (v, store) = (verifier(), MemoryStore::default());
        let posted = encode(&solve(v.challenge().unwrap()));
        v.redeem(&store, &posted).await.unwrap();
        assert_eq!(refusal(v.redeem(&store, &posted).await), Refusal::Replayed);
    }

    #[tokio::test]
    async fn another_counter_is_the_same_challenge() {
        let (v, store) = (verifier(), MemoryStore::default());
        let payload = solve(v.challenge().unwrap());
        v.redeem(&store, &encode(&payload)).await.unwrap();
        let mut other = payload.clone();
        other.solution.counter += 1;
        // Wrong, or a replay: never accepted.
        assert!(v.redeem(&store, &encode(&other)).await.is_err());
    }

    #[tokio::test]
    async fn refuses_forgeries() {
        let (v, store) = (verifier(), MemoryStore::default());
        let other = Verifier::new("another-key", 1).unwrap();
        let theirs = encode(&solve(other.challenge().unwrap()));
        assert_eq!(refusal(v.redeem(&store, &theirs).await), Refusal::Signature);

        let mut cheaper = solve(v.challenge().unwrap());
        cheaper.challenge.parameters.cost = 0;
        assert_eq!(
            refusal(v.redeem(&store, &encode(&cheaper)).await),
            Refusal::Signature
        );

        let mut wrong = solve(v.challenge().unwrap());
        wrong.solution.derived_key = "00".repeat(32);
        assert_eq!(
            refusal(v.redeem(&store, &encode(&wrong)).await),
            Refusal::Wrong
        );

        let never = solve(
            altcha::create_challenge(CreateChallengeOptions {
                algorithm: Verifier::DEFAULT_ALGORITHM.into(),
                cost: 1,
                key_prefix: Verifier::DEFAULT_KEY_PREFIX.into(),
                hmac_signature_secret: Some("test-key".into()),
                ..Default::default()
            })
            .unwrap(),
        );
        assert_eq!(
            refusal(v.redeem(&store, &encode(&never)).await),
            Refusal::NoExpiry
        );
    }

    #[tokio::test]
    async fn refuses_malformed_payloads() {
        let (v, store) = (verifier(), MemoryStore::default());
        for (posted, why) in [
            ("", DecodeError::Missing),
            ("not base64!", DecodeError::Base64),
            (&"A".repeat(MAX_PAYLOAD_LEN + 4), DecodeError::TooLong),
        ] {
            assert_eq!(
                refusal(v.redeem(&store, posted).await),
                Refusal::Malformed(why)
            );
        }
        assert!(matches!(
            refusal(v.redeem(&store, "e30=").await), // {}
            Refusal::Malformed(DecodeError::Json(_))
        ));
    }

    #[test]
    fn decodes_what_it_encodes() {
        let payload = solve(verifier().challenge().unwrap());
        let decoded = decode(&format!(" {} ", encode(&payload))).unwrap();
        assert_eq!(decoded.solution.counter, payload.solution.counter);
        assert_eq!(
            decoded.challenge.parameters.nonce,
            payload.challenge.parameters.nonce
        );
    }
}
