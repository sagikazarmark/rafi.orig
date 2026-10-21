//! Legal documents the organizers accept (`legal_acceptances`): which version
//! of which document an owner accepted, through which of its users, and when.

use serde::Deserialize;

use crate::{Db, Result, now};

/// The documents organizers accept. The operator's terms carry the data
/// processing agreement.
pub const TERMS: &str = "terms";

/// One acceptance.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Acceptance {
    pub owner_id: String,
    pub user_id: String,
    pub document: String,
    pub version: String,
    pub accepted_at: i64,
}

/// Whether `owner` accepted `version` of `document`.
pub async fn accepted(db: &Db, owner: &str, document: &str, version: &str) -> Result<bool> {
    let found: Option<(i64,)> = db
        .query(
            "SELECT 1 FROM legal_acceptances WHERE owner_id = ? AND document = ? AND version = ?",
        )
        .bind(owner)
        .bind(document)
        .bind(version)
        .optional()
        .await?;
    Ok(found.is_some())
}

/// Records that `user`, for `owner`, accepted `version` of `document`. An
/// acceptance already recorded stays as it was.
pub async fn record(db: &Db, owner: &str, user: &str, document: &str, version: &str) -> Result<()> {
    db.query(
        "INSERT INTO legal_acceptances (owner_id, user_id, document, version, accepted_at)
         VALUES (?, ?, ?, ?, ?) ON CONFLICT (owner_id, document, version) DO NOTHING",
    )
    .bind(owner)
    .bind(user)
    .bind(document)
    .bind(version)
    .bind(now())
    .run()
    .await
    .map(|_| ())
}

/// Every acceptance of `owner`, oldest first.
pub async fn list(db: &Db, owner: &str) -> Result<Vec<Acceptance>> {
    db.query("SELECT * FROM legal_acceptances WHERE owner_id = ? ORDER BY accepted_at, version")
        .bind(owner)
        .all()
        .await
}
