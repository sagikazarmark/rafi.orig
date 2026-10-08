//! The join form's proof-of-work: issuing challenges and redeeming solutions
//! ([`rafi_live::altcha`]'s scheme, ALTCHA-compatible).
//!
//! Each challenge is accepted once: a redeemed challenge is stored in
//! `altcha_used` until it expires, and a second redemption fails its primary
//! key. The retention run deletes expired rows.

pub use rafi_live::altcha::Challenge;
use rafi_live::altcha::{self, Solution};

use super::{AppState, ids};

/// A fresh challenge.
pub fn challenge(app: &AppState) -> Challenge {
    altcha::issue(
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
    if !altcha::verify(&app.cfg.altcha_key, &solution, now()) {
        return Ok(false);
    }
    let expires_at = solution.expires_at().unwrap_or_default();
    let inserted = app
        .db
        .query("INSERT INTO altcha_used (challenge, expires_at) VALUES (?, ?)")
        .bind(&solution.challenge)
        .bind(i64::try_from(expires_at).unwrap_or(i64::MAX))
        .run()
        .await;
    match inserted {
        Ok(_) => Ok(true),
        Err(e) if e.is_unique_violation() => Ok(false),
        Err(e) => Err(e),
    }
}

/// Deletes the challenges that have expired (they can't be redeemed anyway).
pub async fn purge_expired(app: &AppState) -> rafi_store::Result<()> {
    app.db
        .query("DELETE FROM altcha_used WHERE expires_at < ?")
        .bind(ids::now())
        .run()
        .await?;
    Ok(())
}
