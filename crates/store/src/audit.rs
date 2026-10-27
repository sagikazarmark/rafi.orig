//! The audit log: who did what to a raffle.

use crate::{Db, Result, now};

/// Who acts: the owner key the raffle belongs to (a Clerk organization, or
/// else the user), and the user acting for it. For a personal account the two
/// are the same; in an organization the user tells the members apart.
/// Housekeeping is [`HOUSEKEEPING`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor<'a> {
    pub owner: &'a str,
    pub user: &'a str,
}

impl<'a> Actor<'a> {
    /// A personal account, or a test's owner: the user is the owner.
    pub const fn solo(owner: &'a str) -> Self {
        Self { owner, user: owner }
    }
}

/// The automatic erasure's actor (`housekeeping`), in both columns.
pub const HOUSEKEEPING: Actor<'static> = Actor::solo("housekeeping");

/// Records that `actor` did `action` (with `detail`) to the raffle.
pub async fn record(
    db: &Db,
    raffle_id: &str,
    actor: Actor<'_>,
    action: &str,
    detail: &str,
) -> Result<()> {
    db.query(
        "INSERT INTO audit (raffle_id, at, actor, user_id, action, detail) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(raffle_id)
    .bind(now())
    .bind(actor.owner)
    .bind(actor.user)
    .bind(action)
    .bind(detail)
    .run()
    .await
    .map(|_| ())
}
