//! The join form's proof-of-work: issuing challenges and redeeming solutions
//! ([`rafi_altcha`]'s scheme, ALTCHA-compatible).
//!
//! Each challenge is accepted once ([`rafi_store::altcha`]): a redeemed
//! challenge is kept until it expires, and a second redemption is refused.
//! The retention run deletes expired ones.

pub use rafi_altcha::Challenge;
use rafi_altcha::Solution;

use super::{AppState, ids};

/// A fresh challenge.
pub fn challenge(app: &AppState) -> Challenge {
    rafi_altcha::issue(
        &app.cfg.altcha_key,
        app.cfg.altcha_max,
        now(),
        u64::from_be_bytes(ids::random_bytes::<8>()),
        ids::random_bytes::<12>(),
    )
}

fn now() -> u64 {
    ids::now().max(0) as u64
}

/// Redeems a posted solution (base64 JSON): `true` if it is valid, unexpired
/// and redeemed for the first time.
pub async fn redeem(app: &AppState, payload: &str) -> rafi_store::Result<bool> {
    let Some(solution) = Solution::from_payload(payload) else {
        return Ok(false);
    };
    // The signature first: only the server's own challenges reach the table.
    if !rafi_altcha::verify(&app.cfg.altcha_key, &solution, now()) {
        return Ok(false);
    }
    let expires_at = solution.expires_at().unwrap_or_default();
    rafi_store::altcha::redeem(
        &app.db,
        &solution.challenge,
        i64::try_from(expires_at).unwrap_or(i64::MAX),
    )
    .await
}

/// Deletes the challenges that have expired (they can't be redeemed anyway).
pub async fn purge_expired(app: &AppState) -> rafi_store::Result<()> {
    rafi_store::altcha::purge_expired(&app.db, ids::now()).await
}
