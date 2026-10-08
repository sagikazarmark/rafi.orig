//! The hourly retention run (natively a task; on Workers, the Cron Trigger).
//!
//! - **Erasure** (when `RAFI_RETENTION` is on): after a raffle finishes,
//!   non-winners are erased after `RAFI_RETENTION_DAYS` (30), winners once
//!   their prize is marked delivered or after `RAFI_RETENTION_WINNERS_DAYS`
//!   (90). Entrant IDs and the draw log stay, so the published draw remains
//!   verifiable. Each erasure is logged in `erasures`, without personal data.
//! - **Proof-of-work:** expired challenges leave `altcha_used`.
//! - **Live updates:** a raffle that finished more than a day ago and still
//!   has live updates gets them ended ([`Notifier::end`](rafi_platform::Notifier::end);
//!   on Workers, its channel object is destroyed), whether or not erasure is on.
//!
//! Every write is set-based (a few statements per run, whatever the number of
//! entrants), to stay under D1's per-invocation query limit. A successful run
//! records `last_retention_ok_at` in `meta`.

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use super::{AppState, altcha, ids, service::ERASED};

/// How long after finishing a raffle keeps its live channel, unless the
/// organizer ends live updates earlier.
pub const CHANNEL_GRACE_SECS: i64 = 86_400;

/// The entrants (of table `entrants`) whose personal data is due for erasure
/// at `?1`: in a finished raffle, not yet erased; non-winners once the
/// raffle's non-winner date passed; winners (they claimed a prize) once the
/// winner date passed, or the non-winner date if their prize was delivered.
const DUE: &str = "entrants.purged = 0 AND EXISTS (
    SELECT 1 FROM raffles r
    WHERE r.id = entrants.raffle_id
      AND r.state IN ('finished', 'purged') AND r.finished_at IS NOT NULL AND r.purged_at IS NULL
      AND CASE WHEN entrants.id IN (
              SELECT json_extract(u.value, '$.entrant_id') FROM json_each(r.draw_state, '$.units') u
              WHERE json_extract(u.value, '$.status') = 'claimed')
          THEN r.purge_winners_at <= ?1
               OR (entrants.delivered_at IS NOT NULL AND r.purge_nonwinners_at <= ?1)
          ELSE r.purge_nonwinners_at <= ?1
      END)";

/// When the last run succeeded (unix seconds), if ever.
pub async fn last_ok(app: &AppState) -> anyhow::Result<Option<i64>> {
    let row: Option<(String,)> = app
        .db
        .query("SELECT value FROM meta WHERE key = 'last_retention_ok_at'")
        .optional()
        .await?;
    Ok(row.and_then(|(v,)| v.parse().ok()))
}

/// What a run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// Entrants whose personal data was erased.
    pub erased: u64,
    /// Live channels destroyed.
    pub channels_ended: u64,
}

/// The native hourly run; on Workers the Cron Trigger calls [`run`].
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn(app: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            match run(&app).await {
                Ok(r) if r == Report::default() => {}
                Ok(r) => tracing::info!(
                    erased = r.erased,
                    channels_ended = r.channels_ended,
                    "retention run"
                ),
                Err(e) => tracing::error!(error = %e, "retention run failed"),
            }
        }
    });
}

/// One retention pass.
pub async fn run(app: &AppState) -> anyhow::Result<Report> {
    let now = ids::now();
    let erased = if app.cfg.retention_enabled {
        erase_due(app, now).await?
    } else {
        0
    };
    altcha::purge_expired(app).await?;
    let channels_ended = end_channels(app, now).await;
    app.db
        .query(
            "INSERT INTO meta (key, value) VALUES ('last_retention_ok_at', ?)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .bind(now.to_string())
        .run()
        .await?;
    Ok(Report {
        erased,
        channels_ended,
    })
}

/// Erases every entrant that is due, logs the erasures, and marks raffles
/// whose winners are due as purged: one batch, all or nothing.
async fn erase_due(app: &AppState, now: i64) -> anyhow::Result<u64> {
    let db = &app.db;
    let (due,): (i64,) = db
        .query(format!("SELECT COUNT(*) FROM entrants WHERE {DUE}"))
        .bind(now)
        .one()
        .await?;
    db.batch(vec![
        db.query(format!(
            "INSERT INTO erasures (raffle_id, entrant_id, at)
             SELECT raffle_id, id, ?1 FROM entrants WHERE {DUE}"
        ))
        .bind(now),
        db.query(format!("UPDATE entrants SET {ERASED} WHERE {DUE}"))
            .bind(now),
        db.query(
            "UPDATE raffles SET state = 'purged', purged_at = ?1
             WHERE state IN ('finished', 'purged') AND finished_at IS NOT NULL
               AND purged_at IS NULL AND purge_winners_at <= ?1",
        )
        .bind(now),
    ])
    .await?;
    // No nudges: these raffles finished weeks ago, their channels are gone.
    Ok(due.max(0) as u64)
}

/// Destroys the live channel of every raffle that finished more than
/// [`CHANNEL_GRACE_SECS`] ago and still has one, then records that live
/// updates ended. A channel that couldn't be destroyed is tried again on the
/// next run. Returns how many were destroyed.
async fn end_channels(app: &AppState, now: i64) -> u64 {
    let due: Vec<(String,)> = match app
        .db
        .query(
            "SELECT id FROM raffles
             WHERE finished_at IS NOT NULL AND finished_at <= ? AND live_ended_at IS NULL",
        )
        .bind(now - CHANNEL_GRACE_SECS)
        .all()
        .await
    {
        Ok(due) => due,
        Err(e) => {
            tracing::warn!(error = %e, "looking for live channels to end failed");
            return 0;
        }
    };
    let mut ended = 0;
    for (id,) in due {
        // Over raffles refuse sockets anyway, so destroying first is safe,
        // and a failure leaves the raffle for the next run.
        if let Err(e) = app.notifier.end(&id).await {
            tracing::warn!(raffle_id = id, error = %e, "ending a live channel failed");
            continue;
        }
        let set = app
            .db
            .query("UPDATE raffles SET live_ended_at = ? WHERE id = ? AND live_ended_at IS NULL")
            .bind(now)
            .bind(&id)
            .run()
            .await;
        match set {
            Ok(_) => ended += 1,
            Err(e) => {
                tracing::warn!(raffle_id = id, error = %e, "recording the end of live updates failed")
            }
        }
    }
    ended
}
