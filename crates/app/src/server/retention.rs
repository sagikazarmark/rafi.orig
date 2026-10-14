//! The hourly retention run (natively a task; on Workers, the Cron Trigger).
//!
//! - **Erasure** (when `RAFI_RETENTION` is on): after a raffle finishes,
//!   non-winners are erased after `RAFI_RETENTION_DAYS` (30), winners once
//!   their prize is marked delivered or after `RAFI_RETENTION_WINNERS_DAYS`
//!   (90). The first run that sees a raffle finished dates its erasure with
//!   these settings ([`store::date_purges`]); the raffle engine, which stores
//!   the finish, knows none of them. Entrant IDs and the draw log stay, so the
//!   published draw remains verifiable. Each erasure is logged in `erasures`,
//!   without personal data.
//! - **Proof-of-work:** expired challenges are forgotten.
//! - **Live updates:** a raffle that finished more than a day ago and still
//!   has live updates gets them ended ([`Notifier::end`](rafi_live_server::Notifier::end);
//!   on Workers, its channel object is destroyed), whether or not erasure is on.
//!
//! The database side is [`rafi_store::retention`]: set-based writes, a few
//! statements per run whatever the number of entrants. A successful run
//! records when it happened ([`last_ok`]).

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use rafi_store::retention as store;

use super::{AppState, altcha, ids};

/// How long after finishing a raffle keeps its live channel, unless the
/// organizer ends live updates earlier.
pub const CHANNEL_GRACE_SECS: i64 = 86_400;

/// When the last run succeeded (unix seconds), if ever.
pub async fn last_ok(app: &AppState) -> anyhow::Result<Option<i64>> {
    Ok(store::last_ok(&app.db).await?)
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
    // No nudges for erasures: these raffles finished weeks ago, their channels are gone.
    let erased = if app.cfg.retention_enabled {
        store::date_purges(&app.db, app.purge_after()).await?;
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

/// Destroys the live channel of every raffle that finished more than
/// [`CHANNEL_GRACE_SECS`] ago and still has one, then records that live
/// updates ended. A channel that couldn't be destroyed is tried again on the
/// next run. Returns how many were destroyed.
async fn end_channels(app: &AppState, now: i64) -> u64 {
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
