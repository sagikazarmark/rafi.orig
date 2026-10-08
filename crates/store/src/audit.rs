//! The audit log: who did what to a raffle.

use crate::{Db, Result, now};

/// Records that `actor` did `action` (with `detail`) to the raffle.
pub async fn record(
    db: &Db,
    raffle_id: &str,
    actor: &str,
    action: &str,
    detail: &str,
) -> Result<()> {
    db.query("INSERT INTO audit (raffle_id, at, actor, action, detail) VALUES (?, ?, ?, ?, ?)")
        .bind(raffle_id)
        .bind(now())
        .bind(actor)
        .bind(action)
        .bind(detail)
        .run()
        .await
        .map(|_| ())
}
