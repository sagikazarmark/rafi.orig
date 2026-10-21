//! Housekeeping's erasure writes: expiring raffles that were never frozen,
//! erasing the entrants whose date has come, and purging raffles at the
//! ceiling. Every write is set-based, one batch per duty, whatever the
//! number of entrants: D1 limits the statements per invocation, and a batch
//! is the only transaction it has. Each is idempotent: what is already
//! erased, expired or purged matches nothing.

use rafi_lifecycle::RaffleState;
use rafi_raffle::Outcome;

use crate::{Db, Result, entrants::ERASED, raffles::states};

/// Housekeeping as the audit log's actor.
pub const ACTOR: &str = "housekeeping";

/// The raffles due to expire at `?1`: never frozen
/// ([`RaffleState::expires`]), past their erasure date, not yet purged.
fn expiring() -> String {
    format!(
        "state IN ({}) AND erase_at <= ?1 AND purged_at IS NULL",
        states(RaffleState::expires)
    )
}

/// Expires every raffle that was never frozen and whose erasure date has
/// passed at `now`: its entrants are deleted outright (there is no manifest
/// to keep verifiable), an audit row counts them, and the raffle becomes
/// purged with its live updates ended. Returns the raffles expired.
pub async fn expire_due(db: &Db, now: i64) -> Result<Vec<String>> {
    let due = expiring();
    let ids: Vec<(String,)> = db
        .query(format!("SELECT id FROM raffles WHERE {due}"))
        .bind(now)
        .all()
        .await?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    db.batch(vec![
        db.query(format!(
            "INSERT INTO audit (raffle_id, at, actor, action, detail)
             SELECT id, ?1, '{ACTOR}', 'expire',
                    CAST((SELECT COUNT(*) FROM entrants WHERE raffle_id = raffles.id) AS TEXT)
             FROM raffles WHERE {due}"
        ))
        .bind(now),
        db.query(format!(
            "DELETE FROM entrants WHERE raffle_id IN (SELECT id FROM raffles WHERE {due})"
        ))
        .bind(now),
        db.query(format!(
            "UPDATE raffles SET state = '{purged}', purged_at = ?1, updated_at = ?1,
                    live_ended_at = COALESCE(live_ended_at, ?1)
             WHERE {due}",
            purged = RaffleState::Purged.as_str(),
        ))
        .bind(now),
    ])
    .await?;
    Ok(ids.into_iter().map(|(id,)| id).collect())
}

/// The entrants due for erasure at `?1`, with a winner hold of `?2` seconds:
/// not erased, in a frozen (or later) raffle past its erasure date, and
/// either not a winner (the entrant of a claimed draw), or a winner whose
/// prize was delivered, or anyone once the hold has passed too.
fn due() -> String {
    format!(
        "entrants.erased = 0 AND EXISTS (
            SELECT 1 FROM raffles r
            WHERE r.id = entrants.raffle_id
              AND r.state IN ({frozen}) AND r.erase_at <= ?1
              AND (entrants.delivered_at IS NOT NULL
                   OR r.erase_at + ?2 <= ?1
                   OR entrants.id NOT IN (
                       SELECT entrant_id FROM draws WHERE raffle_id = r.id AND outcome = '{claimed}')))",
        frozen = states(RaffleState::frozen_or_later),
        claimed = Outcome::Claimed.as_str(),
    )
}

/// The finished raffles due to be purged at `?1` with a hold of `?2`: the
/// ceiling has passed. A frozen or drawing raffle is erased but not moved:
/// the engine owns its state.
fn at_ceiling() -> String {
    format!(
        "state = '{finished}' AND purged_at IS NULL AND erase_at + ?2 <= ?1",
        finished = RaffleState::Finished.as_str(),
    )
}

/// What [`erase_due`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Erased {
    /// Entrants whose personal data was erased.
    pub entrants: u64,
    /// The raffles they were in.
    pub raffles: Vec<String>,
    /// The finished raffles that became purged: the ceiling passed.
    pub purged: Vec<String>,
}

