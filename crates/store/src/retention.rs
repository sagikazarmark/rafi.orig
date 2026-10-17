//! The retention run's reads and writes: dating when finished raffles' personal
//! data is erased, erasing what is due, finding live channels to end, and when
//! the last run succeeded.
//!
//! Every write is set-based (a few statements per run, whatever the number of
//! entrants), to stay under D1's per-invocation query limit.

use std::sync::LazyLock;

use rafi_lifecycle::{Command, RaffleState};

use rafi_raffle::Outcome;

use crate::{Db, PurgeAfter, Result, entrants::ERASED, raffles::states};

/// The states a raffle can be purged from ([`Command::Purge`]): finished, or
/// purged with erasures left to do.
fn purgeable() -> String {
    states(|s| s.apply(Command::Purge).is_some())
}

/// Dates the erasure of every finished raffle not dated yet, `purge` after it
/// finished: the app's retention settings, as they are when the run first
/// sees the raffle finished. A date once set is kept, when the settings
/// change too. Returns whether any raffle was dated (only zero versus
/// non-zero changes are reliable, [`Query::run`](crate::Query::run)).
pub async fn date_purges(db: &Db, purge: PurgeAfter) -> Result<bool> {
    let changed = db
        .query(format!(
            "UPDATE raffles SET
            purge_nonwinners_at = COALESCE(purge_nonwinners_at, finished_at + ?1),
            purge_winners_at = COALESCE(purge_winners_at, finished_at + ?2)
         WHERE state IN ({}) AND finished_at IS NOT NULL
           AND (purge_nonwinners_at IS NULL OR purge_winners_at IS NULL)",
            states(RaffleState::is_over)
        ))
        .bind(purge.nonwinners_secs)
        .bind(purge.winners_secs)
        .run()
        .await?;
    Ok(changed > 0)
}

/// The entrants (of table `entrants`) whose personal data is due for erasure
/// at `?1`: in a finished raffle whose erasure is dated ([`date_purges`]), not
/// yet erased; non-winners once the
/// raffle's non-winner date passed; winners (they claimed a prize) once the
/// winner date passed, or the non-winner date if their prize was delivered.
/// A winner is the entrant of a claimed draw. Built once.
fn due() -> &'static str {
    static DUE: LazyLock<String> = LazyLock::new(|| {
        format!(
            "entrants.purged = 0 AND EXISTS (
                SELECT 1 FROM raffles r
                WHERE r.id = entrants.raffle_id
                  AND r.state IN ({purgeable}) AND r.finished_at IS NOT NULL AND r.purged_at IS NULL
                  AND CASE WHEN entrants.id IN (
                          SELECT entrant_id FROM draws WHERE raffle_id = r.id AND outcome = '{claimed}')
                      THEN r.purge_winners_at <= ?1
                           OR (entrants.delivered_at IS NOT NULL AND r.purge_nonwinners_at <= ?1)
                      ELSE r.purge_nonwinners_at <= ?1
                  END)",
            purgeable = purgeable(),
            claimed = Outcome::Claimed.as_str(),
        )
    });
    &DUE
}

/// Erases every entrant that is due at `now`, logs the erasures, and marks
/// raffles whose winners are due as purged: one batch, all or nothing.
/// Returns how many entrants were erased.
pub async fn erase_due(db: &Db, now: i64) -> Result<u64> {
    let due = due();
    let (count,): (i64,) = db
        .query(format!("SELECT COUNT(*) FROM entrants WHERE {due}"))
        .bind(now)
        .one()
        .await?;
    db.batch(vec![
        db.query(format!(
            "INSERT INTO erasures (raffle_id, entrant_id, at)
             SELECT raffle_id, id, ?1 FROM entrants WHERE {due}"
        ))
        .bind(now),
        db.query(format!("UPDATE entrants SET {ERASED} WHERE {due}"))
            .bind(now),
        db.query(format!(
            "UPDATE raffles SET state = '{purged}', purged_at = ?1
             WHERE state IN ({purgeable}) AND finished_at IS NOT NULL
               AND purged_at IS NULL AND purge_winners_at <= ?1",
            purged = RaffleState::Purged.as_str(),
            purgeable = purgeable(),
        ))
        .bind(now),
    ])
    .await?;
    Ok(count.max(0) as u64)
}

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
        .query("SELECT value FROM meta WHERE key = 'last_retention_ok_at'")
        .optional()
        .await?;
    Ok(row.and_then(|(v,)| v.parse().ok()))
}

/// Records that a run succeeded at `now`.
pub async fn record_ok(db: &Db, now: i64) -> Result<()> {
    db.query(
        "INSERT INTO meta (key, value) VALUES ('last_retention_ok_at', ?)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(now.to_string())
    .run()
    .await
    .map(|_| ())
}
