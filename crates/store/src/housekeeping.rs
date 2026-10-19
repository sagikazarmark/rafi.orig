//! The housekeeping run's reads and writes: finding live channels to end.
//!
//! (Automatic erasure, which also lived here, was removed for now; see
//! `docs/retention.md`.)

use crate::{Db, Result};

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
