//! The housekeeping run: what happens to raffles as they age. The deployment
//! schedules it hourly (natively a task; on Workers, the Cron Trigger).
//!
//! - **Expiry:** a raffle that was never frozen and whose erasure date has
//!   passed loses its entrants outright (there is no draw to keep
//!   verifiable) and becomes purged ([`RaffleState::expire`]).
//! - **Erasure:** once a frozen raffle's erasure date passes, in any state,
//!   the personal data of everyone but the winners is erased; winners (the
//!   entrants of claimed draws) are held [`RetentionPolicy::winner_hold_days`](crate::RetentionPolicy::winner_hold_days)
//!   longer unless their prize was marked delivered. Past the hold, everyone
//!   is erased and a finished raffle becomes purged; a frozen or drawing one
//!   is asked to finish, best effort. Entrant IDs and the draw log stay, so
//!   the published draw remains verifiable. Each erasure is logged in
//!   `erasures`, without personal data, and each raffle gets an audit row.
//! - **Proof-of-work:** expired challenges are forgotten.
//! - **Live updates:** a raffle that finished more than a day ago and still
//!   has live updates gets them ended ([`Notifier::end`](rafi_live_server::Notifier::end);
//!   on Workers, its channel object is destroyed).
//!
//! The database side is [`rafi_store::retention`] and
//! [`rafi_store::housekeeping`]: set-based writes, a few statements per run
//! whatever the number of entrants. A successful run records when it
//! happened; the admin warns when none has for two hours since there was
//! first anything to erase ([`stale`]).
//!
//! The organizer can always erase earlier ([`purge`](crate::purge),
//! [`remove_entrant`](crate::remove_entrant)); nothing extends a date.

use rafi_raffle::{Decision, RaffleId};
use rafi_store::{audit, housekeeping as store, retention};

use crate::{App, altcha, ids};

/// How long after finishing a raffle keeps its live channel, unless the
/// organizer ends live updates earlier.
pub const CHANNEL_GRACE_SECS: i64 = 86_400;

/// How long a run may be missing before erasure counts as stopped
/// ([`stale`]): two hourly runs.
pub const STALE_AFTER_SECS: i64 = 2 * 3600;

/// What a run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// Raffles that were never frozen and expired: their entrants deleted.
    pub expired: u64,
    /// Entrants whose personal data was erased.
    pub erased: u64,
    /// Finished raffles that became purged: the winner hold passed.
    pub purged: u64,
    /// Live channels destroyed.
    pub channels_ended: u64,
}

/// One housekeeping pass, now.
pub async fn run(app: &App) -> anyhow::Result<Report> {
    run_at(app, ids::now()).await
}

/// One housekeeping pass, as of `now`.
pub async fn run_at(app: &App, now: i64) -> anyhow::Result<Report> {
    let hold = app.policy.retention.winner_hold_secs();
    let expired = retention::expire_due(&app.db, now).await?;
    let erased = retention::erase_due(&app.db, now, hold).await?;
    // Screens of raffles whose names just went: best effort, a failed nudge
    // is a stale screen until its next poll. Expired and purged raffles are
    // over: their channels go too (on Workers, the object is destroyed).
    for id in &erased.raffles {
        app.live_updates.notify(id).await;
    }
    for id in expired.iter().chain(&erased.purged) {
        if let Err(e) = app.live_updates.end(id).await {
            tracing::warn!(raffle_id = id, error = %e, "ending the live channel of an over raffle failed");
        }
    }
    finish_at_ceiling(app, now, hold).await;
    altcha::purge_expired(app).await?;
    let channels_ended = end_channels(app, now).await;
    store::record_ok(&app.db, now).await?;
    Ok(Report {
        expired: expired.len() as u64,
        erased: erased.entrants,
        purged: erased.purged.len() as u64,
        channels_ended,
    })
}

