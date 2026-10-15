//! The retention run: what happens to raffles as they age. The deployment
//! schedules it hourly (natively a task; on Workers, the Cron Trigger).
//!
//! - **Erasure** (when [`Retention::enabled`]): after a raffle finishes,
//!   non-winners are erased after [`Retention::nonwinners_days`], winners
//!   once their prize is marked delivered or after
//!   [`Retention::winners_days`]. The first run that sees a raffle finished
//!   dates its erasure with these settings (`rafi_store::retention::date_purges`);
//!   the raffle engine, which stores the finish, knows none of them. Entrant
//!   IDs and the draw log stay, so the published draw remains verifiable.
//!   Each erasure is logged in `erasures`, without personal data. The
//!   organizer can also erase a finished raffle at once ([`purge`](crate::purge)).
//! - **Proof-of-work:** expired challenges are forgotten.
//! - **Live updates:** a raffle that finished more than a day ago and still
//!   has live updates gets them ended ([`Notifier::end`](rafi_live_server::Notifier::end);
//!   on Workers, its channel object is destroyed), whether or not erasure is on.
//!
//! The database side is [`rafi_store::retention`]: set-based writes, a few
//! statements per run whatever the number of entrants. A successful run
//! records when it happened.

use rafi_store::{PurgeAfter, retention as store};

use crate::{App, altcha, ids};

/// How long after finishing a raffle keeps its live channel, unless the
/// organizer ends live updates earlier.
pub const CHANNEL_GRACE_SECS: i64 = 86_400;

/// How long a retention run may be missing before erasure counts as stopped
/// ([`stale`]): two hourly runs.
pub const STALE_AFTER_SECS: i64 = 2 * 3600;

/// The deployment's retention settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    /// Erase personal data automatically.
    pub enabled: bool,
    /// Days after the finish that non-winners are erased.
    pub nonwinners_days: i64,
    /// Days after the finish that winners are erased at the latest.
    pub winners_days: i64,
}

impl Retention {
    fn purge_after(self) -> PurgeAfter {
        PurgeAfter::days(self.nonwinners_days, self.winners_days)
    }
}

/// What a run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// Entrants whose personal data was erased.
    pub erased: u64,
    /// Live channels destroyed.
    pub channels_ended: u64,
}

/// One retention pass.
pub async fn run(app: &App) -> anyhow::Result<Report> {
    let now = ids::now();
    let retention = app.policy.retention;
    // No nudges for erasures: these raffles finished weeks ago, their channels are gone.
    let erased = if retention.enabled {
        store::date_purges(&app.db, retention.purge_after()).await?;
        store::erase_due(&app.db, now).await?
    } else {
        0
    };
    altcha::purge_expired(app).await?;
    let channels_ended = end_channels(app, now).await;
    store::record_ok(&app.db, now).await?;
    Ok(Report {
        erased,
        channels_ended,
    })
}

/// Whether erasure may have stopped: it is on, but no run has succeeded in
/// the last [`STALE_AFTER_SECS`].
pub async fn stale(app: &App) -> anyhow::Result<bool> {
    if !app.policy.retention.enabled {
        return Ok(false);
    }
    let last_ok = store::last_ok(&app.db).await?;
    Ok(is_stale(last_ok, ids::now()))
}

fn is_stale(last_ok: Option<i64>, now: i64) -> bool {
    last_ok.is_none_or(|at| now - at > STALE_AFTER_SECS)
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
        let now = 1_000_000;
        assert!(!is_stale(Some(now - 3600), now), "ran an hour ago");
        assert!(
            is_stale(Some(now - 3 * 3600), now),
            "nothing for three hours"
        );
        assert!(is_stale(None, now), "never ran");
    }
}
