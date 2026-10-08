//! Redeemed proof-of-work challenges (`altcha_used`), so each is accepted once.

use crate::{Db, Result};

/// Records a challenge as redeemed until `expires_at`. Returns `false` if it
/// already was: a replay.
pub async fn redeem(db: &Db, challenge: &str, expires_at: i64) -> Result<bool> {
    let inserted = db
        .query("INSERT INTO altcha_used (challenge, expires_at) VALUES (?, ?)")
        .bind(challenge)
        .bind(expires_at)
        .run()
        .await;
    match inserted {
        Ok(_) => Ok(true),
        Err(e) if e.is_unique_violation() => Ok(false),
        Err(e) => Err(e),
    }
}

/// Forgets the challenges that expired before `now` (they can't be redeemed
/// anyway).
pub async fn purge_expired(db: &Db, now: i64) -> Result<()> {
    db.query("DELETE FROM altcha_used WHERE expires_at < ?")
        .bind(now)
        .run()
        .await
        .map(|_| ())
}