/// Erases every entrant due at `now` (winners held `winner_hold_secs` past
/// the erasure date unless delivered), logs each erasure, writes one audit
/// row per raffle with the count, and purges finished raffles at the
/// ceiling: one batch, all or nothing.
pub async fn erase_due(db: &Db, now: i64, winner_hold_secs: i64) -> Result<Erased> {
    let (due, ceiling) = (due(), at_ceiling());
    let (count,): (i64,) = db
        .query(format!("SELECT COUNT(*) FROM entrants WHERE {due}"))
        .bind(now)
        .bind(winner_hold_secs)
        .one()
        .await?;
    let raffles: Vec<(String,)> = db
        .query(format!(
            "SELECT DISTINCT raffle_id FROM entrants WHERE {due} ORDER BY raffle_id"
        ))
        .bind(now)
        .bind(winner_hold_secs)
        .all()
        .await?;
    let purged: Vec<(String,)> = db
        .query(format!(
            "SELECT id FROM raffles WHERE {ceiling} ORDER BY id"
        ))
        .bind(now)
        .bind(winner_hold_secs)
        .all()
        .await?;
    if count == 0 && purged.is_empty() {
        return Ok(Erased::default());
    }
    db.batch(vec![
        db.query(format!(
            "INSERT INTO audit (raffle_id, at, actor, action, detail)
             SELECT raffle_id, ?1, '{ACTOR}', 'auto_erase', CAST(COUNT(*) AS TEXT)
             FROM entrants WHERE {due} GROUP BY raffle_id"
        ))
        .bind(now)
        .bind(winner_hold_secs),
        db.query(format!(
            "INSERT INTO erasures (raffle_id, entrant_id, at)
             SELECT raffle_id, id, ?1 FROM entrants WHERE {due}"
        ))
        .bind(now)
        .bind(winner_hold_secs),
        db.query(format!("UPDATE entrants SET {ERASED} WHERE {due}"))
            .bind(now)
            .bind(winner_hold_secs),
        db.query(format!(
            "INSERT INTO audit (raffle_id, at, actor, action, detail)
             SELECT id, ?1, '{ACTOR}', 'auto_purge', '' FROM raffles WHERE {ceiling}"
        ))
        .bind(now)
        .bind(winner_hold_secs),
        db.query(format!(
            "UPDATE raffles SET state = '{purged}', purged_at = ?1, updated_at = ?1,
                    live_ended_at = COALESCE(live_ended_at, ?1)
             WHERE {ceiling}",
            purged = RaffleState::Purged.as_str(),
        ))
        .bind(now)
        .bind(winner_hold_secs),
    ])
    .await?;
    Ok(Erased {
        entrants: count.max(0) as u64,
        raffles: raffles.into_iter().map(|(id,)| id).collect(),
        purged: purged.into_iter().map(|(id,)| id).collect(),
    })
}

/// The frozen or drawing raffles past the ceiling at `now`: never finished,
/// everyone erased. Housekeeping asks the engine to finish them, best effort.
pub async fn unfinished_at_ceiling(
    db: &Db,
    now: i64,
    winner_hold_secs: i64,
) -> Result<Vec<String>> {
    let ids: Vec<(String,)> = db
        .query(format!(
            "SELECT id FROM raffles WHERE state IN ({}) AND erase_at + ?2 <= ?1 ORDER BY id",
            states(RaffleState::drawable)
        ))
        .bind(now)
        .bind(winner_hold_secs)
        .all()
        .await?;
    Ok(ids.into_iter().map(|(id,)| id).collect())
}

/// Re-applies the purge to every raffle that was purged but isn't any more
/// (a restored backup): the restore tool's write. Returns how many.
pub async fn reapply_purges(db: &Db) -> Result<u64> {
    let unpurged = format!(
        "purged_at IS NOT NULL AND state <> '{}'",
        RaffleState::Purged.as_str()
    );
    let (count,): (i64,) = db
        .query(format!("SELECT COUNT(*) FROM raffles WHERE {unpurged}"))
        .one()
        .await?;
    db.query(format!(
        "UPDATE raffles SET state = '{}' WHERE {unpurged}",
        RaffleState::Purged.as_str()
    ))
    .run()
    .await?;
    Ok(count.max(0) as u64)
}
