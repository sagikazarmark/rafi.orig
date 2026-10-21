//! An organizer leaving, and the operator restoring a backup: the writes
//! that remove or re-remove personal data across raffles.

use rafi_store::{entrants, raffles, retention};

use crate::{App, Refusal, Result, ids};

/// Removes `owner`'s own data from Rafi: their defaults go; their raffles
/// that were never frozen are deleted; every other raffle loses its
/// entrants' personal data (logged as a purge) and the organizer's name and
/// contacts, and is purged. One guarded batch
/// ([`raffles::owner_leaves`]). The anonymous raffles stay verifiable; the
/// terms they accepted stay recorded. Refused while any of their raffles is
/// frozen or drawing: the event is live. The Clerk account is deleted in
/// Clerk.
pub async fn remove_owner_data(app: &App, owner: &str) -> Result<()> {
    let rows = raffles::rows_by_owner(&app.db, owner).await?;
    if rows.iter().any(|r| r.state.drawable()) {
        return Err(Refusal::EventLive.into());
    }
    let left = raffles::owner_leaves(&app.db, owner, ids::now()).await?;
    // Best effort: the channels find nothing to show either way.
    for id in left.deleted.iter().chain(&left.purged) {
        if let Err(e) = app.live_updates.end(id).await {
            tracing::warn!(raffle_id = id, error = %e, "ending the live channel of a left raffle failed");
        }
    }
    Ok(())
}

/// What [`reapply_erasures`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reapplied {
    /// Entrants erased again.
    pub entrants: u64,
    /// Raffles purged again.
    pub raffles: u64,
}

/// After a database restore: erases again every entrant the erasure log
/// names, purges again every raffle that was purged, and runs housekeeping
/// once, so whatever was due is erased again too. Idempotent.
pub async fn reapply_erasures(app: &App) -> Result<Reapplied> {
    let entrants = entrants::reapply_erasures(&app.db).await?;
    let raffles = retention::reapply_purges(&app.db).await?;
    crate::housekeeping::run(app).await?;
    Ok(Reapplied { entrants, raffles })
}
