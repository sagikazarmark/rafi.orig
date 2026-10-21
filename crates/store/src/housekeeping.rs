//! The housekeeping run's reads and writes: finding live channels to end, and
//! when the last run succeeded. Its erasure writes are [`retention`](crate::retention).

use crate::{Db, Result};

/// `meta`'s key for when housekeeping last succeeded.
const LAST_OK: &str = "last_retention_ok_at";

/// The raffles that finished at or before `finished_by` and still have live
/// updates.
pub async fn live_channels_due(db: &Db, finished_by: i64) -> Result<Vec<String>> {
    let ids: Vec<(String,)> = db
        .query(
            "SELECT id FROM raffles
             WHERE finished_at IS NOT NULL AND finished_at <= ? AND live_ended_at IS NULL",
        )
        .bind(finished_by)
        .all()
        .await?;
    Ok(ids.into_iter().map(|(id,)| id).collect())
}

/// Records that the raffle's live updates ended at `now`, unless they already had.
pub async fn live_ended(db: &Db, raffle_id: &str, now: i64) -> Result<()> {
    db.query("UPDATE raffles SET live_ended_at = ? WHERE id = ? AND live_ended_at IS NULL")
        .bind(now)
        .bind(raffle_id)
        .run()
        .await
        .map(|_| ())
}

/// When the last run succeeded (unix seconds), if ever.
pub async fn last_ok(db: &Db) -> Result<Option<i64>> {
    let row: Option<(String,)> = db
        .query("SELECT value FROM meta WHERE key = ?")
        .bind(LAST_OK)
        .optional()
        .await?;
    Ok(row.and_then(|(v,)| v.parse().ok()))
}

/// Records that a run succeeded at `now`.
pub async fn record_ok(db: &Db, now: i64) -> Result<()> {
    db.query(
        "INSERT INTO meta (key, value) VALUES (?, ?)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(LAST_OK)
    .bind(now.to_string())
    .run()
    .await
    .map(|_| ())
}