/// Asks the engine to finish every frozen or drawing raffle past the
/// ceiling: everyone in it is erased, and nothing more can be drawn. Best
/// effort: erasure never waits for the engine, and a raffle that won't
/// finish is tried again on the next run.
async fn finish_at_ceiling(app: &App, now: i64, hold: i64) {
    let due = match retention::unfinished_at_ceiling(&app.db, now, hold).await {
        Ok(due) => due,
        Err(e) => {
            tracing::warn!(error = %e, "looking for unfinished raffles past the ceiling failed");
            return;
        }
    };
    let Ok(engine) = app.engine() else {
        return;
    };
    for id in due {
        let command_id = format!("housekeeping-finish-{id}");
        match engine
            .decide(&RaffleId::new(&id), &Decision::Finish, &command_id)
            .await
        {
            Ok(_) => {
                if let Err(e) =
                    audit::record(&app.db, &id, audit::HOUSEKEEPING, "auto_finish", "").await
                {
                    tracing::warn!(raffle_id = id, error = %e, "auditing an automatic finish failed");
                }
            }
            Err(rafi_engine::EngineError::Pending) => {}
            // The engine doesn't hold it (never opened, or forgotten): the
            // log in the database is the record, and nothing is left to
            // wait for, so the raffle is purged here.
            Err(rafi_engine::EngineError::Rejected {
                kind: rafi_raffle::ErrorKind::NotFound,
                ..
            }) => {
                if let Err(e) = retention::purge_unfinished(&app.db, &id, now).await {
                    tracing::warn!(raffle_id = id, error = %e, "purging a raffle the engine doesn't hold failed");
                } else if let Err(e) = app.live_updates.end(&id).await {
                    tracing::warn!(raffle_id = id, error = %e, "ending the live channel of a purged raffle failed");
                }
            }
            Err(e) => {
                tracing::warn!(raffle_id = id, error = %e, "finishing a raffle past the ceiling failed")
            }
        }
    }
}

/// Whether erasure may have stopped: no run has succeeded in the last
/// [`STALE_AFTER_SECS`], counting from when there was first anything to
/// erase. A fresh installation with no raffles, or with raffles younger
/// than that, isn't stale: nothing has been promised and missed yet.
pub async fn stale(app: &App) -> anyhow::Result<bool> {
    let last_ok = store::last_ok(&app.db).await?;
    let since = store::oldest_raffle_at(&app.db).await?;
    Ok(is_stale(last_ok, since, ids::now()))
}

/// Whether a run that last succeeded at `last_ok` (if ever) counts as
/// stopped at `now`, given that the oldest raffle was created at `since`
/// (if any).
fn is_stale(last_ok: Option<i64>, since: Option<i64>, now: i64) -> bool {
    match (last_ok, since) {
        (Some(at), _) => now - at > STALE_AFTER_SECS,
        (None, Some(since)) => now - since > STALE_AFTER_SECS,
        (None, None) => false,
    }
}

/// Destroys the live channel of every raffle that finished more than
/// [`CHANNEL_GRACE_SECS`] ago and still has one, then records that live
/// updates ended. A channel that couldn't be destroyed is tried again on the
/// next run. Returns how many were destroyed.
async fn end_channels(app: &App, now: i64) -> u64 {
    let due = match store::live_channels_due(&app.db, now - CHANNEL_GRACE_SECS).await {
        Ok(due) => due,
        Err(e) => {
            tracing::warn!(error = %e, "looking for live channels to end failed");
            return 0;
        }
    };
    let mut ended = 0;
    for id in due {
        // Over raffles refuse sockets anyway, so destroying first is safe,
        // and a failure leaves the raffle for the next run.
        if let Err(e) = app.live_updates.end(&id).await {
            tracing::warn!(raffle_id = id, error = %e, "ending a live channel failed");
            continue;
        }
        match store::live_ended(&app.db, &id, now).await {
            Ok(()) => ended += 1,
            Err(e) => {
                tracing::warn!(raffle_id = id, error = %e, "recording the end of live updates failed")
            }
        }
    }
    ended
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_after_two_hours_without_a_run() {
        let now = 100_000;
        // A run succeeded: two hours from it, whatever the raffles.
        assert!(!is_stale(Some(now - STALE_AFTER_SECS), None, now));
        assert!(is_stale(Some(now - STALE_AFTER_SECS - 1), Some(0), now));
        // Never ran: two hours from the first raffle.
        assert!(!is_stale(None, None, now), "nothing to erase yet");
        assert!(!is_stale(None, Some(now - STALE_AFTER_SECS), now));
        assert!(is_stale(None, Some(now - STALE_AFTER_SECS - 1), now));
    }
}
