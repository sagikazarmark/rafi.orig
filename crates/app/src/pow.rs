//! The join form's proof-of-work, ALTCHA v2 with PBKDF2/SHA-256
//! ([`altcha_server`]); phones solve with `dioxus-altcha`.
//!
//! Each challenge is accepted once: a redeemed challenge's nonce is kept in
//! the database ([`rafi_store::pow`]) until it expires, so it holds across
//! servers and on Workers. The housekeeping run forgets expired ones.

pub use altcha_server::{MAX_PAYLOAD_LEN, decode, encode};

use altcha_server::{RedeemError, ReplayStore, altcha::Challenge};
use rafi_store::Db;

use crate::App;

/// The proof-of-work's settings: the key that signs challenges and the cost
/// of an attempt ([`altcha_server::Verifier::new`]).
pub type ProofOfWork = altcha_server::Verifier;

/// The key-derivation algorithm: PBKDF2 is native WebCrypto on phones.
pub const ALGORITHM: &str = altcha_server::Verifier::DEFAULT_ALGORITHM;

/// The derived key's required prefix, one byte.
pub const KEY_PREFIX: &str = altcha_server::Verifier::DEFAULT_KEY_PREFIX;

/// The database's `pow_used` table, as the replay store.
struct UsedChallenges<'a>(&'a Db);

impl ReplayStore for UsedChallenges<'_> {
    type Error = rafi_store::Error;

    async fn redeem(&self, nonce: &str, expires_at: i64) -> rafi_store::Result<bool> {
        rafi_store::pow::redeem(self.0, nonce, expires_at).await
    }

    async fn forget_expired(&self, now: i64) -> rafi_store::Result<()> {
        rafi_store::pow::purge_expired(self.0, now).await
    }
}

/// A fresh challenge.
pub fn challenge(app: &App) -> Result<Challenge, altcha_server::altcha::Error> {
    app.policy.proof_of_work.challenge()
}

/// Redeems a posted solution: `true` if it is valid, unexpired and redeemed
/// for the first time.
pub async fn redeem(app: &App, payload: &str) -> rafi_store::Result<bool> {
    match app
        .policy
        .proof_of_work
        .redeem(&UsedChallenges(&app.db), payload)
        .await
    {
        Ok(_) => Ok(true),
        Err(RedeemError::Refused(_)) => Ok(false),
        Err(RedeemError::Store(e)) => Err(e),
    }
}

/// Forgets the challenges that have expired (they can't be redeemed anyway).
pub(crate) async fn purge_expired(app: &App) -> rafi_store::Result<()> {
    app.policy
        .proof_of_work
        .forget_expired(&UsedChallenges(&app.db))
        .await
}
