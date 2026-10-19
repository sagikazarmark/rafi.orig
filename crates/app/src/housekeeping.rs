//! The housekeeping run: tidying up after raffles as they age. The deployment
//! schedules it hourly (natively a task; on Workers, the Cron Trigger).
//!
//! - **Proof-of-work:** expired challenges are forgotten.
//! - **Live updates:** a raffle that finished more than a day ago and still
//!   has live updates gets them ended ([`Notifier::end`](rafi_live_server::Notifier::end);
//!   on Workers, its channel object is destroyed).
//!
//! Personal data is erased by hand only ([`purge`](crate::purge)); there is
//! no automatic erasure (`docs/history/retention.md` records an earlier
//! design that had one).

use rafi_store::housekeeping as store;

use crate::{App, altcha, ids};

/// How long after finishing a raffle keeps its live channel, unless the
/// organizer ends live updates earlier.
pub const CHANNEL_GRACE_SECS: i64 = 86_400;

/// What a run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// Live channels destroyed.
    pub channels_ended: u64,
}

/// One housekeeping pass.
pub async fn run(app: &App) -> anyhow::Result<Report> {
    let now = ids::now();
    altcha::purge_expired(app).await?;
    let channels_ended = end_channels(app, now).await;
    Ok(Report { channels_ended })
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
